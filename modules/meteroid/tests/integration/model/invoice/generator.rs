//! Seeded operation sequences for the invoice model. An operation holds only choices, resolved
//! against the model's state when run; the first one, Start, also draws the case's catalog.
use chrono::{Duration, NaiveDate, NaiveTime};

use super::reference::{
    Action, At, Billing, Catalog, CouponSpec, Currency, Discount, Model, PlanSpec, TaxRate,
    last_renewal, midnight,
};
use crate::model::harness::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Start {
        currency: u8,
        plans: [u8; 6],
        percent: u8,
        fixed: u8,
        taxes: u8,
        day: u8,
    },
    Usage {
        units: u8,
        gap: u8,
    },
    Advance,
    ChangeAtEnd,
    CancelAtEnd,
}

const FEES: [i64; 5] = [999, 2_900, 4_999, 12_345, 50_000];
/// Per unit, in ten-thousandths of the minor unit.
const RATES: [i64; 5] = [125, 2_500, 3_333, 12_345, 50_000];
const PERCENTS: [i64; 5] = [10, 15, 33, 50, 100];
const FIXED: [i64; 4] = [500, 1_000, 5_000, 100_000];
const TAXES: [TaxRate; 4] = [
    TaxRate {
        code: "SALES",
        name: "Sales tax",
        ppm: 88_750,
    },
    TaxRate {
        code: "CITY",
        name: "City tax",
        ppm: 45_000,
    },
    TaxRate {
        code: "VAT",
        name: "VAT",
        ppm: 200_000,
    },
    TaxRate {
        code: "LEVY",
        name: "Levy",
        ppm: 72_500,
    },
];

const WEIGHTS: &[(u8, u32)] = &[(1, 50), (2, 25), (3, 8), (4, 3)];

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
    match kind {
        1 => Op::Usage {
            units: rng.byte(),
            gap: rng.byte(),
        },
        2 => Op::Advance,
        3 => Op::ChangeAtEnd,
        _ => Op::CancelAtEnd,
    }
}

pub fn generate(seed: u64, steps: usize) -> Vec<Op> {
    let mut rng = Rng::new(seed);
    let mut model = Model::default();
    let mut plans = [0; 6];
    for b in &mut plans {
        *b = rng.byte();
    }
    let first = Op::Start {
        currency: rng.byte(),
        plans,
        percent: rng.byte(),
        fixed: rng.byte(),
        taxes: rng.byte(),
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
    ops
}

fn plan(fee: u8, billing: u8, rate: u8) -> PlanSpec {
    PlanSpec {
        fee: FEES[usize::from(fee) % FEES.len()],
        billing: if billing.is_multiple_of(2) {
            Billing::Advance
        } else {
            Billing::Arrears
        },
        rate: RATES[usize::from(rate) % RATES.len()],
    }
}

/// At most one coupon of each kind. A fixed coupon lasts one invoice or all of them: over
/// several, the engine spends its amount once across them, a behaviour no rule states yet.
fn catalog(currency: u8, plans: [u8; 6], percent: u8, fixed: u8, taxes: u8) -> Catalog {
    let mut coupons = vec![];
    if percent % 5 >= 2 {
        coupons.push(CouponSpec {
            discount: Discount::Percent(PERCENTS[usize::from(percent / 5) % PERCENTS.len()]),
            cycles: [None, Some(1), Some(2), Some(3)][usize::from(percent / 25) % 4],
        });
    }
    if !fixed.is_multiple_of(3) {
        coupons.push(CouponSpec {
            discount: Discount::Fixed(FIXED[usize::from(fixed / 3) % FIXED.len()]),
            cycles: [None, Some(1)][usize::from(fixed / 12) % 2],
        });
    }
    let first = usize::from(taxes / 3) % TAXES.len();
    let taxes = match taxes % 3 {
        0 => vec![],
        1 => vec![TAXES[first]],
        _ => vec![TAXES[first], TAXES[(first + 1) % TAXES.len()]],
    };
    Catalog {
        currency: [Currency::Usd, Currency::Jpy, Currency::Bhd][usize::from(currency) % 3],
        plans: [
            plan(plans[0], plans[1], plans[2]),
            plan(plans[3], plans[4], plans[5]),
        ],
        coupons,
        taxes,
    }
}

/// The period end of the running subscription.
fn boundary(model: &Model) -> Option<At> {
    let sub = model.sub();
    sub.active.then(|| midnight(sub.period_end))
}

/// An instant at `hour`, `gap` days on, after the present and the day before the period end.
fn slot(model: &Model, gap: u8, hour: u32) -> Option<At> {
    let now = model.now();
    let time = NaiveTime::from_hms_opt(hour, 0, 0).expect("an hour");
    let mut at = (now.date_naive() + Duration::days(i64::from(gap % 6)))
        .and_time(time)
        .and_utc();
    if at <= now {
        at += Duration::days(1);
    }
    let bound = boundary(model)?;
    let last = (bound - Duration::days(1))
        .date_naive()
        .and_time(time)
        .and_utc();
    at = at.min(last);
    (at > now).then_some(at)
}

pub fn resolve(model: &Model, op: &Op) -> Option<Action> {
    if let Op::Start {
        currency,
        plans,
        percent,
        fixed,
        taxes,
        day,
    } = *op
    {
        return model.now.is_none().then(|| Action::Start {
            day: NaiveDate::from_ymd_opt(2025, 1, 1 + u32::from(day) % 28).expect("a day"),
            catalog: catalog(currency, plans, percent, fixed, taxes),
        });
    }
    model.now?;
    let sub = model.sub();
    match *op {
        Op::Start { .. } => None,
        Op::Usage { units, gap } => Some(Action::Usage {
            at: slot(model, gap, 14)?,
            units: 1 + 3 * i64::from(units),
        }),
        Op::Advance => {
            let at = boundary(model)?;
            (at > model.now() && sub.period_end <= last_renewal()).then_some(Action::Advance { at })
        }
        Op::ChangeAtEnd => {
            sub.scheduled.is_none().then_some(())?;
            Some(Action::ChangeAtEnd {
                at: slot(model, 0, 15)?,
                to: 1 - sub.plan,
            })
        }
        Op::CancelAtEnd => {
            sub.scheduled.is_none().then_some(())?;
            Some(Action::CancelAtEnd {
                at: slot(model, 0, 15)?,
                end: sub.period_end,
            })
        }
    }
}
