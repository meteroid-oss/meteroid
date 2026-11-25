//! A usage-levels charge moves between the levels of its plan (R-135): a higher level at once,
//! its fee prorated, each part of the period counting its usage against its own level; a lower one
//! at the period's end. A level not in the subscription's cell, or a price of another product, is
//! refused. Volume plans: €199 for €50K managed revenue, €479 for €125K, €899 for €250K, 0.5% of
//! the revenue above the level.
use std::collections::HashMap;
use std::sync::Arc;

use chrono::{NaiveDate, NaiveTime};
use rstest::rstest;
use rust_decimal::Decimal;
use tonic::Code;
use uuid::uuid;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp, SeedPrice};
use crate::harness::{TestEnv, subscription, test_env, test_env_with_usage};
use common_domain::actor::Actor;
use common_domain::ids::*;
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_grpc::meteroid::portal::subscription::v1 as portal;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageClient, MockUsageDataParams, UsageData,
};
use meteroid_store::domain::enums::InvoiceType;
use meteroid_store::domain::price_components::PriceEntry;
use meteroid_store::domain::prices::Pricing;
use meteroid_store::domain::product_family_journeys::ProductFamilyJourney;
use meteroid_store::domain::subscription_amendment::{
    AddOnChanges, ComponentChanges, EditComponent, LevelChangeOutcome, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::{ChangeDirection, PlanChangeMode};
use meteroid_store::domain::{
    ComponentOverride, ComponentParameterization, ComponentParameters,
    CreateSubscriptionComponents, UsagePeriod,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::ProductFamilyJourneysInterface;
use meteroid_store::repositories::credit_notes::CreditNoteInterface;

use super::portal_plan_change::{client, rank, request};

const PLAN_VOLUME_ID: PlanId = PlanId::from_const(uuid!("019438e0-1a01-7000-8000-0000000001a1"));
const PLAN_VERSION_VOLUME_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("019438e0-1a02-7000-8000-0000000001a2"));
const COMP_LEVELS_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-1a03-7000-8000-0000000001a3"));
const PRICE_50K_ID: PriceId = PriceId::from_const(uuid!("019438e0-1a04-7000-8000-0000000001a4"));
const PRICE_125K_ID: PriceId = PriceId::from_const(uuid!("019438e0-1a05-7000-8000-0000000001a5"));
const PRICE_250K_ID: PriceId = PriceId::from_const(uuid!("019438e0-1a06-7000-8000-0000000001a6"));

/// Another plan selling the same product at a level the Volume plan does not have.
const PLAN_SCALE_ID: PlanId = PlanId::from_const(uuid!("019438e0-1a07-7000-8000-0000000001a7"));
const PLAN_VERSION_SCALE_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("019438e0-1a08-7000-8000-0000000001a8"));
const COMP_SCALE_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-1a09-7000-8000-0000000001a9"));
const PRICE_1M_ID: PriceId = PriceId::from_const(uuid!("019438e0-1a0a-7000-8000-0000000001aa"));

fn d(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, month, day).unwrap()
}

fn level(id: PriceId, rate: i64, included: u64) -> SeedPrice {
    SeedPrice {
        id,
        cadence: DieselBillingPeriodEnum::Monthly,
        pricing: Pricing::Capacity {
            rate: Decimal::from(rate),
            included,
            overage_rate: Decimal::new(5, 3),
        },
    }
}

async fn seed_volume_plans(env: &TestEnv) {
    let mut levels = SeedComp::capacity(
        COMP_LEVELS_ID,
        "Managed revenue",
        PRODUCT_CAPACITY_ID,
        METRIC_BANDWIDTH,
        PRICE_50K_ID,
        DieselBillingPeriodEnum::Monthly,
        Decimal::from(199),
        50_000,
        Decimal::new(5, 3),
    );
    levels.prices.push(level(PRICE_125K_ID, 479, 125_000));
    levels.prices.push(level(PRICE_250K_ID, 899, 250_000));
    PlanSeed::new(PLAN_VOLUME_ID, "Volume", PLAN_VERSION_VOLUME_ID)
        .components(vec![levels])
        .seed(&mut env.conn().await)
        .await
        .expect("volume plan");

    let scale = SeedComp::capacity(
        COMP_SCALE_ID,
        "Managed revenue",
        PRODUCT_CAPACITY_ID,
        METRIC_BANDWIDTH,
        PRICE_1M_ID,
        DieselBillingPeriodEnum::Monthly,
        Decimal::from(1_499),
        1_000_000,
        Decimal::new(5, 3),
    );
    PlanSeed::new(PLAN_SCALE_ID, "Scale", PLAN_VERSION_SCALE_ID)
        .components(vec![scale])
        .seed(&mut env.conn().await)
        .await
        .expect("scale plan");
}

