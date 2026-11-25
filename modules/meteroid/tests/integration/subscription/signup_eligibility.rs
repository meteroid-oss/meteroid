//! A sign-up starts only a customer the sign-up plan's rule lets in.
use std::str::FromStr;

use common_domain::actor::Actor;
use common_domain::country::CountryCode;
use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_store::domain::eligibility::Eligibility;
use meteroid_store::domain::enums::CustomerType;
use meteroid_store::domain::product_family_journeys::ProductFamilyJourney;
use meteroid_store::domain::{Address, CustomerNew, CustomerTaxStatus};
use meteroid_store::repositories::{EligibilityInterface, ProductFamilyJourneysInterface};
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, test_env};

/// A "Starter" plan with a 14-day trial at 10 EUR a month, the sign-up plan of the family.
async fn signup_plan(env: &TestEnv) -> PlanId {
    let (plan, version) = (PlanId::new(), PlanVersionId::new());
    PlanSeed::new(plan, "Starter", version)
        .trial(14, plan, true)
        .components(vec![SeedComp::rate(
            PriceComponentId::new(),
            "Platform",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            dec!(10),
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("a plan with a trial");
    env.store()
        .save_product_family_journey(
            TENANT_ID,
            ProductFamilyJourney {
                product_family_id: PRODUCT_FAMILY_ID,
                signup_plan_id: Some(plan),
                signup_option: None,
                period_switch: true,
                self_serve_downgrade: false,
            },
        )
        .await
        .expect("the sign-up plan");
    plan
}

fn signing_up(alias: &str, country: &str) -> CustomerNew {
    CustomerNew {
        name: alias.to_string(),
        alias: Some(alias.to_string()),
        billing_email: Some(format!("{alias}@example.com")),
        invoicing_emails: vec![],
        phone: None,
        currency: "EUR".to_string(),
        billing_address: Some(Address {
            line1: Some("1 Rua Augusta".to_string()),
            line2: None,
            city: Some("Lisboa".to_string()),
            country: Some(CountryCode::from_str(country).expect("country")),
            state: None,
            zip_code: Some("1100-048".to_string()),
        }),
        shipping_address: None,
        invoicing_entity_id: None,
        force_created_date: None,
        vat_number: None,
        legal_number: None,
        buyer_reference: None,
        custom_taxes: vec![],
        tax_status: CustomerTaxStatus::Taxable,
        exemption_reason: None,
        connected_account_id: None,
        custom_properties: serde_json::json!({}),
        preferred_locales: vec![],
        customer_type: CustomerType::Individual,
        first_name: Some("Ana".to_string()),
        last_name: Some("Sousa".to_string()),
    }
}

/// A sign-up plan for customers in Portugal and Brazil refuses one in France with the rule's
/// words, and creates neither the customer nor a subscription; one in Portugal signs up.
#[tokio::test]
async fn a_signup_is_refused_by_the_plans_country_rule() {
    let env = test_env().await;
    let plan = signup_plan(&env).await;
    env.store()
        .set_plan_eligibility(
            Actor::System,
            TENANT_ID,
            plan,
            Eligibility {
                countries: vec!["PT".to_owned(), "BR".to_owned()],
                ..Default::default()
            },
        )
        .await
        .expect("the rule");
    let offer = env
        .services()
        .signup_offer(TENANT_ID, None, "EUR")
        .await
        .expect("the sign-up offer");

    let refused = env
        .services()
        .create_customer_on_signup(
            Actor::System,
            TENANT_ID,
            signing_up("signup-fr", "FR"),
            offer.clone(),
        )
        .await
        .expect_err("France is not in the rule");
    let refused = format!("{refused:?}");
    assert!(
        refused.contains("This offer is for customers in PT, BR"),
        "{refused}"
    );

    let created = env
        .services()
        .create_customer_on_signup(
            Actor::System,
            TENANT_ID,
            signing_up("signup-pt", "PT"),
            offer,
        )
        .await
        .expect("Portugal is");

    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::{customer, subscription};
    let mut conn = env.conn().await;
    let aliases: Vec<Option<String>> = customer::table
        .filter(customer::alias.eq_any(["signup-fr", "signup-pt"]))
        .select(customer::alias)
        .load(&mut conn)
        .await
        .expect("customers");
    assert_eq!(
        aliases,
        vec![Some("signup-pt".to_string())],
        "both or neither"
    );
    let subscribed: i64 = subscription::table
        .filter(subscription::customer_id.eq(created.id))
        .count()
        .get_result(&mut conn)
        .await
        .expect("subscriptions");
    assert_eq!(subscribed, 1);
}
