//! Sequences the lifecycle model found wrong, replayed as written: each pins the documents the
//! engine writes, and the model's checks after the last action.
use chrono::{DateTime, NaiveDate, Utc};

use super::engine::Engine;
use super::reference::{
    Action, AddOnSpec, CapacitySpec, Catalog, CouponSpec, Model, Off, PlanSpec, TrialSpec, midnight,
};
use crate::model::harness;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("a day")
}

fn at(on: NaiveDate, hour: i64) -> DateTime<Utc> {
    midnight(on) + chrono::Duration::hours(hour)
}

fn plan(fee: i64, arrears: bool, months: u32, rate: i64, seat: i64) -> PlanSpec {
    PlanSpec {
        fee,
        arrears,
        months,
        rate,
        seat,
    }
}

fn catalog(plans: [PlanSpec; 2], seats: i64) -> Catalog {
    Catalog {
        plans,
        auto_advance: true,
        seats,
        add_on: AddOnSpec {
            price: 1_900,
            months: 1,
            on_b: false,
        },
        coupon: None,
        minimum: None,
        trial: None,
        term_end: None,
        companion: None,
        capacity: None,
    }
}

/// Runs `actions` on the engine and the model; returns the engine's documents and what the
/// model's checks find wrong after the last one.
async fn replay(actions: &[Action]) -> (Vec<String>, Vec<String>) {
    let mut engine = Engine::new().await;
    let mut model = Model::default();
    for action in actions {
        let before = model.clone();
        let expected = model.apply(action);
        let applied = engine.apply(action, &before).await;
        assert_eq!(
            expected.is_ok(),
            applied.is_ok(),
            "{action:?}: the engine {applied:?}"
        );
    }
    let mut found: Vec<String> = engine
        .compare(&model)
        .await
        .into_iter()
        .map(|(kind, detail)| format!("{kind}: {detail}"))
        .collect();
    found.extend(engine.invariants().await);
    let documents = engine
        .dump()
        .await
        .into_iter()
        .filter(|line| line.starts_with("document") || line.starts_with("note"))
        .collect();
    tokio::task::spawn_blocking(move || drop(engine))
        .await
        .expect("the test database dropped");
    (documents, found)
}

