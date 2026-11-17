//! Period invoices billed from the facts: the reconcile watermark leaves the
//! periods it settled alone.

use chrono::NaiveDate;
use diesel_models::subscriptions::SubscriptionRow;
use meteroid_store::domain::Invoice;
use rust_decimal::Decimal;

use crate::data::ids::*;
use crate::harness::{subscription, test_env, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).unwrap()
}

fn lines_of(invoice: &Invoice) -> Vec<(String, NaiveDate, NaiveDate, i64)> {
    invoice
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
        .collect()
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
            "Seats".to_string(),
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
