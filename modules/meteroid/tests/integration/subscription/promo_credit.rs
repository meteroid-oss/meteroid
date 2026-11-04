//! Promotional credit: a plan's one-time ("once") credit, billed on a single invoice per product
//! lineage, and promo codes that add credit instead of discounting invoices.
use chrono::NaiveDate;
use rstest::rstest;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use common_domain::ids::{BaseId, CustomerId, ProductId};
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
use meteroid_store::domain::credit::{CreditGrant, CreditGrantSource, GrantClassification};
use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum};
use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::{Invoice, LineItemKind};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::coupons::CouponInterface;
use meteroid_store::repositories::coupons::CouponInterfaceAuto;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::price_components::PriceComponentInterface;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env};

fn once_credit(credit_amount: Decimal) -> Pricing {
    Pricing::Credit {
        unit_price: Decimal::ZERO,
        quantity: 1,
        credit_amount,
        expiry: None,
        credit_unit_id: None,
        rollover: None,
        once: true,
    }
}

async fn add_once_credit(
    env: &TestEnv,
    name: &str,
    product: ProductRef,
    credit_amount: Decimal,
    plan_version_id: common_domain::ids::PlanVersionId,
) -> ProductId {
    crate::helpers::plans::as_draft(
        env.store(),
        plan_version_id,
        env.store().create_price_component_from_ref(
            name.to_owned(),
            product,
            vec![PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_owned(),
                pricing: once_credit(credit_amount),
                credit_unit: None,
            })],
            plan_version_id,
            TENANT_ID,
        ),
    )
    .await
    .expect("a once credit component")
    .product_id
    .expect("a product-backed component")
}

fn new_credit_product(name: &str) -> ProductRef {
    ProductRef::New {
        name: name.to_owned(),
        fee_type: FeeTypeEnum::Credit,
        fee_structure: FeeStructure::Credit {},
    }
}

fn credit_lines(invoice: &Invoice) -> Vec<(String, Option<i64>)> {
    invoice
        .line_items
        .iter()
        .filter(|line| line.kind == LineItemKind::Credit)
        .map(|line| (line.name.clone(), line.credit_grant_amount))
        .collect()
}

async fn grants(
    env: &TestEnv,
    customer_id: CustomerId,
    source: CreditGrantSource,
) -> Vec<CreditGrant> {
    env.store()
        .customer_credit_wallets(TENANT_ID, customer_id)
        .await
        .expect("wallets")
        .into_iter()
        .flat_map(|wallet| wallet.grants)
        .filter(|grant| grant.source == source)
        .collect()
}

/// Starter carries a 20 EUR welcome credit, granted once; Pro carries the same product (one
/// lineage) and a 50 EUR welcome credit of its own. The first invoice grants Starter's credit, no
/// renewal grants it again, and moving to Pro grants only Pro's own product, on its first invoice.
#[rstest]
#[tokio::test]
async fn a_once_credit_is_billed_on_one_invoice_per_product_lineage(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let welcome = add_once_credit(
        &env,
        "Welcome credit",
        new_credit_product("Welcome credit"),
        dec!(20),
        PLAN_VERSION_STARTER_ID,
    )
    .await;
    add_once_credit(
        &env,
        "Welcome credit",
        ProductRef::Existing(welcome),
        dec!(20),
        PLAN_VERSION_PRO_ID,
    )
    .await;
    add_once_credit(
        &env,
        "Pro welcome credit",
        new_credit_product("Pro welcome credit"),
        dec!(50),
        PLAN_VERSION_PRO_ID,
    )
    .await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).has_total(3900);
    assert_eq!(
        credit_lines(&invoices[0]),
        vec![("Welcome credit".to_owned(), Some(2_000))],
        "the first invoice grants the welcome credit"
    );

    env.process_cycles().await;
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    assert!(
        credit_lines(&invoices[1]).is_empty(),
        "a renewal never grants it again"
    );

    env.services()
        .schedule_plan_change(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
        )
        .await
        .expect("plan change scheduled");
    env.process_cycles().await;
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    invoices.assert().invoice_at(2).has_total(12400);
    assert_eq!(
        credit_lines(&invoices[2]),
        vec![("Pro welcome credit".to_owned(), Some(5_000))],
        "the plan change grants only the product the subscription never had"
    );

    env.process_cycles().await;
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(4);
    assert!(credit_lines(&invoices[3]).is_empty());

    let mut included: Vec<(i64, GrantClassification)> =
        grants(&env, CUST_UBER_ID, CreditGrantSource::Included)
            .await
            .into_iter()
            .map(|grant| (grant.amount_minor, grant.classification))
            .collect();
    included.sort_by_key(|(amount, _)| *amount);
    assert_eq!(
        included,
        vec![
            (2_000, GrantClassification::Promotional),
            (5_000, GrantClassification::Promotional)
        ]
    );
}