/// 9.99 a month and 3 seats at 25.00 from Jan 19, its opening invoice voided; the fee raised to
/// 123.45 on Jan 22 bills (12345 − 999) × 28/31 = 10248; cancelled at once on Jan 23. The fee
/// owes 97 + 398 = 495 for its four days: the raise gives back its own 27 unused days, 9882,
/// keeping 366, and the void forgives the opening's 870 (R-096). Issuing the closing invoice
/// settles the raise again, by its latest line first where the ending split by unused days: it
/// took the 366 too. The minimum, 20000 × 4/31 = 2581, is trued up at 2581 − 366 = 2215.
/// Rules: R-096, R-080, R-022
#[test]
fn a_raise_settled_again_after_an_ending_keeps_the_days_the_void_left_it() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(999, false, 1, 1, 2_500),
                plan(50_000, false, 1, 25, 1_000),
            ],
            3,
        );
        start.minimum = Some(20_000);
        let (documents, found) = replay(&[
            Action::Start {
                day: day(2025, 1, 19),
                catalog: start,
            },
            Action::Void {
                at: at(day(2025, 1, 19), 17),
                pick: 0,
            },
            Action::AmendNow {
                at: at(day(2025, 1, 22), 15),
                plan: 0,
                price: 12_345,
                months: 1,
                rate: 1,
            },
            Action::CancelImmediately {
                at: at(day(2025, 1, 23), 15),
            },
        ])
        .await;
        let notes: Vec<&String> = documents.iter().filter(|l| l.starts_with("note")).collect();
        assert_eq!(
            notes,
            [
                "note: on INV-0002 Finalized subtotal -9882",
                "note:   A fee (adjustment) 2025-01-22..2025-02-19 = -9882",
                "note: on INV-0001 Finalized subtotal -8499",
                "note:   A fee 2025-01-19..2025-02-19 = -999",
                "note:   A seats 2025-01-19..2025-02-19 = -7500",
            ],
            "{documents:#?}"
        );
        assert!(
            documents.contains(
                &"document:   Minimum commitment 2025-01-19..2025-01-23 qty None x None = 2215"
                    .to_owned()
            ),
            "{documents:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// A free trial on a yearly 49.99 plan to Jan 15, billed on day 15, changed on Jan 13 to a
/// monthly plan: the trial ends, billing starts that day (R-011). Changed back the same day, the
/// yearly plan's grid starts on Jan 13 too, the change's day on the grid in force (R-084): it
/// renews on Jan 13, 2026, where the trial's day 15 made it bill a stub to Jan 15.
/// Rules: R-011, R-084
#[test]
fn a_change_back_on_the_day_a_change_ended_the_trial_keeps_that_days_grid() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(4_999, false, 12, 1, 500),
                plan(50_000, true, 1, 1, 1_000),
            ],
            2,
        );
        start.trial = Some(TrialSpec {
            end: day(2025, 1, 15),
            card: true,
            anchor: Some(15),
        });
        let (documents, found) = replay(&[
            Action::Start {
                day: day(2025, 1, 8),
                catalog: start,
            },
            Action::ChangeNow {
                at: at(day(2025, 1, 13), 15),
                to: 1,
            },
            Action::ChangeNow {
                at: at(day(2025, 1, 13), 16),
                to: 0,
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-13 subtotal 2000 total 2000 due 0",
                "document:   B seats 2025-01-13..2025-02-13 qty Some(2) x Some(10.00) = 2000",
                "document: Adjustment Finalized dated 2025-01-13 subtotal 5999 total 5999 due 5999",
                "document:   A fee 2025-01-13..2026-01-13 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-01-13..2026-01-13 qty Some(1) x Some(10.00) = 1000",
                "note: on INV-0001 Finalized subtotal -2000",
                "note:   B seats 2025-01-13..2025-02-13 = -2000",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// A free trial of a monthly plan to Jan 9, changed on Jan 5 to a yearly one: the trial ends and
/// billing starts that day, a year to the fixed term's Feb 8. The fee amended on Jan 8 to
/// 123.45 a month moves the subscription onto the month from its billing start, Jan 5 to Feb 5
/// (R-084): the change that ended the trial started the billing on that grid, no grid of its
/// own, where the yearly period ran on to Feb 8.
/// Rules: R-011, R-084
#[test]
fn a_monthly_fee_after_the_change_that_ended_the_trial_renews_monthly() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(50_000, true, 1, 25, 500),
                plan(12_345, true, 12, 7, 500),
            ],
            3,
        );
        start.auto_advance = false;
        start.trial = Some(TrialSpec {
            end: day(2025, 1, 9),
            card: true,
            anchor: Some(9),
        });
        start.term_end = Some(day(2025, 2, 8));
        let (documents, found) = replay(&[
            Action::Start {
                day: day(2025, 1, 2),
                catalog: start,
            },
            Action::ChangeNow {
                at: at(day(2025, 1, 5), 15),
                to: 1,
            },
            Action::AmendNow {
                at: at(day(2025, 1, 8), 15),
                plan: 1,
                price: 12_345,
                months: 1,
                rate: 7,
            },
        ])
        .await;
        // The draft of Jan 5 bills the year's seats to the term's end until it is issued.
        assert_eq!(
            documents,
            [
                "document: Recurring Draft dated 2025-01-05 subtotal 140 total 140 due 140",
                "document:   B seats 2025-01-05..2025-02-08 qty Some(3) x Some(0.4666666666666666666666666667) = 140",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// One 10.00 seat from Jan 5 under a 1000.00 minimum, three more bought on Jan 10 (2516),
/// cancelled on Jan 18 from Jan 16, the documents issued after. The final invoice, issued before
/// the purchase's note, counts the step as that note will leave it, 2516 − (2581 − 645) = 580,
/// where its days served read 581: the floor 100000 × 11/31 = 35484 is trued up at 13294.
/// Rules: R-022, R-079
#[test]
fn a_seat_step_noted_after_the_true_up_counts_what_its_note_leaves() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(50_000, true, 1, 120, 1_000),
                plan(999, false, 1, 1, 1_000),
            ],
            1,
        );
        start.auto_advance = false;
        start.add_on = AddOnSpec {
            price: 700,
            months: 12,
            on_b: true,
        };
        start.minimum = Some(100_000);
        start.term_end = Some(day(2025, 4, 18));
        start.capacity = Some(CapacitySpec {
            fee: 9_900,
            included: 100,
            overage: 25,
        });
        let jan = |d| day(2025, 1, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: jan(5),
                catalog: start,
            },
            Action::BuySeats {
                at: at(jan(10), 15),
                seats: 3,
            },
            Action::Usage {
                at: at(jan(17), 14),
                units: 23,
            },
            Action::OneTime {
                at: at(jan(18), 16),
                price: 1_500,
            },
            Action::CancelBackdated {
                at: at(jan(18), 17),
                day: jan(16),
            },
            Action::Finalize {
                at: at(jan(18), 18),
                pick: 255,
            },
            Action::Finalize {
                at: at(jan(18), 19),
                pick: 189,
            },
            Action::Finalize {
                at: at(jan(18), 20),
                pick: 73,
            },
        ])
        .await;
        assert!(
            documents.contains(
                &"document:   Minimum commitment 2025-01-05..2025-01-16 qty None x None = 13294"
                    .to_owned()
            ),
            "{documents:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// A free trial of a yearly 123.45 plan to Jan 24, its fee amended to 49.99 from the trial's end,
/// then a monthly 7.00 add-on attached in the trial. The amendment, written before the add-on,
/// takes effect with the add-on in force: the subscription renews monthly, Jan 24 to Feb 24, as
/// an amendment that day would move it (R-084), where it ran the year to Jan 24, 2026.
/// Rules: R-084
#[test]
fn an_amendment_written_ahead_realigns_on_the_fees_in_force_when_it_takes_effect() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(12_345, false, 12, 7, 500),
                plan(50_000, false, 12, 120, 2_500),
            ],
            1,
        );
        start.auto_advance = false;
        start.add_on = AddOnSpec {
            price: 700,
            months: 1,
            on_b: false,
        };
        start.trial = Some(TrialSpec {
            end: day(2025, 1, 24),
            card: true,
            anchor: Some(24),
        });
        let (documents, found) = replay(&[
            Action::Start {
                day: day(2025, 1, 10),
                catalog: start,
            },
            Action::AmendAtEnd {
                at: at(day(2025, 1, 10), 15),
                plan: 0,
                price: 4_999,
                months: 12,
            },
            Action::AttachAddOn {
                at: at(day(2025, 1, 10), 16),
                quantity: 1,
            },
            Action::Advance {
                at: midnight(day(2025, 1, 24)),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Draft dated 2025-01-24 subtotal 6199 total 6199 due 6199",
                "document:   A seats 2025-01-24..2026-01-24 qty Some(1) x Some(5.00) = 500",
                "document:   A fee 2025-01-24..2026-01-24 qty Some(1) x Some(49.99) = 4999",
                "document:   Lifecycle add-on 2025-01-24..2025-02-24 qty Some(1) x Some(7.00) = 700",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// A free trial of a yearly 29.00 plan to Jan 27, its fee amended on Jan 17 to 500.00 a month:
/// the subscription renews on the month from the trial's end, Jan 27 to Feb 27, as if created
/// with the fees the amendment leaves (R-084), where it renewed the year and moved onto the month
/// only at the next amendment.
/// Rules: R-084
#[test]
fn an_amendment_in_a_free_trial_renews_on_the_cadence_it_leaves() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(2_900, false, 12, 25, 500),
                plan(2_900, false, 1, 1, 500),
            ],
            4,
        );
        start.trial = Some(TrialSpec {
            end: day(2025, 1, 27),
            card: true,
            anchor: Some(27),
        });
        let (documents, found) = replay(&[
            Action::Start {
                day: day(2025, 1, 13),
                catalog: start,
            },
            Action::AmendNow {
                at: at(day(2025, 1, 17), 15),
                plan: 0,
                price: 50_000,
                months: 1,
                rate: 25,
            },
            Action::Advance {
                at: midnight(day(2025, 1, 27)),
            },
            Action::Advance {
                at: midnight(day(2025, 2, 27)),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-27 subtotal 52000 total 52000 due 52000",
                "document:   A seats 2025-01-27..2026-01-27 qty Some(4) x Some(5.00) = 2000",
                "document:   A fee 2025-01-27..2025-02-27 qty Some(1) x Some(500.00) = 50000",
                "document: Recurring Finalized dated 2025-02-27 subtotal 50000 total 50000 due 50000",
                "document:   A fee 2025-02-27..2025-03-27 qty Some(1) x Some(500.00) = 50000",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// Two seats at 5.00 a month from Feb 2 under a 10% coupon and a 50.00 minimum, three more bought
/// on Feb 7 (1233), cancelled on Feb 9, the documents issued after. Each line gives back its own
/// unused days whatever order it is issued in: the seats 1000 × 21/28 = 750, the step
/// 2500 × 21/28 − 1000 × 21/28 = 1125, where the step issued second split what was left over the
/// seats again, 161 more from them and 964 from it. The true-up counts what the notes leave:
/// 1250 − 653 − (900 − 675) − (1233 − 1125) = 264.
/// Rules: R-096, R-077, R-022
#[test]
fn each_seat_line_gives_back_its_own_unused_days_whatever_order_they_are_issued_in() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(12_345, false, 1, 120, 500),
                plan(2_900, true, 1, 1, 500),
            ],
            2,
        );
        start.auto_advance = false;
        start.add_on = AddOnSpec {
            price: 700,
            months: 1,
            on_b: false,
        };
        start.coupon = Some(CouponSpec {
            off: Off::Percent(10),
            invoices: None,
            expires: None,
        });
        start.minimum = Some(5_000);
        start.capacity = Some(CapacitySpec {
            fee: 3_100,
            included: 310,
            overage: 10,
        });
        let (jan, feb) = (|d| day(2025, 1, d), |d| day(2025, 2, d));
        let (documents, found) = replay(&[
            Action::Start {
                day: jan(2),
                catalog: start,
            },
            Action::Finalize {
                at: at(jan(2), 16),
                pick: 174,
            },
            Action::ChangeAtEnd {
                at: at(jan(2), 17),
                to: 1,
                seats: Some(2),
            },
            Action::Advance {
                at: midnight(feb(2)),
            },
            Action::BuySeats {
                at: at(feb(7), 15),
                seats: 3,
            },
            Action::CancelNow { at: at(feb(9), 15) },
            Action::Finalize {
                at: at(feb(9), 16),
                pick: 247,
            },
            Action::Finalize {
                at: at(feb(9), 17),
                pick: 115,
            },
            Action::Finalize {
                at: at(feb(9), 18),
                pick: 135,
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-02 subtotal 16445 total 14801 due 14801",
                "document:   A fee 2025-01-02..2025-02-02 qty Some(1) x Some(123.45) = 12345",
                "document:   A capacity 2025-01-02..2025-02-02 qty Some(1) x Some(31.00) = 3100",
                "document:   A seats 2025-01-02..2025-02-02 qty Some(2) x Some(5.00) = 1000",
                "document: Recurring Finalized dated 2025-02-09 subtotal 1000 total 900 due 225",
                "document:   B seats 2025-02-02..2025-03-02 qty Some(2) x Some(5.00) = 1000",
                "document: Adjustment Finalized dated 2025-02-09 subtotal 1233 total 1233 due 108",
                "document:   3 seat upgrade 2025-02-07..2025-03-02 qty Some(3) x Some(4.11) = 1233",
                "document: Recurring Finalized dated 2025-02-09 subtotal 989 total 917 due 917",
                "document:   B fee 2025-02-02..2025-02-09 qty Some(1) x Some(7.25) = 725",
                "document:   Minimum commitment 2025-02-02..2025-02-09 qty None x None = 264",
                "note: on INV-0002 Finalized subtotal -1125",
                "note:   3 seat upgrade 2025-02-07..2025-03-02 = -1125",
                "note: on INV-0004 Finalized subtotal -750",
                "note:   B seats 2025-02-02..2025-03-02 = -750",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// Three seats at 25.00 a month from Mar 19 under a 1000.00 minimum, a fourth bought on Mar 22
/// (2258), cancelled on May 19 from Apr 4. The step's note takes what is left at four seats less
/// at three, 10000 × 15/31 − 7500 × 15/31 = 4839 − 3629 = 1210, and the true-up counts the 1048
/// it leaves, read at the counts the step was billed at: 51613 − 33016 = 18597, where the counts
/// read back from its amount alone, two and one, rounded to 1209 and trued up 18596.
/// Rules: R-022, R-079
#[test]
fn a_seat_step_counts_what_its_note_leaves_at_the_counts_it_was_billed_at() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(2_900, false, 1, 120, 2_500),
                plan(4_999, false, 1, 120, 500),
            ],
            4,
        );
        start.auto_advance = false;
        start.minimum = Some(100_000);
        start.capacity = Some(CapacitySpec {
            fee: 3_100,
            included: 310,
            overage: 10,
        });
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 19),
                catalog: start,
            },
            Action::RemoveSeats {
                at: at(on(1, 19), 15),
                seats: 1,
            },
            Action::Advance {
                at: midnight(on(2, 19)),
            },
            Action::Advance {
                at: midnight(on(3, 19)),
            },
            Action::OneTime {
                at: at(on(3, 21), 16),
                price: 25_000,
            },
            Action::BuySeats {
                at: at(on(3, 22), 15),
                seats: 1,
            },
            Action::Finalize {
                at: at(on(3, 22), 16),
                pick: 189,
            },
            Action::Advance {
                at: midnight(on(4, 19)),
            },
            Action::BuySeats {
                at: at(on(4, 23), 15),
                seats: 1,
            },
            Action::Advance {
                at: midnight(on(5, 19)),
            },
            Action::CancelBackdated {
                at: at(on(5, 19), 15),
                day: on(4, 4),
            },
            Action::Finalize {
                at: at(on(5, 19), 16),
                pick: 42,
            },
            Action::Finalize {
                at: at(on(5, 19), 17),
                pick: 10,
            },
            Action::Finalize {
                at: at(on(5, 19), 18),
                pick: 227,
            },
            Action::Finalize {
                at: at(on(5, 19), 19),
                pick: 138,
            },
            Action::Finalize {
                at: at(on(5, 19), 20),
                pick: 196,
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-05-19 subtotal 16000 total 16000 due 16000",
                "document:   A fee 2025-01-19..2025-02-19 qty Some(1) x Some(29.00) = 2900",
                "document:   A capacity 2025-01-19..2025-02-19 qty Some(1) x Some(31.00) = 3100",
                "document:   A seats 2025-01-19..2025-02-19 qty Some(4) x Some(25.00) = 10000",
                "document: Recurring Finalized dated 2025-03-22 subtotal 97500 total 97500 due 97500",
                "document:   A fee 2025-02-19..2025-03-19 qty Some(1) x Some(29.00) = 2900",
                "document:   A capacity 2025-02-19..2025-03-19 qty Some(1) x Some(31.00) = 3100",
                "document:   A seats 2025-02-19..2025-03-19 qty Some(3) x Some(25.00) = 7500",
                "document:   Minimum commitment 2025-01-19..2025-02-19 qty None x None = 84000",
                "document: Recurring Finalized dated 2025-05-19 subtotal 100000 total 100000 due 93468",
                "document:   A fee 2025-03-19..2025-04-19 qty Some(1) x Some(29.00) = 2900",
                "document:   A capacity 2025-03-19..2025-04-19 qty Some(1) x Some(31.00) = 3100",
                "document:   A seats 2025-03-19..2025-04-19 qty Some(3) x Some(25.00) = 7500",
                "document:   Minimum commitment 2025-02-19..2025-03-19 qty None x None = 86500",
                "document: Adjustment Finalized dated 2025-03-21 subtotal 25000 total 25000 due 25000",
                "document:   Setup 1 (one-time) 2025-03-21..2025-03-21 qty Some(1) x Some(250.00) = 25000",
                "document: Adjustment Finalized dated 2025-05-19 subtotal 2258 total 2258 due 1048",
                "document:   1 seat upgrade 2025-03-22..2025-04-19 qty Some(1) x Some(22.58) = 2258",
                "document: Recurring Closed dated 2025-04-19 subtotal 0 total 0 due 0",
                "document: Adjustment Finalized dated 2025-05-19 subtotal 2166 total 2166 due 0",
                "document:   1 seat upgrade 2025-04-23..2025-05-19 qty Some(1) x Some(21.66) = 2166",
                "document: Recurring Closed dated 2025-05-19 subtotal 0 total 0 due 0",
                "document: Final Finalized dated 2025-05-19 subtotal 18597 total 18597 due 18597",
                "document:   Minimum commitment 2025-03-19..2025-04-04 qty None x None = 18597",
                "note: on INV-0003 Finalized subtotal -1210",
                "note:   1 seat upgrade 2025-03-22..2025-04-19 = -1210",
                "note: on INV-0007 Finalized subtotal -6532",
                "note:   A fee 2025-03-19..2025-04-19 = -1403",
                "note:   A capacity 2025-03-19..2025-04-19 = -1500",
                "note:   A seats 2025-03-19..2025-04-19 = -3629",
                "note: on INV-0004 Finalized subtotal -2166",
                "note:   1 seat upgrade 2025-04-23..2025-05-19 = -2166",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// A year amended in on Jan 1, 2025, changed on Dec 5 to a month, cancelled then from Apr 27,
/// gives back 50000 × 249/365 = 34110: 28647 + 1764 more than the change's 3484 + 215.
/// Rules: R-016, R-077
#[test]
fn a_backdated_cancellation_gives_back_a_fee_the_plan_ended_before_it() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(2_900, false, 1, 7, 500),
                plan(4_999, false, 1, 25, 500),
            ],
            2,
        );
        start.coupon = Some(CouponSpec {
            off: Off::Fixed(2_000),
            invoices: Some(1),
            expires: None,
        });
        let on = |m, d| day(2025, m, d);
        let mut actions = vec![
            Action::Start {
                day: on(1, 1),
                catalog: start,
            },
            Action::AmendNow {
                at: at(on(1, 1), 15),
                plan: 0,
                price: 50_000,
                months: 12,
                rate: 7,
            },
        ];
        actions.extend((2..=12).map(|m| Action::Advance {
            at: midnight(on(m, 1)),
        }));
        actions.extend([
            Action::ChangeNow {
                at: at(on(12, 5), 15),
                to: 1,
            },
            Action::CancelBackdated {
                at: at(on(12, 5), 16),
                day: on(4, 27),
            },
        ]);
        let (documents, found) = replay(&actions).await;
        // Notes written at one instant list in no set order: each with its lines, sorted.
        let notes_of = |lines: Vec<&str>| {
            let mut notes: Vec<Vec<String>> = vec![];
            for line in lines.into_iter().filter(|l| l.starts_with("note")) {
                match notes.last_mut() {
                    Some(note) if !line.starts_with("note: on") => note.push(line.to_owned()),
                    _ => notes.push(vec![line.to_owned()]),
                }
            }
            notes.sort();
            notes
        };
        assert_eq!(
            notes_of(documents.iter().map(String::as_str).collect()),
            notes_of(vec![
                "note: on INV-0014 Finalized subtotal -4354",
                "note:   B fee (prorated) 2025-12-05..2026-01-01 = -4354",
                "note: on INV-0013 Finalized subtotal -1000",
                "note:   A seats 2025-12-01..2026-01-01 = -1000",
                "note: on INV-0012 Finalized subtotal -1000",
                "note:   A seats 2025-11-01..2025-12-01 = -1000",
                "note: on INV-0011 Finalized subtotal -1000",
                "note:   A seats 2025-10-01..2025-11-01 = -1000",
                "note: on INV-0010 Finalized subtotal -1000",
                "note:   A seats 2025-09-01..2025-10-01 = -1000",
                "note: on INV-0009 Finalized subtotal -1000",
                "note:   A seats 2025-08-01..2025-09-01 = -1000",
                "note: on INV-0008 Finalized subtotal -1000",
                "note:   A seats 2025-07-01..2025-08-01 = -1000",
                "note: on INV-0007 Finalized subtotal -1000",
                "note:   A seats 2025-06-01..2025-07-01 = -1000",
                "note: on INV-0006 Finalized subtotal -1000",
                "note:   A seats 2025-05-01..2025-06-01 = -1000",
                "note: on INV-0005 Finalized subtotal -133",
                "note:   A seats 2025-04-01..2025-05-01 = -133",
                "note: on INV-0002 Finalized subtotal -28647",
                "note:   A fee 2025-01-01..2026-01-01 = -28647",
                "note: on INV-0001 Finalized subtotal -1764",
                "note:   A fee 2025-01-01..2025-02-01 = -1764",
                "note: on INV-0002 Finalized subtotal -3484",
                "note:   A fee 2025-01-01..2026-01-01 = -3484",
                "note: on INV-0001 Finalized subtotal -215",
                "note:   A fee 2025-01-01..2025-02-01 = -215",
            ]),
            "{documents:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// Yearly from Jan 13, 2025, monthly from Jan 20 (first renewal Feb 13), its fee a year from
/// Jan 22 to a term ending Mar 19: 4999 × 22/366 = 300, then 4999 − 4999 × 331/365 = 466.
/// Rules: R-084, R-079
#[test]
fn a_yearly_fee_after_a_change_of_cadence_bills_on_the_grid_through_its_first_renewal() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(4_999, true, 12, 25, 1_000),
                plan(50_000, true, 1, 25, 1_000),
            ],
            4,
        );
        start.auto_advance = false;
        start.add_on = AddOnSpec {
            price: 9_900,
            months: 12,
            on_b: false,
        };
        start.term_end = Some(day(2025, 3, 19));
        start.companion = Some(2_900);
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 13),
                catalog: start,
            },
            Action::Usage {
                at: at(on(1, 17), 14),
                units: 28,
            },
            Action::ChangeNow {
                at: at(on(1, 20), 15),
                to: 1,
            },
            Action::AmendNow {
                at: at(on(1, 22), 15),
                plan: 1,
                price: 4_999,
                months: 12,
                rate: 25,
            },
            Action::Advance {
                at: midnight(on(2, 13)),
            },
            Action::Advance {
                at: midnight(on(3, 13)),
            },
            Action::Advance {
                at: midnight(on(3, 19)),
            },
            Action::FinalizeAll {
                at: at(on(3, 19), 16),
            },
        ])
        .await;
        // Read by period and amount: a stub's name carries its dates.
        let fee: Vec<String> = documents
            .iter()
            .filter(|l| l.starts_with("document:   B fee"))
            .filter_map(|l| {
                let (named, priced) = l.split_once(" qty ")?;
                Some(format!("{} {priced}", named.rsplit_once(' ')?.1))
            })
            .collect();
        assert_eq!(
            fee,
            [
                "2025-01-22..2025-02-13 Some(1) x Some(3) = 300",
                "2025-01-20..2025-01-22 Some(1) x Some(32.26) = 3226",
                "2025-02-13..2025-03-19 Some(1) x Some(4.66) = 466",
            ],
            "{documents:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// Yearly from Jan 8, its monthly add-ons attached on Jan 11 dropped that day by a change to a
/// yearly plan: no change of cadence, so its fee amended to monthly renews Jan 8 to Feb 8.
/// Rules: R-084
#[test]
fn a_change_replacing_the_window_an_add_on_opened_that_day_is_no_change_of_cadence() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(999, false, 12, 7, 2_500),
                plan(50_000, true, 12, 120, 2_500),
            ],
            2,
        );
        start.add_on = AddOnSpec {
            price: 9_900,
            months: 1,
            on_b: false,
        };
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 8),
                catalog: start,
            },
            Action::AttachAddOn {
                at: at(on(1, 11), 15),
                quantity: 3,
            },
            Action::ChangeNow {
                at: at(on(1, 11), 16),
                to: 1,
            },
            Action::AmendNow {
                at: at(on(1, 11), 17),
                plan: 1,
                price: 999,
                months: 1,
                rate: 120,
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-08 subtotal 5999 total 5999 due 5008",
                "document:   A fee 2025-01-08..2026-01-08 qty Some(1) x Some(9.99) = 999",
                "document:   A seats 2025-01-08..2026-01-08 qty Some(2) x Some(25.00) = 5000",
                "document: Adjustment Finalized dated 2025-01-11 subtotal 26826 total 26826 due 0",
                "document:   Lifecycle add-on (prorated) 2025-01-11..2025-02-08 qty Some(3) x Some(89.42) = 26826",
                "note: on INV-0002 Finalized subtotal -26826",
                "note:   Lifecycle add-on (prorated) 2025-01-11..2025-02-08 = -26826",
                "note: on INV-0001 Finalized subtotal -991",
                "note:   A fee 2025-01-08..2026-01-08 = -991",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// A change ending a free trial on Jan 25 to a yearly plan keeping a 19.00 monthly add-on renews
/// Jan 25 to Feb 25 and bills 1425 on Feb 25, where it renewed the year to Jan 25, 2026.
/// Rules: R-011, R-084
#[test]
fn a_change_ending_a_free_trial_renews_on_the_cadence_of_the_add_on_it_keeps() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(12_345, true, 1, 25, 500),
                plan(2_900, true, 12, 25, 500),
            ],
            4,
        );
        start.add_on = AddOnSpec {
            price: 1_900,
            months: 1,
            on_b: true,
        };
        start.auto_advance = false;
        start.coupon = Some(CouponSpec {
            off: Off::Percent(25),
            invoices: None,
            expires: None,
        });
        start.trial = Some(TrialSpec {
            end: day(2025, 2, 1),
            card: true,
            anchor: Some(1),
        });
        let on = |m, d| day(2025, m, d);
        let opening = [
            "document: Recurring Draft dated 2025-01-25 subtotal 3900 total 2925 due 2925",
            "document:   B seats 2025-01-25..2026-01-25 qty Some(4) x Some(5.00) = 2000",
            "document:   Lifecycle add-on 2025-01-25..2025-02-25 qty Some(1) x Some(19.00) = 1900",
        ];
        let setup = [
            "document: Adjustment Finalized dated 2025-01-26 subtotal 1500 total 1500 due 1500",
            "document:   Setup 1 (one-time) 2025-01-26..2025-01-26 qty Some(1) x Some(15.00) = 1500",
        ];
        let renewal = [
            "document: Recurring Draft dated 2025-02-25 subtotal 1900 total 1425 due 1425",
            "document:   Lifecycle add-on 2025-02-25..2025-03-25 qty Some(1) x Some(19.00) = 1900",
        ];
        for one_time in [false, true] {
            let mut actions = vec![
                Action::Start {
                    day: on(1, 18),
                    catalog: start.clone(),
                },
                Action::AttachAddOn {
                    at: at(on(1, 21), 15),
                    quantity: 1,
                },
                Action::ChangeNow {
                    at: at(on(1, 25), 15),
                    to: 1,
                },
            ];
            if one_time {
                actions.push(Action::OneTime {
                    at: at(on(1, 26), 16),
                    price: 1_500,
                });
            }
            actions.push(Action::Advance {
                at: midnight(on(2, 25)),
            });
            let (documents, found) = replay(&actions).await;
            let expected: Vec<&str> = opening
                .iter()
                .chain(setup.iter().filter(|_| one_time))
                .chain(renewal.iter())
                .copied()
                .collect();
            assert_eq!(documents, expected, "{found:#?}");
            assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
        }
    });
}

