//! A change on a subscription that doesn't auto-advance leaves its adjustment invoice a draft
//! (R-166): nothing is issued until the operator finalizes, and once all is finalized the life
//! bills what it bills when every document finalizes on its own, in whatever order.
use super::pinned::{at, catalog, day, plan, replay};
use super::reference::{Action, Catalog, CouponSpec, Off, midnight};
use crate::model::harness;
use chrono::NaiveDate;

/// A document's or a note's first line, its lines left out.
fn headers(documents: &[String]) -> Vec<&str> {
    documents
        .iter()
        .filter(|line| {
            (line.starts_with("document: ") && !line.starts_with("document:  "))
                || line.starts_with("note: on ")
        })
        .map(String::as_str)
        .collect()
}

/// A at 31.00 and B at 62.00 a month in advance, each with a 5.00 seat, one seat.
fn monthly(auto_advance: bool) -> Catalog {
    let mut start = catalog(
        [plan(3_100, false, 1, 1, 500), plan(6_200, false, 1, 1, 500)],
        1,
    );
    start.auto_advance = auto_advance;
    start
}

fn on(m: u32, d: u32) -> NaiveDate {
    day(2025, m, d)
}

/// Started Jan 1, `before`, upgraded to B on Jan 16, advanced to Feb 1, then `after`.
fn upgraded(start: Catalog, before: &[Action], after: &[Action]) -> Vec<Action> {
    let mut actions = vec![Action::Start {
        day: on(1, 1),
        catalog: start,
    }];
    actions.extend_from_slice(before);
    actions.extend([
        Action::ChangeNow {
            at: at(on(1, 16), 15),
            to: 1,
        },
        Action::Advance {
            at: midnight(on(2, 1)),
        },
    ]);
    actions.extend_from_slice(after);
    actions
}

/// January finalized and paid on Jan 2, as an operator holding drafts would.
fn january_paid() -> [Action; 2] {
    [
        Action::Finalize {
            at: at(on(1, 2), 9),
            pick: 0,
        },
        Action::Pay {
            at: at(on(1, 2), 10),
            pick: 0,
        },
    ]
}

/// February's renewal issued before the adjustment.
fn renewal_first() -> [Action; 2] {
    [
        Action::Finalize {
            at: at(on(2, 1), 16),
            pick: 1,
        },
        Action::Finalize {
            at: at(on(2, 1), 17),
            pick: 0,
        },
    ]
}

async fn documents_of(actions: &[Action]) -> Vec<String> {
    let (documents, found) = replay(actions).await;
    assert_eq!(found, Vec::<String>::new(), "{documents:#?}");
    documents
}

/// Upgraded on Jan 16 while January is a draft: B's stub 6200 × 16/31 = 3200 waits, and no note
/// is written until January is issued; then A's unused 3100 × 16/31 = 1600 comes back on it,
/// leaving it 2000 due, as auto-advancing. Rules: R-166, R-078, R-079
#[test]
fn an_upgrade_without_auto_advance_issues_nothing_and_bills_the_same_once_issued() {
    harness::on_large_stacks(|| async {
        let held = documents_of(&upgraded(monthly(false), &[], &[])).await;
        assert_eq!(
            headers(&held),
            [
                "document: Recurring Draft dated 2025-01-01 subtotal 3600 total 3600 due 3600",
                "document: Adjustment Draft dated 2025-01-16 subtotal 3200 total 3200 due 3200",
                "document: Recurring Draft dated 2025-02-01 subtotal 6700 total 6700 due 6700",
            ],
            "{held:#?}"
        );

        let issued = at(on(2, 1), 16);
        let in_order = documents_of(&upgraded(
            monthly(false),
            &[],
            &[Action::FinalizeAll { at: issued }],
        ))
        .await;
        assert_eq!(
            headers(&in_order),
            [
                "document: Recurring Finalized dated 2025-02-01 subtotal 3600 total 3600 due 2000",
                "document: Adjustment Finalized dated 2025-02-01 subtotal 3200 total 3200 due 3200",
                "document: Recurring Finalized dated 2025-02-01 subtotal 6700 total 6700 due 6700",
                "note: on INV-0001 Finalized subtotal -1600",
            ],
            "{in_order:#?}"
        );

        let latest_first: Vec<Action> = (0..3)
            .rev()
            .map(|pick| Action::Finalize { at: issued, pick })
            .collect();
        let reversed = documents_of(&upgraded(monthly(false), &[], &latest_first)).await;
        // Issued latest first, January is numbered last: what a note is on is left out.
        let unnumbered = |documents: &[String]| -> Vec<String> {
            let mut lines: Vec<String> = headers(documents)
                .into_iter()
                .map(|line| match line.strip_prefix("note: on ") {
                    Some(rest) => format!("note: {}", rest.split_once(' ').map_or(rest, |r| r.1)),
                    None => line.to_owned(),
                })
                .collect();
            lines.sort_unstable();
            lines
        };
        assert_eq!(
            unnumbered(&reversed),
            unnumbered(&in_order),
            "{reversed:#?}"
        );
    });
}

