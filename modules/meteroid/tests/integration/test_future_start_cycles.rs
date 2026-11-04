//! A subscription the lifecycle activates on a later start date is first invoiced then, on its
//! start date (FOLLOWUPS 74), and bills its periods in order: activation opens cycle 0 and its first
//! renewal is cycle 1, billing the first period's arrears and never its one-time fee again. Rows
//! activated before that fix heal at their renewal; rows an earlier release invoiced at creation
//! are not invoiced again. Manual activation opens cycle 0 the same way (FOLLOWUPS 73).

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Duration, NaiveDate, NaiveTime, Utc};
use common_domain::ids::{
    BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId, SubscriptionId,
};
use diesel_async::SimpleAsyncConnection;
use diesel_models::enums::{BillingPeriodEnum as DieselBillingPeriodEnum, SubscriptionStatusEnum};
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageClient, MockUsageDataParams, UsageData,
};
use meteroid_store::domain::{UsagePeriod, UsagePricingModel};
use rust_decimal::Decimal;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;

const RATE_CENTS: i64 = 2_000;
const SETUP_CENTS: i64 = 5_000;
/// 100 units at 0.10.
const USAGE_CENTS: i64 = 1_000;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// 100 bandwidth units in each of the first two monthly periods of 2024.
async fn env() -> TestEnv {
    let mut data = HashMap::new();
    for (start, end) in [
        (day(2024, 1, 1), day(2024, 2, 1)),
        (day(2024, 2, 1), day(2024, 3, 1)),
    ] {
        let period = UsagePeriod {
            start: start.and_time(NaiveTime::MIN),
            end: end.and_time(NaiveTime::MIN),
        };
        data.insert(
            MockUsageDataParams {
                metric_id: METRIC_BANDWIDTH,
                period_start: period.start,
                period_end: period.end,
            },
            UsageData {
                data: vec![GroupedUsageData {
                    value: Decimal::from(100),
                    dimensions: HashMap::new(),
                }],
                period,
            },
        );
    }
    test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient { data })).await
}

/// 20.00 a month in advance, 0.10 a bandwidth unit in arrears and a 50.00 setup fee.
async fn plan(env: &TestEnv) -> PlanVersionId {
    let version_id = PlanVersionId::new();
    let mut conn = env.conn().await;
    PlanSeed::new(PlanId::new(), "Setup and usage", version_id)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(RATE_CENTS, 2),
            ),
            SeedComp::usage(
                PriceComponentId::new(),
                "Bandwidth",
                PRODUCT_BANDWIDTH_ID,
                METRIC_BANDWIDTH,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit {
                    rate: Decimal::new(10, 2),
                },
            ),
            SeedComp::one_time(
                PriceComponentId::new(),
                "Setup Fee",
                PRODUCT_ONETIME_SETUP_ID,
                PriceId::new(),
                Decimal::new(SETUP_CENTS, 2),
                1,
            ),
        ])
        .seed(&mut conn)
        .await
        .expect("seeding the plan");
    version_id
}

async fn sql(env: &TestEnv, statements: &str) {
    env.conn()
        .await
        .batch_execute(statements)
        .await
        .unwrap_or_else(|e| panic!("{e:?}\n{statements}"));
}

async fn totals(env: &TestEnv, id: SubscriptionId) -> Vec<i64> {
    env.get_invoices(id).await.iter().map(|i| i.total).collect()
}

async fn invoice_cycles(env: &TestEnv, id: SubscriptionId) -> Vec<Option<i32>> {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::invoice::dsl as i;
    i::invoice
        .filter(i::subscription_id.eq(id.as_uuid()))
        .order(i::invoice_date.asc())
        .select(i::cycle_index)
        .load(&mut env.conn().await)
        .await
        .unwrap()
}

/// Created to start in ten days, then moved to start on 2024-01-01 so the lifecycle activates it.
async fn lifecycle_activated(env: &TestEnv, version: PlanVersionId) -> SubscriptionId {
    let id = subscription()
        .plan_version(version)
        .start_date(Utc::now().date_naive() + Duration::days(10))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let created = env.get_subscription(id).await;
    assert_eq!(created.status, SubscriptionStatusEnum::PendingActivation);
    assert_eq!(created.cycle_index, None);
    assert!(
        env.get_invoices(id).await.is_empty(),
        "nothing is invoiced before the start date"
    );
    move_start_back(env, id).await;
    id
}

