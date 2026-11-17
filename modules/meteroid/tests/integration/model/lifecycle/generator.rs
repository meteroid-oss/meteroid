//! Seeded operation sequences for the lifecycle model. An operation holds only choices, resolved
//! against the model's state when run; every sequence ends with a cancellation at the period end,
//! its renewal and its drafts finalized, so what the subscription owes over its life is known.
use chrono::{Duration, NaiveDate, NaiveTime};

use super::reference::{
    Action, AddOnSpec, At, CapacitySpec, Catalog, CouponSpec, Model, Off, PlanSpec, Scheduled,
    TrialSpec, last_renewal, midnight,
};
use crate::model::harness::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Start {
        plans: [u8; 10],
        extra: [u8; 16],
        auto: u8,
        day: u8,
    },
    Usage {
        units: u8,
        gap: u8,
    },
    Advance,
    AmendNow {
        gap: u8,
        price: u8,
        flip: bool,
    },
    AmendAtEnd {
        price: u8,
    },
    ChangeNow {
        gap: u8,
    },
    ChangeAtEnd {
        explicit: bool,
    },
    OneTime {
        gap: u8,
        price: u8,
    },
    CancelNow {
        gap: u8,
    },
    CancelImmediately {
        gap: u8,
    },
    CancelBackdated {
        back: u8,
    },
    CancelAtEnd,
    BuySeats {
        gap: u8,
        seats: u8,
    },
    RemoveSeats {
        seats: u8,
    },
    AttachAddOn {
        gap: u8,
        quantity: u8,
    },
    AddOnQuantity {
        gap: u8,
        quantity: u8,
    },
    DetachAddOn {
        gap: u8,
    },
    Void {
        pick: u8,
    },
    Pay {
        pick: u8,
    },
    Finalize {
        pick: u8,
    },
    FinalizeAll,
}

const FEES: [i64; 5] = [999, 2_900, 4_999, 12_345, 50_000];
/// Per unit, in whole cents.
const RATES: [i64; 4] = [1, 7, 25, 120];
const ONE_TIME: [i64; 3] = [1_500, 9_900, 25_000];
const SEATS: [i64; 3] = [500, 1_000, 2_500];
const ADD_ONS: [i64; 3] = [700, 1_900, 9_900];
const MINIMUMS: [i64; 3] = [5_000, 20_000, 100_000];
const COMPANIONS: [i64; 2] = [1_000, 2_900];
/// 31.00 for 310 units, 0.10 a unit past them: the user's example.
const CAPACITIES: [CapacitySpec; 2] = [
    CapacitySpec {
        fee: 3_100,
        included: 310,
        overage: 10,
    },
    CapacitySpec {
        fee: 9_900,
        included: 100,
        overage: 25,
    },
];

const WEIGHTS: &[(u8, u32)] = &[
    (1, 28),
    (2, 20),
    (3, 8),
    (4, 3),
    (5, 6),
    (6, 3),
    (7, 3),
    (8, 2),
    (9, 2),
    (10, 4),
    (11, 4),
    (12, 3),
    (13, 2),
    (14, 2),
    (15, 4),
    (16, 2),
    (17, 3),
    (18, 2),
    (19, 2),
];

fn random_op(rng: &mut Rng) -> Op {
    let total: u32 = WEIGHTS.iter().map(|(_, w)| w).sum();
    let mut roll = rng.below(u64::from(total)) as u32;
    let kind = WEIGHTS
        .iter()
        .find(|(_, w)| {
            if roll < *w {
                true
            } else {
                roll -= w;
                false
            }
        })
        .map(|(k, _)| *k)
        .expect("a weighted kind");
    let mut b = || rng.byte();
    match kind {
        1 => Op::Usage {
            units: b(),
            gap: b(),
        },
        2 => Op::Advance,
        3 => Op::AmendNow {
            gap: b(),
            price: b(),
            flip: b() % 3 == 0,
        },
        4 => Op::AmendAtEnd { price: b() },
        5 => Op::ChangeNow { gap: b() },
        6 => Op::ChangeAtEnd {
            explicit: b().is_multiple_of(2),
        },
        7 => Op::OneTime {
            gap: b(),
            price: b(),
        },
        8 => Op::CancelNow { gap: b() },
        9 => Op::CancelAtEnd,
        10 => Op::Void { pick: b() },
        11 => Op::Pay { pick: b() },
        12 => Op::Finalize { pick: b() },
        13 => Op::CancelImmediately { gap: b() },
        14 => Op::CancelBackdated { back: b() },
        15 => Op::BuySeats {
            gap: b(),
            seats: b(),
        },
        16 => Op::RemoveSeats { seats: b() },
        17 => Op::AttachAddOn {
            gap: b(),
            quantity: b(),
        },
        18 => Op::AddOnQuantity {
            gap: b(),
            quantity: b(),
        },
        _ => Op::DetachAddOn { gap: b() },
    }
}

