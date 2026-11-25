//! A sign-up subscribes in the customer's currency in the journey's option.
use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_store::domain::enums::BillingPeriodEnum;
use meteroid_store::domain::price_components::UsagePricingModel;
use meteroid_store::domain::prices::Pricing;
use meteroid_store::domain::product_family_journeys::ProductFamilyJourney;
use meteroid_store::repositories::ProductFamilyJourneysInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp, SeedPrice};
use crate::harness::{TestEnv, test_env};

fn journey(plan: Option<PlanId>, option: Option<&str>) -> ProductFamilyJourney {
    ProductFamilyJourney {
        product_family_id: PRODUCT_FAMILY_ID,
        signup_plan_id: plan,
        signup_option: option.map(str::to_owned),
        period_switch: true,
        self_serve_downgrade: false,
    }
}

/// A "Starter" plan with a 14-day free trial, sold Monthly (10 a month) or Annual (100 a year) in
/// EUR.
async fn starter(env: &TestEnv) -> PlanId {
    let (plan, version) = (PlanId::new(), PlanVersionId::new());
    let mut platform = SeedComp::rate(
        PriceComponentId::new(),
        "Platform",
        PRODUCT_PLATFORM_FEE_ID,
        PriceId::new(),
        DieselBillingPeriodEnum::Monthly,
        dec!(10),
    );
    platform.prices.push(SeedPrice {
        id: PriceId::new(),
        cadence: DieselBillingPeriodEnum::Annual,
        pricing: Pricing::Rate { rate: dec!(100) },
    });
    PlanSeed::new(plan, "Starter", version)
        .trial(14, plan, true)
        .options(&[
            ("Monthly", DieselBillingPeriodEnum::Monthly),
            ("Annual", DieselBillingPeriodEnum::Annual),
        ])
        .components(vec![platform])
        .seed(&mut env.conn().await)
        .await
        .expect("a plan with a trial");
    plan
}

/// The journey names a plan and one of its options by alias; none: the plan's default; an alias
/// the plan lacks is refused.
#[tokio::test]
async fn a_signup_journey_names_a_plan_and_its_option() {
    let env = test_env().await;
    let (paid, paid_version) = (PlanId::new(), PlanVersionId::new());
    PlanSeed::new(paid, "Pro", paid_version)
        .components(vec![SeedComp::rate(
            PriceComponentId::new(),
            "Platform",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            dec!(20),
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("a paid plan");
    let save = |journey| env.store().save_product_family_journey(TENANT_ID, journey);
    assert!(
        save(journey(Some(paid), Some("standard"))).await.is_err(),
        "a paid plan with no trial: a new customer has no payment method"
    );

    let plan = starter(&env).await;
    let default = save(journey(Some(plan), None))
        .await
        .expect("the plan's default option");
    assert_eq!(default.signup_option, None);
    assert!(
        save(journey(Some(plan), Some("quarterly"))).await.is_err(),
        "no such option"
    );
    let saved = save(journey(Some(plan), Some("annual")))
        .await
        .expect("its annual option");
    assert_eq!(saved.signup_option.as_deref(), Some("annual"));
    assert!(
        save(journey(None, Some("annual"))).await.is_err(),
        "an option needs a sign-up plan"
    );

    let (usage_plan, usage_version) = (PlanId::new(), PlanVersionId::new());
    PlanSeed::new(usage_plan, "Usage", usage_version)
        .free()
        .components(vec![SeedComp::usage(
            PriceComponentId::new(),
            "Bandwidth",
            PRODUCT_BANDWIDTH_ID,
            METRIC_BANDWIDTH,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            UsagePricingModel::PerUnit { rate: dec!(0.1) },
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("a usage-only plan");
    assert!(
        save(journey(Some(usage_plan), Some("annual")))
            .await
            .is_err(),
        "the usage-only plan has no annual option"
    );
    save(journey(Some(usage_plan), Some("standard")))
        .await
        .expect("a usage-only plan in its one option");
}

/// A customer created asking for it subscribes in their currency in the journey's option; a
/// currency the plan is not priced in is refused, the customer with it.
#[tokio::test]
async fn a_signup_subscribes_in_the_customers_currency_in_the_journeys_option() {
    let env = test_env().await;
    let plan = starter(&env).await;
    env.store()
        .save_product_family_journey(TENANT_ID, journey(Some(plan), Some("annual")))
        .await
        .expect("a sign-up plan in its annual option");
    let offer = env
        .services()
        .signup_offer(TENANT_ID, None, "EUR")
        .await
        .expect("the only line with a sign-up plan");
    assert_eq!(offer.currency, "EUR");
    assert_eq!(
        offer.option.as_ref().map(|option| option.alias.as_str()),
        Some("annual")
    );
    assert!(
        env.services()
            .signup_offer(TENANT_ID, None, "GBP")
            .await
            .is_err(),
        "the plan is not priced in GBP"
    );
    assert!(
        env.services()
            .signup_offer(TENANT_ID, None, "USD")
            .await
            .is_err(),
        "nor in USD"
    );

    let auth = crate::meteroid_it::svc_auth::login(env.setup.channel.clone()).await;
    let clients = crate::meteroid_it::clients::AllClients::from_channel(
        env.setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let create = |alias: &str, currency: &str| {
        meteroid_grpc::meteroid::api::customers::v1::CreateCustomerRequest {
            subscribe_to_signup_plan: true,
            signup_product_family_id: None,
            signup_levels: vec![],
            data: Some(meteroid_grpc::meteroid::api::customers::v1::CustomerNew {
                name: alias.to_string(),
                alias: Some(alias.to_string()),
                currency: currency.to_string(),
                ..Default::default()
            }),
        }
    };
    clients
        .customers
        .clone()
        .create_customer(create("signup-eur", "EUR"))
        .await
        .expect("a customer on the sign-up plan");
    assert!(
        clients
            .customers
            .clone()
            .create_customer(create("signup-usd", "USD"))
            .await
            .is_err(),
        "no offer in USD"
    );
    {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::{customer, subscription};
        let mut conn = env.conn().await;
        let customers: Vec<Option<String>> = customer::table
            .filter(customer::alias.eq_any(["signup-eur", "signup-usd"]))
            .select(customer::alias)
            .load(&mut conn)
            .await
            .expect("customers");
        assert_eq!(
            customers,
            vec![Some("signup-eur".to_string())],
            "both or neither"
        );
        let currencies: Vec<String> = subscription::table
            .inner_join(customer::table)
            .filter(customer::alias.eq("signup-eur"))
            .select(subscription::currency)
            .load(&mut conn)
            .await
            .expect("subscriptions");
        assert_eq!(currencies, vec!["EUR".to_string()]);
    }

    let created = env
        .services()
        .subscribe_to_signup_plan(
            common_domain::actor::Actor::System,
            TENANT_ID,
            CUST_UBER_ID,
            offer,
        )
        .await
        .expect("subscribed");
    let details = env
        .store()
        .get_subscription_details(TENANT_ID, created.id)
        .await
        .expect("details");
    assert_eq!(
        details.subscription.period,
        BillingPeriodEnum::Annual,
        "in the journey's option"
    );
    assert_eq!(
        details.plan_option.map(|option| option.alias).as_deref(),
        Some("annual")
    );
}