/// A once credit never rolls over: there is no next period to carry it into.
#[rstest]
#[tokio::test]
async fn a_once_credit_cannot_roll_over(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let refused = crate::helpers::plans::as_draft(
        env.store(),
        PLAN_VERSION_STARTER_ID,
        env.store().create_price_component_from_ref(
            "Welcome credit".to_owned(),
            new_credit_product("Welcome credit"),
            vec![PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_owned(),
                pricing: Pricing::Credit {
                    unit_price: Decimal::ZERO,
                    quantity: 1,
                    credit_amount: dec!(20),
                    expiry: Some(meteroid_store::domain::price_components::CreditExpiry::Month(1)),
                    credit_unit_id: None,
                    rollover: Some(meteroid_store::domain::price_components::CreditRollover {
                        max_amount: None,
                        expiry: meteroid_store::domain::price_components::CreditExpiry::Month(1),
                    }),
                    once: true,
                },
                credit_unit: None,
            })],
            PLAN_VERSION_STARTER_ID,
            TENANT_ID,
        ),
    )
    .await
    .expect_err("rollover with once is refused");
    assert!(matches!(
        refused.current_context(),
        StoreError::InvalidArgument(_)
    ));
}

const NOT_REDEEMABLE: &str = "This code can't be redeemed on your account.";

fn credit_coupon(code: &str, currency: &str, redemption_limit: Option<i32>) -> CouponNew {
    CouponNew {
        code: code.to_owned(),
        description: "Welcome offer".to_owned(),
        tenant_id: TENANT_ID,
        discount: CouponDiscount::Credit {
            currency: currency.to_owned(),
            amount: dec!(25),
            expires_in_days: Some(30),
            scope: Default::default(),
        },
        expires_at: None,
        redemption_limit,
        recurring_value: None,
        reusable: false,
        plan_ids: vec![],
    }
}

fn refusal(result: Result<impl std::fmt::Debug, error_stack::Report<StoreError>>) -> String {
    match result.expect_err("refused").current_context() {
        StoreError::InvalidArgument(message) => message.clone(),
        other => panic!("unexpected refusal {other:?}"),
    }
}

/// A promo code mints one promotional grant per redemption and never discounts an invoice; it is
/// redeemed at most once per customer and within its redemption limit and currency.
#[rstest]
#[tokio::test]
async fn a_promo_code_adds_credit_once_and_never_discounts(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let coupon_id = env
        .create_coupon(credit_coupon("WELCOME25", "EUR", Some(2)))
        .await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;
    let invoices = env.get_invoices(sub_id).await;
    invoices
        .assert()
        .invoice_at(0)
        .has_subtotal(3900)
        .has_discount(0)
        .has_promotional_credits(2500)
        .has_total(1400)
        .has_coupons_count(0);

    let granted = grants(&env, CUST_UBER_ID, CreditGrantSource::Coupon).await;
    assert_eq!(granted.len(), 1, "one grant per redemption");
    let grant = &granted[0];
    assert_eq!(grant.amount_minor, 2_500);
    assert_eq!(grant.classification, GrantClassification::Promotional);
    assert_eq!(grant.reference.as_deref(), Some("WELCOME25"));
    assert_eq!(
        grant.expires_at.map(|at| at - grant.effective_at),
        Some(chrono::Duration::days(30))
    );

    let again = env
        .services()
        .redeem_coupon_credit_code(TENANT_ID, CUST_UBER_ID, "WELCOME25", None)
        .await;
    assert_eq!(refusal(again), "You've already redeemed this code.");
    assert_eq!(
        grants(&env, CUST_UBER_ID, CreditGrantSource::Coupon)
            .await
            .len(),
        1,
        "redeeming again adds nothing"
    );

    env.process_cycles().await;
    let invoices = env.get_invoices(sub_id).await;
    invoices
        .assert()
        .invoice_at(1)
        .has_discount(0)
        .has_promotional_credits(0)
        .has_total(3900);

    for customer_id in [CUST_SPOTIFY_ID, CUST_COMODO_ID] {
        subscription()
            .customer(customer_id)
            .plan_version(PLAN_VERSION_STARTER_ID)
            .on_start()
            .no_trial()
            .create(env.services())
            .await;
    }
    let redeemed = env
        .services()
        .redeem_coupon_credit_code(TENANT_ID, CUST_SPOTIFY_ID, " WELCOME25 ", None)
        .await
        .expect("redeemed from the portal");
    assert_eq!(
        (redeemed.amount_minor, redeemed.currency.as_str()),
        (2_500, "EUR")
    );
    assert_eq!(
        grants(&env, CUST_SPOTIFY_ID, CreditGrantSource::Coupon)
            .await
            .len(),
        1
    );

    let over_limit = env
        .services()
        .redeem_coupon_credit_code(TENANT_ID, CUST_COMODO_ID, "WELCOME25", None)
        .await;
    assert_eq!(refusal(over_limit), NOT_REDEEMABLE);
    assert!(
        grants(&env, CUST_COMODO_ID, CreditGrantSource::Coupon)
            .await
            .is_empty()
    );
    let coupon = env
        .store()
        .get_coupon_by_id(TENANT_ID, coupon_id)
        .await
        .expect("coupon");
    assert_eq!(coupon.redemption_count, 2);

    env.create_coupon(credit_coupon("DOLLARS", "USD", None))
        .await;
    let wrong_currency = env
        .services()
        .redeem_coupon_credit_code(TENANT_ID, CUST_COMODO_ID, "DOLLARS", None)
        .await;
    assert_eq!(refusal(wrong_currency), NOT_REDEEMABLE);

    let unknown = env
        .services()
        .redeem_coupon_credit_code(TENANT_ID, CUST_COMODO_ID, "NOPE", None)
        .await;
    assert_eq!(refusal(unknown), NOT_REDEEMABLE);
    assert!(
        grants(&env, CUST_COMODO_ID, CreditGrantSource::Coupon)
            .await
            .is_empty()
    );
}