/// A Volume subscription of `customer` from `start`, at the level including `included` units.
async fn on_level(
    env: &TestEnv,
    customer: CustomerId,
    start: NaiveDate,
    included: u64,
) -> SubscriptionId {
    let mut create = subscription()
        .customer(customer)
        .plan_version(PLAN_VERSION_VOLUME_ID)
        .start_date(start)
        .on_start()
        .no_trial()
        .request();
    create.price_components = Some(CreateSubscriptionComponents {
        parameterized_components: vec![ComponentParameterization {
            component_id: COMP_LEVELS_ID,
            parameters: ComponentParameters {
                initial_slot_count: None,
                billing_period: None,
                committed_capacity: Some(included),
            },
        }],
        overridden_components: vec![],
        extra_components: vec![],
        remove_components: vec![],
    });
    env.services()
        .insert_subscription(Actor::User { id: USER_ID }, create, TENANT_ID)
        .await
        .expect("subscription on its level")
        .id
}

/// The subscription's usage-levels charge, as it is billed now.
async fn levels_row(
    env: &TestEnv,
    sub_id: SubscriptionId,
) -> diesel_models::subscription_components::SubscriptionComponentRow {
    env.get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.price_component_id == Some(COMP_LEVELS_ID))
        .expect("the usage-levels charge")
}

fn level_change(
    mode: PlanChangeMode,
    component: SubscriptionPriceComponentId,
    price: PriceId,
) -> SubscriptionAmendment {
    SubscriptionAmendment {
        apply_mode: mode,
        component_changes: ComponentChanges {
            edited: vec![EditComponent {
                subscription_component_id: component,
                name: None,
                price_entry: PriceEntry::Existing(price),
            }],
            added: vec![],
            removed: vec![],
        },
        add_on_changes: AddOnChanges::default(),
    }
}

fn usage_on(entries: &[(NaiveDate, NaiveDate, i64)]) -> Arc<MockUsageClient> {
    let mut data = HashMap::new();
    for (from, until, value) in entries {
        let params = MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: from.and_time(NaiveTime::MIN),
            period_end: until.and_time(NaiveTime::MIN),
        };
        data.insert(
            params,
            UsageData {
                data: vec![GroupedUsageData {
                    value: Decimal::from(*value),
                    dimensions: HashMap::new(),
                }],
                period: UsagePeriod {
                    start: from.and_time(NaiveTime::MIN),
                    end: until.and_time(NaiveTime::MIN),
                },
            },
        );
    }
    Arc::new(MockUsageClient { data })
}

fn refused_with(err: &error_stack::Report<StoreError>, text: &str) -> bool {
    format!("{err:?}").contains(text)
}