/// Yearly add-ons (489 to the term's Apr 15) dropped by a change on Feb 15: its 340 note precedes
/// the renewal, which trues up 100000 − 14345 − 149 = 85506, where the check read 85166.
/// Rules: R-022, R-077
#[test]
fn a_note_the_change_writes_before_the_renewal_counts_in_its_true_up() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(12_345, false, 1, 25, 1_000),
                plan(999, false, 1, 120, 2_500),
            ],
            2,
        );
        start.add_on = AddOnSpec {
            price: 700,
            months: 12,
            on_b: false,
        };
        start.minimum = Some(100_000);
        start.term_end = Some(day(2025, 4, 15));
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 15),
                catalog: start,
            },
            Action::AttachAddOn {
                at: at(on(1, 20), 15),
                quantity: 3,
            },
            Action::ChangeAtEnd {
                at: at(on(1, 20), 16),
                to: 1,
                seats: Some(2),
            },
            Action::Advance {
                at: midnight(on(2, 15)),
            },
            Action::Advance {
                at: midnight(on(3, 15)),
            },
            Action::Advance {
                at: midnight(on(4, 15)),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-15 subtotal 14345 total 14345 due 14345",
                "document:   A fee 2025-01-15..2025-02-15 qty Some(1) x Some(123.45) = 12345",
                "document:   A seats 2025-01-15..2025-02-15 qty Some(2) x Some(10.00) = 2000",
                "document: Adjustment Finalized dated 2025-01-20 subtotal 489 total 489 due 149",
                "document:   Lifecycle add-on (prorated) 2025-01-20..2025-04-15 qty Some(3) x Some(1.63) = 489",
                "document: Recurring Finalized dated 2025-02-15 subtotal 91505 total 91505 due 91505",
                "document:   B fee 2025-02-15..2025-03-15 qty Some(1) x Some(9.99) = 999",
                "document:   B seats 2025-02-15..2025-03-15 qty Some(2) x Some(25.00) = 5000",
                "document:   Minimum commitment 2025-01-15..2025-02-15 qty None x None = 85506",
                "document: Recurring Finalized dated 2025-03-15 subtotal 100000 total 100000 due 100000",
                "document:   B fee 2025-03-15..2025-04-15 qty Some(1) x Some(9.99) = 999",
                "document:   B seats 2025-03-15..2025-04-15 qty Some(2) x Some(25.00) = 5000",
                "document:   Minimum commitment 2025-02-15..2025-03-15 qty None x None = 94001",
                "document: Recurring Finalized dated 2025-04-15 subtotal 94001 total 94001 due 94001",
                "document:   Minimum commitment 2025-03-15..2025-04-15 qty None x None = 94001",
                "note: on INV-0002 Finalized subtotal -340",
                "note:   Lifecycle add-on (prorated) 2025-01-20..2025-04-15 = -340",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// Two of three seats removed for the trial's end, Jan 25; a change on Jan 15 ends the trial, so
/// B bills 3 seats to Feb 15 and 1 from it, where 5000 × 21/31 = 3387 was owed back, unwritten.
/// Rules: R-108, R-011
#[test]
fn a_removal_for_the_trials_end_a_change_moved_waits_for_the_renewal() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(12_345, true, 1, 1, 500),
                plan(50_000, false, 1, 1, 2_500),
            ],
            3,
        );
        start.add_on = AddOnSpec {
            price: 9_900,
            months: 12,
            on_b: false,
        };
        start.trial = Some(TrialSpec {
            end: day(2025, 1, 25),
            card: true,
            anchor: Some(25),
        });
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 11),
                catalog: start,
            },
            Action::RemoveSeats {
                at: at(on(1, 11), 15),
                seats: 2,
            },
            Action::Usage {
                at: at(on(1, 15), 14),
                units: 25,
            },
            Action::Usage {
                at: at(on(1, 15), 15),
                units: 13,
            },
            Action::ChangeNow {
                at: at(on(1, 15), 16),
                to: 1,
            },
            Action::Advance {
                at: midnight(on(2, 15)),
            },
            Action::CancelAtEnd {
                at: at(on(2, 15), 15),
                end: on(3, 15),
            },
            Action::Advance {
                at: midnight(on(3, 15)),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-15 subtotal 57500 total 57500 due 57500",
                "document:   B fee 2025-01-15..2025-02-15 qty Some(1) x Some(500.00) = 50000",
                "document:   B seats 2025-01-15..2025-02-15 qty Some(3) x Some(25.00) = 7500",
                "document: Recurring Finalized dated 2025-02-15 subtotal 52538 total 52538 due 52538",
                "document:   B units 2025-01-15..2025-02-15 qty Some(38) x Some(0.01) = 38",
                "document:   B fee 2025-02-15..2025-03-15 qty Some(1) x Some(500.00) = 50000",
                "document:   B seats 2025-02-15..2025-03-15 qty Some(1) x Some(25.00) = 2500",
                "document: Recurring Closed dated 2025-03-15 subtotal 0 total 0 due 0",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// A seat removed for Jan 14, 2026, the year moved to Feb 14 by two changes: it leaves on Feb 14,
/// 2026 (2 seats), 16299 billed, where 1000 × 31/365 = 85 was owed back from Jan 14, unwritten.
/// Rules: R-108, R-084
#[test]
fn a_removal_for_a_years_end_a_change_moved_waits_for_the_renewal() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(2_900, false, 12, 1, 1_000),
                plan(4_999, false, 1, 7, 1_000),
            ],
            3,
        );
        start.add_on = AddOnSpec {
            price: 700,
            months: 1,
            on_b: false,
        };
        start.capacity = Some(CapacitySpec {
            fee: 3_100,
            included: 310,
            overage: 10,
        });
        let (documents, found) = replay(&[
            Action::Start {
                day: day(2025, 1, 14),
                catalog: start,
            },
            Action::RemoveSeats {
                at: at(day(2025, 1, 14), 15),
                seats: 1,
            },
            Action::ChangeNow {
                at: at(day(2025, 1, 17), 15),
                to: 1,
            },
            Action::ChangeAtEnd {
                at: at(day(2025, 1, 17), 16),
                to: 0,
                seats: None,
            },
            Action::Advance {
                at: midnight(day(2025, 2, 14)),
            },
            Action::Advance {
                at: midnight(day(2026, 2, 14)),
            },
            Action::CancelNow {
                at: at(day(2026, 2, 14), 15),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-14 subtotal 9000 total 9000 due 74",
                "document:   A fee 2025-01-14..2026-01-14 qty Some(1) x Some(29.00) = 2900",
                "document:   A capacity 2025-01-14..2026-01-14 qty Some(1) x Some(31.00) = 3100",
                "document:   A seats 2025-01-14..2026-01-14 qty Some(3) x Some(10.00) = 3000",
                "document: Adjustment Finalized dated 2025-01-17 subtotal 7225 total 7225 due 7225",
                "document:   B fee (prorated) 2025-01-17..2025-02-14 qty Some(1) x Some(45.15) = 4515",
                "document:   B seats (prorated) 2025-01-17..2025-02-14 qty Some(1) x Some(27.10) = 2710",
                "document: Recurring Finalized dated 2025-02-14 subtotal 9000 total 9000 due 9000",
                "document:   A fee 2025-02-14..2026-02-14 qty Some(1) x Some(29.00) = 2900",
                "document:   A capacity 2025-02-14..2026-02-14 qty Some(1) x Some(31.00) = 3100",
                "document:   A seats 2025-02-14..2026-02-14 qty Some(3) x Some(10.00) = 3000",
                "document: Recurring Finalized dated 2026-02-14 subtotal 8000 total 8000 due 0",
                "document:   A fee 2026-02-14..2027-02-14 qty Some(1) x Some(29.00) = 2900",
                "document:   A capacity 2026-02-14..2027-02-14 qty Some(1) x Some(31.00) = 3100",
                "document:   A seats 2026-02-14..2027-02-14 qty Some(2) x Some(10.00) = 2000",
                "note: on INV-0004 Finalized subtotal -8000",
                "note:   A fee 2026-02-14..2027-02-14 = -2900",
                "note:   A capacity 2026-02-14..2027-02-14 = -3100",
                "note:   A seats 2026-02-14..2027-02-14 = -2000",
                "note: on INV-0001 Finalized subtotal -8926",
                "note:   A fee 2025-01-14..2026-01-14 = -2876",
                "note:   A capacity 2025-01-14..2026-01-14 = -3075",
                "note:   A seats 2025-01-14..2026-01-14 = -2975",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// A 10 % coupon for two invoices, 1000.00 minimum: February trues January up at 55362, its draft
/// owed the second use; March, issued next, takes none (47750), where it took it and left 55112.
/// Rules: R-089, R-022
#[test]
fn a_coupons_last_use_waits_for_the_oldest_open_draft() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(4_999, true, 1, 7, 2_500),
                plan(4_999, true, 1, 1, 2_500),
            ],
            1,
        );
        start.auto_advance = false;
        start.add_on = AddOnSpec {
            price: 9_900,
            months: 1,
            on_b: false,
        };
        start.coupon = Some(CouponSpec {
            off: Off::Percent(10),
            invoices: Some(2),
            expires: None,
        });
        start.minimum = Some(100_000);
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 25),
                catalog: start,
            },
            Action::AmendNow {
                at: at(on(1, 27), 15),
                plan: 0,
                price: 50_000,
                months: 1,
                rate: 7,
            },
            Action::Advance {
                at: midnight(on(2, 25)),
            },
            Action::CancelAtEnd {
                at: at(on(2, 25), 15),
                end: on(3, 25),
            },
            Action::Advance {
                at: midnight(on(3, 25)),
            },
            Action::Finalize {
                at: at(on(3, 25), 16),
                pick: 85,
            },
            Action::Finalize {
                at: at(on(3, 25), 17),
                pick: 167,
            },
            Action::FinalizeAll {
                at: at(on(3, 25), 18),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-03-25 subtotal 2500 total 2250 due 2250",
                "document:   A seats 2025-01-25..2025-02-25 qty Some(1) x Some(25.00) = 2500",
                "document: Recurring Finalized dated 2025-03-25 subtotal 104959 total 100000 due 100000",
                "document:   A seats 2025-02-25..2025-03-25 qty Some(1) x Some(25.00) = 2500",
                "document:   A fee (2025-01-27 - 2025-02-25) (2025-01-27 - 2025-02-25) (2025-01-27 - 2025-02-25) (2025-01-27 - 2025-02-25) 2025-01-27..2025-02-25 qty Some(1) x Some(467.74) = 46774",
                "document:   A fee (2025-01-25 - 2025-01-27) (2025-01-25 - 2025-01-27) (2025-01-25 - 2025-01-27) (2025-01-25 - 2025-01-27) 2025-01-25..2025-01-27 qty Some(1) x Some(3.23) = 323",
                "document:   Minimum commitment 2025-01-25..2025-02-25 qty None x None = 55362",
                "document: Recurring Finalized dated 2025-03-25 subtotal 97750 total 97750 due 97750",
                "document:   A fee 2025-02-25..2025-03-25 qty Some(1) x Some(500.00) = 50000",
                "document:   Minimum commitment 2025-02-25..2025-03-25 qty None x None = 47750",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// A yearly 500.00 fee on a month a monthly add-on set, lowered to 120.00 a year for Feb 1; the
/// add-on detached on Jan 10 runs the year to Jan 1, 2026, which the lowering waits for: 51317
/// over the life, where the reference priced both prices' days from Feb 1, 16545.
/// Rules: R-106, R-084
#[test]
fn a_lowering_for_the_months_end_a_detach_moved_waits_for_the_renewal() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(50_000, false, 12, 1, 1_000),
                plan(4_999, false, 1, 7, 1_000),
            ],
            1,
        );
        start.add_on = AddOnSpec {
            price: 1_900,
            months: 1,
            on_b: false,
        };
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 1),
                catalog: start,
            },
            Action::AttachAddOn {
                at: at(on(1, 5), 15),
                quantity: 1,
            },
            Action::AmendAtEnd {
                at: at(on(1, 6), 15),
                plan: 0,
                price: 12_000,
                months: 12,
            },
            Action::DetachAddOn {
                at: at(on(1, 10), 15),
            },
            Action::Usage {
                at: at(on(2, 3), 14),
                units: 10,
            },
            Action::Advance {
                at: midnight(day(2026, 1, 1)),
            },
            Action::CancelNow {
                at: at(day(2026, 1, 1), 15),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-01 subtotal 51000 total 51000 due 51000",
                "document:   A fee 2025-01-01..2026-01-01 qty Some(1) x Some(500.00) = 50000",
                "document:   A seats 2025-01-01..2026-01-01 qty Some(1) x Some(10.00) = 1000",
                "document: Adjustment Finalized dated 2025-01-05 subtotal 1655 total 1655 due 307",
                "document:   Lifecycle add-on (prorated) 2025-01-05..2025-02-01 qty Some(1) x Some(16.55) = 1655",
                "document: Recurring Finalized dated 2026-01-01 subtotal 13010 total 13010 due 10",
                "document:   A units 2025-01-01..2026-01-01 qty Some(10) x Some(0.01) = 10",
                "document:   A seats 2026-01-01..2027-01-01 qty Some(1) x Some(10.00) = 1000",
                "document:   A fee 2026-01-01..2027-01-01 qty Some(1) x Some(120.00) = 12000",
                "note: on INV-0003 Finalized subtotal -13000",
                "note:   A seats 2026-01-01..2027-01-01 = -1000",
                "note:   A fee 2026-01-01..2027-01-01 = -12000",
                "note: on INV-0002 Finalized subtotal -1348",
                "note:   Lifecycle add-on (prorated) 2025-01-05..2025-02-01 = -1348",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// A yearly 49.99 fee on a month a monthly add-on set, raised to 500.00 a year for Feb 1; the
/// add-on detached on Jan 10 runs the year to Jan 1, 2026, which the raise waits for: 6316 over
/// the life, where the reference priced both prices' days from Feb 1, 47495.
/// Rules: R-106, R-084
#[test]
fn a_raise_for_the_months_end_a_detach_moved_waits_for_the_renewal() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(4_999, false, 12, 1, 1_000),
                plan(4_999, false, 1, 7, 1_000),
            ],
            1,
        );
        start.add_on = AddOnSpec {
            price: 1_900,
            months: 1,
            on_b: false,
        };
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 1),
                catalog: start,
            },
            Action::AttachAddOn {
                at: at(on(1, 5), 15),
                quantity: 1,
            },
            Action::AmendAtEnd {
                at: at(on(1, 6), 15),
                plan: 0,
                price: 50_000,
                months: 12,
            },
            Action::DetachAddOn {
                at: at(on(1, 10), 15),
            },
            Action::Usage {
                at: at(on(2, 3), 14),
                units: 10,
            },
            Action::Advance {
                at: midnight(day(2026, 1, 1)),
            },
            Action::CancelNow {
                at: at(day(2026, 1, 1), 15),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-01 subtotal 5999 total 5999 due 5999",
                "document:   A fee 2025-01-01..2026-01-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-01-01..2026-01-01 qty Some(1) x Some(10.00) = 1000",
                "document: Adjustment Finalized dated 2025-01-05 subtotal 1655 total 1655 due 307",
                "document:   Lifecycle add-on (prorated) 2025-01-05..2025-02-01 qty Some(1) x Some(16.55) = 1655",
                "document: Recurring Finalized dated 2026-01-01 subtotal 51010 total 51010 due 10",
                "document:   A units 2025-01-01..2026-01-01 qty Some(10) x Some(0.01) = 10",
                "document:   A seats 2026-01-01..2027-01-01 qty Some(1) x Some(10.00) = 1000",
                "document:   A fee 2026-01-01..2027-01-01 qty Some(1) x Some(500.00) = 50000",
                "note: on INV-0003 Finalized subtotal -51000",
                "note:   A seats 2026-01-01..2027-01-01 = -1000",
                "note:   A fee 2026-01-01..2027-01-01 = -50000",
                "note: on INV-0002 Finalized subtotal -1348",
                "note:   Lifecycle add-on (prorated) 2025-01-05..2025-02-01 = -1348",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// A yearly 500.00 fee on a month a monthly add-on set, amended to 49.99 a month for Feb 1; the
/// add-on detached on Jan 10 runs the year to Jan 1, 2026, which the amendment waits for: 51317
/// over the life, where Feb 1 gave 45753 of the year back and eleven months billed 54989, 60553.
/// Rules: R-106, R-084
#[test]
fn a_change_of_cadence_for_the_months_end_a_detach_moved_waits_for_the_renewal() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(50_000, false, 12, 1, 1_000),
                plan(4_999, false, 1, 7, 1_000),
            ],
            1,
        );
        start.add_on = AddOnSpec {
            price: 1_900,
            months: 1,
            on_b: false,
        };
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 1),
                catalog: start,
            },
            Action::AttachAddOn {
                at: at(on(1, 5), 15),
                quantity: 1,
            },
            Action::AmendAtEnd {
                at: at(on(1, 6), 15),
                plan: 0,
                price: 4_999,
                months: 1,
            },
            Action::DetachAddOn {
                at: at(on(1, 10), 15),
            },
            Action::Usage {
                at: at(on(2, 3), 14),
                units: 10,
            },
            Action::Advance {
                at: midnight(day(2026, 1, 1)),
            },
            Action::CancelNow {
                at: at(day(2026, 1, 1), 15),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-01 subtotal 51000 total 51000 due 51000",
                "document:   A fee 2025-01-01..2026-01-01 qty Some(1) x Some(500.00) = 50000",
                "document:   A seats 2025-01-01..2026-01-01 qty Some(1) x Some(10.00) = 1000",
                "document: Adjustment Finalized dated 2025-01-05 subtotal 1655 total 1655 due 307",
                "document:   Lifecycle add-on (prorated) 2025-01-05..2025-02-01 qty Some(1) x Some(16.55) = 1655",
                "document: Recurring Finalized dated 2026-01-01 subtotal 6009 total 6009 due 10",
                "document:   A units 2025-01-01..2026-01-01 qty Some(10) x Some(0.01) = 10",
                "document:   A seats 2026-01-01..2027-01-01 qty Some(1) x Some(10.00) = 1000",
                "document:   A fee 2026-01-01..2026-02-01 qty Some(1) x Some(49.99) = 4999",
                "note: on INV-0003 Finalized subtotal -5999",
                "note:   A seats 2026-01-01..2027-01-01 = -1000",
                "note:   A fee 2026-01-01..2026-02-01 = -4999",
                "note: on INV-0002 Finalized subtotal -1348",
                "note:   Lifecycle add-on (prorated) 2025-01-05..2025-02-01 = -1348",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// A yearly 29.00 plan in a free trial to Feb 5, changed for its end to a monthly plan and changed
/// back on Feb 5: the yearly plan's window takes the place of the monthly one, its year from Feb 5
/// billed on that day's adjustment, 10400, and renewed on Feb 5, 2026, 10400. The step stored the
/// change back as made on Jan 22, the first plan's start, a second switch that cut the trial's
/// end there for the readers of switches: the per-operation path billed nothing on Feb 5, 2026.
/// Rules: R-084, R-100
#[test]
fn a_yearly_plan_changed_back_to_on_the_day_its_scheduled_change_applied_renews_its_year() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(2_900, false, 12, 120, 2_500),
                plan(2_900, true, 1, 7, 500),
            ],
            3,
        );
        start.auto_advance = false;
        start.add_on = AddOnSpec {
            price: 1_900,
            months: 12,
            on_b: false,
        };
        start.coupon = Some(CouponSpec {
            off: Off::Percent(50),
            invoices: Some(1),
            expires: None,
        });
        start.trial = Some(TrialSpec {
            end: day(2025, 2, 5),
            card: true,
            anchor: Some(5),
        });
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 22),
                catalog: start,
            },
            Action::ChangeAtEnd {
                at: at(on(1, 22), 15),
                to: 1,
                seats: Some(3),
            },
            Action::Advance {
                at: midnight(on(2, 5)),
            },
            Action::ChangeNow {
                at: at(on(2, 5), 15),
                to: 0,
            },
            Action::Advance {
                at: midnight(day(2026, 2, 5)),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Draft dated 2025-02-05 subtotal 1500 total 750 due 750",
                "document:   B seats 2025-02-05..2025-03-05 qty Some(3) x Some(5.00) = 1500",
                "document: Adjustment Finalized dated 2025-02-05 subtotal 10400 total 10400 due 10400",
                "document:   A fee 2025-02-05..2026-02-05 qty Some(1) x Some(29.00) = 2900",
                "document:   A seats 2025-02-05..2026-02-05 qty Some(1) x Some(75.00) = 7500",
                "document: Recurring Draft dated 2026-02-05 subtotal 10400 total 10400 due 10400",
                "document:   A fee 2026-02-05..2027-02-05 qty Some(1) x Some(29.00) = 2900",
                "document:   A seats 2026-02-05..2027-02-05 qty Some(3) x Some(25.00) = 7500",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// A monthly 123.45 fee amended on Jan 11 to 49.99 a year (4972 to Jan 9, 2026), then to 123.45 a
/// year for the period's end, Feb 9, the month's renewal: Feb 9 bills the year's rest at the new
/// price less the old one's, 11297 − 4574 = 6723 (R-080), of which the per-operation path bills
/// nothing on that invoice.
/// Rules: R-080, R-078, R-106
#[test]
fn a_yearly_fee_repriced_at_a_monthly_renewal_bills_the_rest_of_its_year() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(12_345, false, 1, 120, 500),
                plan(999, true, 1, 25, 500),
            ],
            3,
        );
        start.add_on = AddOnSpec {
            price: 9_900,
            months: 12,
            on_b: true,
        };
        start.capacity = Some(CapacitySpec {
            fee: 3_100,
            included: 310,
            overage: 10,
        });
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 9),
                catalog: start,
            },
            Action::AmendNow {
                at: at(on(1, 11), 15),
                plan: 0,
                price: 4_999,
                months: 12,
                rate: 120,
            },
            Action::AmendAtEnd {
                at: at(on(1, 11), 16),
                plan: 0,
                price: 12_345,
                months: 12,
            },
            Action::Advance {
                at: midnight(on(2, 9)),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-09 subtotal 16945 total 16945 due 5396",
                "document:   A fee 2025-01-09..2025-02-09 qty Some(1) x Some(123.45) = 12345",
                "document:   A capacity 2025-01-09..2025-02-09 qty Some(1) x Some(31.00) = 3100",
                "document:   A seats 2025-01-09..2025-02-09 qty Some(3) x Some(5.00) = 1500",
                "document: Adjustment Finalized dated 2025-01-11 subtotal 4972 total 4972 due 4972",
                "document:   A fee (adjustment) 2025-01-11..2026-01-09 qty Some(1) x Some(49.72) = 4972",
                "document: Recurring Finalized dated 2025-02-09 subtotal 11323 total 11323 due 11323",
                "document:   A capacity 2025-02-09..2025-03-09 qty Some(1) x Some(31.00) = 3100",
                "document:   A seats 2025-02-09..2025-03-09 qty Some(3) x Some(5.00) = 1500",
                "document:   A fee 2025-02-09..2026-01-09 qty Some(1) x Some(67.23) = 6723",
                "note: on INV-0001 Finalized subtotal -11549",
                "note:   A fee 2025-01-09..2025-02-09 = -11549",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// 49.99 a month and 2 seats at 10.00 from Jan 23, a 50 % coupon for one invoice and a 200.00
/// minimum, drafts finalized by hand: March's renewal is issued while January holds the use, so
/// February's draft counts 6999 and is trued up 13001. Voiding January then gives the use to
/// February's draft, issued at 3500: the true-up issued before stays as it is.
/// Rules: R-022, R-089, R-096
#[test]
fn a_use_a_void_frees_after_the_true_up_leaves_that_true_up_as_it_is() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(4_999, false, 1, 1, 1_000),
                plan(999, false, 1, 120, 1_000),
            ],
            2,
        );
        start.auto_advance = false;
        start.add_on = AddOnSpec {
            price: 700,
            months: 12,
            on_b: false,
        };
        start.coupon = Some(CouponSpec {
            off: Off::Percent(50),
            invoices: Some(1),
            expires: None,
        });
        start.minimum = Some(20_000);
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 23),
                catalog: start,
            },
            Action::Advance {
                at: midnight(on(2, 23)),
            },
            Action::Advance {
                at: midnight(on(3, 23)),
            },
            Action::BuySeats {
                at: at(on(3, 26), 15),
                seats: 2,
            },
            Action::CancelImmediately {
                at: at(on(3, 29), 15),
            },
            Action::Finalize {
                at: at(on(3, 29), 16),
                pick: 158,
            },
            Action::Pay {
                at: at(on(3, 29), 18),
                pick: 81,
            },
            Action::Finalize {
                at: at(on(3, 29), 19),
                pick: 138,
            },
            Action::Finalize {
                at: at(on(3, 29), 20),
                pick: 0,
            },
            Action::Void {
                at: at(on(3, 29), 21),
                pick: 74,
            },
            Action::FinalizeAll {
                at: at(on(3, 29), 22),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Void dated 2025-03-29 subtotal 6999 total 3500 due 1887",
                "document:   A fee 2025-01-23..2025-02-23 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-01-23..2025-02-23 qty Some(2) x Some(10.00) = 2000",
                "document: Recurring Finalized dated 2025-03-29 subtotal 26999 total 23500 due 21887",
                "document:   A fee 2025-02-23..2025-03-23 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-02-23..2025-03-23 qty Some(2) x Some(10.00) = 2000",
                "document:   Minimum commitment 2025-01-23..2025-02-23 qty None x None = 20000",
                "document: Recurring Finalized dated 2025-03-29 subtotal 20000 total 20000 due 14356",
                "document:   A fee 2025-03-23..2025-04-23 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-03-23..2025-04-23 qty Some(2) x Some(10.00) = 2000",
                "document:   Minimum commitment 2025-02-23..2025-03-23 qty None x None = 13001",
                "document: Adjustment Finalized dated 2025-03-29 subtotal 1807 total 1807 due 0",
                "document:   2 seat upgrade 2025-03-26..2025-04-23 qty Some(2) x Some(9.0350) = 1807",
                "document: Recurring Finalized dated 2025-03-29 subtotal 2322 total 2322 due 2322",
                "document:   Minimum commitment 2025-03-23..2025-03-29 qty None x None = 2322",
                "note: on INV-0003 Finalized subtotal -6999",
                "note:   A fee 2025-01-23..2025-02-23 = -4999",
                "note:   A seats 2025-01-23..2025-02-23 = -2000",
                "note: on INV-0001 Finalized subtotal -1613",
                "note:   2 seat upgrade 2025-03-26..2025-04-23 = -1613",
                "note: on INV-0002 Finalized subtotal -5644",
                "note:   A fee 2025-03-23..2025-04-23 = -4031",
                "note:   A seats 2025-03-23..2025-04-23 = -1613",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// 29.00 a month in arrears from Jan 26, amended on Jan 29 to 123.45 a year in arrears and again
/// on Feb 2 at the same price and cadence: the two yearly rows are one run, billed in arrears at
/// the year's end, Jan 26, 2026, 136 + 12108 = 12244. Feb 26 bills the month's 3 days, 281, and
/// none of the run's, where the per-operation path billed the first row's 135 there.
/// Rules: R-084, R-011, R-079
#[test]
fn a_yearly_fee_in_arrears_amended_again_now_bills_its_run_at_the_years_end() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(2_900, true, 1, 120, 500),
                plan(2_900, true, 1, 7, 1_000),
            ],
            1,
        );
        start.add_on = AddOnSpec {
            price: 700,
            months: 1,
            on_b: false,
        };
        start.coupon = Some(CouponSpec {
            off: Off::Fixed(500),
            invoices: None,
            expires: Some(day(2025, 2, 15)),
        });
        let on = |m, d| day(2025, m, d);
        let amend = |on| Action::AmendNow {
            at: at(on, 15),
            plan: 0,
            price: 12_345,
            months: 12,
            rate: 120,
        };
        let mut actions = vec![
            Action::Start {
                day: on(1, 26),
                catalog: start,
            },
            amend(on(1, 29)),
            amend(on(2, 2)),
        ];
        actions.extend((2..=13u32).map(|m| Action::Advance {
            at: midnight(if m > 12 {
                day(2026, m - 12, 26)
            } else {
                on(m, 26)
            }),
        }));
        let (documents, found) = replay(&actions).await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-26 subtotal 500 total 0 due 0",
                "document:   A seats 2025-01-26..2025-02-26 qty Some(1) x Some(5.00) = 500",
                "document: Recurring Finalized dated 2025-02-26 subtotal 781 total 281 due 281",
                "document:   A seats 2025-02-26..2025-03-26 qty Some(1) x Some(5.00) = 500",
                "document:   A fee 2025-01-26..2025-01-29 qty Some(1) x Some(2.81) = 281",
                "document: Recurring Finalized dated 2025-03-26 subtotal 500 total 0 due 0",
                "document:   A seats 2025-03-26..2025-04-26 qty Some(1) x Some(5.00) = 500",
                "document: Recurring Finalized dated 2025-04-26 subtotal 500 total 0 due 0",
                "document:   A seats 2025-04-26..2025-05-26 qty Some(1) x Some(5.00) = 500",
                "document: Recurring Finalized dated 2025-05-26 subtotal 500 total 0 due 0",
                "document:   A seats 2025-05-26..2025-06-26 qty Some(1) x Some(5.00) = 500",
                "document: Recurring Finalized dated 2025-06-26 subtotal 500 total 0 due 0",
                "document:   A seats 2025-06-26..2025-07-26 qty Some(1) x Some(5.00) = 500",
                "document: Recurring Finalized dated 2025-07-26 subtotal 500 total 0 due 0",
                "document:   A seats 2025-07-26..2025-08-26 qty Some(1) x Some(5.00) = 500",
                "document: Recurring Finalized dated 2025-08-26 subtotal 500 total 0 due 0",
                "document:   A seats 2025-08-26..2025-09-26 qty Some(1) x Some(5.00) = 500",
                "document: Recurring Finalized dated 2025-09-26 subtotal 500 total 0 due 0",
                "document:   A seats 2025-09-26..2025-10-26 qty Some(1) x Some(5.00) = 500",
                "document: Recurring Finalized dated 2025-10-26 subtotal 500 total 0 due 0",
                "document:   A seats 2025-10-26..2025-11-26 qty Some(1) x Some(5.00) = 500",
                "document: Recurring Finalized dated 2025-11-26 subtotal 500 total 0 due 0",
                "document:   A seats 2025-11-26..2025-12-26 qty Some(1) x Some(5.00) = 500",
                "document: Recurring Finalized dated 2025-12-26 subtotal 500 total 0 due 0",
                "document:   A seats 2025-12-26..2026-01-26 qty Some(1) x Some(5.00) = 500",
                "document: Recurring Finalized dated 2026-01-26 subtotal 12744 total 12244 due 12244",
                "document:   A seats 2026-01-26..2026-02-26 qty Some(1) x Some(5.00) = 500",
                "document:   A fee (2025-02-02 - 2026-01-26) (2025-02-02 - 2026-01-26) 2025-02-02..2026-01-26 qty Some(1) x Some(121.08) = 12108",
                "document:   A fee (2025-01-29 - 2025-02-02) (2025-01-29 - 2025-02-02) 2025-01-29..2025-02-02 qty Some(1) x Some(1.36) = 136",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// 123.45 a month in arrears after a free trial to Feb 4, amended on Feb 7 to 29.00 a year in