/// A credit coupon has no recurring value, and never switches to an invoice discount.
#[rstest]
#[tokio::test]
async fn a_credit_coupon_is_validated_on_save(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let fractional = CouponNew {
        discount: CouponDiscount::Credit {
            currency: "EUR".to_owned(),
            amount: dec!(0.001),
            expires_in_days: None,
            scope: Default::default(),
        },
        ..credit_coupon("FRACTION", "EUR", None)
    };
    let recurring = CouponNew {
        recurring_value: Some(3),
        ..credit_coupon("RECURRING", "EUR", None)
    };
    for refused in [fractional, recurring] {
        let result = env
            .store()
            .create_coupon(common_domain::actor::Actor::System, refused)
            .await;
        assert!(matches!(
            result.expect_err("refused").current_context(),
            StoreError::InvalidArgument(_)
        ));
    }

    let coupon_id = env
        .create_coupon(credit_coupon("SWITCH", "EUR", None))
        .await;
    let switched = env
        .store()
        .update_coupon(
            common_domain::actor::Actor::System,
            meteroid_store::domain::coupons::CouponPatch {
                id: coupon_id,
                tenant_id: TENANT_ID,
                description: None,
                discount: Some(CouponDiscount::Percentage(dec!(10))),
                plan_ids: None,
            },
        )
        .await;
    assert!(matches!(
        switched.expect_err("refused").current_context(),
        StoreError::InvalidArgument(_)
    ));
}

async fn coupon_grants(env: &TestEnv, customer_id: CustomerId) -> Vec<CreditGrant> {
    grants(env, customer_id, CreditGrantSource::Coupon).await
}

async fn complete_checkout(
    env: &TestEnv,
    sub_id: common_domain::ids::SubscriptionId,
    coupon_code: Option<&str>,
) -> Result<meteroid_store::services::CheckoutPaymentOutcome, error_stack::Report<StoreError>> {
    let mut conn = env.conn().await;
    env.services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3500,
            "EUR".to_string(),
            coupon_code.map(str::to_owned),
        )
        .await
        .map(|(_, outcome)| outcome)
}

fn pending_checkout() -> crate::harness::SubscriptionBuilder {
    subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .on_checkout()
        .no_trial()
        .auto_charge()
}

