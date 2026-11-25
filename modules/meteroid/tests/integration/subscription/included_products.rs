//! A product a plan includes with no price: the subscription gets its features, never a fee.
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_store::domain::entitlements::{EntitlementValue, FeatureNew, FeatureType};
use meteroid_store::domain::products::ProductNew;
use meteroid_store::repositories::entitlements::{EntitlementsInterface, ResolveTarget};
use meteroid_store::repositories::products::ProductInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{subscription, test_env};

#[tokio::test]
async fn an_included_product_brings_its_features_and_no_fee() {
    let env = test_env().await;
    let bundle = env
        .store()
        .create_product(
            Actor::System,
            ProductNew {
                name: "Security bundle".to_string(),
                description: None,
                tenant_id: TENANT_ID,
                family_id: PRODUCT_FAMILY_ID,
                catalog: true,
                tax_category_id: None,
                icon: None,
                color: None,
            },
        )
        .await
        .expect("a product")
        .id;
    let sso = env
        .store()
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: Some(bundle),
                name: "SSO".to_string(),
                code: "sso".to_string(),
                description: None,
                feature_type: FeatureType::Boolean,
                entitlement: Some(EntitlementValue::Boolean { enabled: true }),
            },
        )
        .await
        .expect("a feature of the bundle")
        .id;
    let version = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Pro", version)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                dec!(20),
            ),
            SeedComp::included(PriceComponentId::new(), "Security bundle", bundle),
        ])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");

    let sub_id = subscription()
        .plan_version(version)
        .start_date(chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let details = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("details");
    let fees: Vec<&str> = details
        .price_components
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(fees, ["Platform"], "no fee for the included product");

    let mut conn = env.conn().await;
    let resolved = env
        .store()
        .resolve_entitlements_for_entity(&mut conn, TENANT_ID, ResolveTarget::Subscription(sub_id))
        .await
        .expect("entitlements");
    assert!(
        resolved
            .iter()
            .any(|entitlement| entitlement.feature.id == sso),
        "the included product's feature"
    );
}

/// A subscription created without an included product never gets its features.
#[tokio::test]
async fn an_included_product_removed_at_creation_brings_no_features() {
    use meteroid_store::domain::CreateSubscriptionComponents;

    let env = test_env().await;
    let bundle = env
        .store()
        .create_product(
            Actor::System,
            ProductNew {
                name: "Security bundle".to_string(),
                description: None,
                tenant_id: TENANT_ID,
                family_id: PRODUCT_FAMILY_ID,
                catalog: true,
                tax_category_id: None,
                icon: None,
                color: None,
            },
        )
        .await
        .expect("a product")
        .id;
    let sso = env
        .store()
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: Some(bundle),
                name: "SSO".to_string(),
                code: "sso".to_string(),
                description: None,
                feature_type: FeatureType::Boolean,
                entitlement: Some(EntitlementValue::Boolean { enabled: true }),
            },
        )
        .await
        .expect("a feature of the bundle")
        .id;
    let (version, included) = (PlanVersionId::new(), PriceComponentId::new());
    PlanSeed::new(PlanId::new(), "Pro", version)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                dec!(20),
            ),
            SeedComp::included(included, "Security bundle", bundle),
        ])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");

    let mut request = subscription()
        .plan_version(version)
        .start_date(chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .request();
    request.price_components = Some(CreateSubscriptionComponents {
        parameterized_components: vec![],
        overridden_components: vec![],
        extra_components: vec![],
        remove_components: vec![included],
    });
    let sub_id = env
        .services()
        .insert_subscription(Actor::System, request, TENANT_ID)
        .await
        .expect("a subscription without the bundle")
        .id;

    let mut conn = env.conn().await;
    let resolved = env
        .store()
        .resolve_entitlements_for_entity(&mut conn, TENANT_ID, ResolveTarget::Subscription(sub_id))
        .await
        .expect("entitlements");
    assert!(
        resolved
            .iter()
            .all(|entitlement| entitlement.feature.id != sso),
        "the removed product's feature"
    );
}