/// From €125K to €250K on January 16, 16 of 31 days left: the move bills now what the new level
/// costs for those days less what the old one does, 46400 − 24723 = 21677, nothing credited on a
/// note. February renews at €899. January's usage counts against each level for its days: 100,000
/// to the 16th against 15/31 of €125K (60,483.87), 39,516.13 over at 0.5% = 19758; 200,000 from
/// the 16th against 16/31 of €250K (129,032.26), 70,967.74 over = 35484. Read whole at the new
/// level, the 300,000 would bill 250.00 instead.
/// Rules: R-135, R-002, R15
#[tokio::test]
async fn a_higher_level_applies_now_prorated_and_each_part_counts_its_own_overage() {
    let env = test_env_with_usage(usage_on(&[
        (d(1, 1), d(1, 16), 100_000),
        (d(1, 16), d(2, 1), 200_000),
    ]))
    .await;
    seed_volume_plans(&env).await;
    let sub_id = on_level(&env, CUST_UBER_ID, d(1, 1), 125_000).await;

    let opening = env.get_invoices(sub_id).await;
    assert_eq!(opening.len(), 1);
    assert_eq!(opening[0].subtotal, 47_900, "January at €479");
    let held = levels_row(&env, sub_id).await;
    assert_eq!(held.price_id, Some(PRICE_125K_ID));

    let preview = env
        .services()
        .preview_amendment(
            sub_id,
            TENANT_ID,
            level_change(PlanChangeMode::EndOfPeriod, held.id, PRICE_250K_ID),
        )
        .await
        .expect("preview the move up");
    assert_eq!(preview.change_direction, ChangeDirection::Upgrade);

    let result = env
        .services()
        .apply_amendment_immediate_at(
            Actor::User { id: USER_ID },
            sub_id,
            TENANT_ID,
            level_change(PlanChangeMode::Immediate, held.id, PRICE_250K_ID),
            d(1, 16),
        )
        .await
        .expect("a higher level applies now");
    assert!(result.credit_note_ids.is_empty(), "{result:?}");
    let adjustment_id = result.adjustment_invoice_id.expect("an adjustment invoice");

    let invoices = env.get_invoices(sub_id).await;
    let adjustment = invoices
        .iter()
        .find(|i| i.id == adjustment_id)
        .expect("the adjustment invoice");
    assert_eq!(adjustment.invoice_type, InvoiceType::Adjustment);
    assert_eq!(adjustment.subtotal, 21_677);
    assert!(
        adjustment.line_items.iter().all(|l| l.metric_id.is_none()),
        "the usage of the period is billed at its end: {:?}",
        adjustment.line_items
    );
    let moved = levels_row(&env, sub_id).await;
    assert_eq!(moved.price_id, Some(PRICE_250K_ID));
    assert_eq!(moved.effective_from, d(1, 16));

    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    let renewal = invoices
        .iter()
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.billing_period_start == d(2, 1))
        .expect("the February renewal");
    let fixed: i64 = renewal
        .line_items
        .iter()
        .filter(|l| l.metric_id.is_none() && l.start_date == d(2, 1))
        .map(|l| l.amount_subtotal)
        .sum();
    assert_eq!(fixed, 89_900, "February at €899");

    let mut overage: Vec<(NaiveDate, NaiveDate, i64)> = renewal
        .line_items
        .iter()
        .filter(|l| l.metric_id == Some(METRIC_BANDWIDTH))
        .map(|l| (l.start_date, l.end_date, l.amount_subtotal))
        .collect();
    overage.sort();
    assert_eq!(
        overage,
        vec![(d(1, 1), d(1, 16), 19_758), (d(1, 16), d(2, 1), 35_484)],
        "each part of January against its own level"
    );
}

/// From €250K to €125K: scheduled for February 1, nothing credited, January stays at €899 and
/// February renews at €479. An immediate move down is refused, and a change already waiting blocks
/// another one.
/// Rules: R-135, X5
#[tokio::test]
async fn a_lower_level_waits_for_the_period_end() {
    let env = test_env_with_usage(usage_on(&[(d(1, 1), d(2, 1), 200_000)])).await;
    seed_volume_plans(&env).await;
    let sub_id = on_level(&env, CUST_UBER_ID, d(1, 1), 250_000).await;
    let held = levels_row(&env, sub_id).await;

    let refused = env
        .services()
        .apply_amendment_immediate_at(
            Actor::User { id: USER_ID },
            sub_id,
            TENANT_ID,
            level_change(PlanChangeMode::Immediate, held.id, PRICE_125K_ID),
            d(1, 16),
        )
        .await
        .expect_err("a lower level never applies mid-period");
    assert!(
        refused_with(&refused, "end of the billing period"),
        "{refused:?}"
    );

    let preview = env
        .services()
        .preview_capacity_level_change(TENANT_ID, sub_id, held.id, PRICE_125K_ID)
        .await
        .expect("preview the move down");
    assert_eq!(preview.direction, ChangeDirection::Downgrade);
    assert_eq!(preview.apply_mode, PlanChangeMode::EndOfPeriod);
    assert_eq!(preview.effective_date, d(2, 1));
    assert_eq!(
        (preview.from_included, preview.to_included),
        (250_000, 125_000)
    );

    let outcome = env
        .services()
        .change_capacity_level(
            Actor::User { id: USER_ID },
            TENANT_ID,
            sub_id,
            held.id,
            PRICE_125K_ID,
        )
        .await
        .expect("a lower level is scheduled");
    let LevelChangeOutcome::Scheduled(event) = outcome else {
        panic!("a lower level waits for the period's end: {outcome:?}");
    };
    assert_eq!(event.scheduled_time.date(), d(2, 1));
    assert_eq!(levels_row(&env, sub_id).await.price_id, Some(PRICE_250K_ID));

    let refused = env
        .services()
        .change_capacity_level(
            Actor::User { id: USER_ID },
            TENANT_ID,
            sub_id,
            held.id,
            PRICE_50K_ID,
        )
        .await
        .expect_err("a change waiting blocks another");
    assert!(refused_with(&refused, "already scheduled"), "{refused:?}");

    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    let january = invoices
        .iter()
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.billing_period_start == d(1, 1))
        .expect("January");
    assert!(
        env.store()
            .list_credit_notes_by_invoice_id(TENANT_ID, january.id)
            .await
            .expect("credit notes")
            .is_empty(),
        "a move down gives nothing back"
    );
    let renewal = invoices
        .iter()
        .find(|i| i.invoice_type == InvoiceType::Recurring && i.billing_period_start == d(2, 1))
        .expect("the February renewal");
    let fixed: i64 = renewal
        .line_items
        .iter()
        .filter(|l| l.metric_id.is_none() && l.start_date == d(2, 1))
        .map(|l| l.amount_subtotal)
        .sum();
    assert_eq!(fixed, 47_900, "February at €479");
    // January's 200,000 against its €250K level, whole: nothing over.
    assert!(
        renewal
            .line_items
            .iter()
            .filter(|l| l.metric_id == Some(METRIC_BANDWIDTH))
            .all(|l| l.amount_subtotal == 0),
        "{:?}",
        renewal.line_items
    );
    let moved = levels_row(&env, sub_id).await;
    assert_eq!(moved.price_id, Some(PRICE_125K_ID));
    assert_eq!(moved.effective_from, d(2, 1));
}