/// A credit code entered at checkout neither discounts nor pays the checkout invoice: the amount
/// the customer confirmed is charged, and the credit is granted once payment activates it.
#[rstest]
#[tokio::test]
async fn a_promo_code_at_checkout_funds_credit_once_paid(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.create_coupon(credit_coupon("WELCOME25", "EUR", None))
        .await;
    let sub_id = pending_checkout().create(env.services()).await;

    let outcome = complete_checkout(&env, sub_id, Some("WELCOME25"))
        .await
        .expect("checkout completes at the confirmed amount");
    assert_eq!(
        outcome,
        meteroid_store::services::CheckoutPaymentOutcome::Settled
    );
    env.get_subscription(sub_id).await.assert().is_active();
    let invoices = env.get_invoices(sub_id).await;
    invoices
        .assert()
        .invoice_at(0)
        .has_total(3500)
        .has_discount(0);
    assert_eq!(
        invoices[0].applied_credits, 0,
        "the grant comes after payment"
    );

    let granted = coupon_grants(&env, CUST_UBER_ID).await;
    assert_eq!(granted.len(), 1, "exactly one grant on activation");
    assert_eq!(granted[0].amount_minor, 2_500);
    assert_eq!(granted[0].reason, "Promo code WELCOME25");
    env.process_cycles().await;
    assert_eq!(
        coupon_grants(&env, CUST_UBER_ID).await.len(),
        1,
        "a renewal funds nothing more"
    );
}

/// A checkout left waiting for 3DS never grants the credit.
#[rstest]
#[tokio::test]
async fn an_abandoned_3ds_checkout_grants_no_credit(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.set_mock_charge_behavior(
        meteroid_store::domain::connectors::MockChargeBehavior::RequiresAction,
    )
    .await;
    env.create_coupon(credit_coupon("WELCOME25", "EUR", None))
        .await;
    let sub_id = pending_checkout().create(env.services()).await;

    let outcome = complete_checkout(&env, sub_id, Some("WELCOME25"))
        .await
        .expect("awaiting the customer");
    assert_eq!(
        outcome,
        meteroid_store::services::CheckoutPaymentOutcome::AwaitingCustomerAction
    );
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_pending_activation();
    assert!(coupon_grants(&env, CUST_UBER_ID).await.is_empty());
}

/// A plan-restricted credit code is refused at checkout on another plan.
#[rstest]
#[tokio::test]
async fn a_plan_restricted_promo_code_is_refused_at_checkout_on_another_plan(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;
    env.create_coupon(CouponNew {
        plan_ids: vec![PLAN_STARTER_ID],
        ..credit_coupon("STARTERONLY", "EUR", None)
    })
    .await;
    let sub_id = pending_checkout().create(env.services()).await;

    let refused = complete_checkout(&env, sub_id, Some("STARTERONLY")).await;
    assert!(matches!(
        refused.expect_err("refused").current_context(),
        StoreError::InvalidArgument(_)
    ));
    assert!(coupon_grants(&env, CUST_UBER_ID).await.is_empty());
}

/// A subscription starting later gets its promo credit when it starts, not when it is created.
#[rstest]
#[tokio::test]
async fn a_future_subscription_gets_its_promo_credit_when_it_starts(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let coupon_id = env
        .create_coupon(credit_coupon("WELCOME25", "EUR", None))
        .await;
    let start = chrono::Utc::now().date_naive() + chrono::Duration::days(5);
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_status(diesel_models::enums::SubscriptionStatusEnum::PendingActivation);
    assert!(coupon_grants(&env, CUST_UBER_ID).await.is_empty());

    // The start date arrives.
    let mut conn = env.conn().await;
    for statement in [
        "UPDATE subscription SET start_date = CURRENT_DATE, billing_start_date = CURRENT_DATE,
             current_period_start = CURRENT_DATE, current_period_end = CURRENT_DATE
         WHERE id = $1",
        "UPDATE subscription_component SET effective_from = CURRENT_DATE WHERE subscription_id = $1",
    ] {
        diesel_async::RunQueryDsl::execute(
            diesel::sql_query(statement).bind::<diesel::sql_types::Uuid, _>(sub_id),
            &mut conn,
        )
        .await
        .expect("start date reached");
    }
    drop(conn);
    env.process_cycles().await;

    env.get_subscription(sub_id).await.assert().is_active();
    assert_eq!(coupon_grants(&env, CUST_UBER_ID).await.len(), 1);
}

