//! Credit restricted to products pays only their charges: restricted grants are drawn first, the
//! fewest products first, then usage credit, then credit for everything; no charge is paid twice.
use crate::data::ids::{
    CUST_UBER_ID, PRODUCT_CAPACITY_ID, PRODUCT_PLATFORM_FEE_ID, PRODUCT_SEATS_ID, TENANT_ID,
};
use crate::helpers::credit::{Grant, assert_ledger_balances, fund_product_grant};
use crate::meteroid_it::container::MeteroidSetup;
use crate::meteroid_it::container::SeedLevel;
use crate::test_credit_promotional::{
    assert_reconciles, paid, quoted, remaining, taxed_draft, test_error,
};
use crate::test_customer_credit::{finalize, line};
use common_domain::ids::{CreditGrantId, ProductId};
use meteroid_store::domain::LineItem;
use meteroid_store::domain::credit::{CreditApplication, GrantClassification};
use meteroid_store::domain::credit_units::CreditUnitNew;
use meteroid_store::repositories::CreditUnitsInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// With the catalog's products, which credit can be restricted to.
async fn setup() -> MeteroidSetup {
    crate::helpers::init::logging();
    let database =
        crate::meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    crate::meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await
}

fn of(product: ProductId, mut line: LineItem) -> LineItem {
    line.product_id = Some(product);
    line
}

async fn restricted(
    setup: &MeteroidSetup,
    grant: Grant,
    classification: GrantClassification,
    products: &[ProductId],
) -> CreditGrantId {
    fund_product_grant(
        &setup.store,
        TENANT_ID,
        CUST_UBER_ID,
        grant,
        classification,
        products.to_vec(),
    )
    .await
}

