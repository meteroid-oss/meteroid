//! A published plan version never changes: a pending plan change resolves against its
//! target version when it is written as facts, so every store path editing a published version's
//! components, prices or minimum refuses. The API delegates to these paths.
use std::fmt::Debug;

use common_domain::ids::PlanVersionId;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::data::ids::*;
use crate::harness::{TestEnv, test_env};
use common_domain::actor::Actor;
use meteroid_store::StoreResult;
use meteroid_store::domain::enums::BillingPeriodEnum;
use meteroid_store::domain::plan_minimums::MinimumScope;
use meteroid_store::domain::price_components::{
    PriceComponentNew, PriceEntry, PriceInput, ProductRef,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::products::ProductNew;
use meteroid_store::repositories::PlansInterface;
use meteroid_store::repositories::price_components::{
    PriceComponentInterface, PriceComponentInterfaceAuto,
};
use meteroid_store::repositories::products::{ProductInterface, ProductUpdate};

fn monthly_price(cents: i64) -> PriceInput {
    PriceInput {
        cadence: BillingPeriodEnum::Monthly,
        currency: "EUR".to_string(),
        pricing: Pricing::Rate {
            rate: Decimal::new(cents, 2),
        },
        credit_unit: None,
    }
}

/// The components of a plan version with their prices, sorted.
async fn catalog_of(env: &TestEnv, version: PlanVersionId) -> Vec<String> {
    let mut catalog: Vec<String> = env
        .store()
        .list_price_components(version, TENANT_ID)
        .await
        .expect("components")
        .into_iter()
        .map(|component| {
            let mut prices: Vec<String> = component
                .prices
                .iter()
                .map(|price| format!("{:?}", price.id))
                .collect();
            prices.sort();
            format!("{:?} {} {prices:?}", component.id, component.name)
        })
        .collect();
    catalog.sort();
    catalog
}

/// Refused because the version is published, not for another reason.
fn assert_refused<T: Debug>(result: StoreResult<T>, what: &str) {
    match result {
        Ok(done) => panic!("{what} changed a published version: {done:?}"),
        Err(err) => assert!(
            format!("{err:?}").contains("published plan version"),
            "{what}: {err:?}"
        ),
    }
}

/// Rules: R-101
#[rstest]
#[tokio::test]
async fn a_published_plan_version_refuses_every_edit_of_its_components(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let store = env.store();
    let before = catalog_of(&env, PLAN_VERSION_PRO_ID).await;
    let new_component = || PriceComponentNew {
        name: "Extra".to_string(),
        product_id: Some(PRODUCT_PLATFORM_FEE_ID),
        plan_version_id: PLAN_VERSION_PRO_ID,
    };

    assert_refused(
        store
            .create_price_component(new_component(), TENANT_ID)
            .await,
        "create",
    );
    assert_refused(
        store
            .create_price_component_with_prices(
                new_component(),
                vec![monthly_price(100)],
                TENANT_ID,
            )
            .await,
        "create with prices",
    );
    assert_refused(
        store
            .create_price_component_batch(vec![new_component()], TENANT_ID)
            .await,
        "create a batch",
    );
    assert_refused(
        store
            .create_price_component_from_ref(
                "Extra".to_string(),
                ProductRef::Existing(PRODUCT_PLATFORM_FEE_ID),
                vec![PriceEntry::New(monthly_price(100))],
                PLAN_VERSION_PRO_ID,
                TENANT_ID,
            )
            .await,
        "create from a product",
    );
    let seats = store
        .get_price_component_by_id(TENANT_ID, COMP_PRO_SEATS_ID)
        .await
        .expect("Pro's seats");
    assert_refused(
        store
            .update_price_component_with_prices(
                seats,
                vec![monthly_price(100)],
                TENANT_ID,
                PLAN_VERSION_PRO_ID,
            )
            .await,
        "replace its prices",
    );
    assert_refused(
        store
            .delete_price_component(COMP_PRO_SEATS_ID, TENANT_ID)
            .await,
        "delete",
    );
    assert_refused(
        store
            .set_plan_minimum(
                PLAN_VERSION_PRO_ID,
                TENANT_ID,
                10_000,
                MinimumScope::AllComponents,
            )
            .await,
        "set its minimum",
    );
    assert_refused(
        store
            .clear_plan_minimum(PLAN_VERSION_PRO_ID, TENANT_ID)
            .await,
        "clear its minimum",
    );
    assert_eq!(catalog_of(&env, PLAN_VERSION_PRO_ID).await, before);

    store
        .create_price_component(
            PriceComponentNew {
                plan_version_id: PLAN_VERSION_PRO_DRAFT_ID,
                ..new_component()
            },
            TENANT_ID,
        )
        .await
        .expect("a draft takes a new component");
}

/// A product's fee structure is how its prices are read: once it has a price, a new structure
/// is refused, the same one and its other fields stay editable.
/// Rules: R-098
#[rstest]
#[tokio::test]
async fn a_priced_product_refuses_another_fee_structure(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let update = |id, fee_structure| ProductUpdate {
        id,
        tenant_id: TENANT_ID,
        name: None,
        description: None,
        fee_type: None,
        fee_structure,
        tax_category_id: None,
    };
    let platform = store
        .find_product_by_id(PRODUCT_PLATFORM_FEE_ID, TENANT_ID)
        .await
        .expect("the platform fee");

    let refused = store
        .update_product(
            Actor::System,
            update(platform.id, Some(FeeStructure::OneTime {})),
        )
        .await;
    match refused {
        Ok(done) => panic!("a priced product took another fee structure: {done:?}"),
        Err(err) => assert!(
            format!("{err:?}").contains("fee_structure cannot change"),
            "{err:?}"
        ),
    }

    let renamed = store
        .update_product(
            Actor::System,
            ProductUpdate {
                name: Some("Platform".to_string()),
                description: Some(Some("Renamed".to_string())),
                ..update(platform.id, Some(platform.fee_structure.clone()))
            },
        )
        .await
        .expect("the same structure, renamed");
    assert_eq!(renamed.name, "Platform");
    assert_eq!(
        serde_json::to_value(&renamed.fee_structure).unwrap(),
        serde_json::to_value(&platform.fee_structure).unwrap()
    );

    let unpriced = store
        .create_product(
            Actor::System,
            ProductNew {
                name: "Unpriced".to_string(),
                description: None,
                tenant_id: TENANT_ID,
                family_id: PRODUCT_FAMILY_ID,
                fee_type: platform.fee_type,
                fee_structure: FeeStructure::Rate {},
                catalog: true,
                tax_category_id: None,
            },
        )
        .await
        .expect("a product with no price");
    let changed = store
        .update_product(
            Actor::System,
            update(unpriced.id, Some(FeeStructure::OneTime {})),
        )
        .await
        .expect("no price reads its structure yet");
    assert!(matches!(changed.fee_structure, FeeStructure::OneTime {}));
}