/// Moves a subscription starting later, its invoices and its components' effective dates to
/// 2024-01-01, so the lifecycle activates it.
async fn move_start_back(env: &TestEnv, id: SubscriptionId) {
    sql(
        env,
        &format!(
            "UPDATE subscription SET start_date = '2024-01-01', billing_start_date = '2024-01-01',
                current_period_start = '2024-01-01', current_period_end = '2024-01-01',
                billing_day_anchor = 1, activated_at = '2024-01-01 00:00:00',
                created_at = '2023-12-20 09:00:00'
             WHERE id = '{id}';
             UPDATE invoice SET invoice_date = '2024-01-01', billing_period_start = '2024-01-01'
             WHERE subscription_id = '{id}';
             UPDATE subscription_component SET effective_from = '2024-01-01'
             WHERE subscription_id = '{id}';",
            id = id.as_uuid()
        ),
    )
    .await;
}

#[tokio::test]
async fn a_lifecycle_activated_subscription_renews_into_cycle_one() {
    let env = env().await;
    let version = plan(&env).await;
    let id = lifecycle_activated(&env, version).await;

    env.process_cycles().await;
    assert_eq!(env.get_subscription(id).await.cycle_index, Some(0));
    assert_eq!(totals(&env, id).await, vec![RATE_CENTS + SETUP_CENTS]);

    env.process_cycles().await;
    assert_eq!(env.get_subscription(id).await.cycle_index, Some(1));
    assert_eq!(
        totals(&env, id).await,
        vec![RATE_CENTS + SETUP_CENTS, RATE_CENTS + USAGE_CENTS],
        "the renewal bills the first period's usage, and the setup fee only once"
    );
    assert_eq!(invoice_cycles(&env, id).await.last(), Some(&Some(1)));
}

/// A paid trial starting later bills like any period once the lifecycle starts it: its first
/// period is the one already invoiced (99.00, never twice), the trial's end only makes it Active,
/// and the renewal is cycle 1.
#[tokio::test]
async fn a_paid_trial_starting_later_bills_its_period_and_ends_on_schedule() {
    let env = env().await;
    let id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID)
        .start_date(Utc::now().date_naive() + Duration::days(10))
        .on_start()
        .trial_days(7)
        .no_auto_charge()
        .create(env.services())
        .await;
    assert_eq!(
        env.get_subscription(id).await.status,
        SubscriptionStatusEnum::PendingActivation
    );
    assert!(env.get_invoices(id).await.is_empty());
    move_start_back(&env, id).await;

    env.process_cycles().await;
    let started = env.get_subscription(id).await;
    assert_eq!(
        (
            started.status,
            started.cycle_index,
            started.current_period_end
        ),
        (
            SubscriptionStatusEnum::Active,
            Some(0),
            Some(day(2024, 2, 1))
        ),
        "billed for its period, the trial ended on Jan 8 by its scheduled event"
    );
    assert_eq!(totals(&env, id).await, vec![9_900]);

    env.process_cycles().await;
    assert_eq!(env.get_subscription(id).await.cycle_index, Some(1));
    assert_eq!(totals(&env, id).await, vec![9_900, 9_900]);
}

