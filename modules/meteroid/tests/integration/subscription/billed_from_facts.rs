//! Period invoices billed from the facts: the reconcile watermark leaves the
//! periods it settled alone.

use chrono::NaiveDate;
use common_domain::ids::SubscriptionId;
use diesel_models::subscriptions::SubscriptionRow;
use meteroid_store::domain::Invoice;
use rust_decimal::Decimal;

use crate::data::ids::*;
use crate::harness::{subscription, test_env, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).unwrap()
}

/// The invoice's lines by name, then start: a draft and its issued copy order them differently.
fn lines_of(invoice: &Invoice) -> Vec<(String, NaiveDate, NaiveDate, i64)> {
    let mut lines: Vec<(String, NaiveDate, NaiveDate, i64)> = invoice
        .line_items
        .iter()
        .map(|line| {
            (
                line.name.clone(),
                line.start_date,
                line.end_date,
                line.amount_subtotal,
            )
        })
        .collect();
    lines.sort();
    lines
}

/// A period ending by the watermark was settled before billing moved to the facts: its usage is
/// never billed again, the periods after it are. Monthly 20.00 in advance, 0.10 per unit of 100
/// units each month, watermark on Feb 1 (`Platform Fee` is the plan's 20.00 rate):
/// - Feb 1 bills February's 20.00 and not January's usage, its period [Jan 1, Feb 1) ending by it;
/// - Mar 1 bills February's usage, 100 × 0.10 = 10.00, and March's 20.00.
/// Rules: R-094
#[tokio::test]
async fn a_period_ending_by_the_watermark_is_never_billed_again() {
    let usage_client = super::usage::build_usage_client(
        Decimal::from(100),
        &[
            (day(1, 1), day(2, 1)),
            (day(2, 1), day(3, 1)),
            (day(3, 1), day(4, 1)),
        ],
    );
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, std::sync::Arc::new(usage_client)).await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    {
        let mut conn = env.conn().await;
        SubscriptionRow::set_reconcile_from(&mut conn, sub_id, Some(day(2, 1)), None)
            .await
            .expect("watermark");
    }

    env.process_cycles().await;
    env.process_cycles().await;
    let invoices = env.get_invoices(sub_id).await;
    let periods: Vec<Vec<(String, NaiveDate, NaiveDate, i64)>> =
        invoices.iter().map(lines_of).collect();
    assert_eq!(
        periods,
        vec![
            vec![("Platform Fee".to_owned(), day(1, 1), day(2, 1), 2_000)],
            vec![("Platform Fee".to_owned(), day(2, 1), day(3, 1), 2_000)],
            vec![
                ("Bandwidth".to_owned(), day(2, 1), day(3, 1), 1_000),
                ("Platform Fee".to_owned(), day(3, 1), day(4, 1), 2_000),
            ],
        ]
    );
}

/// Seats bought while a removal waits for the period's end add to the count it leaves: Starter
/// with 10 seats, 4 removed on Feb 10 from Mar 1, 2 bought on Feb 12. Mar 1 bills 8 seats,
/// 2900 + 1000 × 8 = 10900; the removal's own count, 6, billed 8900.
/// Rules: R-108
#[rstest::rstest]
#[tokio::test]
async fn seats_bought_while_a_removal_waits_add_to_its_count(
    #[future] test_env: crate::harness::TestEnv,
) {
    use chrono::NaiveTime;
    use meteroid_store::domain::SlotUpgradeBillingMode;
    let env = test_env.await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let change = |seats: i32, on: NaiveDate| {
        let env = &env;
        async move {
            env.services()
                .update_subscription_slots_for_test(
                    TENANT_ID,
                    sub_id,
                    COMP_STARTER_SEATS_ID,
                    seats,
                    SlotUpgradeBillingMode::Optimistic,
                    Some(on.and_time(NaiveTime::MIN)),
                )
                .await
                .expect("change seats");
        }
    };
    change(9, day(1, 2)).await;
    env.process_cycles().await;
    change(-4, day(2, 10)).await;
    change(2, day(2, 12)).await;
    env.process_cycles().await;

    let march = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| invoice.billing_period_start == day(3, 1))
        .expect("the Mar 1 invoice");
    assert_eq!(march.subtotal, 10_900, "{:?}", lines_of(&march));
}