/// A level is one of the subscription's cell: another plan's price of the same product, or the
/// level held, is no move. A price of another product is refused wherever an existing price names
/// a component's, at creation as in an amendment.
/// Rules: R-135
#[rstest]
#[tokio::test]
async fn only_a_level_of_the_cell_or_a_price_of_the_product_is_accepted(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    seed_volume_plans(&env).await;
    let sub_id = on_level(&env, CUST_UBER_ID, d(1, 1), 125_000).await;
    let held = levels_row(&env, sub_id).await;

    for price in [PRICE_1M_ID, PRICE_125K_ID] {
        let _ = env
            .services()
            .change_capacity_level(
                Actor::User { id: USER_ID },
                TENANT_ID,
                sub_id,
                held.id,
                price,
            )
            .await
            .expect_err("not another level of the cell");
    }
    let refused = env
        .services()
        .preview_amendment(
            sub_id,
            TENANT_ID,
            level_change(PlanChangeMode::EndOfPeriod, held.id, PRICE_1M_ID),
        )
        .await
        .expect_err("another plan's price of the product is no level of this one");
    assert!(refused_with(&refused, "is not a level"), "{refused:?}");

    let refused = env
        .services()
        .preview_amendment(
            sub_id,
            TENANT_ID,
            level_change(
                PlanChangeMode::EndOfPeriod,
                held.id,
                PRICE_STARTER_PLATFORM_FEE_ID,
            ),
        )
        .await
        .expect_err("a price of another product");
    assert!(
        refused_with(&refused, "not a price of this component's product"),
        "{refused:?}"
    );

    let mut create = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_VOLUME_ID)
        .start_date(d(1, 1))
        .on_start()
        .no_trial()
        .request();
    create.price_components = Some(CreateSubscriptionComponents {
        parameterized_components: vec![],
        overridden_components: vec![ComponentOverride {
            component_id: COMP_LEVELS_ID,
            name: "Managed revenue".to_string(),
            price_entry: PriceEntry::Existing(PRICE_STARTER_PLATFORM_FEE_ID),
        }],
        extra_components: vec![],
        remove_components: vec![],
    });
    let refused = env
        .services()
        .insert_subscription(Actor::User { id: USER_ID }, create, TENANT_ID)
        .await
        .expect_err("a creation overriding with a price of another product");
    assert!(
        refused_with(&refused, "not a price of this component's product"),
        "{refused:?}"
    );
}

fn adjustable_levels(response: &portal::ListAdjustableComponentsResponse) -> Vec<(u64, bool)> {
    response
        .components
        .iter()
        .find(|c| c.component_id == COMP_LEVELS_ID.to_string())
        .expect("the usage-levels charge")
        .levels
        .iter()
        .map(|l| (l.included, l.current))
        .collect()
}

async fn journey(env: &TestEnv, self_serve_downgrade: bool) {
    env.store()
        .save_product_family_journey(
            TENANT_ID,
            ProductFamilyJourney {
                self_serve_downgrade,
                ..ProductFamilyJourney::default_for(PRODUCT_FAMILY_ID)
            },
        )
        .await
        .expect("journey");
}