/// A subscription starting later that an earlier release (or an old pod during the deploy)
/// invoiced at creation: its start finds the period invoiced and bills it no second time.
#[tokio::test]
async fn a_future_start_invoiced_at_creation_is_not_invoiced_again_at_its_start() {
    let env = env().await;
    let version = plan(&env).await;
    let id = subscription()
        .plan_version(version)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    assert_eq!(totals(&env, id).await, vec![RATE_CENTS + SETUP_CENTS]);
    sql(
        &env,
        &format!(
            "UPDATE subscription SET status = 'PENDING_ACTIVATION', cycle_index = NULL,
                next_cycle_action = 'ACTIVATE_SUBSCRIPTION'
             WHERE id = '{id}';
             UPDATE invoice SET cycle_index = NULL WHERE subscription_id = '{id}';",
            id = id.as_uuid()
        ),
    )
    .await;
    move_start_back(&env, id).await;

    env.process_cycles().await;
    let started = env.get_subscription(id).await;
    assert_eq!(
        (started.status, started.cycle_index),
        (SubscriptionStatusEnum::Active, Some(0))
    );
    assert_eq!(totals(&env, id).await, vec![RATE_CENTS + SETUP_CENTS]);

    env.process_cycles().await;
    assert_eq!(
        totals(&env, id).await,
        vec![RATE_CENTS + SETUP_CENTS, RATE_CENTS + USAGE_CENTS]
    );
}

/// Manual activation starts the subscription as the lifecycle would: cycle 0 is billed at once,
/// its setup fee included, and the first renewal is cycle 1.
#[tokio::test]
async fn a_manually_activated_subscription_bills_its_first_period() {
    let env = env().await;
    let version = plan(&env).await;
    let id = subscription()
        .plan_version(version)
        .start_date(day(2024, 1, 1))
        .manual()
        .no_trial()
        .create(env.services())
        .await;
    assert!(env.get_invoices(id).await.is_empty());

    env.services()
        .activate_subscription_manual(TENANT_ID, id)
        .await
        .expect("activated");
    let activated = env.get_subscription(id).await;
    assert_eq!(
        (activated.status, activated.cycle_index),
        (SubscriptionStatusEnum::Active, Some(0))
    );
    assert_eq!(totals(&env, id).await, vec![RATE_CENTS + SETUP_CENTS]);

    env.process_cycles().await;
    assert_eq!(env.get_subscription(id).await.cycle_index, Some(1));
    assert_eq!(
        totals(&env, id).await,
        vec![RATE_CENTS + SETUP_CENTS, RATE_CENTS + USAGE_CENTS],
        "the renewal bills the first period's usage, and the setup fee only once"
    );
}

/// An old pod invoiced the subscription at creation and left the invoice a draft: its start
/// issues that draft, once, and creates no other.
#[tokio::test]
async fn a_draft_invoiced_at_creation_is_issued_once_at_the_start() {
    let env = env().await;
    let version = plan(&env).await;
    let id = subscription()
        .plan_version(version)
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    let draft = env.get_invoices(id).await;
    assert_eq!(draft.len(), 1);
    assert_eq!(
        draft[0].status,
        meteroid_store::domain::enums::InvoiceStatusEnum::Draft
    );
    sql(
        &env,
        &format!(
            "UPDATE subscription SET status = 'PENDING_ACTIVATION', cycle_index = NULL,
                next_cycle_action = 'ACTIVATE_SUBSCRIPTION', auto_advance_invoices = true
             WHERE id = '{id}';
             UPDATE invoice SET cycle_index = NULL, auto_advance = true
             WHERE subscription_id = '{id}';",
            id = id.as_uuid()
        ),
    )
    .await;
    move_start_back(&env, id).await;

    env.process_cycles().await;
    env.process_due_events().await;
    let invoices = env.get_invoices(id).await;
    assert_eq!(
        invoices
            .iter()
            .map(|i| (i.id, i.status, i.total))
            .collect::<Vec<_>>(),
        vec![(
            draft[0].id,
            meteroid_store::domain::enums::InvoiceStatusEnum::Finalized,
            RATE_CENTS + SETUP_CENTS
        )]
    );
}

/// Cancelled before its start date, a subscription starting later is never invoiced and books no
/// MRR.
#[tokio::test]
async fn a_future_start_cancelled_before_its_start_is_never_invoiced() {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::bi_mrr_movement_log::dsl as m;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let env = env().await;
    let version = plan(&env).await;
    let id = subscription()
        .plan_version(version)
        .start_date(Utc::now().date_naive() + Duration::days(10))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await
        .expect("cancel before the start");
    env.process_cycles().await;

    let cancelled = env.get_subscription(id).await;
    assert!(cancelled.status.is_terminal(), "{:?}", cancelled.status);
    assert!(env.get_invoices(id).await.is_empty());
    assert_eq!(cancelled.mrr_cents, 0);
    let movements: i64 = m::bi_mrr_movement_log
        .filter(m::tenant_id.eq(TENANT_ID))
        .count()
        .get_result(&mut env.conn().await)
        .await
        .expect("MRR movements");
    assert_eq!(movements, 0);
}

