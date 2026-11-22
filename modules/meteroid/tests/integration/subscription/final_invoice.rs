//! A subscription cancelled at once on the first day of a period already invoiced bills what that
//! day consumed up to the cancellation on a final invoice of its own, apart from the period's key.
//! Usage Alpha: €10.00 a month in advance, API calls at €0.10; the mock meters 100 calls (€10.00)
//! in any window, the final one included.
use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Datelike, NaiveDate, NaiveTime, Utc};
use rust_decimal::Decimal;
use uuid::uuid;

use common_domain::actor::Actor;
use common_domain::ids::*;
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_store::clients::usage::{GroupedUsageData, MutableMockUsageClient, UsageData};
use meteroid_store::domain::enums::{CreditNoteStatus, InvoiceStatusEnum, InvoiceType};
use meteroid_store::domain::{Invoice, UsagePeriod};
use meteroid_store::repositories::credit_notes::CreditNoteInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env_with_seed_and_usage};
use crate::helpers::credit_recompute::clear_of_midnight;
use crate::meteroid_it::container::SeedLevel;

const CALLS: i64 = 100;

/// Capacity: €10.00 a month for 50 calls, €0.10 per call beyond.
const PLAN_VERSION_CAPACITY_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("019438e0-0f11-7000-8000-0000000000f1"));
const COMP_CAPACITY_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0f12-7000-8000-0000000000f2"));
const PRICE_CAPACITY_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0f13-7000-8000-0000000000f3"));

async fn env() -> TestEnv {
    clear_of_midnight(120).await;
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    let day = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    usage.set_usage_for_any_nonempty_window(
        METRIC_BANDWIDTH,
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(CALLS),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod {
                start: day.and_time(NaiveTime::MIN),
                end: day.and_time(NaiveTime::MIN),
            },
        },
    );
    // A negative grace settles a termination inline.
    env.set_consolidate_and_grace(false, -1).await;
    env
}

