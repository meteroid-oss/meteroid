//! Tax follows what was paid: paid credit sold below its face is minted as paid credit worth its
//! cost plus promotional "Bonus credit" for the rest, which comes off before tax and is never
//! refunded in cash once used.
use crate::data::ids::{CUST_UBER_ID, TENANT_ID};
use crate::helpers::credit::{assert_ledger_balances, wallets};
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use crate::test_credit_promotional::{remaining, taxed, taxed_draft, test_error};
use crate::test_customer_credit::{draft, finalize, line, pay_amount};
use chrono::Utc;
use common_domain::actor::Actor;
use meteroid_store::domain::credit::{
    CreditGrant, CreditGrantSource, GrantClassification, LedgerEntryKind,
};
use meteroid_store::domain::price_components::CreditExpiry;
use meteroid_store::domain::{Invoice, LineItem, LineItemKind};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit_wallets::FundedCustomerCreditWallet;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

async fn setup() -> MeteroidSetup {
    crate::helpers::init::logging();
    let database =
        crate::meteroid_it::container::create_seeded_test_database(SeedLevel::PRODUCT).await;
    crate::meteroid_it::container::start_meteroid(database, SeedLevel::PRODUCT).await
}

/// Paid credit of `face` added by hand, stated to have cost `cost`.
async fn add_paid(
    setup: &MeteroidSetup,
    face: i64,
    cost: i64,
) -> meteroid_store::StoreResult<FundedCustomerCreditWallet> {
    crate::helpers::credit::add_paid(&setup.store, TENANT_ID, CUST_UBER_ID, face, cost).await
}

async fn grants(setup: &MeteroidSetup) -> Vec<CreditGrant> {
    wallets(&setup.store, TENANT_ID, CUST_UBER_ID)
        .await
        .into_iter()
        .flat_map(|wallet| wallet.grants)
        .collect()
}

/// The paid grant and the bonus credit sold with it.
fn paid_and_bonus(grants: &[CreditGrant]) -> (CreditGrant, CreditGrant) {
    let paid = grants
        .iter()
        .find(|grant| grant.classification == GrantClassification::Paid)
        .expect("a paid grant")
        .clone();
    let bonus = grants
        .iter()
        .find(|grant| grant.classification == GrantClassification::Promotional)
        .expect("a bonus grant")
        .clone();
    (paid, bonus)
}

fn assert_bonus_of(paid: &CreditGrant, bonus: &CreditGrant) {
    assert_eq!(bonus.reason, "Bonus credit");
    assert_eq!(bonus.cost.as_ref().map(|cost| cost.amount_minor), Some(0));
    assert_eq!(bonus.source, paid.source);
    assert_eq!(bonus.expires_at, paid.expires_at);
    assert_eq!(bonus.effective_at, paid.effective_at);
    assert_eq!(bonus.priority, paid.priority);
    assert_eq!(bonus.invoice_id, paid.invoice_id);
    assert_ne!(bonus.source_id, paid.source_id);
}