/// Two removals waiting for the same period's end each take their seats once: Starter with 10
/// seats, 1 removed on Feb 10 and 1 on Feb 11, both from Mar 1. The ledger holds 8 on Mar 1, so
/// a third removal of 7 is accepted, and Mar 1 bills 2900 + 1000 × 8 = 10900; read from the
/// latest removal's count, the earlier one counted twice, 7.
/// Rules: R-108
#[rstest::rstest]
#[tokio::test]
async fn removals_waiting_for_the_same_end_take_their_seats_once(
    #[future] test_env: crate::harness::TestEnv,
) {
    use chrono::NaiveTime;
    use meteroid_store::domain::SlotUpgradeBillingMode;
    use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;
    let env = test_env.await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let change = |seats: i32, on: NaiveDate| {
        let env = &env;
        async move {
            env.services()
                .update_subscription_slots_for_test(
                    TENANT_ID,
                    sub_id,
                    COMP_STARTER_SEATS_ID,
                    seats,
                    SlotUpgradeBillingMode::Optimistic,
                    Some(on.and_time(NaiveTime::MIN)),
                )
                .await
        }
    };
    change(9, day(1, 2)).await.expect("10 seats");
    env.process_cycles().await;
    change(-1, day(2, 10)).await.expect("one seat off");
    change(-1, day(2, 11)).await.expect("another seat off");
    let seats = env
        .store()
        .get_active_slots_value(
            TENANT_ID,
            sub_id,
            QUANTITY_SEATS_ID,
            Some(day(3, 1).and_time(NaiveTime::MIN)),
        )
        .await
        .expect("seats on Mar 1");
    assert_eq!(seats, 8);
    env.process_cycles().await;
    let march = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| invoice.billing_period_start == day(3, 1))
        .expect("the Mar 1 invoice");
    assert_eq!(march.subtotal, 10_900, "{:?}", lines_of(&march));
    change(-7, day(3, 2))
        .await
        .expect("down to the minimum of one seat");
}

/// Each invoice's period start and lines.
type PeriodLines = (NaiveDate, Vec<(String, NaiveDate, NaiveDate, i64)>);

/// The release's watermark backfills (`Reconcile watermark`, `Gaps at the cursor`, `Drafts never
/// issued`) run on the subscriptions as they stand, their watermarks cleared first.
async fn backfill_watermarks(env: &crate::harness::TestEnv, sub_id: SubscriptionId) {
    use crate::release_migrations::{BILLING_ON_FACTS_UP, migration_section};
    use diesel_async::SimpleAsyncConnection;

    let mut conn = env.conn().await;
    SubscriptionRow::set_reconcile_from(&mut conn, sub_id, None, None)
        .await
        .expect("watermarks cleared");
    for title in [
        "Reconcile watermark",
        "Gaps at the cursor",
        "Drafts never issued",
    ] {
        let section = migration_section(BILLING_ON_FACTS_UP, title);
        let backfill = &section[section.find("UPDATE subscription").expect("the backfill")..];
        conn.batch_execute(backfill)
            .await
            .expect("the backfill runs");
    }
}

/// Moves the subscription's invoices to the state the previous release left: each period start
/// given a status, or deleted (a deleted draft) with None.
async fn leave_invoices(
    env: &crate::harness::TestEnv,
    sub_id: SubscriptionId,
    states: &[(NaiveDate, Option<diesel_models::enums::InvoiceStatusEnum>)],
) {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use diesel_models::invoices::InvoiceRow;
    use diesel_models::schema::invoice::dsl as i;

    let invoices = env.get_invoices(sub_id).await;
    let mut conn = env.conn().await;
    for (period_start, state) in states {
        let invoice = invoices
            .iter()
            .find(|invoice| invoice.billing_period_start == *period_start)
            .expect("the period's invoice");
        match state {
            Some(status) => {
                diesel::update(i::invoice.filter(i::id.eq(invoice.id)))
                    .set(i::status.eq(status.clone()))
                    .execute(&mut conn)
                    .await
                    .expect("status set");
            }
            None => {
                assert_eq!(
                    invoice.status,
                    meteroid_store::domain::enums::InvoiceStatusEnum::Draft,
                    "only a draft is deleted"
                );
                InvoiceRow::delete(&mut conn, invoice.id, TENANT_ID)
                    .await
                    .expect("draft deleted")
            }
        }
    }
}