pub fn generate(seed: u64, steps: usize) -> Vec<Op> {
    let mut rng = Rng::new(seed);
    let mut model = Model::default();
    let mut plans = [0; 10];
    for b in &mut plans {
        *b = rng.byte();
    }
    let mut extra = [0; 16];
    for b in &mut extra {
        *b = rng.byte();
    }
    let first = Op::Start {
        plans,
        extra,
        auto: rng.byte(),
        day: rng.byte(),
    };
    let mut ops = vec![first];
    let start = resolve(&model, &first).expect("the subscription");
    let _ = model.apply(&start);
    while ops.len() < steps {
        let op = (0..64)
            .map(|_| random_op(&mut rng))
            .find(|op| resolve(&model, op).is_some())
            .unwrap_or(Op::Advance);
        let Some(action) = resolve(&model, &op) else {
            break;
        };
        let _ = model.apply(&action);
        ops.push(op);
    }
    // The subscription ends, a pending change applied first: its life is then whole.
    for op in [
        Op::Advance,
        Op::CancelAtEnd,
        Op::Advance,
        Op::FinalizeAll,
        Op::Pay { pick: 0 },
    ] {
        if let Some(action) = resolve(&model, &op) {
            let _ = model.apply(&action);
            ops.push(op);
        }
    }
    ops
}

fn plan(fee: u8, arrears: u8, months: u8, rate: u8, seat: u8) -> PlanSpec {
    PlanSpec {
        fee: FEES[usize::from(fee) % FEES.len()],
        arrears: arrears.is_multiple_of(3),
        months: if months.is_multiple_of(3) { 12 } else { 1 },
        rate: RATES[usize::from(rate) % RATES.len()],
        seat: SEATS[usize::from(seat) % SEATS.len()],
    }
}

/// An instant at `hour`, `gap` days on, after the present (an hour after it on the same day when
/// the hour has passed) and before the period end.
fn slot(model: &Model, gap: u8, hour: u32) -> Option<At> {
    let now = model.now();
    let time = NaiveTime::from_hms_opt(hour, 0, 0).expect("an hour");
    let mut at = (now.date_naive() + Duration::days(i64::from(gap % 6)))
        .and_time(time)
        .and_utc();
    if at <= now {
        at = now + Duration::hours(1);
    }
    let sub = model.sub();
    if sub.active {
        at = at.min(midnight(sub.period_end) - Duration::hours(1));
    }
    (at > now).then_some(at)
}

/// LIFECYCLE_MODEL_WIDE=1 draws coupons, minimums, trials, fixed terms, companions and
/// capacities too.
fn wide() -> bool {
    std::env::var("LIFECYCLE_MODEL_WIDE").is_ok_and(|v| v == "1")
}