/// $500 paid for $550 at 20% VAT, then $466.67 of charges: the $50 bonus comes off before tax,
/// $416.67 is taxed $83.33, and the $500.00 of paid credit pays the taxed total.
#[tokio::test]
async fn paid_credit_bought_below_face_is_taxed_on_what_was_paid() -> TestResult {
    let setup = setup().await;
    let funded = add_paid(&setup, 55_000, 50_000).await.map_err(test_error)?;
    let (paid, bonus) = paid_and_bonus(&grants(&setup).await);
    assert_eq!(funded.grant_id, paid.id);
    assert_eq!(funded.bonus_grant_id, Some(bonus.id));
    assert_eq!((paid.amount_minor, bonus.amount_minor), (50_000, 5_000));
    assert_eq!(
        paid.cost.as_ref().map(|cost| cost.amount_minor),
        Some(50_000)
    );
    assert_bonus_of(&paid, &bonus);

    let invoice = taxed_draft(&setup, vec![taxed("Inference", 46_667)], false).await?;
    let invoice = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(invoice.promotional_credits, 5_000, "the bonus, before tax");
    assert_eq!(invoice.tax_amount, 8_333, "20% of 41,667");
    assert_eq!(invoice.total, 50_000);
    assert_eq!(invoice.applied_credits, 50_000, "paid credit, after tax");
    assert_eq!(invoice.amount_due, 0);
    assert_eq!(remaining(&setup, paid.id).await, 0);
    assert_eq!(remaining(&setup, bonus.id).await, 0);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// Paid credit that cost nothing is promotional; paid credit dearer than its face is refused.
#[tokio::test]
async fn free_paid_credit_is_promotional_and_dearer_credit_is_refused() -> TestResult {
    let setup = setup().await;
    let free = add_paid(&setup, 1_000, 0).await.map_err(test_error)?;
    assert_eq!(free.bonus_grant_id, None);
    let granted = grants(&setup).await;
    assert_eq!(granted.len(), 1);
    assert_eq!(granted[0].id, free.grant_id);
    assert_eq!(granted[0].classification, GrantClassification::Promotional);
    assert_eq!(granted[0].amount_minor, 1_000);
    assert_eq!(
        granted[0].cost.as_ref().map(|cost| cost.amount_minor),
        Some(0)
    );

    let refused = add_paid(&setup, 1_000, 1_200)
        .await
        .expect_err("credit dearer than its face");
    assert!(
        matches!(refused.current_context(), StoreError::InvalidArgument(_)),
        "{refused:?}"
    );
    assert_eq!(grants(&setup).await.len(), 1);
    Ok(())
}

/// A pack selling `credit` for a line of `subtotal`, discounted to `net`, kept 12 months.
fn pack(credit: i64, subtotal: i64, net: i64) -> LineItem {
    LineItem {
        kind: LineItemKind::Credit,
        credit_grant_amount: Some(credit),
        credit_grant_expiry: Some(CreditExpiry::Month(12)),
        amount_subtotal: subtotal,
        taxable_amount: net,
        amount_total: net,
        ..line("Scale credit pack", subtotal, false)
    }
}

async fn bought(setup: &MeteroidSetup, pack: LineItem) -> TestResult<Invoice> {
    let invoice = draft(setup, vec![pack]).await?;
    let invoice = finalize(setup, invoice.id).await?.invoice;
    pay_amount(setup, invoice.id, invoice.amount_due).await?;
    Ok(invoice)
}

/// A $550 pack sold 90% off, for $50: $50.00 of paid credit and $500.00 of bonus credit.
#[tokio::test]
async fn a_pack_sold_ninety_percent_off_is_fifty_paid_and_five_hundred_bonus() -> TestResult {
    let setup = setup().await;
    let invoice = bought(&setup, pack(55_000, 50_000, 5_000)).await?;
    let granted = grants(&setup).await;
    assert_eq!(granted.len(), 2);
    let (paid, bonus) = paid_and_bonus(&granted);
    assert_eq!(paid.source, CreditGrantSource::Purchase);
    assert_eq!(paid.invoice_id, Some(invoice.id));
    assert_eq!((paid.amount_minor, bonus.amount_minor), (5_000, 50_000));
    assert_eq!(
        paid.cost.as_ref().map(|cost| cost.amount_minor),
        Some(5_000)
    );
    assert_bonus_of(&paid, &bonus);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

async fn refund(setup: &MeteroidSetup, invoice: &Invoice, amount_cents: i64) -> TestResult<i64> {
    let note = setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: invoice.line_items[0].local_id.clone(),
                    amount_cents,
                }],
                reason: Some("Refund".into()),
                memo: None,
                credit_type: CreditType::Refund,
                custom_properties: None,
            },
        )
        .await
        .map_err(test_error)?;
    Ok(note.refunded_amount_cents)
}

async fn reversed(setup: &MeteroidSetup, grant: &CreditGrant) -> TestResult<Vec<i64>> {
    let (entries, _) = setup
        .store
        .customer_credit_ledger(TENANT_ID, grant.wallet_id, 1_000, 0)
        .await
        .map_err(test_error)?;
    let mut amounts = entries
        .iter()
        .filter(|entry| entry.grant_id == grant.id && entry.kind == LedgerEntryKind::Reversal)
        .map(|entry| (entry.sequence, entry.amount_minor))
        .collect::<Vec<_>>();
    amounts.sort();
    Ok(amounts.into_iter().map(|(_, amount)| amount).collect())
}