/// Promotional credit is redeemed once per customer, even on another subscription, so a
/// credit coupon cannot be created reusable.
#[rstest]
#[tokio::test]
async fn a_promo_code_is_redeemed_once_per_customer(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let reusable = env
        .store()
        .create_coupon(
            common_domain::actor::Actor::System,
            CouponNew {
                reusable: true,
                ..credit_coupon("WELCOME25", "EUR", None)
            },
        )
        .await
        .expect_err("a reusable credit coupon is refused");
    assert!(matches!(
        reusable.current_context(),
        StoreError::InvalidArgument(_)
    ));
    let coupon_id = env
        .create_coupon(credit_coupon("WELCOME25", "EUR", None))
        .await;
    subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;
    assert_eq!(coupon_grants(&env, CUST_UBER_ID).await.len(), 1);

    let second = env
        .services()
        .insert_subscription(
            common_domain::actor::Actor::System,
            subscription()
                .plan_version(PLAN_VERSION_PRO_ID)
                .on_start()
                .no_trial()
                .coupon(coupon_id)
                .request(),
            TENANT_ID,
        )
        .await;
    assert!(second.is_err(), "a second subscription cannot redeem it");

    subscription()
        .plan_version(PLAN_VERSION_PRO_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let portal = env
        .services()
        .redeem_coupon_credit_code(TENANT_ID, CUST_UBER_ID, "WELCOME25", None)
        .await;
    assert_eq!(refusal(portal), "You've already redeemed this code.");
    assert_eq!(coupon_grants(&env, CUST_UBER_ID).await.len(), 1);
}

/// Two checkouts of one customer that settled with the same code each applied it, since payment
/// came first; only the first to start funds the credit.
#[rstest]
#[tokio::test]
async fn a_code_applied_by_two_settled_checkouts_funds_credit_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let coupon_id = env
        .create_coupon(credit_coupon("WELCOME25", "EUR", Some(1)))
        .await;
    let first = pending_checkout().create(env.services()).await;
    let second = pending_checkout().create(env.services()).await;
    // What settling an asynchronous checkout does: apply the code without validating it again.
    let mut conn = env.conn().await;
    for sub_id in [first, second] {
        diesel_async::RunQueryDsl::execute(
            diesel::sql_query(
                "INSERT INTO applied_coupon (id, coupon_id, customer_id, subscription_id, is_active)
                 VALUES ($1, $2, $3, $4, true)",
            )
            .bind::<diesel::sql_types::Uuid, _>(uuid::Uuid::now_v7())
            .bind::<diesel::sql_types::Uuid, _>(coupon_id)
            .bind::<diesel::sql_types::Uuid, _>(CUST_UBER_ID)
            .bind::<diesel::sql_types::Uuid, _>(sub_id),
            &mut conn,
        )
        .await
        .expect("code applied");
    }
    drop(conn);

    complete_checkout(&env, first, None)
        .await
        .expect("checkout completes");
    // The second checkout's first invoice is already paid in part by the first one's credit.
    let mut conn = env.conn().await;
    env.services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            second,
            CUST_UBER_PAYMENT_METHOD_ID,
            1000,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("checkout completes");
    drop(conn);
    for sub_id in [first, second] {
        env.get_subscription(sub_id).await.assert().is_active();
    }
    let granted = coupon_grants(&env, CUST_UBER_ID).await;
    assert_eq!(
        granted.len(),
        1,
        "promotional credit is funded once per customer"
    );
    assert_eq!(granted[0].amount_minor, 2_500);
}

