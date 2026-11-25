//! Checkouts priced in the customer's currency in the option bought, else in the plan's, as the
//! subscription they create bills.
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_store::domain::SubscriptionFee;
use meteroid_store::domain::checkout_sessions::CheckoutType;
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew, CouponTerms};
use meteroid_store::domain::prices::Pricing;
use meteroid_store::domain::{CheckoutSession, CreateCheckoutSession};
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::coupons::CouponInterface;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp, SeedPrice};
use crate::harness::{TestEnv, test_env};

fn priced(id: PriceId, cadence: DieselBillingPeriodEnum, rate: Decimal) -> SeedPrice {
    SeedPrice {
        id,
        cadence,
        pricing: Pricing::Rate { rate },
    }
}

/// A plan in `currency` sold Monthly (its platform fee 20 a month) or Annual (200 a year).
async fn pro_plan(env: &TestEnv, currency: &'static str) -> PlanVersionId {
    let (version, component) = (PlanVersionId::new(), PriceComponentId::new());
    let (monthly, yearly) = (PriceId::new(), PriceId::new());
    let mut platform = SeedComp::rate(
        component,
        "Platform",
        PRODUCT_PLATFORM_FEE_ID,
        monthly,
        DieselBillingPeriodEnum::Monthly,
        dec!(20),
    );
    platform.prices = vec![
        priced(monthly, DieselBillingPeriodEnum::Monthly, dec!(20)),
        priced(yearly, DieselBillingPeriodEnum::Annual, dec!(200)),
    ];
    PlanSeed::new(PlanId::new(), "Pro", version)
        .currency(currency)
        .options(&[
            ("Monthly", DieselBillingPeriodEnum::Monthly),
            ("Annual", DieselBillingPeriodEnum::Annual),
        ])
        .components(vec![platform])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    version
}

fn session(version: PlanVersionId, option: Option<&str>) -> CreateCheckoutSession {
    CreateCheckoutSession {
        tenant_id: TENANT_ID,
        customer_id: CUST_UBER_ID,
        plan_version_id: version,
        option: option.map(str::to_owned),
        billing_start_date: Some(chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()),
        billing_day_anchor: Some(1),
        net_terms: None,
        trial_duration_days: None,
        end_date: None,
        auto_advance_invoices: true,
        charge_automatically: true,
        invoice_memo: None,
        invoice_threshold: None,
        purchase_order: None,
        payment_methods_config: None,
        components: None,
        add_ons: None,
        coupon_code: None,
        coupon_ids: vec![],
        expires_in_hours: Some(24),
        metadata: None,
        checkout_type: CheckoutType::SelfServe,
        subscription_id: None,
        change_date: None,
        success_url: None,
        cancel_url: None,
    }
}

async fn preview(
    env: &TestEnv,
    session: &CheckoutSession,
) -> meteroid_store::domain::SubscriptionDetails {
    env.services()
        .build_preview_subscription_details(session, TENANT_ID, None)
        .await
        .expect("a preview")
}

async fn previewed_rate(env: &TestEnv, session: &CheckoutSession) -> Decimal {
    let details = preview(env, session).await;
    match &details.price_components[0].fee {
        SubscriptionFee::Rate { rate } => *rate,
        fee => panic!("a rate, got {fee:?}"),
    }
}

/// A checkout is priced in the customer's currency in its option, kept by alias, as creating the
/// subscription bills it; none asked: the default; an option the plan lacks is refused; a plan not
/// priced in that currency is bought in its own, as the subscription would bill it.
#[tokio::test]
async fn a_checkout_previews_the_price_of_its_option_in_the_customers_currency() {
    let env = test_env().await;
    let version = pro_plan(&env, "EUR").await;
    let store = env.store();

    let default = store
        .create_checkout_session(session(version, None))
        .await
        .expect("a checkout of the default option");
    assert_eq!(default.option.as_deref(), Some("monthly"));
    assert_eq!(previewed_rate(&env, &default).await, dec!(20));

    let yearly = store
        .create_checkout_session(session(version, Some("annual")))
        .await
        .expect("a yearly checkout");
    assert_eq!(yearly.option.as_deref(), Some("annual"));
    assert_eq!(previewed_rate(&env, &yearly).await, dec!(200));

    assert!(
        store
            .create_checkout_session(session(version, Some("quarterly")))
            .await
            .is_err(),
        "an option the plan does not have"
    );
    let in_usd = pro_plan(&env, "USD").await;
    let usd = store
        .create_checkout_session(session(in_usd, Some("monthly")))
        .await
        .expect("a USD-only plan, bought in USD");
    let details = preview(&env, &usd).await;
    assert_eq!(details.subscription.currency, "USD");
    assert_eq!(previewed_rate(&env, &usd).await, dec!(20));
}

