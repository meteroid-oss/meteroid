use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDate;
use meteroid_store::Services;
use meteroid_store::domain::checkout_sessions::{
    CheckoutCompletionResult, CheckoutSession, CheckoutType,
};
use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::{
    BillingPeriodEnum, CreateCheckoutSession, CreateSubscriptionAddOn, CreateSubscriptionAddOns,
    SubscriptionAddOnCustomization, enums::FeeTypeEnum,
};
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use meteroid_store::store::{PgConfig, PgPoolConfig, diesel_make_pg_pool};
use rstest::rstest;

use crate::data::ids::{
    CUST_COMODO_ID, CUST_SPOTIFY_ID, CUST_UBER_ID, CUST_UBER_PAYMENT_METHOD_ID,
    PLAN_VERSION_1_LEETCODE_ID, PRODUCT_FAMILY_ID, TENANT_ID,
};
use crate::harness::{TestEnv, subscription, test_env};

/// Services whose pool has one connection: a path holding a connection while it waits for
/// another starves on it, the pool deadlock a loaded backend falls into.
fn on_a_pool_of_one(env: &TestEnv) -> Services {
    on_a_pool_of(env, 1)
}

fn on_a_pool_of(env: &TestEnv, max_size: usize) -> Services {
    let mut store = env.store().clone();
    store.pool = diesel_make_pg_pool(PgConfig {
        database_url: env.setup.config.pg.database_url.clone(),
        pool: PgPoolConfig {
            max_size,
            wait_timeout_secs: 3,
            ..PgPoolConfig::default()
        },
    })
    .expect("a pool of one");
    Services::new(Arc::new(store), env.services().usage_clients())
}

async fn within<T>(what: &str, work: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(30), work)
        .await
        .unwrap_or_else(|_| panic!("{what} hung on a pool of one"))
}

#[rstest]
#[tokio::test]
async fn test_on_checkout_creation_runs_on_a_pool_of_one(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;
    let services = on_a_pool_of_one(&env);

    within(
        "creating an On-Checkout subscription",
        subscription()
            .plan_version(PLAN_VERSION_1_LEETCODE_ID)
            .on_checkout()
            .no_trial()
            .auto_charge()
            .create(&services),
    )
    .await;
}

#[rstest]
#[tokio::test]
async fn test_on_start_creation_billed_at_once_runs_on_a_pool_of_one(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;
    let services = on_a_pool_of_one(&env);

    within(
        "creating an On-Start subscription",
        subscription()
            .plan_version(PLAN_VERSION_1_LEETCODE_ID)
            .on_start()
            .no_trial()
            .auto_charge()
            .create(&services),
    )
    .await;
}

#[rstest]
#[tokio::test]
async fn test_checkout_payment_resolution_runs_on_a_pool_of_one(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;
    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;
    let details = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("subscription details");
    let services = on_a_pool_of_one(&env);

    let resolved = within(
        "resolving the checkout's payment methods",
        services.resolve_subscription_payment_methods(
            TENANT_ID,
            details.subscription.payment_methods_config.as_ref(),
            &details.customer,
        ),
    )
    .await
    .expect("payment methods resolve");
    assert!(
        resolved.card_connection_id.is_some(),
        "the customer's provider connection is created at checkout"
    );
}

/// More creations at once than the pool has connections, each for its own customer: none may
/// wait on the pool while holding a connection, or they all stall until the pool's wait timeout.
#[rstest]
#[tokio::test]
async fn test_concurrent_creations_outnumbering_the_pool_all_complete(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;
    let services = on_a_pool_of(&env, 2);

    let creations = [CUST_SPOTIFY_ID, CUST_UBER_ID, CUST_COMODO_ID].map(|customer| {
        let services = services.clone();
        tokio::spawn(async move {
            subscription()
                .customer(customer)
                .plan_version(PLAN_VERSION_1_LEETCODE_ID)
                .on_start()
                .no_trial()
                .auto_charge()
                .create(&services)
                .await
        })
    });
    within("concurrent creations", futures::future::join_all(creations))
        .await
        .into_iter()
        .for_each(|created| {
            created.expect("every creation completes");
        });
}

/// A self-serve session with a coupon and an add-on, so the preview prices each of them.
async fn self_serve_session(env: &TestEnv) -> CheckoutSession {
    let coupon_id = env.create_percentage_coupon("POOLOFONE", 10).await;
    let add_on_id = env
        .store()
        .create_add_on_from_ref(
            "Priority support".to_string(),
            ProductRef::New {
                name: "Priority support".to_string(),
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: Pricing::Rate {
                    rate: rust_decimal::Decimal::new(1_000, 2),
                },
                credit_unit: None,
            }),
            None,
            true,
            Some(10),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("add-on")
        .id;
    env.store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()),
            billing_day_anchor: Some(15),
            net_terms: None,
            trial_duration_days: Some(0),
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            payment_methods_config: None,
            components: None,
            add_ons: Some(CreateSubscriptionAddOns {
                add_ons: vec![CreateSubscriptionAddOn {
                    add_on_id,
                    customization: SubscriptionAddOnCustomization::None,
                    quantity: 1,
                }],
            }),
            coupon_code: None,
            coupon_ids: vec![coupon_id],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: CheckoutType::SelfServe,
            subscription_id: None,
            change_date: None,
            success_url: None,
            cancel_url: None,
        })
        .await
        .expect("checkout session")
}

#[rstest]
#[tokio::test]
async fn test_self_serve_checkout_preview_runs_on_a_pool_of_one(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let session = self_serve_session(&env).await;
    let services = on_a_pool_of_one(&env);

    let details = within(
        "previewing a self-serve checkout",
        services.build_preview_subscription_details(&session, TENANT_ID, None),
    )
    .await
    .expect("preview");
    assert_eq!(details.add_ons.len(), 1, "the add-on is previewed");
    assert_eq!(details.applied_coupons.len(), 1, "the coupon is previewed");
}

#[rstest]
#[tokio::test]
async fn test_self_serve_checkout_completion_runs_on_a_pool_of_one(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let session = self_serve_session(&env).await;
    let services = on_a_pool_of_one(&env);

    // 35.00 plan + 10.00 add-on, less 10%
    let completed = within(
        "completing a self-serve checkout",
        services.complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            4_050,
            "EUR".to_string(),
            None,
        ),
    )
    .await
    .expect("checkout completes");
    assert!(
        matches!(completed, CheckoutCompletionResult::Completed { .. }),
        "the mock provider settles at once"
    );
}