/// A usage-only subscription under a minimum issues no invoice at its start, yet the lifecycle
/// starting it books the floor as New Business on its start date; nothing before.
#[tokio::test]
async fn a_usage_only_minimum_starting_later_is_booked_at_its_start() {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::enums::MrrMovementType;
    use diesel_models::schema::bi_mrr_movement_log::dsl as m;

    let env = env().await;
    let version = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Usage under a floor", version)
        .components(vec![SeedComp::usage(
            PriceComponentId::new(),
            "Bandwidth",
            PRODUCT_BANDWIDTH_ID,
            METRIC_BANDWIDTH,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            UsagePricingModel::PerUnit {
                rate: Decimal::new(10, 2),
            },
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("seeding the plan");
    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(
            &diesel_models::plan_version_adjustments::PlanVersionAdjustmentRowNew {
                kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                plan_version_id: version,
                amount_subunit: 9_000,
                includes_all_components: true,
            },
        )
        .execute(&mut env.conn().await)
        .await
        .unwrap();

    let id = lifecycle_activated(&env, version).await;
    assert_eq!(env.get_subscription(id).await.mrr_cents, 0);
    sql(
        &env,
        &format!(
            "UPDATE subscription_event SET applies_to = '2024-01-01'
             WHERE subscription_id = '{}' AND event_type = 'CREATED'",
            id.as_uuid()
        ),
    )
    .await;

    env.process_cycles().await;
    assert_eq!(env.get_subscription(id).await.cycle_index, Some(0));
    assert!(env.get_invoices(id).await.is_empty());
    assert_eq!(env.get_subscription(id).await.mrr_cents, 9_000);
    let movements: Vec<(MrrMovementType, i64, NaiveDate)> = m::bi_mrr_movement_log
        .filter(m::subscription_id.eq(id))
        .select((m::movement_type, m::net_mrr_change, m::applies_to))
        .load(&mut env.conn().await)
        .await
        .unwrap();
    assert_eq!(
        movements,
        vec![(MrrMovementType::NewBusiness, 9_000, day(2024, 1, 1))]
    );
}

/// A paid trial manually activated bills its first period at once and ends on schedule, even when
/// the trial is set on the subscription and not on its plan version (FOLLOWUPS 73).
#[tokio::test]
async fn a_manually_activated_paid_trial_bills_its_first_period() {
    let env = env().await;
    sql(
        &env,
        &format!(
            "UPDATE plan_version SET trial_duration_days = NULL, trial_is_free = false
             WHERE id = '{}'",
            PLAN_VERSION_PAID_TRIAL_ID.as_uuid()
        ),
    )
    .await;
    let id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID)
        .start_date(day(2024, 1, 1))
        .manual()
        .trial_days(7)
        .no_auto_charge()
        .create(env.services())
        .await;
    assert!(env.get_invoices(id).await.is_empty());

    env.services()
        .activate_subscription_manual(TENANT_ID, id)
        .await
        .expect("activated");
    let activated = env.get_subscription(id).await;
    assert_eq!(
        (
            activated.status,
            activated.cycle_index,
            activated.current_period_end
        ),
        (
            SubscriptionStatusEnum::TrialActive,
            Some(0),
            Some(day(2024, 2, 1))
        )
    );
    assert_eq!(totals(&env, id).await, vec![9_900]);

    env.process_due_events().await;
    assert_eq!(
        env.get_subscription(id).await.status,
        SubscriptionStatusEnum::Active,
        "the trial ended on January 8"
    );
    assert_eq!(totals(&env, id).await, vec![9_900]);
}

/// A row the lifecycle activated before activation opened cycle 0 still has none: its renewal is
/// cycle 1 all the same.
#[tokio::test]
async fn a_renewal_without_a_cycle_is_cycle_one() {
    let env = env().await;
    let version = plan(&env).await;
    let id = subscription()
        .plan_version(version)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    sql(
        &env,
        &format!(
            "UPDATE subscription SET cycle_index = NULL WHERE id = '{}'",
            id.as_uuid()
        ),
    )
    .await;

    env.process_cycles().await;
    assert_eq!(env.get_subscription(id).await.cycle_index, Some(1));
    assert_eq!(
        totals(&env, id).await,
        vec![RATE_CENTS + SETUP_CENTS, RATE_CENTS + USAGE_CENTS]
    );
}

/// The replay's entry points rewrite a sandbox's past only.
#[tokio::test]
async fn replay_entry_points_refuse_a_tenant_that_is_not_a_sandbox() {
    use common_domain::actor::Actor;
    use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
    use meteroid_store::domain::enums::SubscriptionActivationCondition;
    use meteroid_store::domain::{CreateSubscription, SubscriptionNew};
    use meteroid_store::services::InvoiceDelivery;
    use meteroid_store::services::sandbox_replay::SandboxReplayInterface;

    let env = env().await;
    let services = env.services();
    let at = Utc::now() - Duration::days(30);
    let refused = |result: Result<(), String>| {
        let error = result.expect_err("refused outside a sandbox");
        assert!(error.contains("only a sandbox"), "{error}");
    };
    let subscription = CreateSubscription {
        subscription: SubscriptionNew {
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_USAGE_ID,
            net_terms: None,
            invoice_memo: None,
            invoice_threshold: None,
            start_date: at.date_naive(),
            end_date: None,
            billing_start_date: None,
            activation_condition: SubscriptionActivationCondition::OnStart,
            trial_duration: None,
            billing_day_anchor: None,
            payment_methods_config: None,
            auto_advance_invoices: true,
            charge_automatically: false,
            purchase_order: None,
            backdate_invoices: true,
            skip_checkout_session: false,
            skip_past_invoices: false,
            usage_filter: None,
            custom_properties: serde_json::Value::Null,
        },
        price_components: None,
        add_ons: None,
        coupons: None,
        entitlements: vec![],
        minimum_override: None,
        drop_minimum: false,
    };
    refused(
        services
            .insert_subscription_started_at(
                Actor::System,
                subscription,
                TENANT_ID,
                at,
                InvoiceDelivery::Suppressed,
            )
            .await
            .map(|_| ())
            .map_err(|e| format!("{e:?}")),
    );
    refused(
        services
            .charge_invoice_at(
                TENANT_ID,
                common_domain::ids::InvoiceId::new(),
                CUST_UBER_PAYMENT_METHOD_ID,
                at,
                InvoiceDelivery::Suppressed,
            )
            .await
            .map(|_| ())
            .map_err(|e| format!("{e:?}")),
    );
    refused(
        services
            .redeem_coupon_credit_code_at(TENANT_ID, CUST_UBER_ID, "LAUNCH", at)
            .await
            .map(|_| ())
            .map_err(|e| format!("{e:?}")),
    );
    refused(
        services
            .replay_automatic_top_up_at(
                TENANT_ID,
                common_domain::ids::WalletId::new(),
                1_000,
                at,
                InvoiceDelivery::Suppressed,
            )
            .await
            .map(|_| ())
            .map_err(|e| format!("{e:?}")),
    );
    refused(
        services
            .create_coupon_at(
                Actor::System,
                CouponNew {
                    code: "LAUNCH".to_owned(),
                    description: "Launch".to_owned(),
                    tenant_id: TENANT_ID,
                    discount: CouponDiscount::Credit {
                        currency: "EUR".to_owned(),
                        amount: Decimal::from(10),
                        expires_in_days: None,
                        scope: Default::default(),
                    },
                    expires_at: None,
                    redemption_limit: None,
                    recurring_value: None,
                    reusable: false,
                    plan_ids: vec![],
                },
                at,
            )
            .await
            .map(|_| ())
            .map_err(|e| format!("{e:?}")),
    );
}