pub fn resolve(model: &Model, op: &Op) -> Option<Action> {
    if let Op::Start {
        plans,
        extra,
        auto,
        day,
    } = *op
    {
        let day = NaiveDate::from_ymd_opt(2025, 1, 1 + u32::from(day) % 28).expect("a day");
        let wide = wide();
        let mut extra = extra;
        if !wide {
            extra[4..].fill(1);
        }
        let mut plans = [
            plan(plans[0], plans[1], plans[2], plans[3], plans[8]),
            plan(plans[4], plans[5], plans[6], plans[7], plans[9]),
        ];
        let minimum = extra[5]
            .is_multiple_of(4)
            .then(|| MINIMUMS[usize::from(extra[6]) % MINIMUMS.len()]);
        if minimum.is_some() {
            // A minimum's period is the month: its true-up is checked per month.
            for plan in &mut plans {
                plan.months = 1;
            }
        }
        let coupon = (extra[4] % 3 == 0).then(|| CouponSpec {
            off: match extra[7] % 5 {
                0 => Off::Fixed(500),
                1 => Off::Fixed(2_000),
                2 => Off::Percent(25),
                3 => Off::Percent(50),
                _ => Off::Percent(10),
            },
            invoices: match extra[8] % 3 {
                0 => None,
                1 => Some(1),
                _ => Some(2),
            },
            expires: extra[8].is_multiple_of(2).then(|| day + Duration::days(20)),
        });
        let trial = (minimum.is_none() && extra[9].is_multiple_of(4))
            .then(|| {
                use chrono::Datelike;
                let end = match extra[10] % 3 {
                    0 => day + Duration::days(7),
                    1 => day + Duration::days(14),
                    // On the start's renewal day.
                    _ => day
                        .checked_add_months(chrono::Months::new(1))
                        .expect("a day"),
                };
                (end.day() <= 28).then(|| TrialSpec {
                    end,
                    // Without a card (an OnCheckout start) the subscription awaits a checkout
                    // the model does not drive: PendingActivation, refusing every change.
                    card: true,
                    anchor: (end.day() != day.day()).then(|| end.day()),
                })
            })
            .flatten();
        let term_end = extra[11].is_multiple_of(5).then(|| {
            let months = 1 + u32::from(extra[12] % 3);
            day.checked_add_months(chrono::Months::new(months))
                .expect("a day")
                + Duration::days(i64::from(extra[12] % 20))
        });
        let companion = (coupon.is_none()
            && minimum.is_none()
            && trial.is_none()
            && extra[13].is_multiple_of(4))
        .then(|| COMPANIONS[usize::from(extra[13] / 4) % COMPANIONS.len()]);
        return model.now.is_none().then(|| Action::Start {
            day,
            catalog: Catalog {
                plans,
                auto_advance: auto % 3 != 0,
                seats: 1 + i64::from(extra[0] % 4),
                add_on: AddOnSpec {
                    price: ADD_ONS[usize::from(extra[1]) % ADD_ONS.len()],
                    months: if extra[2].is_multiple_of(3) { 12 } else { 1 },
                    on_b: extra[3].is_multiple_of(2),
                },
                coupon,
                minimum,
                trial,
                term_end,
                companion,
                capacity: extra[14]
                    .is_multiple_of(3)
                    .then(|| CAPACITIES[usize::from(extra[15]) % 2]),
            },
        });
    }
    model.now?;
    let sub = model.sub();
    let open = sub.active && sub.scheduled.is_none();
    match *op {
        Op::Start { .. } => None,
        Op::Void { pick } => Some(Action::Void {
            at: slot(model, 0, 17)?,
            pick,
        }),
        Op::Pay { pick } => Some(Action::Pay {
            at: slot(model, 0, 18)?,
            pick,
        }),
        Op::Finalize { pick } => Some(Action::Finalize {
            at: slot(model, 0, 16)?,
            pick,
        }),
        Op::FinalizeAll => Some(Action::FinalizeAll {
            at: slot(model, 0, 16)?,
        }),
        _ if !sub.active => None,
        Op::Usage { units, gap } => Some(Action::Usage {
            at: slot(model, gap, 14)?,
            units: 1 + i64::from(units % 50),
        }),
        Op::Advance => {
            let at = midnight(sub.period_end);
            (at > model.now() && sub.period_end <= last_renewal()).then_some(Action::Advance { at })
        }
        Op::AmendNow { gap, price, flip } => {
            open.then_some(())?;
            let months = model.months();
            Some(Action::AmendNow {
                at: slot(model, gap, 15)?,
                plan: sub.plan,
                price: FEES[usize::from(price) % FEES.len()],
                months: if flip { 13 - months } else { months },
                rate: model.catalog().plans[sub.plan].rate,
            })
        }
        Op::AmendAtEnd { price } => {
            open.then_some(())?;
            let at = slot(model, 0, 15)?;
            // Half keep the fee's own cadence, which a shorter add-on may not be the
            // subscription's: detaching it then moves the period's end past the amendment.
            let own = model
                .rows
                .iter()
                .rev()
                .find(|row| row.lineage == sub.fixed && row.from <= at.date_naive())
                .map(|row| row.months);
            Some(Action::AmendAtEnd {
                at,
                plan: sub.plan,
                price: FEES[usize::from(price) % FEES.len()],
                months: own
                    .filter(|_| price >= 128)
                    .unwrap_or_else(|| model.months()),
            })
        }
        Op::ChangeNow { gap } => {
            open.then_some(())?;
            Some(Action::ChangeNow {
                at: slot(model, gap, 15)?,
                to: 1 - sub.plan,
            })
        }
        Op::ChangeAtEnd { explicit } => {
            open.then_some(())?;
            Some(Action::ChangeAtEnd {
                at: slot(model, 0, 15)?,
                to: 1 - sub.plan,
                // Set to the seats held: it overrides the removals pending for its day.
                seats: explicit.then_some(sub.seat_count),
            })
        }
        Op::OneTime { gap, price } => Some(Action::OneTime {
            at: slot(model, gap, 16)?,
            price: ONE_TIME[usize::from(price) % ONE_TIME.len()],
        }),
        Op::CancelNow { gap } => {
            open.then_some(())?;
            Some(Action::CancelNow {
                at: slot(model, gap, 15)?,
            })
        }
        Op::CancelAtEnd => {
            open.then_some(())?;
            Some(Action::CancelAtEnd {
                at: slot(model, 0, 15)?,
                end: sub.period_end,
            })
        }
        Op::CancelImmediately { gap } => {
            open.then_some(())?;
            Some(Action::CancelImmediately {
                at: slot(model, gap, 15)?,
            })
        }
        Op::CancelBackdated { back } => {
            open.then_some(())?;
            let at = slot(model, 0, 15)?;
            let today = at.date_naive();
            // The store takes any earlier day: back to the start.
            let days = (today - sub.start).num_days();
            (days > 0).then_some(())?;
            Some(Action::CancelBackdated {
                at,
                day: today - Duration::days(1 + i64::from(back) % days),
            })
        }
        // Seats changed under a scheduled change carry into its count, only under
        // LIFECYCLE_MODEL_WIDE.
        Op::BuySeats { .. } | Op::RemoveSeats { .. }
            if matches!(sub.scheduled, Some(Scheduled::Change(_))) && !wide() =>
        {
            None
        }
        Op::BuySeats { gap, seats } => Some(Action::BuySeats {
            at: slot(model, gap, 15)?,
            seats: 1 + i64::from(seats % 3),
        }),
        Op::RemoveSeats { seats } => {
            let left = sub.seat_count - sub.seats_off();
            (left > 1).then_some(())?;
            Some(Action::RemoveSeats {
                at: slot(model, 0, 15)?,
                seats: 1 + i64::from(seats) % (left - 1),
            })
        }
        Op::AttachAddOn { gap, quantity } => {
            // Any plan: the store attaches an add-on its plan does not offer.
            sub.add_on.is_none().then_some(())?;
            Some(Action::AttachAddOn {
                at: slot(model, gap, 15)?,
                quantity: 1 + i64::from(quantity % 3),
            })
        }
        Op::AddOnQuantity { gap, quantity } => {
            let (_, current) = sub.add_on?;
            let quantity = 1 + i64::from(quantity % 3);
            (quantity != current).then_some(())?;
            Some(Action::AddOnQuantity {
                at: slot(model, gap, 15)?,
                quantity,
            })
        }
        Op::DetachAddOn { gap } => {
            sub.add_on?;
            Some(Action::DetachAddOn {
                at: slot(model, gap, 15)?,
            })
        }
    }
}
