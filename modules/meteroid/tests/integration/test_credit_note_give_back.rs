//! A credit note gives drawn credit back only to the grants that could pay the credited lines;
//! what none of them can take becomes new, unrestricted credit that never expires.
use crate::data::ids::{CUST_UBER_ID, PRODUCT_PLATFORM_FEE_ID, PRODUCT_SEATS_ID, TENANT_ID};
use crate::helpers::credit::{Grant, assert_ledger_balances, fund_product_grant, wallets};
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use crate::test_credit_promotional::{promotional, remaining, taxed_draft, test_error};
use crate::test_customer_credit::{draft, finalize, line, pay_amount};
use chrono::{Duration, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{CreditGrantId, ProductId};
use meteroid_store::domain::credit::{CreditGrant, CreditGrantSource, GrantClassification};
use meteroid_store::domain::{Invoice, LineItem, TaxDetail};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use rust_decimal::Decimal;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

async fn setup() -> MeteroidSetup {
    crate::helpers::init::logging();
    let database =
        crate::meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    crate::meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await
}

/// A line of `product` for `subtotal`, 10% off, taxed 10% on what the coupon leaves.
fn discounted(product: ProductId, name: &str, subtotal: i64) -> LineItem {
    let rate = Decimal::new(10, 2);
    let net = subtotal * 9 / 10;
    let tax = net / 10;
    LineItem {
        product_id: Some(product),
        taxable_amount: net,
        tax_rate: rate,
        tax_amount: tax,
        amount_total: net + tax,
        tax_details: vec![TaxDetail {
            tax_rate: rate,
            tax_name: "VAT".to_owned(),
            tax_amount: tax,
        }],
        ..line(name, subtotal, false)
    }
}

/// Inference (P1) $300 and Fine-tuning (P2) $100, 10% off, 10% tax: 29,700 + 9,900 = 39,600. A
/// paid grant restricted to P1 pays its 29,700 and the card the 9,900 of P2.
async fn scoped_and_cash(setup: &MeteroidSetup) -> TestResult<(Invoice, CreditGrantId)> {
    let p1_credit = fund_product_grant(
        &setup.store,
        TENANT_ID,
        CUST_UBER_ID,
        Grant::invoice(29_700),
        GrantClassification::Paid,
        vec![PRODUCT_SEATS_ID],
    )
    .await;
    let invoice = taxed_draft(
        setup,
        vec![
            discounted(PRODUCT_SEATS_ID, "Inference", 30_000),
            discounted(PRODUCT_PLATFORM_FEE_ID, "Fine-tuning", 10_000),
        ],
        false,
    )
    .await?;
    let invoice = finalize(setup, invoice.id).await?.invoice;
    assert_eq!(invoice.total, 39_600);
    assert_eq!(invoice.applied_credits, 29_700);
    assert_eq!(invoice.amount_due, 9_900);
    assert_eq!(remaining(setup, p1_credit).await, 0);
    pay_amount(setup, invoice.id, 9_900).await?;
    Ok((invoice, p1_credit))
}

async fn credit_to_balance(
    setup: &MeteroidSetup,
    invoice: &Invoice,
    name: &str,
    amount_cents: i64,
) -> TestResult {
    let local_id = invoice
        .line_items
        .iter()
        .find(|line| line.name == name)
        .ok_or("the credited line")?
        .local_id
        .clone();
    setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id,
                    amount_cents,
                }],
                reason: Some("Give-back test".into()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
                custom_properties: None,
            },
        )
        .await
        .map_err(test_error)?;
    Ok(())
}

async fn note_grants(setup: &MeteroidSetup) -> Vec<CreditGrant> {
    wallets(&setup.store, TENANT_ID, CUST_UBER_ID)
        .await
        .into_iter()
        .flat_map(|wallet| wallet.grants)
        .filter(|grant| grant.source == CreditGrantSource::CreditNote)
        .collect()
}