/// Two customers' checkouts settling at once with the last redemption of a code, each applying it
/// without validation: both complete, without deadlock, and only one gets the credit.
#[rstest]
#[tokio::test]
async fn concurrent_settlements_respect_the_redemption_limit(#[future] test_env: TestEnv) {
    use scoped_futures::ScopedFutureExt;

    let env = test_env.await;
    env.seed_payments().await;
    let coupon_id = env
        .create_coupon(credit_coupon("LAST1", "EUR", Some(1)))
        .await;
    let checkouts = [
        (CUST_UBER_ID, CUST_UBER_PAYMENT_METHOD_ID),
        (CUST_SPOTIFY_ID, CUST_SPOTIFY_PAYMENT_METHOD_ID),
    ];
    let mut subscriptions = Vec::new();
    for (customer_id, _) in checkouts {
        subscriptions.push(
            pending_checkout()
                .customer(customer_id)
                .create(env.services())
                .await,
        );
    }
    // Both hold their applied coupon's key-share lock on the coupon before either updates it.
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let settle = |(customer_id, payment_method_id): (CustomerId, _), sub_id| {
        let barrier = barrier.clone();
        let services = env.services().clone();
        let store = env.store().clone();
        async move {
            let (services, barrier) = (&services, &barrier);
            store
                .transaction(|conn| {
                    async move {
                        diesel_async::RunQueryDsl::execute(
                            diesel::sql_query(
                                "INSERT INTO applied_coupon
                                   (id, coupon_id, customer_id, subscription_id, is_active)
                                 VALUES ($1, $2, $3, $4, true)",
                            )
                            .bind::<diesel::sql_types::Uuid, _>(uuid::Uuid::now_v7())
                            .bind::<diesel::sql_types::Uuid, _>(coupon_id)
                            .bind::<diesel::sql_types::Uuid, _>(customer_id)
                            .bind::<diesel::sql_types::Uuid, _>(sub_id),
                            conn,
                        )
                        .await
                        .expect("code applied");
                        barrier.wait().await;
                        diesel_async::RunQueryDsl::execute(
                            diesel::sql_query(
                                "UPDATE coupon SET redemption_count = redemption_count + 1 WHERE id = $1",
                            )
                            .bind::<diesel::sql_types::Uuid, _>(coupon_id),
                            conn,
                        )
                        .await
                        .expect("redemption recorded");
                        services
                            .complete_subscription_checkout_tx(
                                conn,
                                TENANT_ID,
                                sub_id,
                                payment_method_id,
                                3500,
                                "EUR".to_string(),
                                None,
                            )
                            .await
                    }
                    .scope_boxed()
                })
                .await
        }
    };
    let (uber, spotify) = tokio::join!(
        settle(checkouts[0], subscriptions[0]),
        settle(checkouts[1], subscriptions[1]),
    );
    uber.expect("the first settlement completes");
    spotify.expect("the second settlement completes");

    let funded = coupon_grants(&env, CUST_UBER_ID).await.len()
        + coupon_grants(&env, CUST_SPOTIFY_ID).await.len();
    assert_eq!(funded, 1, "one redemption left, one credit");
    let mut conn = env.conn().await;
    let inactive: i64 = diesel_async::RunQueryDsl::get_result(
        diesel::QueryDsl::filter(
            diesel::QueryDsl::count(diesel_models::schema::applied_coupon::table),
            diesel::ExpressionMethods::eq(diesel_models::schema::applied_coupon::is_active, false),
        ),
        &mut conn,
    )
    .await
    .expect("applied coupons");
    assert_eq!(
        inactive, 1,
        "the unfunded redemption is recorded as inactive"
    );
}

/// Two customers racing for the last redemption: exactly one gets the credit.
#[rstest]
#[tokio::test]
async fn concurrent_redemptions_respect_the_redemption_limit(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.create_coupon(credit_coupon("LAST1", "EUR", Some(1)))
        .await;
    for customer_id in [CUST_SPOTIFY_ID, CUST_COMODO_ID] {
        subscription()
            .customer(customer_id)
            .plan_version(PLAN_VERSION_STARTER_ID)
            .on_start()
            .no_trial()
            .create(env.services())
            .await;
    }
    let (spotify, comodo) = tokio::join!(
        env.services()
            .redeem_coupon_credit_code(TENANT_ID, CUST_SPOTIFY_ID, "LAST1", None),
        env.services()
            .redeem_coupon_credit_code(TENANT_ID, CUST_COMODO_ID, "LAST1", None),
    );
    assert_eq!(
        [spotify.is_ok(), comodo.is_ok()]
            .iter()
            .filter(|redeemed| **redeemed)
            .count(),
        1
    );
    let total = coupon_grants(&env, CUST_SPOTIFY_ID).await.len()
        + coupon_grants(&env, CUST_COMODO_ID).await.len();
    assert_eq!(total, 1);
}

/// A one-time pack added by an amendment on the billing start day, after the first invoice, is
/// billed on the next one.
#[rstest]
#[tokio::test]
async fn a_once_credit_added_on_the_first_day_is_billed_at_the_first_renewal(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    let env = test_env.await;
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![AddExtraComponent {
                        name: "Welcome credit".to_owned(),
                        product: new_credit_product("Welcome credit"),
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_owned(),
                            pricing: once_credit(dec!(20)),
                            credit_unit: None,
                        }),
                    }],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            start,
        )
        .await
        .expect("amendment applied");

    env.process_cycles().await;
    let invoices = env.get_invoices(sub_id).await;
    let renewal = invoices
        .iter()
        .find(|invoice| invoice.cycle_index == Some(1))
        .expect("the first renewal");
    assert_eq!(
        credit_lines(renewal),
        vec![("Welcome credit".to_owned(), Some(2_000))]
    );
    env.process_cycles().await;
    let billed: usize = env
        .get_invoices(sub_id)
        .await
        .iter()
        .map(|invoice| credit_lines(invoice).len())
        .sum();
    assert_eq!(billed, 1, "billed once");
}