async fn cancel_now(env: &TestEnv, id: SubscriptionId) {
    env.services()
        .cancel_subscription(
            Actor::System,
            id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await
        .expect("cancelled");
    env.process_due_events().await;
}

/// Started today on `plan_version`, its first invoice issued at creation, then cancelled at once.
async fn ended_on_its_first_day(
    env: &TestEnv,
    plan_version: PlanVersionId,
    coupon: Option<CouponId>,
) -> SubscriptionId {
    let mut builder = subscription()
        .plan_version(plan_version)
        .start_date(Utc::now().date_naive())
        .on_start()
        .no_trial();
    if let Some(coupon) = coupon {
        builder = builder.coupon(coupon);
    }
    let id = builder.create(env.services()).await;
    let opening = env.get_invoices(id).await;
    assert_eq!(opening.len(), 1);
    assert_eq!(opening[0].status, InvoiceStatusEnum::Finalized);
    cancel_now(env, id).await;
    id
}

fn finals(invoices: &[Invoice]) -> Vec<&Invoice> {
    invoices
        .iter()
        .filter(|invoice| invoice.invoice_type == InvoiceType::Final)
        .collect()
}

fn the_final(invoices: &[Invoice]) -> Invoice {
    let live = finals(invoices)
        .into_iter()
        .filter(|invoice| invoice.status != InvoiceStatusEnum::Void)
        .collect::<Vec<_>>();
    let [final_invoice] = live[..] else {
        panic!("one live final invoice, not {}", live.len());
    };
    assert_eq!(final_invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(final_invoice.consolidated_into_invoice_id, None);
    final_invoice.clone()
}

/// What the subscription's live invoices billed for its usage.
fn usage_billed(invoices: &[Invoice]) -> i64 {
    invoices
        .iter()
        .filter(|invoice| invoice.status != InvoiceStatusEnum::Void)
        .flat_map(|invoice| &invoice.line_items)
        .filter(|line| line.metric_id == Some(METRIC_BANDWIDTH))
        .map(|line| line.amount_subtotal)
        .sum()
}

async fn churn_movements(env: &TestEnv, id: SubscriptionId) -> i64 {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::bi_mrr_movement_log::dsl as m;
    m::bi_mrr_movement_log
        .filter(m::subscription_id.eq(id))
        .filter(m::movement_type.eq(diesel_models::enums::MrrMovementType::Churn))
        .count()
        .get_result(&mut env.conn().await)
        .await
        .expect("churn movements")
}

/// The day's 100 calls are billed once, on the final invoice, read from midnight to the
/// cancellation; the first invoice's fee comes back whole; the churn is booked once.
#[tokio::test]
async fn a_cancellation_on_the_first_day_bills_its_usage_once_and_churns_once() {
    let env = env().await;
    let id = ended_on_its_first_day(&env, PLAN_VERSION_USAGE_ALPHA_ID, None).await;
    let today = Utc::now().date_naive();

    let invoices = env.get_invoices(id).await;
    let final_invoice = the_final(&invoices);
    assert_eq!(final_invoice.billing_period_start, today);
    assert_eq!(final_invoice.subtotal, 1_000);
    assert_eq!(final_invoice.total, 1_000);
    assert!(final_invoice.line_items.iter().all(|line| {
        line.metric_id == Some(METRIC_BANDWIDTH)
            && line.start_date == today
            && line.end_date == today
            && line.usage_ends_at.is_some()
    }));
    assert_eq!(usage_billed(&invoices), 1_000);

    let opening = invoices
        .iter()
        .find(|invoice| invoice.invoice_type == InvoiceType::Recurring)
        .expect("the first invoice");
    let credited: i64 = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, opening.id)
        .await
        .expect("credit notes")
        .iter()
        .filter(|note| note.status == CreditNoteStatus::Finalized && note.voided_at.is_none())
        .map(|note| note.total.abs())
        .sum();
    assert_eq!(credited, opening.total);

    assert_eq!(env.get_subscription(id).await.mrr_cents, 0);
    assert_eq!(churn_movements(&env, id).await, 1);
}

/// A forever €3.00 coupon takes €3.00 off the final invoice as off every invoice (Stripe's
/// `amount_off`); a 20% one takes 20% of its €10.00.
#[tokio::test]
async fn coupons_apply_to_the_final_invoice_as_to_any_invoice() {
    let env = env().await;
    let fixed = env.create_fixed_coupon("FIXED3", 3, "EUR").await;
    let id = ended_on_its_first_day(&env, PLAN_VERSION_USAGE_ALPHA_ID, Some(fixed)).await;
    let final_invoice = the_final(&env.get_invoices(id).await);
    assert_eq!(final_invoice.subtotal, 1_000);
    assert_eq!(final_invoice.discount, 300);
    assert_eq!(final_invoice.total, 700);

    let percent = env.create_percentage_coupon("PERCENT20", 20).await;
    let id = ended_on_its_first_day(&env, PLAN_VERSION_USAGE_ALPHA_ID, Some(percent)).await;
    let final_invoice = the_final(&env.get_invoices(id).await);
    assert_eq!(final_invoice.subtotal, 1_000);
    assert_eq!(final_invoice.discount, 200);
    assert_eq!(final_invoice.total, 800);
}

/// The period's fee comes back whole, its allowance with it: the final invoice bills all 100 calls
/// at €0.10, none included (R-016).
#[tokio::test]
async fn the_final_invoice_counts_none_of_the_credited_period_s_allowance() {
    let env = env().await;
    // A capacity price needs a product priced by capacity, not the usage-priced API calls.
    PlanSeed::new(PlanId::new(), "Capacity", PLAN_VERSION_CAPACITY_ID)
        .components(vec![SeedComp::capacity(
            COMP_CAPACITY_ID,
            "Capacity Bandwidth",
            PRODUCT_CAPACITY_ID,
            METRIC_BANDWIDTH,
            PRICE_CAPACITY_ID,
            DieselBillingPeriodEnum::Monthly,
            Decimal::new(1000, 2),
            50,
            Decimal::new(10, 2),
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("capacity plan");

    let id = ended_on_its_first_day(&env, PLAN_VERSION_CAPACITY_ID, None).await;
    let final_invoice = the_final(&env.get_invoices(id).await);
    assert_eq!(final_invoice.subtotal, 1_000);
    assert_eq!(final_invoice.total, 1_000);
}

/// A voided final invoice is billed again only by its regeneration, once, on the same window.
#[tokio::test]
async fn regenerating_a_voided_final_invoice_bills_its_window_once() {
    let env = env().await;
    let id = ended_on_its_first_day(&env, PLAN_VERSION_USAGE_ALPHA_ID, None).await;
    let voided = the_final(&env.get_invoices(id).await);
    env.try_void_invoice(voided.id)
        .await
        .expect("void the final invoice");
    let voided = env
        .get_invoices(id)
        .await
        .into_iter()
        .find(|invoice| invoice.id == voided.id)
        .expect("the voided final invoice");
    assert_eq!(voided.status, InvoiceStatusEnum::Void);
    assert_eq!(usage_billed(&env.get_invoices(id).await), 0);
    assert!(
        env.services()
            .can_regenerate_invoice(TENANT_ID, &voided)
            .await
            .expect("can regenerate")
    );

    let regenerated = env
        .services()
        .regenerate_invoice(Actor::System, TENANT_ID, voided.id)
        .await
        .expect("regenerated")
        .invoice;
    assert_eq!(regenerated.invoice_type, InvoiceType::Final);
    assert_eq!(regenerated.status, InvoiceStatusEnum::Finalized);
    assert_eq!(regenerated.total, 1_000);

    let invoices = env.get_invoices(id).await;
    assert_eq!(the_final(&invoices).id, regenerated.id);
    assert_eq!(usage_billed(&invoices), 1_000);
    assert!(
        !env.services()
            .can_regenerate_invoice(TENANT_ID, &voided)
            .await
            .expect("can regenerate")
    );
    assert!(
        env.services()
            .regenerate_invoice(Actor::System, TENANT_ID, voided.id)
            .await
            .is_err()
    );
    assert_eq!(churn_movements(&env, id).await, 1);
}

/// On an entity that consolidates, the renewals of the day merge while the final invoice of the
/// one cancelled that day is issued alone.
#[tokio::test]
async fn a_final_invoice_is_issued_apart_from_the_consolidated_renewals() {
    let env = env().await;
    env.set_consolidate_and_grace(true, -1).await;
    let today = Utc::now().date_naive();
    // A month back with today's day of the month (Mar 30 has no Feb 30).
    let (months, start) = (1..=12)
        .filter_map(|months| {
            today
                .checked_sub_months(chrono::Months::new(months))
                .map(|start| (months, start))
        })
        .find(|(_, start)| start.day() == today.day())
        .expect("a past month with today's day");
    let renewing = || {
        subscription()
            .customer(CUST_UBER_ID)
            .plan_version(PLAN_VERSION_USAGE_ALPHA_ID)
            .start_date(start)
            .on_start()
            .no_trial()
            .create(env.services())
    };
    let (a, b) = (renewing().await, renewing().await);
    for _ in 0..months {
        env.process_cycles().await;
        env.process_due_events().await;
    }
    // The merge waits out its floored grace from midnight: bring it to now.
    {
        use diesel_async::RunQueryDsl;
        diesel::sql_query(
            "UPDATE invoice SET finalize_after = now() - interval '1 minute' \
             WHERE tenant_id = $1 AND finalize_after IS NOT NULL",
        )
        .bind::<diesel::sql_types::Uuid, _>(TENANT_ID)
        .execute(&mut env.conn().await)
        .await
        .expect("finalizations due");
    }
    env.process_due_events().await;
    let renewal_of = |invoices: Vec<Invoice>| {
        invoices
            .into_iter()
            .find(|invoice| {
                invoice.invoice_type == InvoiceType::Recurring
                    && invoice.billing_period_start == today
            })
            .expect("today's renewal")
    };
    let (renewal_a, renewal_b) = (
        renewal_of(env.get_invoices(a).await),
        renewal_of(env.get_invoices(b).await),
    );
    let parent = renewal_a
        .consolidated_into_invoice_id
        .expect("A's renewal merged");
    assert_eq!(renewal_b.consolidated_into_invoice_id, Some(parent));

    cancel_now(&env, a).await;
    let final_invoice = the_final(&env.get_invoices(a).await);
    assert_eq!(final_invoice.billing_period_start, today);
    assert_eq!(final_invoice.total, 1_000);
    assert!(finals(&env.get_invoices(b).await).is_empty());
}