/// The note credits the cash-paid P2 line: the P1-only grant cannot pay it, so it gets nothing
/// back and the customer gets 9,900 of unrestricted, non-expiring paid credit.
#[tokio::test]
async fn a_note_on_a_cash_paid_line_leaves_the_grant_scoped_to_another_product() -> TestResult {
    let setup = setup().await;
    let (invoice, p1_credit) = scoped_and_cash(&setup).await?;

    credit_to_balance(&setup, &invoice, "Fine-tuning", 10_000).await?;
    assert_eq!(
        remaining(&setup, p1_credit).await,
        0,
        "P1 credit stays spent"
    );
    let funded = note_grants(&setup).await;
    assert_eq!(funded.len(), 1);
    assert_eq!(funded[0].amount_minor, 9_900);
    assert_eq!(funded[0].classification, GrantClassification::Paid);
    assert_eq!(funded[0].expires_at, None);
    assert_eq!(
        funded[0].scope,
        meteroid_store::domain::credit::GrantScope::customer_wide(),
        "a note's remainder is the customer's, for any charge"
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// The note credits the P1 line the restricted grant paid: its 29,700 goes back to that grant,
/// with its terms, and no new credit is funded.
#[tokio::test]
async fn a_note_on_a_line_paid_by_restricted_credit_gives_it_back_to_that_grant() -> TestResult {
    let setup = setup().await;
    let (invoice, p1_credit) = scoped_and_cash(&setup).await?;

    credit_to_balance(&setup, &invoice, "Inference", 30_000).await?;
    assert_eq!(remaining(&setup, p1_credit).await, 29_700);
    assert!(note_grants(&setup).await.is_empty());
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A line of 1,333 whose promotional share is 333, credited in two halves (666 and 667): 166,
/// then the 167 left, never one unit short.
#[tokio::test]
async fn two_half_notes_give_back_the_whole_promotional_share() -> TestResult {
    let setup = setup().await;
    let promo = promotional(&setup, Grant::invoice(333)).await;
    let invoice = draft(&setup, vec![line("Seats", 1_333, false)]).await?;
    let invoice = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(invoice.promotional_credits, 333);
    assert_eq!(remaining(&setup, promo).await, 0);

    let local_id = invoice.line_items[0].local_id.clone();
    let mut returned = Vec::new();
    for amount_cents in [666, 667] {
        let note = setup
            .store
            .create_and_finalize_credit_note(
                Actor::System,
                TENANT_ID,
                CreateCreditNoteParams {
                    invoice_id: invoice.id,
                    line_items: vec![CreditLineItem::Amount {
                        local_id: local_id.clone(),
                        amount_cents,
                    }],
                    reason: Some("Half".into()),
                    memo: None,
                    credit_type: CreditType::DebtCancellation,
                    custom_properties: None,
                },
            )
            .await
            .map_err(test_error)?;
        returned.push(note.line_items[0].promotional_credit);
    }
    assert_eq!(returned, vec![Some(-166), Some(-167)]);
    assert_eq!(remaining(&setup, promo).await, 333, "all 333 came back");
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// The same idempotency key with another expiry is a different request, not a replay.
#[tokio::test]
async fn replaying_a_funding_with_another_expiry_is_a_conflict() -> TestResult {
    let setup = setup().await;
    let currency = setup
        .store
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .map_err(test_error)?
        .currency;
    let wallet = setup
        .store
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, &currency)
        .await
        .map_err(test_error)?;
    let key = uuid::Uuid::now_v7();
    let in_a_month = Utc::now() + Duration::days(30);
    let command = |expires_at| FundCustomerCreditWallet {
        tenant_id: TENANT_ID,
        customer_id: CUST_UBER_ID,
        wallet_id: wallet.id,
        amount_minor: 1_000,
        classification: GrantClassification::Paid,
        priority: 100,
        expires_at: Some(expires_at),
        idempotency_key: key,
        reason: "Paid outside Meteroid".to_owned(),
        reference: None,
        actor_id: None,
        cost: None,
    };
    let first = setup
        .store
        .fund_customer_credit_wallet(command(in_a_month))
        .await
        .map_err(test_error)?;
    let replay = setup
        .store
        .fund_customer_credit_wallet(command(in_a_month))
        .await
        .map_err(test_error)?;
    assert!(replay.replayed);
    assert_eq!(replay.grant_id, first.grant_id);

    let refused = setup
        .store
        .fund_customer_credit_wallet(command(in_a_month + Duration::days(1)))
        .await
        .expect_err("another expiry under the same key");
    assert!(
        matches!(refused.current_context(), StoreError::Conflict(_)),
        "{refused:?}"
    );
    Ok(())
}