/// A period the previous release closed at zero was issued: the watermark is the start of the
/// latest issued period, closed included. Monthly 20.00 in advance and 0.10 a unit of 100 units:
/// January issued, February's draft deleted, March closed, April's draft open. The watermark is
/// Mar 1, so February is never billed again; April's draft bills March's usage, 10.00, and
/// April's 20.00. Counting only finalized invoices, Jan 1 was the watermark and February was
/// issued again, January's usage and February's fee.
/// Rules: R-094
#[tokio::test]
async fn a_period_closed_by_the_previous_release_is_never_billed_again() {
    use chrono::TimeZone;
    use diesel_models::enums::InvoiceStatusEnum;
    let usage_client = super::usage::build_usage_client(
        Decimal::from(100),
        &[
            (day(1, 1), day(2, 1)),
            (day(2, 1), day(3, 1)),
            (day(3, 1), day(4, 1)),
            (day(4, 1), day(5, 1)),
        ],
    );
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, std::sync::Arc::new(usage_client)).await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2024, 1, 1, 1, 0, 0).unwrap());
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2024, 4, 1, 1, 0, 0).unwrap());
    for _ in 0..3 {
        env.process_cycle_transitions().await;
    }
    leave_invoices(
        &env,
        sub_id,
        &[
            (day(1, 1), Some(InvoiceStatusEnum::Finalized)),
            (day(2, 1), None),
            (day(3, 1), Some(InvoiceStatusEnum::Closed)),
        ],
    )
    .await;
    backfill_watermarks(&env, sub_id).await;
    {
        let mut conn = env.conn().await;
        let watermarks = SubscriptionRow::reconcile_from(&mut conn, sub_id)
            .await
            .expect("watermarks");
        assert_eq!(watermarks, (Some(day(3, 1)), None));
    }

    env.process_cycles().await;
    let periods: Vec<PeriodLines> = env
        .get_invoices(sub_id)
        .await
        .iter()
        .map(|invoice| (invoice.billing_period_start, lines_of(invoice)))
        .collect();
    assert_eq!(
        periods,
        vec![
            (
                day(1, 1),
                vec![("Platform Fee".to_owned(), day(1, 1), day(2, 1), 2_000)]
            ),
            (
                day(3, 1),
                vec![
                    ("Bandwidth".to_owned(), day(2, 1), day(3, 1), 1_000),
                    ("Platform Fee".to_owned(), day(3, 1), day(4, 1), 2_000),
                ]
            ),
            (
                day(4, 1),
                vec![
                    ("Bandwidth".to_owned(), day(3, 1), day(4, 1), 1_000),
                    ("Platform Fee".to_owned(), day(4, 1), day(5, 1), 2_000),
                ]
            ),
        ]
    );
}

/// A current period left with no invoice by the previous release (its draft deleted) is billed
/// from its end on, as the previous release would: monthly 20.00 in advance and 0.10 a unit of
/// 100 units, January and February issued, March's draft deleted. The watermark is Apr 1 and
/// what a period bills at its end has Mar 1: March is never billed, and Apr 1 bills March's
/// usage, 10.00, and April's 20.00. With February's start as the watermark, March was issued
/// again, February's usage and March's fee.
/// Rules: R-094
#[tokio::test]
async fn a_current_period_left_without_invoice_is_billed_from_its_end() {
    use chrono::TimeZone;
    use diesel_models::enums::InvoiceStatusEnum;
    let usage_client = super::usage::build_usage_client(
        Decimal::from(100),
        &[
            (day(1, 1), day(2, 1)),
            (day(2, 1), day(3, 1)),
            (day(3, 1), day(4, 1)),
            (day(4, 1), day(5, 1)),
        ],
    );
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, std::sync::Arc::new(usage_client)).await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2024, 1, 1, 1, 0, 0).unwrap());
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2024, 3, 1, 1, 0, 0).unwrap());
    for _ in 0..2 {
        env.process_cycle_transitions().await;
    }
    leave_invoices(
        &env,
        sub_id,
        &[
            (day(1, 1), Some(InvoiceStatusEnum::Finalized)),
            (day(2, 1), Some(InvoiceStatusEnum::Finalized)),
            (day(3, 1), None),
        ],
    )
    .await;
    backfill_watermarks(&env, sub_id).await;
    {
        let mut conn = env.conn().await;
        let watermarks = SubscriptionRow::reconcile_from(&mut conn, sub_id)
            .await
            .expect("watermarks");
        assert_eq!(watermarks, (Some(day(4, 1)), Some(day(3, 1))));
    }

    env.run_until(day(4, 2)).await;
    let periods: Vec<PeriodLines> = env
        .get_invoices(sub_id)
        .await
        .iter()
        .map(|invoice| (invoice.billing_period_start, lines_of(invoice)))
        .collect();
    assert_eq!(
        periods,
        vec![
            (
                day(1, 1),
                vec![("Platform Fee".to_owned(), day(1, 1), day(2, 1), 2_000)]
            ),
            (
                day(2, 1),
                vec![
                    ("Bandwidth".to_owned(), day(1, 1), day(2, 1), 1_000),
                    ("Platform Fee".to_owned(), day(2, 1), day(3, 1), 2_000),
                ]
            ),
            (
                day(4, 1),
                vec![
                    ("Bandwidth".to_owned(), day(3, 1), day(4, 1), 1_000),
                    ("Platform Fee".to_owned(), day(4, 1), day(5, 1), 2_000),
                ]
            ),
        ]
    );
}