/// A once credit merged into a consolidated invoice voided without taking its credit back still
/// counts as billed: the credit it granted stands, so it is never granted twice. (A void that
/// takes it back: `a_voided_welcome_credit_is_billed_again_only_when_the_void_took_it_back`.)
#[rstest]
#[tokio::test]
async fn a_voided_consolidated_invoice_counts_while_its_once_credit_stands(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let welcome = add_once_credit(
        &env,
        "Welcome credit",
        new_credit_product("Welcome credit"),
        dec!(20),
        PLAN_VERSION_STARTER_ID,
    )
    .await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let other_id = subscription()
        .customer(CUST_SPOTIFY_ID)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let invoice = env.get_invoices(sub_id).await.remove(0);
    let parent = env.get_invoices(other_id).await.remove(0);

    let mut conn = env.conn().await;
    async fn billed(
        conn: &mut diesel_models::PgConn,
        sub_id: common_domain::ids::SubscriptionId,
        product_id: ProductId,
    ) -> bool {
        diesel_models::invoices::InvoiceRow::subscription_billed_once_credit(
            conn, TENANT_ID, sub_id, product_id, None,
        )
        .await
        .expect("lookup")
    }
    assert!(billed(&mut conn, sub_id, welcome).await);

    diesel_async::RunQueryDsl::execute(
        diesel::sql_query(
            "UPDATE invoice SET status = 'DRAFT', consolidated_into_invoice_id = $2 WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(invoice.id)
        .bind::<diesel::sql_types::Uuid, _>(parent.id),
        &mut conn,
    )
    .await
    .expect("merged into the parent");
    assert!(
        billed(&mut conn, sub_id, welcome).await,
        "merged into a live invoice, it is billed"
    );

    diesel_async::RunQueryDsl::execute(
        diesel::sql_query("UPDATE invoice SET status = 'VOID' WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(parent.id),
        &mut conn,
    )
    .await
    .expect("parent voided");
    assert!(
        billed(&mut conn, sub_id, welcome).await,
        "its credit was never taken back"
    );
}

#[derive(Debug, Clone, Copy)]
enum StartPath {
    CreatedStarted,
    StartDateReached,
    ManualActivation,
    CheckoutPaid,
    CheckoutDuringFreeTrial,
    CheckoutAfterTrialExpired,
    CheckoutSettledAfter3ds,
    TrialExpiredInvoicePaid,
}

/// Every way a subscription starts funds the promo credit applied before it started, exactly
/// once, and nothing while it has not started.
#[rstest]
#[case::created_started(StartPath::CreatedStarted)]
#[case::start_date_reached(StartPath::StartDateReached)]
#[case::manual_activation(StartPath::ManualActivation)]
#[case::checkout_paid(StartPath::CheckoutPaid)]
#[case::checkout_during_free_trial(StartPath::CheckoutDuringFreeTrial)]
#[case::checkout_after_trial_expired(StartPath::CheckoutAfterTrialExpired)]
#[case::checkout_settled_after_3ds(StartPath::CheckoutSettledAfter3ds)]
#[case::trial_expired_invoice_paid(StartPath::TrialExpiredInvoicePaid)]
#[tokio::test]
async fn every_start_path_funds_promo_credit_once(
    #[future] test_env: TestEnv,
    #[case] path: StartPath,
) {
    let env = test_env.await;
    env.seed_payments().await;
    if matches!(path, StartPath::CheckoutSettledAfter3ds) {
        env.set_mock_charge_behavior(
            meteroid_store::domain::connectors::MockChargeBehavior::RequiresAction,
        )
        .await;
    }
    let coupon_id = env
        .create_coupon(credit_coupon("WELCOME25", "EUR", None))
        .await;
    let today = chrono::Utc::now().date_naive();
    let builder = match path {
        StartPath::CreatedStarted => subscription()
            .plan_version(PLAN_VERSION_STARTER_ID)
            .on_start()
            .no_trial(),
        StartPath::StartDateReached => subscription()
            .plan_version(PLAN_VERSION_STARTER_ID)
            .start_date(today + chrono::Duration::days(5))
            .on_start()
            .no_trial(),
        StartPath::ManualActivation => subscription()
            .plan_version(PLAN_VERSION_1_LEETCODE_ID)
            .manual()
            .no_trial(),
        StartPath::CheckoutPaid => pending_checkout(),
        StartPath::CheckoutDuringFreeTrial => subscription()
            .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
            .on_checkout()
            .trial_days(14)
            .auto_charge(),
        StartPath::CheckoutAfterTrialExpired => subscription()
            .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
            .on_checkout()
            .trial_days(14)
            .no_auto_charge(),
        StartPath::CheckoutSettledAfter3ds => pending_checkout(),
        StartPath::TrialExpiredInvoicePaid => subscription()
            .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
            .on_checkout()
            .trial_days(14)
            .auto_charge(),
    };
    let sub_id = builder.coupon(coupon_id).create(env.services()).await;

    if !matches!(path, StartPath::CreatedStarted) {
        assert!(
            coupon_grants(&env, CUST_UBER_ID).await.is_empty(),
            "{path:?}: nothing before the subscription starts"
        );
    }

    let checkout = |amount: u64| {
        let env = &env;
        async move {
            let mut conn = env.conn().await;
            env.services()
                .complete_subscription_checkout_tx(
                    &mut conn,
                    TENANT_ID,
                    sub_id,
                    CUST_UBER_PAYMENT_METHOD_ID,
                    amount,
                    "EUR".to_string(),
                    None,
                )
                .await
                .expect("checkout completes");
        }
    };
    match path {
        StartPath::CreatedStarted => {}
        StartPath::StartDateReached => {
            let mut conn = env.conn().await;
            for statement in [
                "UPDATE subscription SET start_date = CURRENT_DATE, billing_start_date = CURRENT_DATE,
                     current_period_start = CURRENT_DATE, current_period_end = CURRENT_DATE
                 WHERE id = $1",
                "UPDATE subscription_component SET effective_from = CURRENT_DATE
                 WHERE subscription_id = $1",
            ] {
                diesel_async::RunQueryDsl::execute(
                    diesel::sql_query(statement).bind::<diesel::sql_types::Uuid, _>(sub_id),
                    &mut conn,
                )
                .await
                .expect("start date reached");
            }
            drop(conn);
            env.process_cycles().await;
        }
        StartPath::ManualActivation => {
            env.services()
                .activate_subscription_manual(TENANT_ID, sub_id)
                .await
                .expect("activated");
        }
        StartPath::CheckoutPaid => checkout(3500).await,
        StartPath::CheckoutDuringFreeTrial => checkout(0).await,
        StartPath::CheckoutAfterTrialExpired => {
            env.process_cycles().await;
            assert!(
                coupon_grants(&env, CUST_UBER_ID).await.is_empty(),
                "an expired trial has not started the subscription"
            );
            checkout(4900).await;
        }
        StartPath::CheckoutSettledAfter3ds => {
            checkout(3500).await;
            env.get_subscription(sub_id)
                .await
                .assert()
                .has_status(diesel_models::enums::SubscriptionStatusEnum::PendingActivation);
            assert!(
                coupon_grants(&env, CUST_UBER_ID).await.is_empty(),
                "nothing while 3DS is pending"
            );
            let invoice_id = env.get_invoices(sub_id).await[0].id;
            let pending = env
                .store()
                .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
                .await
                .expect("transactions")
                .into_iter()
                .map(|detailed| detailed.transaction)
                .find(|tx| {
                    tx.payment_type == meteroid_store::domain::enums::PaymentTypeEnum::Payment
                })
                .expect("the pending charge");
            env.deliver_mock_webhook(
                format!(
                    r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"{ext}","amount":3500,"currency":"EUR"}}"#,
                    tx = pending.id.as_base62(),
                    ext = pending.provider_transaction_id.clone().unwrap_or_default(),
                )
                .as_bytes(),
            )
            .await;
            env.get_subscription(sub_id).await.assert().is_active();
        }
        StartPath::TrialExpiredInvoicePaid => {
            // The trial checkout starts it; the paid trial-end invoice then moves it from
            // TrialExpired to Active through the same start transition, funding nothing twice.
            checkout(0).await;
            env.process_cycles().await;
            env.get_subscription(sub_id)
                .await
                .assert()
                .is_trial_expired();
            env.run_outbox_and_orchestration().await;
            env.get_subscription(sub_id).await.assert().is_active();
        }
    }

    let granted = coupon_grants(&env, CUST_UBER_ID).await;
    assert_eq!(granted.len(), 1, "{path:?}: exactly one grant");
    assert_eq!(granted[0].amount_minor, 2_500);
    env.process_cycles().await;
    assert_eq!(
        coupon_grants(&env, CUST_UBER_ID).await.len(),
        1,
        "{path:?}: later transitions fund nothing more"
    );
}