/// $500 for $550, unused: refunding half takes back $250 of paid and $25 of bonus credit, and
/// the other half the rest of both.
#[tokio::test]
async fn refunds_of_an_unused_discounted_pack_claw_back_both_grants() -> TestResult {
    let setup = setup().await;
    let invoice = bought(&setup, pack(55_000, 50_000, 50_000)).await?;
    let (paid, bonus) = paid_and_bonus(&grants(&setup).await);
    assert_eq!((paid.amount_minor, bonus.amount_minor), (50_000, 5_000));

    assert_eq!(refund(&setup, &invoice, 25_000).await?, 25_000);
    assert_eq!(remaining(&setup, paid.id).await, 25_000);
    assert_eq!(remaining(&setup, bonus.id).await, 2_500);
    assert_eq!(refund(&setup, &invoice, 25_000).await?, 25_000);
    assert_eq!(remaining(&setup, paid.id).await, 0);
    assert_eq!(remaining(&setup, bonus.id).await, 0);
    assert_eq!(reversed(&setup, &paid).await?, vec![-25_000, -25_000]);
    assert_eq!(reversed(&setup, &bonus).await?, vec![-2_500, -2_500]);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// $500 for $550, then $100 used: the bonus pays first, so $50 of each is spent. A full refund
/// returns the $450 of paid credit left in cash; the used bonus is neither refunded nor owed.
#[tokio::test]
async fn a_refund_after_use_returns_the_unspent_paid_credit_only() -> TestResult {
    let setup = setup().await;
    let invoice = bought(&setup, pack(55_000, 50_000, 50_000)).await?;
    let (paid, bonus) = paid_and_bonus(&grants(&setup).await);

    let usage = draft(&setup, vec![line("Seats", 10_000, false)]).await?;
    let usage = finalize(&setup, usage.id).await?.invoice;
    assert_eq!(usage.promotional_credits, 5_000, "the bonus first");
    assert_eq!(usage.applied_credits, 5_000);
    assert_eq!(remaining(&setup, paid.id).await, 45_000);
    assert_eq!(remaining(&setup, bonus.id).await, 0);

    assert_eq!(
        refund(&setup, &invoice, 50_000).await?,
        45_000,
        "clamped to the paid credit left"
    );
    assert_eq!(remaining(&setup, paid.id).await, 0);
    assert_eq!(reversed(&setup, &paid).await?, vec![-45_000]);
    assert!(reversed(&setup, &bonus).await?.is_empty());
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// $500 for $550, then $100 used ($50 of bonus, $50 of paid credit), then the whole $500 refunded
/// in the provider's dashboard: the $450 of paid credit left goes back, the $50 of paid credit
/// spent is the reported loss, and the spent bonus, never paid for, is no loss at all.
#[tokio::test]
async fn a_provider_refund_of_a_discounted_pack_reports_only_the_spent_paid_credit() -> TestResult {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use meteroid_store::domain::payment_transactions::ObservedRefundStatus;
    use meteroid_store::repositories::payment_transactions::{
        ObservedRefund, ObservedRefunds, PaymentTransactionInterface,
    };
    use scoped_futures::ScopedFutureExt;
    let setup = setup().await;
    let invoice = bought(&setup, pack(55_000, 50_000, 50_000)).await?;
    let (paid, bonus) = paid_and_bonus(&grants(&setup).await);
    let usage = draft(&setup, vec![line("Seats", 10_000, false)]).await?;
    finalize(&setup, usage.id).await?;
    assert_eq!(remaining(&setup, paid.id).await, 45_000);
    assert_eq!(remaining(&setup, bonus.id).await, 0);

    let payment = setup
        .store
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice.id)
        .await
        .map_err(test_error)?
        .into_iter()
        .map(|tx| tx.transaction)
        .find(|tx| tx.amount == 50_000)
        .ok_or("the pack's payment")?;
    let store = setup.store.clone();
    setup
        .store
        .transaction(|conn| {
            async move {
                let parent = store
                    .get_payment_tx_by_id_for_update(conn, payment.id, TENANT_ID)
                    .await?;
                store
                    .record_observed_refunds_tx(
                        conn,
                        &Actor::System,
                        parent,
                        ObservedRefunds {
                            external_transaction_id: "ch_pack".to_owned(),
                            cumulative_refunded_minor: 50_000,
                            refund_ids: vec![ObservedRefund {
                                external_refund_id: "re_pack".to_owned(),
                                amount_minor: None,
                                status: ObservedRefundStatus::Succeeded,
                            }],
                            observed_at: Utc::now().naive_utc(),
                        },
                    )
                    .await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
        .map_err(test_error)?;

    assert_eq!(remaining(&setup, paid.id).await, 0);
    assert_eq!(reversed(&setup, &paid).await?, vec![-45_000]);
    assert!(reversed(&setup, &bonus).await?.is_empty());
    use diesel_models::schema::customer_credit_unrecovered::dsl as u;
    let reported: Vec<i64> = u::customer_credit_unrecovered
        .filter(u::tenant_id.eq(TENANT_ID))
        .filter(u::invoice_id.eq(invoice.id))
        .select(u::reported)
        .load(&mut setup.store.get_conn().await.map_err(test_error)?)
        .await
        .map_err(test_error)?;
    assert_eq!(
        reported,
        vec![5_000],
        "the spent paid credit, not the bonus"
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}