/// arrears and, for the period's end, Mar 4, to 9.99 a year: one run of the year, billed at its
/// end. Mar 4 bills the month's 3 days, 1323, and none of the 29.00 row's days to Mar 4.
/// Rules: R-084, R-011
#[test]
fn a_yearly_fee_in_arrears_amended_again_for_the_periods_end_bills_none_of_its_run_on_the_renewal()
{
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(12_345, true, 1, 120, 500),
                plan(2_900, true, 1, 120, 500),
            ],
            2,
        );
        start.auto_advance = false;
        start.add_on = AddOnSpec {
            price: 9_900,
            months: 12,
            on_b: false,
        };
        start.trial = Some(TrialSpec {
            end: day(2025, 2, 4),
            card: true,
            anchor: Some(4),
        });
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 21),
                catalog: start,
            },
            Action::Advance {
                at: midnight(on(2, 4)),
            },
            Action::AmendNow {
                at: at(on(2, 7), 15),
                plan: 0,
                price: 2_900,
                months: 12,
                rate: 120,
            },
            Action::AmendAtEnd {
                at: at(on(2, 7), 16),
                plan: 0,
                price: 999,
                months: 12,
            },
            Action::Advance {
                at: midnight(on(3, 4)),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Draft dated 2025-02-04 subtotal 1000 total 1000 due 1000",
                "document:   A seats 2025-02-04..2025-03-04 qty Some(2) x Some(5.00) = 1000",
                "document: Recurring Draft dated 2025-03-04 subtotal 2323 total 2323 due 2323",
                "document:   A seats 2025-03-04..2025-04-04 qty Some(2) x Some(5.00) = 1000",
                "document:   A fee 2025-02-04..2025-02-07 qty Some(1) x Some(13.23) = 1323",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// 123.45 a month and 4 seats at 10.00 from Jan 11 under a 200.00 minimum, two 19.00 yearly
/// add-ons attached on Jan 16 (3748 to Jan 11, 2026): January counts 20093 and is trued up with
/// nothing on Feb 11. Cancelled from Feb 13, the add-ons keep 292 of their stub, January then
/// counts 16637, and the ending's invoice of Feb 13 bills what its true-up now misses, 3363,
/// beside the 261 of Feb 11–13, where January was left at 0.
/// Rules: R-022, R-016, R-077
#[test]
fn a_cancellation_giving_back_spend_of_a_period_trued_up_bills_what_its_true_up_now_misses() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(12_345, false, 1, 1, 1_000),
                plan(50_000, false, 1, 1, 500),
            ],
            4,
        );
        start.add_on = AddOnSpec {
            price: 1_900,
            months: 12,
            on_b: true,
        };
        start.minimum = Some(20_000);
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 11),
                catalog: start,
            },
            Action::AttachAddOn {
                at: at(on(1, 16), 15),
                quantity: 2,
            },
            Action::Advance {
                at: midnight(on(2, 11)),
            },
            Action::CancelNow {
                at: at(on(2, 13), 15),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-11 subtotal 16345 total 16345 due 16345",
                "document:   A fee 2025-01-11..2025-02-11 qty Some(1) x Some(123.45) = 12345",
                "document:   A seats 2025-01-11..2025-02-11 qty Some(4) x Some(10.00) = 4000",
                "document: Adjustment Finalized dated 2025-01-16 subtotal 3748 total 3748 due 292",
                "document:   Lifecycle add-on (prorated) 2025-01-16..2026-01-11 qty Some(2) x Some(18.74) = 3748",
                "document: Recurring Finalized dated 2025-02-11 subtotal 16345 total 16345 due 1168",
                "document:   A fee 2025-02-11..2025-03-11 qty Some(1) x Some(123.45) = 12345",
                "document:   A seats 2025-02-11..2025-03-11 qty Some(4) x Some(10.00) = 4000",
                "document: Recurring Finalized dated 2025-02-13 subtotal 3624 total 3624 due 3624",
                "document:   Minimum commitment 2025-02-11..2025-02-13 qty None x None = 261",
                "document:   Minimum commitment 2025-01-11..2025-02-11 qty None x None = 3363",
                "note: on INV-0003 Finalized subtotal -15177",
                "note:   A fee 2025-02-11..2025-03-11 = -11463",
                "note:   A seats 2025-02-11..2025-03-11 = -3714",
                "note: on INV-0002 Finalized subtotal -3456",
                "note:   Lifecycle add-on (prorated) 2025-01-16..2026-01-11 = -3456",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// 9.99 a month, a 99.00 capacity and 2 seats at 25.00 from Jan 7 under a 1000.00 minimum, a
/// 7.00 yearly add-on attached on Jan 12 (690): Feb 7 trues January up at 100000 − 15899 − 690 =
/// 83411. Cancelled from Feb 7 that afternoon, the add-on keeps 49 of its stub: January counts
/// 15948, and a final invoice, written for it alone, bills the 641 its true-up now misses: 84052
/// in all, where January was left at 83411.
/// Rules: R-022, R-016, R-077
#[test]
fn a_cancellation_on_the_renewal_day_bills_what_the_true_up_it_closed_now_misses() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(999, false, 1, 25, 2_500),
                plan(2_900, false, 1, 7, 2_500),
            ],
            2,
        );
        start.add_on = AddOnSpec {
            price: 700,
            months: 12,
            on_b: true,
        };
        start.minimum = Some(100_000);
        start.capacity = Some(CapacitySpec {
            fee: 9_900,
            included: 100,
            overage: 25,
        });
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 7),
                catalog: start,
            },
            Action::AttachAddOn {
                at: at(on(1, 12), 15),
                quantity: 1,
            },
            Action::Advance {
                at: midnight(on(2, 7)),
            },
            Action::CancelNow {
                at: at(on(2, 7), 15),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-07 subtotal 15899 total 15899 due 15899",
                "document:   A fee 2025-01-07..2025-02-07 qty Some(1) x Some(9.99) = 999",
                "document:   A capacity 2025-01-07..2025-02-07 qty Some(1) x Some(99.00) = 9900",
                "document:   A seats 2025-01-07..2025-02-07 qty Some(2) x Some(25.00) = 5000",
                "document: Adjustment Finalized dated 2025-01-12 subtotal 690 total 690 due 49",
                "document:   Lifecycle add-on (prorated) 2025-01-12..2026-01-07 qty Some(1) x Some(6.90) = 690",
                "document: Recurring Finalized dated 2025-02-07 subtotal 99310 total 99310 due 83411",
                "document:   A fee 2025-02-07..2025-03-07 qty Some(1) x Some(9.99) = 999",
                "document:   A capacity 2025-02-07..2025-03-07 qty Some(1) x Some(99.00) = 9900",
                "document:   A seats 2025-02-07..2025-03-07 qty Some(2) x Some(25.00) = 5000",
                "document:   Minimum commitment 2025-01-07..2025-02-07 qty None x None = 83411",
                "document: Final Finalized dated 2025-02-07 subtotal 641 total 641 due 641",
                "document:   Minimum commitment 2025-01-07..2025-02-07 qty None x None = 641",
                "note: on INV-0003 Finalized subtotal -15899",
                "note:   A fee 2025-02-07..2025-03-07 = -999",
                "note:   A capacity 2025-02-07..2025-03-07 = -9900",
                "note:   A seats 2025-02-07..2025-03-07 = -5000",
                "note: on INV-0002 Finalized subtotal -641",
                "note:   Lifecycle add-on (prorated) 2025-01-12..2026-01-07 = -641",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// A yearly plan from Jan 23 with three 99.00 yearly add-ons from Jan 26 (29456 to Jan 23, 2026),
/// changed on Jan 31 to a monthly plan keeping them, set to one on Feb 4: the add-on's run keeps
/// its year (R-084), so Feb 23's renewal bills none of it, where the per-operation path billed the
/// new row a year from Feb 23, 9900.
/// Rules: R-084
#[test]
fn an_add_on_set_to_fewer_after_a_change_of_cadence_bills_no_new_year_at_the_renewal() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(4_999, true, 12, 7, 1_000),
                plan(4_999, true, 1, 1, 500),
            ],
            4,
        );
        start.add_on = AddOnSpec {
            price: 9_900,
            months: 12,
            on_b: true,
        };
        start.coupon = Some(CouponSpec {
            off: Off::Percent(10),
            invoices: Some(2),
            expires: Some(day(2025, 2, 12)),
        });
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 23),
                catalog: start,
            },
            Action::AttachAddOn {
                at: at(on(1, 26), 15),
                quantity: 3,
            },
            Action::ChangeNow {
                at: at(on(1, 31), 15),
                to: 1,
            },
            Action::AddOnQuantity {
                at: at(on(2, 4), 15),
                quantity: 1,
            },
            Action::Advance {
                at: midnight(on(2, 23)),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-23 subtotal 4000 total 3600 due 79",
                "document:   A seats 2025-01-23..2026-01-23 qty Some(4) x Some(10.00) = 4000",
                "document: Adjustment Finalized dated 2025-01-26 subtotal 29456 total 29456 due 10307",
                "document:   Lifecycle add-on (prorated) 2025-01-26..2026-01-23 qty Some(3) x Some(98.18666667) = 29456",
                "document: Adjustment Finalized dated 2025-01-31 subtotal 1484 total 1484 due 1484",
                "document:   B seats (prorated) 2025-01-31..2025-02-23 qty Some(1) x Some(14.84) = 1484",
                "document: Recurring Finalized dated 2025-02-23 subtotal 5819 total 5238 due 5238",
                "document:   B fee 2025-01-31..2025-02-23 qty Some(1) x Some(37.09) = 3709",
                "document:   B seats 2025-02-23..2025-03-23 qty Some(4) x Some(5.00) = 2000",
                "document:   A fee 2025-01-23..2025-01-31 qty Some(1) x Some(1.10) = 110",
                "note: on INV-0002 Finalized subtotal -19149",
                "note:   Lifecycle add-on (prorated) 2025-01-26..2026-01-23 = -19149",
                "note: on INV-0001 Finalized subtotal -3912",
                "note:   A seats 2025-01-23..2026-01-23 = -3912",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// Drafts issued together on Mar 1: Jan 1's, first, gives back 27178 of a 29700 yearly add-on a
/// change ended Feb 1, so Feb 1's trues January up at 100000 − 5499 − 2522 = 91979, where the
/// reference read that note as written after the close and owed 64801. Rules: R-022, R-077
#[test]
fn a_note_a_late_draft_writes_before_the_closing_draft_counts_in_its_true_up() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [plan(4_999, false, 1, 1, 500), plan(4_999, false, 1, 1, 500)],
            1,
        );
        start.auto_advance = false;
        start.add_on = AddOnSpec {
            price: 9_900,
            months: 12,
            on_b: false,
        };
        start.minimum = Some(100_000);
        let on = |m, d| day(2025, m, d);
        let next = |m, d| day(2026, m, d);
        let mut actions = vec![
            Action::Start {
                day: on(1, 1),
                catalog: start,
            },
            Action::AttachAddOn {
                at: at(on(1, 1), 15),
                quantity: 3,
            },
        ];
        actions.extend((2..=12).map(|m| Action::Advance {
            at: midnight(on(m, 1)),
        }));
        actions.extend([
            Action::Advance {
                at: midnight(next(1, 1)),
            },
            Action::ChangeAtEnd {
                at: at(next(1, 1), 15),
                to: 1,
                seats: Some(1),
            },
            Action::Advance {
                at: midnight(next(2, 1)),
            },
            Action::CancelAtEnd {
                at: at(next(2, 1), 15),
                end: next(3, 1),
            },
            Action::Advance {
                at: midnight(next(3, 1)),
            },
            Action::FinalizeAll {
                at: at(next(3, 1), 16),
            },
        ]);
        let (documents, found) = replay(&actions).await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2026-03-01 subtotal 5499 total 5499 due 5499",
                "document:   A fee 2025-01-01..2025-02-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-01-01..2025-02-01 qty Some(1) x Some(5.00) = 500",
                "document: Adjustment Finalized dated 2025-01-01 subtotal 29700 total 29700 due 29700",
                "document:   Lifecycle add-on 2025-01-01..2026-01-01 qty Some(3) x Some(99.00) = 29700",
                "document: Recurring Finalized dated 2026-03-01 subtotal 70300 total 70300 due 70300",
                "document:   A fee 2025-02-01..2025-03-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-02-01..2025-03-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-01-01..2025-02-01 qty None x None = 64801",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-03-01..2025-04-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-03-01..2025-04-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-02-01..2025-03-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-04-01..2025-05-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-04-01..2025-05-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-03-01..2025-04-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-05-01..2025-06-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-05-01..2025-06-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-04-01..2025-05-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-06-01..2025-07-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-06-01..2025-07-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-05-01..2025-06-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-07-01..2025-08-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-07-01..2025-08-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-06-01..2025-07-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-08-01..2025-09-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-08-01..2025-09-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-07-01..2025-08-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-09-01..2025-10-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-09-01..2025-10-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-08-01..2025-09-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-10-01..2025-11-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-10-01..2025-11-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-09-01..2025-10-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-11-01..2025-12-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-11-01..2025-12-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-10-01..2025-11-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-12-01..2026-01-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-12-01..2026-01-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-11-01..2025-12-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 129700 total 129700 due 102522",
                "document:   A fee 2026-01-01..2026-02-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2026-01-01..2026-02-01 qty Some(1) x Some(5.00) = 500",
                "document:   Lifecycle add-on 2026-01-01..2027-01-01 qty Some(3) x Some(99.00) = 29700",
                "document:   Minimum commitment 2025-12-01..2026-01-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 97478 total 97478 due 97478",
                "document:   B fee 2026-02-01..2026-03-01 qty Some(1) x Some(49.99) = 4999",
                "document:   B seats 2026-02-01..2026-03-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2026-01-01..2026-02-01 qty None x None = 91979",
                "document: Recurring Finalized dated 2026-03-01 subtotal 94501 total 94501 due 94501",
                "document:   Minimum commitment 2026-02-01..2026-03-01 qty None x None = 94501",
                "note: on INV-0014 Finalized subtotal -27178",
                "note:   Lifecycle add-on 2026-01-01..2027-01-01 = -27178",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// Cancelled from Mar 1, Feb 1's draft issued before Jan 1's: January counts the add-on whole, 64801,