async fn sales_led(env: &TestEnv, sales_led: bool) {
    use meteroid_store::domain::pricing_pages::PlanPricingPage;
    use meteroid_store::repositories::PricingPagesInterface;
    env.store()
        .update_plan_pricing_page(
            TENANT_ID,
            PlanPricingPage {
                plan_id: PLAN_VOLUME_ID,
                tagline: None,
                popular: false,
                cta_label: None,
                highlights: vec![],
                sales_led,
                hide_price: false,
                contact_url: Some("https://acme.test/contact".to_string()),
                product_order: vec![],
                hidden_product_ids: vec![],
            },
        )
        .await
        .expect("how the plan is sold");
}

async fn change_level(
    env: &TestEnv,
    sub_id: SubscriptionId,
    price: PriceId,
) -> Result<portal::ChangeLevelResponse, tonic::Status> {
    client(env)
        .change_level(request(
            env,
            portal::ChangeLevelRequest {
                subscription_id: sub_id.to_string(),
                component_id: COMP_LEVELS_ID.to_string(),
                price_id: price.to_string(),
            },
        ))
        .await
        .map(tonic::Response::into_inner)
}

/// In the portal a customer moves up at once and down at the period's end, only on a self-serve
/// plan, and down only when their journey lets them.
/// Rules: R-135, X5
#[rstest]
#[tokio::test]
async fn the_portal_changes_level_where_self_serve_allows_it(#[future] test_env: TestEnv) {
    let env = test_env.await;
    seed_volume_plans(&env).await;
    rank(&env, PLAN_VOLUME_ID, Some(1)).await;
    let today = chrono::Utc::now().date_naive();
    // Today: the portal changes a level within the current period.
    let sub_id = on_level(&env, CUST_UBER_ID, today, 125_000).await;

    let listed = client(&env)
        .list_adjustable_components(request(
            &env,
            portal::ListAdjustableComponentsRequest {
                subscription_id: sub_id.to_string(),
            },
        ))
        .await
        .expect("adjustable components")
        .into_inner();
    assert!(listed.can_self_serve && listed.level_downgrade);
    assert_eq!(
        adjustable_levels(&listed),
        vec![(50_000, false), (125_000, true), (250_000, false)]
    );

    let preview = client(&env)
        .preview_level_change(request(
            &env,
            portal::PreviewLevelChangeRequest {
                subscription_id: sub_id.to_string(),
                component_id: COMP_LEVELS_ID.to_string(),
                price_id: PRICE_250K_ID.to_string(),
            },
        ))
        .await
        .expect("preview the move up")
        .into_inner()
        .preview
        .expect("a preview");
    assert!(preview.immediate);
    assert_eq!(
        preview.change_direction,
        portal::ChangeDirection::Upgrade as i32
    );

    let moved = change_level(&env, sub_id, PRICE_250K_ID)
        .await
        .expect("a move up applies now");
    assert_eq!(
        moved.status,
        portal::AdjustmentStatus::AdjustmentCompleted as i32
    );
    assert_eq!(levels_row(&env, sub_id).await.price_id, Some(PRICE_250K_ID));

    // Down is the account team's when the journey keeps downgrades for them.
    journey(&env, false).await;
    let refused = change_level(&env, sub_id, PRICE_125K_ID)
        .await
        .expect_err("no self-serve downgrade");
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");

    // A sales-led plan is managed by the account team: not even up.
    journey(&env, true).await;
    sales_led(&env, true).await;
    let refused = change_level(&env, sub_id, PRICE_125K_ID)
        .await
        .expect_err("a sales-led plan");
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    sales_led(&env, false).await;

    let scheduled = change_level(&env, sub_id, PRICE_125K_ID)
        .await
        .expect("a move down is scheduled");
    assert_eq!(
        scheduled.status,
        portal::AdjustmentStatus::AdjustmentScheduled as i32
    );
    assert!(scheduled.scheduled_event_id.is_some());
    assert_eq!(levels_row(&env, sub_id).await.price_id, Some(PRICE_250K_ID));

    let refused = change_level(&env, sub_id, PRICE_50K_ID)
        .await
        .expect_err("a change waiting blocks another");
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
}