/// Deleting a subscription's recurring draft closes it, and its lines forgive what they billed:
/// LeetCode 35.00 a month from Jan 1, February's draft deleted, cancelled from Feb 15. Nothing more
/// is billed nor credited: the 35.00 of January only. Read as never billed, February's days served,
/// 3500 × 14/29 = 1690, were billed on the cancellation.
/// Rules: R-124, R-096
#[rstest::rstest]
#[tokio::test]
async fn a_deleted_draft_is_closed_and_its_period_never_billed(
    #[future] test_env: crate::harness::TestEnv,
) {
    use chrono::TimeZone;
    use common_domain::actor::Actor;
    use meteroid_store::domain::enums::InvoiceStatusEnum;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
    use meteroid_store::repositories::{CreditNoteInterface, InvoiceInterface};

    let env = test_env.await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2024, 1, 1, 1, 0, 0).unwrap());
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2024, 2, 1, 1, 0, 0).unwrap());
    env.process_cycle_transitions().await;
    let february = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| invoice.billing_period_start == day(2, 1))
        .expect("February's draft");
    assert_eq!(february.status, InvoiceStatusEnum::Draft);
    env.store()
        .delete_invoice(Actor::User { id: USER_ID }, february.id, TENANT_ID)
        .await
        .expect("draft deleted");

    env.set_now(chrono::Utc.with_ymd_and_hms(2024, 2, 15, 12, 0, 0).unwrap());
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(2, 15)),
        )
        .await
        .expect("cancelled");
    env.run_until(day(3, 2)).await;

    let invoices = env.get_invoices(sub_id).await;
    let closed: Vec<NaiveDate> = invoices
        .iter()
        .filter(|invoice| invoice.status == InvoiceStatusEnum::Closed)
        .map(|invoice| invoice.billing_period_start)
        .collect();
    assert_eq!(closed, vec![day(2, 1)]);
    let billed: i64 = invoices
        .iter()
        .filter(|invoice| {
            !matches!(
                invoice.status,
                InvoiceStatusEnum::Void | InvoiceStatusEnum::Closed
            )
        })
        .map(|invoice| invoice.total)
        .sum();
    assert_eq!(billed, 3_500);
    let mut credited = 0;
    for invoice in &invoices {
        credited += env
            .store()
            .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
            .await
            .expect("notes")
            .iter()
            .map(|note| note.total)
            .sum::<i64>();
    }
    assert_eq!(credited, 0);
}

/// A subscription the previous release only left an empty draft is billed from that draft: usage
/// only, 0.10 a unit, 100 units in March, the previous release's one document an empty draft of
/// Mar 1. The watermarks are Mar 1, so no document is written before it, and Apr 1 bills March's
/// usage, 10.00. With no watermark, February's empty window was written from the start.
/// Rules: R-094
#[tokio::test]
async fn a_subscription_left_with_empty_drafts_is_billed_from_its_first_draft() {
    use crate::data::plans::{PlanSeed, SeedComp};
    use chrono::TimeZone;
    use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId};
    use diesel_models::enums::BillingPeriodEnum;
    use meteroid_store::domain::price_components::UsagePricingModel;

    let usage_client =
        super::usage::build_usage_client(Decimal::from(100), &[(day(3, 1), day(4, 1))]);
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, std::sync::Arc::new(usage_client)).await;
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Usage only", version_id)
        .components(vec![SeedComp::usage(
            PriceComponentId::new(),
            "Bandwidth",
            PRODUCT_BANDWIDTH_ID,
            METRIC_BANDWIDTH,
            PriceId::new(),
            BillingPeriodEnum::Monthly,
            UsagePricingModel::PerUnit {
                rate: Decimal::new(10, 2),
            },
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("usage-only plan");
    env.set_now(chrono::Utc.with_ymd_and_hms(2024, 1, 1, 1, 0, 0).unwrap());
    let sub_id = subscription()
        .plan_version(version_id)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.insert_draft_invoice(sub_id, day(3, 1)).await;
    backfill_watermarks(&env, sub_id).await;
    {
        let mut conn = env.conn().await;
        let watermarks = SubscriptionRow::reconcile_from(&mut conn, sub_id)
            .await
            .expect("watermarks");
        assert_eq!(watermarks, (Some(day(3, 1)), Some(day(3, 1))));
    }

    env.run_until(day(4, 2)).await;
    let invoices = env.get_invoices(sub_id).await;
    let periods: Vec<PeriodLines> = invoices
        .iter()
        .map(|invoice| (invoice.billing_period_start, lines_of(invoice)))
        .collect();
    assert_eq!(
        periods,
        vec![
            (day(3, 1), vec![]),
            (
                day(4, 1),
                vec![("Bandwidth".to_owned(), day(3, 1), day(4, 1), 1_000)]
            ),
        ]
    );
}