/// and the final invoice bills the 24899 its note leaves missing, 89700 in all; January is never
/// cut at its own end as if the subscription had ended then. Rules: R-022, R-016
#[test]
fn a_true_up_issued_before_the_note_of_a_later_ending_counts_the_days_to_that_ending() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [plan(4_999, false, 1, 1, 500), plan(4_999, false, 1, 1, 500)],
            1,
        );
        start.auto_advance = false;
        start.add_on = AddOnSpec {
            price: 9_900,
            months: 12,
            on_b: false,
        };
        start.minimum = Some(100_000);
        let on = |m, d| day(2025, m, d);
        let next = |m, d| day(2026, m, d);
        let mut actions = vec![
            Action::Start {
                day: on(1, 1),
                catalog: start,
            },
            Action::AttachAddOn {
                at: at(on(1, 1), 15),
                quantity: 3,
            },
        ];
        actions.extend((2..=12).map(|m| Action::Advance {
            at: midnight(on(m, 1)),
        }));
        actions.extend([
            Action::Advance {
                at: midnight(next(1, 1)),
            },
            Action::Advance {
                at: midnight(next(2, 1)),
            },
            Action::CancelAtEnd {
                at: at(next(2, 1), 15),
                end: next(3, 1),
            },
            Action::Advance {
                at: midnight(next(3, 1)),
            },
            Action::Finalize {
                at: at(next(3, 1), 16),
                pick: 13,
            },
            Action::FinalizeAll {
                at: at(next(3, 1), 17),
            },
        ]);
        let (documents, found) = replay(&actions).await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2026-03-01 subtotal 5499 total 5499 due 5499",
                "document:   A fee 2025-01-01..2025-02-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-01-01..2025-02-01 qty Some(1) x Some(5.00) = 500",
                "document: Adjustment Finalized dated 2025-01-01 subtotal 29700 total 29700 due 29700",
                "document:   Lifecycle add-on 2025-01-01..2026-01-01 qty Some(3) x Some(99.00) = 29700",
                "document: Recurring Finalized dated 2026-03-01 subtotal 70300 total 70300 due 70300",
                "document:   A fee 2025-02-01..2025-03-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-02-01..2025-03-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-01-01..2025-02-01 qty None x None = 64801",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-03-01..2025-04-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-03-01..2025-04-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-02-01..2025-03-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-04-01..2025-05-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-04-01..2025-05-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-03-01..2025-04-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-05-01..2025-06-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-05-01..2025-06-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-04-01..2025-05-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-06-01..2025-07-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-06-01..2025-07-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-05-01..2025-06-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-07-01..2025-08-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-07-01..2025-08-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-06-01..2025-07-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-08-01..2025-09-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-08-01..2025-09-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-07-01..2025-08-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-09-01..2025-10-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-09-01..2025-10-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-08-01..2025-09-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-10-01..2025-11-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-10-01..2025-11-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-09-01..2025-10-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-11-01..2025-12-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-11-01..2025-12-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-10-01..2025-11-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 100000 total 100000 due 100000",
                "document:   A fee 2025-12-01..2026-01-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2025-12-01..2026-01-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2025-11-01..2025-12-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 129700 total 129700 due 104801",
                "document:   A fee 2026-01-01..2026-02-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2026-01-01..2026-02-01 qty Some(1) x Some(5.00) = 500",
                "document:   Lifecycle add-on 2026-01-01..2027-01-01 qty Some(3) x Some(99.00) = 29700",
                "document:   Minimum commitment 2025-12-01..2026-01-01 qty None x None = 94501",
                "document: Recurring Finalized dated 2026-03-01 subtotal 70300 total 70300 due 70300",
                "document:   A fee 2026-02-01..2026-03-01 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2026-02-01..2026-03-01 qty Some(1) x Some(5.00) = 500",
                "document:   Minimum commitment 2026-01-01..2026-02-01 qty None x None = 64801",
                "document: Recurring Finalized dated 2026-03-01 subtotal 119400 total 119400 due 119400",
                "document:   Minimum commitment 2026-02-01..2026-03-01 qty None x None = 94501",
                "document:   Minimum commitment 2026-01-01..2026-02-01 qty None x None = 24899",
                "note: on INV-0015 Finalized subtotal -24899",
                "note:   Lifecycle add-on 2026-01-01..2027-01-01 = -24899",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// January's 16899 counted at its close on Feb 2, which trued up nothing, then voided; cancelled
