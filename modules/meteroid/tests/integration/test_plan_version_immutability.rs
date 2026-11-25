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
        fee_structure: Some(FeeStructure::Rate {}),
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
                vec![],
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

/// A product has no fee structure: it is created and edited without one, and its charges keep
/// how they bill it whatever the product becomes.
/// Rules: R-139
#[rstest]
#[tokio::test]
async fn a_product_is_edited_without_touching_how_its_charges_bill_it(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let update = |id| ProductUpdate {
        id,
        tenant_id: TENANT_ID,
        name: None,
        description: None,
        tax_category_id: None,
        icon: None,
        color: None,
    };

    let renamed = store
        .update_product(
            Actor::System,
            ProductUpdate {
                name: Some("Seat licences".to_string()),
                description: Some(Some("Renamed".to_string())),
                ..update(PRODUCT_SEATS_ID)
            },
        )
        .await
        .expect("a priced product renamed");
    assert_eq!(renamed.name, "Seat licences");
    let seats = store
        .list_price_components(PLAN_VERSION_PRO_ID, TENANT_ID)
        .await
        .expect("components")
        .into_iter()
        .find(|component| component.id == COMP_PRO_SEATS_ID)
        .expect("Pro's seats");
    assert!(
        matches!(seats.fee_structure, Some(FeeStructure::Slot { .. })),
        "the charge still bills per seat: {:?}",
        seats.fee_structure
    );

    let unpriced = store
        .create_product(
            Actor::System,
            ProductNew {
                name: "Unpriced".to_string(),
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
        .expect("a product with no structure");
    assert_eq!(unpriced.name, "Unpriced");
}