/// An EUR customer on a USD-only plan checks out in USD: a code for a fixed USD amount, with no
/// EUR one, is redeemed there, and an EUR-only one is refused.
#[tokio::test]
async fn a_coupon_code_is_redeemed_in_the_currency_the_checkout_bills() {
    let env = test_env().await;
    let store = env.store();
    let in_usd = pro_plan(&env, "USD").await;
    let fixed = |code: &str, currency: &str| CouponNew {
        code: code.to_string(),
        description: format!("5 {currency} off"),
        tenant_id: TENANT_ID,
        discount: CouponDiscount::Fixed {
            amount: dec!(5),
            currency: currency.to_string(),
        },
        expires_at: None,
        redemption_limit: None,
        recurring_value: None,
        reusable: true,
        plan_ids: vec![],
    };
    for (code, currency) in [("USD5", "USD"), ("EUR5", "EUR")] {
        store
            .create_coupon_with_terms(Actor::System, fixed(code, currency), CouponTerms::default())
            .await
            .expect("a coupon");
    }

    let mut with_usd = session(in_usd, Some("monthly"));
    with_usd.coupon_code = Some("USD5".to_owned());
    let checkout = store
        .create_checkout_session(with_usd)
        .await
        .expect("a USD code on a USD checkout");
    let details = preview(&env, &checkout).await;
    assert_eq!(details.subscription.currency, "USD");
    assert_eq!(details.applied_coupons.len(), 1, "the USD code applies");

    let mut with_eur = session(in_usd, Some("monthly"));
    with_eur.coupon_code = Some("EUR5".to_owned());
    let err = store
        .create_checkout_session(with_eur)
        .await
        .expect_err("an EUR-only code on a USD checkout");
    assert!(format!("{err:?}").contains("no amount in USD"), "{err:?}");
}

/// A coupon the operator put on a checkout for another plan: the preview refuses it, as its
/// completion does, never showing a discount the payment would not give.
#[tokio::test]
async fn a_preview_refuses_a_coupon_scoped_to_another_plan() {
    let env = test_env().await;
    let store = env.store();
    let version = pro_plan(&env, "EUR").await;
    let other = PlanId::new();
    PlanSeed::new(other, "Other", PlanVersionId::new())
        .seed(&mut env.conn().await)
        .await
        .expect("another plan");
    let coupon = store
        .create_coupon_with_terms(
            Actor::System,
            CouponNew {
                code: "OTHER10".to_string(),
                description: "10% off the other plan".to_string(),
                tenant_id: TENANT_ID,
                discount: CouponDiscount::Percentage(dec!(10)),
                expires_at: None,
                redemption_limit: None,
                recurring_value: None,
                reusable: true,
                plan_ids: vec![other],
            },
            CouponTerms::default(),
        )
        .await
        .expect("a scoped coupon");

    let mut scoped = session(version, Some("monthly"));
    scoped.coupon_ids = vec![coupon.id];
    let checkout = store
        .create_checkout_session(scoped)
        .await
        .expect("a checkout");
    let err = env
        .services()
        .build_preview_subscription_details(&checkout, TENANT_ID, None)
        .await
        .expect_err("out of the coupon's scope");
    assert!(
        format!("{err:?}").contains("cannot be applied to this plan"),
        "{err:?}"
    );
}
