//! Rates by dimension price a metric that adds up, as a rate card does: on a distinct count they
//! are refused wherever a price meets its metric, an add-on's as a subscription's own (R-136).
use common_domain::actor::Actor;
use common_domain::ids::BillableMetricId;
use meteroid_store::StoreResult;
use meteroid_store::domain::BillableMetricNew;
use meteroid_store::domain::enums::{BillingMetricAggregateEnum, BillingPeriodEnum, FeeTypeEnum};
use meteroid_store::domain::price_components::{
    DimensionRate, DimensionRates, PriceEntry, PriceInput, ProductRef, UsagePricingModel,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::rate_cards::RateCardUnmatched;
use meteroid_store::domain::{CreateSubscriptionComponents, add_ons::AddOn};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use rust_decimal::Decimal;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};
use crate::helpers::credit_recompute::metric_component;

async fn metric(
    env: &TestEnv,
    code: &str,
    aggregation_type: BillingMetricAggregateEnum,
) -> BillableMetricId {
    env.store()
        .insert_billable_metric(
            Actor::System,
            BillableMetricNew {
                name: code.to_owned(),
                description: None,
                code: code.to_owned(),
                aggregation_type,
                aggregation_key: Some("user_id".to_owned()),
                unit_conversion_factor: None,
                unit_conversion_rounding: None,
                segmentation_matrix: None,
                filters: None,
                usage_group_key: None,
                usage_breakdown_key: None,
                unit_label: None,
                tenant_id: TENANT_ID,
                product_family_id: PRODUCT_FAMILY_ID,
                product_id: None,
            },
        )
        .await
        .expect("metric")
        .id
}

/// 2.00 per user of a lite model, 12.00 of a standard one.
fn by_model() -> UsagePricingModel {
    let rate = |model: &str, rate: i64| DimensionRate {
        key: vec![model.to_owned()],
        rate: Decimal::from(rate),
        label: None,
    };
    UsagePricingModel::DimensionRates(DimensionRates {
        dimensions: vec!["model".to_owned()],
        rates: vec![rate("lite", 2), rate("standard", 12)],
        unit_size: Decimal::ONE,
        on_unmatched: RateCardUnmatched::Ignore,
    })
}

async fn add_on(env: &TestEnv, metric_id: BillableMetricId) -> StoreResult<AddOn> {
    let model = by_model();
    env.store()
        .create_add_on_from_ref(
            "Seats by model".to_owned(),
            ProductRef::New {
                name: "Seats by model".to_owned(),
                fee_type: FeeTypeEnum::Usage,
                fee_structure: FeeStructure::Usage {
                    metric_id,
                    model: UsageModel::from(&model),
                },
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_owned(),
                pricing: Pricing::Usage(model),
                credit_unit: None,
            }),
            None,
            true,
            None,
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
}

fn refused_as_not_adding_up(err: &error_stack::Report<StoreError>) -> bool {
    format!("{err:?}").contains("rates by dimension price a metric that adds up")
}

/// An add-on rating active users by model is refused on a distinct count, which would bill the sum
/// of each model's distinct users; on a sum it is created.
#[tokio::test]
async fn an_add_on_rates_by_dimension_only_a_metric_that_adds_up() {
    let env = test_env().await;
    let distinct = metric(
        &env,
        "active_users",
        BillingMetricAggregateEnum::CountDistinct,
    )
    .await;
    let summed = metric(&env, "user_minutes", BillingMetricAggregateEnum::Sum).await;

    let err = add_on(&env, distinct)
        .await
        .expect_err("a distinct count is refused");
    assert!(refused_as_not_adding_up(&err), "{err:?}");
    add_on(&env, summed)
        .await
        .expect("a sum is rated by dimension");
}

/// A subscription's own price rating by dimension is refused on a distinct count, allowed on a
/// sum.
#[tokio::test]
async fn a_subscriptions_own_price_rates_by_dimension_only_a_metric_that_adds_up() {
    let env = test_env().await;
    let distinct = metric(
        &env,
        "active_users",
        BillingMetricAggregateEnum::CountDistinct,
    )
    .await;
    let summed = metric(&env, "user_minutes", BillingMetricAggregateEnum::Sum).await;
    let subscribe = |metric_id| {
        let mut request = subscription()
            .plan_version(PLAN_VERSION_STARTER_ID)
            .on_start()
            .no_trial()
            .request();
        request.price_components = Some(CreateSubscriptionComponents {
            parameterized_components: vec![],
            overridden_components: vec![],
            extra_components: vec![metric_component("Users by model", metric_id, by_model())],
            remove_components: vec![],
        });
        env.services()
            .insert_subscription(Actor::System, request, TENANT_ID)
    };

    let err = subscribe(distinct)
        .await
        .expect_err("a distinct count is refused");
    assert!(refused_as_not_adding_up(&err), "{err:?}");
    subscribe(summed)
        .await
        .expect("a sum is rated by dimension");
}