/// on Apr 2 from Jan 5: January counts nothing, so the final invoice trues up 5000 × 3/31 = 484,
/// where the reference bounded the close's true-up by a spend without the void. Rules: R-022, R-096
#[test]
fn a_void_after_the_close_leaves_that_close_as_it_counted_and_a_backdated_cut_bills_the_rest() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(4_999, false, 1, 25, 500),
                plan(999, false, 1, 25, 2_500),
            ],
            4,
        );
        start.add_on = AddOnSpec {
            price: 700,
            months: 1,
            on_b: false,
        };
        start.minimum = Some(5_000);
        start.capacity = Some(CapacitySpec {
            fee: 9_900,
            included: 100,
            overage: 25,
        });
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 2),
                catalog: start,
            },
            Action::Advance {
                at: midnight(on(2, 2)),
            },
            Action::Void {
                at: at(on(2, 2), 17),
                pick: 0,
            },
            Action::Advance {
                at: midnight(on(3, 2)),
            },
            Action::Advance {
                at: midnight(on(4, 2)),
            },
            Action::CancelBackdated {
                at: at(on(4, 2), 15),
                day: on(1, 5),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Void dated 2025-01-02 subtotal 16899 total 16899 due 16899",
                "document:   A fee 2025-01-02..2025-02-02 qty Some(1) x Some(49.99) = 4999",
                "document:   A capacity 2025-01-02..2025-02-02 qty Some(1) x Some(99.00) = 9900",
                "document:   A seats 2025-01-02..2025-02-02 qty Some(4) x Some(5.00) = 2000",
                "document: Recurring Finalized dated 2025-02-02 subtotal 16899 total 16899 due 0",
                "document:   A fee 2025-02-02..2025-03-02 qty Some(1) x Some(49.99) = 4999",
                "document:   A capacity 2025-02-02..2025-03-02 qty Some(1) x Some(99.00) = 9900",
                "document:   A seats 2025-02-02..2025-03-02 qty Some(4) x Some(5.00) = 2000",
                "document: Recurring Finalized dated 2025-03-02 subtotal 16899 total 16899 due 0",
                "document:   A fee 2025-03-02..2025-04-02 qty Some(1) x Some(49.99) = 4999",
                "document:   A capacity 2025-03-02..2025-04-02 qty Some(1) x Some(99.00) = 9900",
                "document:   A seats 2025-03-02..2025-04-02 qty Some(4) x Some(5.00) = 2000",
                "document: Recurring Finalized dated 2025-04-02 subtotal 16899 total 16899 due 0",
                "document:   A fee 2025-04-02..2025-05-02 qty Some(1) x Some(49.99) = 4999",
                "document:   A capacity 2025-04-02..2025-05-02 qty Some(1) x Some(99.00) = 9900",
                "document:   A seats 2025-04-02..2025-05-02 qty Some(4) x Some(5.00) = 2000",
                "document: Final Finalized dated 2025-04-02 subtotal 484 total 484 due 484",
                "document:   Minimum commitment 2025-01-02..2025-01-05 qty None x None = 484",
                "note: on INV-0004 Finalized subtotal -16899",
                "note:   A fee 2025-04-02..2025-05-02 = -4999",
                "note:   A capacity 2025-04-02..2025-05-02 = -9900",
                "note:   A seats 2025-04-02..2025-05-02 = -2000",
                "note: on INV-0003 Finalized subtotal -16899",
                "note:   A fee 2025-03-02..2025-04-02 = -4999",
                "note:   A capacity 2025-03-02..2025-04-02 = -9900",
                "note:   A seats 2025-03-02..2025-04-02 = -2000",
                "note: on INV-0002 Finalized subtotal -16899",
                "note:   A fee 2025-02-02..2025-03-02 = -4999",
                "note:   A capacity 2025-02-02..2025-03-02 = -9900",
                "note:   A seats 2025-02-02..2025-03-02 = -2000",
                "note: on INV-0001 Finalized subtotal -16899",
                "note:   A fee 2025-01-02..2025-02-02 = -4999",
                "note:   A capacity 2025-01-02..2025-02-02 = -9900",
                "note:   A seats 2025-01-02..2025-02-02 = -2000",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// 29.00 a month and 4 seats at 10.00 from Jan 12 under a 200.00 minimum and a 25% coupon for one
/// invoice (5175), two 19.00 yearly add-ons attached on Jan 17 (3748 to Jan 12, 2026): Feb 12
/// trues January up at 20000 − 5175 − 3748 = 11077. Cancelled at the period's end, Mar 12, the
/// add-ons give back 3748 × 306/360 = 3186 on the cancellation's note, and the recurring invoice
/// of Mar 12 trues February up at 13100 and January again at 3186, 16286 in all: 40000 over the
/// two periods. The give-back refused that invoice, January's true-up billed on two invoices of
/// the year, and once issued its January line settled the Jan 17 amendment again, writing the
/// 3186 as that amendment's note.
/// Rules: R-022, R-075, R-077
#[test]
fn a_cancellation_at_the_period_end_reopening_a_true_up_issues_its_invoice_of_that_day() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(2_900, false, 1, 25, 1_000),
                plan(2_900, false, 1, 1, 1_000),
            ],
            4,
        );
        start.add_on = AddOnSpec {
            price: 1_900,
            months: 12,
            on_b: true,
        };
        start.coupon = Some(CouponSpec {
            off: Off::Percent(25),
            invoices: Some(1),
            expires: Some(day(2025, 2, 1)),
        });
        start.minimum = Some(20_000);
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 12),
                catalog: start,
            },
            Action::AttachAddOn {
                at: at(on(1, 17), 15),
                quantity: 2,
            },
            Action::Advance {
                at: midnight(on(2, 12)),
            },
            Action::CancelAtEnd {
                at: at(on(2, 12), 15),
                end: on(3, 12),
            },
            Action::Advance {
                at: midnight(on(3, 12)),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-12 subtotal 6900 total 5175 due 5175",
                "document:   A fee 2025-01-12..2025-02-12 qty Some(1) x Some(29.00) = 2900",
                "document:   A seats 2025-01-12..2025-02-12 qty Some(4) x Some(10.00) = 4000",
                "document: Adjustment Finalized dated 2025-01-17 subtotal 3748 total 3748 due 562",
                "document:   Lifecycle add-on (prorated) 2025-01-17..2026-01-12 qty Some(2) x Some(18.74) = 3748",
                "document: Recurring Finalized dated 2025-02-12 subtotal 17977 total 17977 due 17977",
                "document:   A fee 2025-02-12..2025-03-12 qty Some(1) x Some(29.00) = 2900",
                "document:   A seats 2025-02-12..2025-03-12 qty Some(4) x Some(10.00) = 4000",
                "document:   Minimum commitment 2025-01-12..2025-02-12 qty None x None = 11077",
                "document: Recurring Finalized dated 2025-03-12 subtotal 16286 total 16286 due 16286",
                "document:   Minimum commitment 2025-02-12..2025-03-12 qty None x None = 13100",
                "document:   Minimum commitment 2025-01-12..2025-02-12 qty None x None = 3186",
                "note: on INV-0002 Finalized subtotal -3186",
                "note:   Lifecycle add-on (prorated) 2025-01-17..2026-01-12 = -3186",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// Four 5.00 yearly seats on plan B from Jan 10, 2026 (1998 with two bought on Jan 11), its fee
/// amended to 500.00 a month on Jan 10, 2027, then changed to plan A, yearly, on Feb 10. Both
/// plans sell their seats under one product: the change carries them (R-108) and their run goes
/// on, on its Jan 10 grid (R-084), billed 2000 on Jan 10, 2027 and 2000 on Jan 10, 2028, of
/// which a cancellation at once on Feb 14 gives back 2000 × 331/366 = 1809: the seats owe 2191
/// from Jan 10, 2027, where the reference opened a lineage on Feb 10 and owed 2192.
/// Rules: R-084, R-108, R-079
#[test]
fn seats_carried_into_a_plan_of_another_cadence_keep_their_run_on_its_grid() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(50_000, false, 12, 120, 500),
                plan(999, true, 12, 25, 500),
            ],
            3,
        );
        start.auto_advance = false;
        start.add_on = AddOnSpec {
            price: 9_900,
            months: 1,
            on_b: true,
        };
        let on = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).expect("a day");
        let (documents, found) = replay(&[
            Action::Start {
                day: on(2025, 1, 10),
                catalog: start,
            },
            Action::ChangeAtEnd {
                at: at(on(2025, 1, 10), 15),
                to: 1,
                seats: None,
            },
            Action::RemoveSeats {
                at: at(on(2025, 1, 10), 16),
                seats: 1,
            },
            Action::Advance {
                at: midnight(on(2026, 1, 10)),
            },
            Action::BuySeats {
                at: at(on(2026, 1, 11), 15),
                seats: 2,
            },
            Action::Advance {
                at: midnight(on(2027, 1, 10)),
            },
            Action::AmendNow {
                at: at(on(2027, 1, 10), 15),
                plan: 1,
                price: 50_000,
                months: 1,
                rate: 25,
            },
            Action::Advance {
                at: midnight(on(2027, 2, 10)),
            },
            Action::ChangeNow {
                at: at(on(2027, 2, 10), 15),
                to: 0,
            },
            Action::Advance {
                at: midnight(on(2028, 2, 10)),
            },
            Action::Usage {
                at: at(on(2028, 2, 11), 14),
                units: 18,
            },
            Action::CancelImmediately {
                at: at(on(2028, 2, 14), 15),
            },
            Action::FinalizeAll {
                at: at(on(2028, 2, 14), 16),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2028-02-14 subtotal 51500 total 51500 due 51500",
                "document:   A fee 2025-01-10..2026-01-10 qty Some(1) x Some(500.00) = 50000",
                "document:   A seats 2025-01-10..2026-01-10 qty Some(3) x Some(5.00) = 1500",
                "document: Recurring Finalized dated 2028-02-14 subtotal 1000 total 1000 due 1000",
                "document:   B seats 2026-01-10..2027-01-10 qty Some(2) x Some(5.00) = 1000",
                "document: Adjustment Finalized dated 2028-02-14 subtotal 998 total 998 due 998",
                "document:   2 seat upgrade 2026-01-11..2027-01-10 qty Some(2) x Some(4.99) = 998",
                "document: Recurring Finalized dated 2028-02-14 subtotal 2999 total 2999 due 2999",
                "document:   B fee 2026-01-10..2027-01-10 qty Some(1) x Some(9.99) = 999",
                "document:   B seats 2027-01-10..2028-01-10 qty Some(4) x Some(5.00) = 2000",
                "document: Recurring Finalized dated 2028-02-14 subtotal 50000 total 50000 due 50000",
                "document:   B fee 2027-01-10..2027-02-10 qty Some(1) x Some(500.00) = 50000",
                "document: Adjustment Finalized dated 2027-02-10 subtotal 50000 total 50000 due 50000",
                "document:   A fee 2027-02-10..2028-02-10 qty Some(1) x Some(500.00) = 50000",
                "document: Recurring Finalized dated 2028-02-14 subtotal 52000 total 52000 due 737",
                "document:   A fee 2028-02-10..2029-02-10 qty Some(1) x Some(500.00) = 50000",
                "document:   A seats 2028-01-10..2029-01-10 qty Some(4) x Some(5.00) = 2000",
                "document: Recurring Finalized dated 2028-02-14 subtotal 2160 total 2160 due 2160",
                "document:   A units 2028-02-10..2028-02-14 qty Some(18) x Some(1.20) = 2160",
                "note: on INV-0007 Finalized subtotal -51263",
                "note:   A seats 2028-01-10..2029-01-10 = -1809",
                "note:   A fee 2028-02-10..2029-02-10 = -49454",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// One 5.00 yearly seat on plan A from Jan 3, 2025 and three bought on Feb 4 (1369), changed on
/// Feb 9 to plan B, whose yearly seats cost 10.00: the target bills the seats at another price,
/// so the change opens their lineage on the grid from its origin, Mar 3 (R-084). The old seats
/// give back their unused days, 449 and 1348, and B's four bill 4000 × 22/365 = 241 to Mar 3,
/// then 4000 a year from Mar 3, given back whole by the cancellation at once that day.
/// Rules: R-084, R-108, R-079
#[test]
fn seats_a_change_bills_at_another_price_open_a_run_on_its_grid() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(12_345, false, 12, 120, 500),
                plan(4_999, false, 12, 7, 1_000),
            ],
            1,
        );
        start.add_on = AddOnSpec {
            price: 9_900,
            months: 1,
            on_b: false,
        };
        start.coupon = Some(CouponSpec {
            off: Off::Percent(50),
            invoices: Some(1),
            expires: Some(day(2025, 1, 23)),
        });
        let on = |m, d| day(2025, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(1, 3),
                catalog: start,
            },
            Action::AttachAddOn {
                at: at(on(1, 8), 15),
                quantity: 2,
            },
            Action::Advance {
                at: midnight(on(2, 3)),
            },
            Action::BuySeats {
                at: at(on(2, 4), 15),
                seats: 3,
            },
            Action::ChangeNow {
                at: at(on(2, 9), 15),
                to: 1,
            },
            Action::Advance {
                at: midnight(on(3, 3)),
            },
            Action::CancelImmediately {
                at: at(on(3, 3), 15),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-03 subtotal 12845 total 6423 due 651",
                "document:   A fee 2025-01-03..2026-01-03 qty Some(1) x Some(123.45) = 12345",
                "document:   A seats 2025-01-03..2026-01-03 qty Some(1) x Some(5.00) = 500",
                "document: Adjustment Finalized dated 2025-01-08 subtotal 16606 total 16606 due 16606",
                "document:   Lifecycle add-on (prorated) 2025-01-08..2025-02-03 qty Some(2) x Some(83.03) = 16606",
                "document: Recurring Finalized dated 2025-02-03 subtotal 19800 total 19800 due 4243",
                "document:   Lifecycle add-on 2025-02-03..2025-03-03 qty Some(2) x Some(99.00) = 19800",
                "document: Adjustment Finalized dated 2025-02-04 subtotal 1369 total 1369 due 21",
                "document:   3 seat upgrade 2025-02-04..2026-01-03 qty Some(3) x Some(4.56333333) = 1369",
                "document: Adjustment Finalized dated 2025-02-09 subtotal 542 total 542 due 542",
                "document:   B fee (prorated) 2025-02-09..2025-03-03 qty Some(1) x Some(3.01) = 301",
                "document:   B seats (prorated) 2025-02-09..2025-03-03 qty Some(1) x Some(2.41) = 241",
                "document: Recurring Finalized dated 2025-03-03 subtotal 8999 total 8999 due 0",
                "document:   B fee 2025-03-03..2026-03-03 qty Some(1) x Some(49.99) = 4999",
                "document:   B seats 2025-03-03..2026-03-03 qty Some(4) x Some(10.00) = 4000",
                "document: Final Closed dated 2025-03-03 subtotal 0 total 0 due 0",
                "note: on INV-0006 Finalized subtotal -8999",
                "note:   B fee 2025-03-03..2026-03-03 = -4999",
                "note:   B seats 2025-03-03..2026-03-03 = -4000",
                "note: on INV-0004 Finalized subtotal -1348",
                "note:   3 seat upgrade 2025-02-04..2026-01-03 = -1348",
                "note: on INV-0003 Finalized subtotal -15557",
                "note:   Lifecycle add-on 2025-02-03..2025-03-03 = -15557",
                "note: on INV-0001 Finalized subtotal -11543",
                "note:   A fee 2025-01-03..2026-01-03 = -11094",
                "note:   A seats 2025-01-03..2026-01-03 = -449",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}

/// Four 25.00 yearly seats from Jan 9, 2025, a change ending the free trial that day, carried
/// into plan A on Jan 14 on their own Jan 9 grid while the subscription renews on Feb 9. Three
/// removed on Feb 9 for the end of the seats' period leave on Jan 9, 2026: the renewal of Feb 9
/// bills that year at 1 seat, 2500, where the removal was dated on the subscription's Feb 9 and
/// the year billed 4 seats, 10000. Cancelled at once that day, the seats give back 2500 × 334/365
/// = 2288 and keep 212.
/// Rules: R-108, R-084, R-079
#[test]
fn a_seat_removal_for_its_fees_period_end_bills_the_next_period_at_the_new_count() {
    harness::on_large_stacks(|| async {
        let mut start = catalog(
            [
                plan(4_999, false, 12, 1, 2_500),
                plan(2_900, false, 12, 7, 2_500),
            ],
            4,
        );
        start.add_on = AddOnSpec {
            price: 700,
            months: 1,
            on_b: false,
        };
        start.trial = Some(TrialSpec {
            end: day(2025, 1, 20),
            card: true,
            anchor: Some(20),
        });
        let on = |y, m, d| day(y, m, d);
        let (documents, found) = replay(&[
            Action::Start {
                day: on(2025, 1, 6),
                catalog: start,
            },
            Action::ChangeNow {
                at: at(on(2025, 1, 9), 15),
                to: 1,
            },
            Action::AttachAddOn {
                at: at(on(2025, 1, 11), 15),
                quantity: 1,
            },
            Action::ChangeNow {
                at: at(on(2025, 1, 14), 15),
                to: 0,
            },
            Action::Advance {
                at: midnight(on(2025, 2, 9)),
            },
            Action::RemoveSeats {
                at: at(on(2025, 2, 9), 15),
                seats: 3,
            },
            Action::Advance {
                at: midnight(on(2026, 2, 9)),
            },
            Action::CancelImmediately {
                at: at(on(2026, 2, 9), 16),
            },
        ])
        .await;
        assert_eq!(
            documents,
            [
                "document: Recurring Finalized dated 2025-01-09 subtotal 12900 total 12900 due 10040",
                "document:   B fee 2025-01-09..2026-01-09 qty Some(1) x Some(29.00) = 2900",
                "document:   B seats 2025-01-09..2026-01-09 qty Some(4) x Some(25.00) = 10000",
                "document: Adjustment Finalized dated 2025-01-11 subtotal 655 total 655 due 655",
                "document:   Lifecycle add-on (prorated) 2025-01-11..2025-02-09 qty Some(1) x Some(6.55) = 655",
                "document: Adjustment Finalized dated 2025-01-14 subtotal 355 total 355 due 355",
                "document:   A fee (prorated) 2025-01-14..2025-02-09 qty Some(1) x Some(3.55) = 355",
                "document: Recurring Finalized dated 2025-02-09 subtotal 5699 total 5699 due 5699",
                "document:   A fee 2025-02-09..2026-02-09 qty Some(1) x Some(49.99) = 4999",
                "document:   Lifecycle add-on 2025-02-09..2025-03-09 qty Some(1) x Some(7.00) = 700",
                "document: Recurring Finalized dated 2026-02-09 subtotal 15899 total 15899 due 7912",
                "document:   A fee 2026-02-09..2027-02-09 qty Some(1) x Some(49.99) = 4999",
                "document:   A seats 2026-01-09..2027-01-09 qty Some(1) x Some(25.00) = 2500",
                "document:   Lifecycle add-on 2025-03-09..2025-04-09 qty Some(1) x Some(7.00) = 700",
                "document:   Lifecycle add-on 2025-04-09..2025-05-09 qty Some(1) x Some(7.00) = 700",
                "document:   Lifecycle add-on 2025-05-09..2025-06-09 qty Some(1) x Some(7.00) = 700",
                "document:   Lifecycle add-on 2025-06-09..2025-07-09 qty Some(1) x Some(7.00) = 700",
                "document:   Lifecycle add-on 2025-07-09..2025-08-09 qty Some(1) x Some(7.00) = 700",
                "document:   Lifecycle add-on 2025-08-09..2025-09-09 qty Some(1) x Some(7.00) = 700",
                "document:   Lifecycle add-on 2025-09-09..2025-10-09 qty Some(1) x Some(7.00) = 700",
                "document:   Lifecycle add-on 2025-10-09..2025-11-09 qty Some(1) x Some(7.00) = 700",
                "document:   Lifecycle add-on 2025-11-09..2025-12-09 qty Some(1) x Some(7.00) = 700",
                "document:   Lifecycle add-on 2025-12-09..2026-01-09 qty Some(1) x Some(7.00) = 700",
                "document:   Lifecycle add-on 2026-01-09..2026-02-09 qty Some(1) x Some(7.00) = 700",
                "document:   Lifecycle add-on 2026-02-09..2026-03-09 qty Some(1) x Some(7.00) = 700",
                "document: Final Closed dated 2026-02-09 subtotal 0 total 0 due 0",
                "note: on INV-0005 Finalized subtotal -7987",
                "note:   A seats 2026-01-09..2027-01-09 = -2288",
                "note:   A fee 2026-02-09..2027-02-09 = -4999",
                "note:   Lifecycle add-on 2026-02-09..2026-03-09 = -700",
                "note: on INV-0001 Finalized subtotal -2860",
                "note:   B fee 2025-01-09..2026-01-09 = -2860",
            ],
            "{found:#?}"
        );
        assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    });
}