#[tokio::test]
async fn overlapping_restrictions_never_pay_a_charge_twice() -> TestResult {
    let setup = setup().await;
    let wide = restricted(
        &setup,
        Grant::invoice(10_000),
        GrantClassification::Paid,
        &[PRODUCT_SEATS_ID, PRODUCT_PLATFORM_FEE_ID],
    )
    .await;
    let narrow = restricted(
        &setup,
        Grant::invoice(4_000),
        GrantClassification::Paid,
        &[PRODUCT_SEATS_ID],
    )
    .await;
    let invoice = taxed_draft(
        &setup,
        vec![
            of(PRODUCT_SEATS_ID, line("Seats", 6_000, false)),
            of(PRODUCT_PLATFORM_FEE_ID, line("Platform", 1_000, false)),
            of(PRODUCT_CAPACITY_ID, line("Capacity", 500, false)),
        ],
        false,
    )
    .await?;
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(finalized.applied_credits, 7_000);
    assert_eq!(finalized.amount_due, 500, "no credit pays for capacity");
    assert_eq!(
        remaining(&setup, narrow).await,
        0,
        "the narrower grant first"
    );
    assert_eq!(
        remaining(&setup, wide).await,
        7_000,
        "the wider grant pays only what is left of seats, and the platform fee"
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[tokio::test]
async fn restricted_then_usage_then_unrestricted_credit() -> TestResult {
    let setup = setup().await;
    let all = paid(
        &setup,
        Grant {
            priority: 1,
            ..Grant::invoice(10_000)
        },
    )
    .await;
    let usage = paid(
        &setup,
        Grant {
            application: CreditApplication::Usage,
            ..Grant::invoice(5_000)
        },
    )
    .await;
    let only_platform = restricted(
        &setup,
        Grant::invoice(300),
        GrantClassification::Paid,
        &[PRODUCT_PLATFORM_FEE_ID],
    )
    .await;
    let invoice = taxed_draft(
        &setup,
        vec![
            of(PRODUCT_SEATS_ID, line("Seats usage", 2_000, true)),
            of(PRODUCT_PLATFORM_FEE_ID, line("Platform usage", 1_000, true)),
            of(PRODUCT_CAPACITY_ID, line("Capacity", 3_000, false)),
        ],
        false,
    )
    .await?;
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(finalized.applied_credits, 6_000);
    assert_eq!(finalized.amount_due, 0);
    assert_eq!(remaining(&setup, only_platform).await, 0);
    assert_eq!(
        remaining(&setup, usage).await,
        2_300,
        "usage credit pays the usage the restricted grant left"
    );
    assert_eq!(remaining(&setup, all).await, 7_000, "and the rest");
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// On a tax-inclusive invoice a product's charge is read in the figures it was quoted, tax
/// included; promotional credit restricted to a product comes off that product before tax.
#[tokio::test]
async fn restricted_credit_reads_a_tax_inclusive_charge_gross() -> TestResult {
    let setup = setup().await;
    let seats_credit = restricted(
        &setup,
        Grant::invoice(13_000),
        GrantClassification::Paid,
        &[PRODUCT_SEATS_ID],
    )
    .await;
    let platform_promo = restricted(
        &setup,
        Grant::invoice(1_200),
        GrantClassification::Promotional,
        &[PRODUCT_PLATFORM_FEE_ID],
    )
    .await;
    let invoice = taxed_draft(
        &setup,
        vec![
            of(PRODUCT_SEATS_ID, quoted("Seats", 12_000)),
            of(PRODUCT_PLATFORM_FEE_ID, quoted("Platform", 6_000)),
        ],
        true,
    )
    .await?;
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(
        finalized.total, 16_800,
        "120.00 + 60.00 less 12.00 of promotion"
    );
    assert_eq!(
        finalized.tax_amount, 2_800,
        "20.00 + 8.00 carved out of 48.00"
    );
    assert_eq!(finalized.promotional_credits, 1_000);
    assert_eq!(finalized.applied_credits, 12_000, "the seats, tax included");
    assert_eq!(finalized.amount_due, 4_800);
    assert_reconciles(&finalized);
    assert_eq!(remaining(&setup, seats_credit).await, 1_000);
    assert_eq!(remaining(&setup, platform_promo).await, 0);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[tokio::test]
async fn credit_unit_grants_cannot_be_restricted_to_products() -> TestResult {
    let setup = setup().await;
    setup
        .store
        .create_credit_unit(
            CreditUnitNew::new(TENANT_ID, "TOKENS", "Token", "Tokens", None, 0)
                .map_err(test_error)?,
        )
        .await
        .map_err(test_error)?;
    let wallet = setup
        .store
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "TOKENS")
        .await
        .map_err(test_error)?;
    let refused = setup
        .store
        .fund_scoped_customer_credit_wallet(
            FundCustomerCreditWallet {
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id: wallet.id,
                amount_minor: 1_000,
                classification: GrantClassification::Promotional,
                priority: 100,
                expires_at: None,
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "Restricted tokens".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            meteroid_store::domain::credit::GrantScope::customer_wide()
                .with_products(vec![PRODUCT_SEATS_ID]),
        )
        .await
        .expect_err("credit units pay for every product");
    assert!(
        format!("{refused:?}").contains("cannot be restricted to products"),
        "{refused:?}"
    );
    Ok(())
}

/// A grant restricted to a product pays what that product's lines cost after their discount,
/// never the rest of the invoice.
#[tokio::test]
async fn restricted_credit_pays_its_discounted_lines_only() -> TestResult {
    let setup = setup().await;
    let seats_credit = restricted(
        &setup,
        Grant::invoice(10_000),
        GrantClassification::Paid,
        &[PRODUCT_SEATS_ID],
    )
    .await;
    let mut half_off = of(PRODUCT_SEATS_ID, line("Seats", 10_000, false));
    half_off.taxable_amount = 5_000;
    half_off.amount_total = 5_000;
    let invoice = taxed_draft(
        &setup,
        vec![
            half_off,
            of(PRODUCT_PLATFORM_FEE_ID, line("Platform", 4_000, false)),
        ],
        false,
    )
    .await?;
    assert_eq!(invoice.discount, 5_000);
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(finalized.total, 9_000);
    assert_eq!(
        finalized.applied_credits, 5_000,
        "the seats after their coupon"
    );
    assert_eq!(
        finalized.amount_due, 4_000,
        "the platform fee is not the seats'"
    );
    assert_eq!(remaining(&setup, seats_credit).await, 5_000);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}