/// January paid, upgraded on Jan 16: its 1600 note returns credit to the balance. Auto-advancing,
/// the adjustment takes it (3200 → 1600 due); held, February's renewal issued first takes it
/// (6700 → 5100) and the adjustment is due whole. The credit moves; the money owed doesn't.
/// Rules: R-166, R-078
#[test]
fn the_credit_a_draft_adjustment_leaves_on_the_balance_pays_whatever_is_issued_first() {
    harness::on_large_stacks(|| async {
        let pay_january = [Action::Pay {
            at: at(on(1, 2), 10),
            pick: 0,
        }];
        let by_itself = documents_of(&upgraded(monthly(true), &pay_january, &[])).await;
        assert_eq!(
            headers(&by_itself),
            [
                "document: Recurring Finalized dated 2025-01-01 subtotal 3600 total 3600 due 0",
                "document: Adjustment Finalized dated 2025-01-16 subtotal 3200 total 3200 due 1600",
                "document: Recurring Finalized dated 2025-02-01 subtotal 6700 total 6700 due 6700",
                "note: on INV-0001 Finalized subtotal -1600",
            ],
            "{by_itself:#?}"
        );

        let held = documents_of(&upgraded(monthly(false), &january_paid(), &renewal_first())).await;
        assert_eq!(
            headers(&held),
            [
                "document: Recurring Finalized dated 2025-01-02 subtotal 3600 total 3600 due 0",
                "document: Adjustment Finalized dated 2025-02-01 subtotal 3200 total 3200 due 3200",
                "document: Recurring Finalized dated 2025-02-01 subtotal 6700 total 6700 due 5100",
                "note: on INV-0001 Finalized subtotal -1600",
            ],
            "{held:#?}"
        );
    });
}

/// A 10% coupon for 2 invoices goes to January (3240) and February (6030), the period invoices,
/// never to the draft adjustment, though February is issued first; the note's credit is net of
/// January's discount, 1600 × 0.9 = 1440, so February is due 4590. Rules: R-166, R-089
#[test]
fn a_limited_coupon_skips_a_lingering_draft_adjustment() {
    harness::on_large_stacks(|| async {
        let mut start = monthly(false);
        start.coupon = Some(CouponSpec {
            off: Off::Percent(10),
            invoices: Some(2),
            expires: None,
        });
        let held = documents_of(&upgraded(start, &january_paid(), &renewal_first())).await;
        assert_eq!(
            headers(&held),
            [
                "document: Recurring Finalized dated 2025-01-02 subtotal 3600 total 3240 due 0",
                "document: Adjustment Finalized dated 2025-02-01 subtotal 3200 total 3200 due 3200",
                "document: Recurring Finalized dated 2025-02-01 subtotal 6700 total 6030 due 4590",
                "note: on INV-0001 Finalized subtotal -1600",
            ],
            "{held:#?}"
        );
    });
}

/// A 100.00 minimum; the add-on attached Jan 16 bills 1900 × 16/31 = 981 on a draft. February's
/// true-up, issued before it, counts it: 10000 − 3600 − 981 = 5419, never a second 981 over the
/// floor, whichever is issued first. Rules: R-166, R-022, R-079
#[test]
fn a_true_up_issued_before_a_draft_adjustment_counts_what_it_bills() {
    harness::on_large_stacks(|| async {
        let life = |after: &[Action]| {
            let mut start = monthly(false);
            start.minimum = Some(10_000);
            let mut actions = vec![Action::Start {
                day: on(1, 1),
                catalog: start,
            }];
            actions.extend(january_paid());
            actions.extend([
                Action::AttachAddOn {
                    at: at(on(1, 16), 15),
                    quantity: 1,
                },
                Action::Advance {
                    at: midnight(on(2, 1)),
                },
            ]);
            actions.extend_from_slice(after);
            actions
        };
        let adjustment_first = [
            Action::Finalize {
                at: at(on(2, 1), 16),
                pick: 0,
            },
            Action::Finalize {
                at: at(on(2, 1), 17),
                pick: 0,
            },
        ];
        let expected = [
            "document: Recurring Finalized dated 2025-01-02 subtotal 3600 total 3600 due 0",
            "document: Adjustment Finalized dated 2025-02-01 subtotal 981 total 981 due 981",
            "document: Recurring Finalized dated 2025-02-01 subtotal 10919 total 10919 due 10919",
        ];
        for after in [&renewal_first()[..], &adjustment_first[..]] {
            let documents = documents_of(&life(after)).await;
            assert_eq!(headers(&documents), expected, "{documents:#?}");
            assert!(
                documents.iter().any(|line| line
                    == "document:   Minimum commitment 2025-01-01..2025-02-01 qty None x None = 5419"),
                "{documents:#?}"
            );
        }
    });
}