/// A change from Scale to Volume written ahead by main names no option: once it applies, the
/// subscription bills Volume's option (its Standard, by alias), so its usage-levels charge lists
/// Volume's levels.
#[tokio::test]
async fn a_change_written_by_main_lists_the_levels_of_the_new_plan() {
    let env = test_env().await;
    seed_volume_plans(&env).await;
    let mut create = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_SCALE_ID)
        .start_date(d(1, 1))
        .on_start()
        .no_trial()
        .request();
    create.price_components = Some(CreateSubscriptionComponents {
        parameterized_components: vec![ComponentParameterization {
            component_id: COMP_SCALE_ID,
            parameters: ComponentParameters {
                initial_slot_count: None,
                billing_period: None,
                committed_capacity: Some(1_000_000),
            },
        }],
        overridden_components: vec![],
        extra_components: vec![],
        remove_components: vec![],
    });
    let sub_id = env
        .services()
        .insert_subscription(Actor::User { id: USER_ID }, create, TENANT_ID)
        .await
        .expect("a Scale subscription")
        .id;

    env.services()
        .schedule_plan_change(
            Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_VOLUME_ID,
            vec![ComponentParameterization {
                component_id: COMP_LEVELS_ID,
                parameters: ComponentParameters {
                    initial_slot_count: None,
                    billing_period: None,
                    committed_capacity: Some(125_000),
                },
            }],
        )
        .await
        .expect("a change to Volume at period end");
    super::plan_change::as_written_by_main(&env, sub_id).await;
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_VOLUME_ID);
    assert_eq!(
        sub.plan_option_id,
        Some(crate::data::plans::default_option_id(
            PLAN_VERSION_VOLUME_ID
        ))
    );
    let levels = env
        .services()
        .list_capacity_levels(TENANT_ID, sub_id)
        .await
        .expect("the usage-levels charges");
    let [charge] = levels.as_slice() else {
        panic!("one usage-levels charge, got {levels:?}");
    };
    assert_eq!(
        charge
            .levels
            .iter()
            .map(|level| (level.included, level.current))
            .collect::<Vec<_>>(),
        vec![(50_000, false), (125_000, true), (250_000, false)]
    );
}

/// Waits until `n` backends of this database wait on a lock.
async fn waiting_on_locks(env: &TestEnv, n: i64) {
    use diesel::sql_types::BigInt;
    use diesel_async::RunQueryDsl;
    let mut conn = env.conn().await;
    for _ in 0..300 {
        let waiting: i64 = diesel::select(diesel::dsl::sql::<BigInt>(
            "(SELECT count(*) FROM pg_stat_activity \
              WHERE datname = current_database() AND wait_event_type = 'Lock')",
        ))
        .get_result(&mut conn)
        .await
        .expect("read pg_stat_activity");
        if waiting >= n {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("fewer than {n} backends wait on a lock");
}

/// A downgrade at the period's end and an upgrade now, both checked while nothing waits, then
/// queued on the customer in that order: the downgrade is scheduled, and the upgrade, re-reading
/// the pending changes under the lock, is refused rather than applied beneath it.
/// Rules: R-135, R-106
#[tokio::test]
async fn a_level_change_rereads_the_pending_changes_under_the_lock() {
    use diesel_async::SimpleAsyncConnection;

    let env = test_env().await;
    seed_volume_plans(&env).await;
    let today = chrono::Utc::now().date_naive();
    let sub_id = on_level(&env, CUST_UBER_ID, today, 125_000).await;
    let component = levels_row(&env, sub_id).await.id;

    let mut holder = env.conn().await;
    holder
        .batch_execute(&format!(
            "BEGIN; SELECT id FROM customer WHERE id = '{}' FOR NO KEY UPDATE;",
            CUST_UBER_ID.as_uuid()
        ))
        .await
        .expect("the customer held");
    let move_to = |price: PriceId| {
        let services = env.services().clone();
        tokio::spawn(async move {
            services
                .change_capacity_level(Actor::System, TENANT_ID, sub_id, component, price)
                .await
        })
    };
    let downgrade = move_to(PRICE_50K_ID);
    waiting_on_locks(&env, 1).await;
    let upgrade = move_to(PRICE_250K_ID);
    waiting_on_locks(&env, 2).await;
    holder.batch_execute("ROLLBACK").await.unwrap();

    let downgraded = downgrade.await.expect("the downgrade task");
    assert!(
        matches!(downgraded, Ok(LevelChangeOutcome::Scheduled(_))),
        "{downgraded:?}"
    );
    let upgraded = upgrade.await.expect("the upgrade task");
    let refused = upgraded.expect_err("a downgrade waits: the upgrade is refused under the lock");
    assert!(
        refused_with(&refused, "A change is already scheduled"),
        "{refused:?}"
    );
    assert_eq!(levels_row(&env, sub_id).await.price_id, Some(PRICE_125K_ID));
}
