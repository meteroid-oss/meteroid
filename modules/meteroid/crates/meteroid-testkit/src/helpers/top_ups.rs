//! Wallets and top-ups of the seeded customers, charged on the mock payment provider.
use std::sync::Arc;

use common_domain::ids::{BaseId, CreditGrantId, CustomerId, WalletId};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use diesel_models::customer_credit::CustomerCreditOperationRow;
use meteroid::workers::pgmq::processors::run_once_invoice_orchestration;
use meteroid_store::adapters::payment::reset_mock_provider_invocations;
use meteroid_store::domain::credit::{
    AutoTopUpChange, AutomaticTopUpConfig, AutomaticTopUpMethod, ConfigureCreditWallet,
};
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit_wallets::DebitCustomerCreditGrant;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use uuid::Uuid;

use crate::data::ids::{CUST_UBER_ID, TENANT_ID, USER_ID};
use crate::data::payment::{run_customer_payment_methods_seed, run_mock_payment_provider_seed};
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn test_error(error: impl std::fmt::Debug) -> Box<dyn std::error::Error> {
    format!("{error:?}").into()
}

/// Products, the mock payment provider and the customers' payment methods.
pub async fn setup() -> MeteroidSetup {
    crate::helpers::init::logging();
    let database = crate::meteroid_it::container::create_test_database().await;
    let setup = crate::meteroid_it::container::start_meteroid(database, SeedLevel::PRODUCT).await;
    run_mock_payment_provider_seed(&setup.store.pool, false).await;
    run_customer_payment_methods_seed(&setup.store.pool).await;
    reset_mock_provider_invocations();
    setup
}

pub async fn wallet(setup: &MeteroidSetup, customer: CustomerId) -> TestResult<WalletId> {
    let currency = setup
        .store
        .find_customer_by_id(customer, TENANT_ID)
        .await
        .map_err(test_error)?
        .currency;
    Ok(setup
        .store
        .create_customer_credit_wallet(TENANT_ID, customer, &currency)
        .await
        .map_err(test_error)?
        .id)
}

/// Runs the invoice orchestration until it has nothing left, as its worker would.
pub async fn drain(setup: &MeteroidSetup) {
    for _ in 0..4 {
        run_once_invoice_orchestration(
            Arc::new(setup.store.clone()),
            Arc::new(setup.services.clone()),
        )
        .await;
    }
}

pub async fn operations(
    setup: &MeteroidSetup,
    customer: CustomerId,
) -> TestResult<Vec<CustomerCreditOperationRow>> {
    use diesel_models::schema::customer_credit_operation::dsl as o;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    o::customer_credit_operation
        .filter(o::tenant_id.eq(TENANT_ID))
        .filter(o::customer_id.eq(customer))
        .order(o::created_at.asc())
        .select(CustomerCreditOperationRow::as_select())
        .load(&mut conn)
        .await
        .map_err(test_error)
}

pub async fn debit(setup: &MeteroidSetup, grant: CreditGrantId, amount_minor: i64) -> TestResult {
    setup
        .store
        .debit_customer_credit_grant(DebitCustomerCreditGrant {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            grant_id: grant,
            amount_minor: Some(amount_minor),
            idempotency_key: Uuid::now_v7(),
            reason: "Top-up test spend".to_owned(),
            actor_id: None,
        })
        .await
        .map(|_| ())
        .map_err(test_error)
}

pub fn fixed_top_up(threshold: i64, amount: i64) -> AutomaticTopUpConfig {
    AutomaticTopUpConfig {
        daily_cap_minor: Some(50_000),
        ..AutomaticTopUpConfig::new(
            AutomaticTopUpMethod::Fixed,
            threshold,
            Some(amount),
            None,
            None,
        )
    }
}

pub async fn configure(
    setup: &MeteroidSetup,
    wallet: WalletId,
    top_up: Option<AutomaticTopUpConfig>,
    threshold: Option<i64>,
) -> TestResult {
    setup
        .services
        .configure_credit_wallet(ConfigureCreditWallet {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            wallet_id: wallet,
            auto_top_up: match top_up {
                Some(config) => AutoTopUpChange::Set(config),
                None => AutoTopUpChange::Remove,
            },
            low_balance_thresholds_minor: Some(threshold.into_iter().collect()),
            usage_at_zero: Some(meteroid_store::domain::credit_exhausted::UsageAtZero::FollowPlan),
            unit_price: meteroid_store::domain::credit::UnitPriceChange::Keep,
            actor_id: Some(USER_ID.as_uuid()),
            consent_attested_by: Some(common_domain::actor::Actor::User { id: USER_ID }),
        })
        .await
        .map(|_| ())
        .map_err(test_error)
}

/// One pass of core's purchase executor, then of the invoice orchestration.
pub async fn run_executor(setup: &MeteroidSetup) -> TestResult {
    setup
        .services
        .run_wallet_purchase_executor()
        .await
        .map_err(test_error)?;
    drain(setup).await;
    Ok(())
}