/// Upgraded Jan 16 and back to A on Jan 24, nothing issued: two drafts, no note. Issued in order,
/// January gives back A's 1600 and the first adjustment B's unused 6200 × 8/31 = 1600, the second
/// bills A's 3100 × 8/31 = 800: what auto-advancing bills. Rules: R-166, R-078, R-077
#[test]
fn a_second_change_before_the_first_draft_is_issued_bills_the_same() {
    harness::on_large_stacks(|| async {
        let life = |auto_advance: bool, after: &[Action]| {
            let mut actions = vec![
                Action::Start {
                    day: on(1, 1),
                    catalog: monthly(auto_advance),
                },
                Action::ChangeNow {
                    at: at(on(1, 16), 15),
                    to: 1,
                },
                Action::ChangeNow {
                    at: at(on(1, 24), 15),
                    to: 0,
                },
                Action::Advance {
                    at: midnight(on(2, 1)),
                },
            ];
            actions.extend_from_slice(after);
            actions
        };
        let held = documents_of(&life(false, &[])).await;
        assert_eq!(
            headers(&held),
            [
                "document: Recurring Draft dated 2025-01-01 subtotal 3600 total 3600 due 3600",
                "document: Adjustment Draft dated 2025-01-16 subtotal 3200 total 3200 due 3200",
                "document: Adjustment Draft dated 2025-01-24 subtotal 800 total 800 due 800",
                "document: Recurring Draft dated 2025-02-01 subtotal 3600 total 3600 due 3600",
            ],
            "{held:#?}"
        );

        let issued = documents_of(&life(
            false,
            &[Action::FinalizeAll {
                at: at(on(2, 1), 16),
            }],
        ))
        .await;
        assert_eq!(
            headers(&issued),
            [
                "document: Recurring Finalized dated 2025-02-01 subtotal 3600 total 3600 due 2000",
                "document: Adjustment Finalized dated 2025-02-01 subtotal 3200 total 3200 due 1600",
                "document: Adjustment Finalized dated 2025-02-01 subtotal 800 total 800 due 800",
                "document: Recurring Finalized dated 2025-02-01 subtotal 3600 total 3600 due 3600",
                "note: on INV-0002 Finalized subtotal -1600",
                "note: on INV-0001 Finalized subtotal -1600",
            ],
            "{issued:#?}"
        );
        let by_itself = documents_of(&life(true, &[])).await;
        // Issued a week later, the documents keep what auto-advancing bills, only their date moves.
        let undated = |documents: &[String]| -> Vec<String> {
            headers(documents)
                .into_iter()
                .map(|line| match line.split_once(" dated ") {
                    Some((kind, rest)) => {
                        format!("{kind} {}", rest.split_once(' ').map_or(rest, |r| r.1))
                    }
                    None => line.to_owned(),
                })
                .collect()
        };
        assert_eq!(undated(&issued), undated(&by_itself), "{by_itself:#?}");
    });
}

/// Without auto-advance, a seat bought and an add-on attached are both drafts: the amendment no
/// longer issues what the purchase holds. Rules: R-166
#[test]
fn a_seat_bought_and_an_add_on_attached_both_wait_for_the_operator() {
    harness::on_large_stacks(|| async {
        let documents = documents_of(&[
            Action::Start {
                day: on(1, 1),
                catalog: monthly(false),
            },
            Action::BuySeats {
                at: at(on(1, 10), 15),
                seats: 1,
            },
            Action::AttachAddOn {
                at: at(on(1, 12), 15),
                quantity: 1,
            },
        ])
        .await;
        let adjustments: Vec<&str> = headers(&documents)
            .into_iter()
            .filter(|line| line.starts_with("document: Adjustment "))
            .collect();
        assert_eq!(adjustments.len(), 2, "{documents:#?}");
        assert!(
            adjustments.iter().all(|line| line.contains(" Draft ")),
            "{documents:#?}"
        );
    });
}
