//! Seeded generation of operation sequences. An operation holds only choices; it is resolved
//! against the model's state when run, so removing operations while shrinking keeps a sequence
//! valid (an operation with nothing to act on is skipped).
use chrono::{Duration, NaiveDate, NaiveTime};

use super::harness::Rng;
use super::reference::{
    Action, At, How, Late, LateUse, Model, Plan, Sale, Scheduled, Token, midnight,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Start {
        scale: bool,
        day: u8,
    },
    Usage {
        sub: u8,
        token: u8,
        mtok: u8,
        gap: u8,
    },
    Advance,
    TopUp {
        amount: u8,
        pay: bool,
    },
    Pay {
        pick: u8,
        how: u8,
    },
    Fail {
        pick: u8,
    },
    Chargeback {
        pick: u8,
        late: u8,
    },
    RefundNote {
        pick: u8,
        part: u8,
    },
    RefundNoNote {
        pick: u8,
        part: u8,
    },
    Void {
        pick: u8,
    },
    Redeem,
    ChangeNow {
        sub: u8,
        gap: u8,
        late: u8,
        token: u8,
        mtok: u8,
    },
    ChangeAtEnd {
        sub: u8,
    },
    CancelNow {
        sub: u8,
        gap: u8,
        late: u8,
        token: u8,
        mtok: u8,
    },
    CancelAtEnd {
        sub: u8,
    },
    Second {
        scale: bool,
    },
    LateUsage {
        sub: u8,
        token: u8,
        mtok: u8,
    },
}

const WEIGHTS: &[(u8, u32)] = &[
    (1, 26),
    (2, 15),
    (3, 5),
    (4, 13),
    (5, 2),
    (6, 5),
    (7, 3),
    (8, 3),
    (9, 3),
    (10, 3),
    (11, 6),
    (12, 4),
    (13, 2),
    (14, 2),
    (15, 3),
    (16, 3),
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
            sub: b(),
            token: b(),
            mtok: b(),
            gap: b(),
        },
        2 => Op::Advance,
        3 => Op::TopUp {
            amount: b(),
            pay: b() % 3 != 0,
        },
        4 => Op::Pay {
            pick: b(),
            how: b(),
        },
        5 => Op::Fail { pick: b() },
        6 => Op::Chargeback {
            pick: b(),
            late: b(),
        },
        7 => Op::RefundNote {
            pick: b(),
            part: b(),
        },
        8 => Op::RefundNoNote {
            pick: b(),
            part: b(),
        },
        9 => Op::Void { pick: b() },
        10 => Op::Redeem,
        11 => Op::ChangeNow {
            sub: b(),
            gap: b(),
            late: b(),
            token: b(),
            mtok: b(),
        },
        12 => Op::ChangeAtEnd { sub: b() },
        13 => Op::CancelNow {
            sub: b(),
            gap: b(),
            late: b(),
            token: b(),
            mtok: b(),
        },
        14 => Op::CancelAtEnd { sub: b() },
        15 => Op::Second {
            scale: b() % 2 == 0,
        },
        _ => Op::LateUsage {
            sub: b(),
            token: b(),
            mtok: b(),
        },
    }
}

/// `steps` operations, each one valid in the model's state when generated.
pub fn generate(seed: u64, steps: usize) -> Vec<Op> {
    let mut rng = Rng::new(seed);
    let mut model = Model::default();
    let first = Op::Start {
        scale: rng.below(5) < 3,
        day: rng.byte(),
    };
    let mut ops = vec![first];
    let start = resolve(&model, &first).expect("the first subscription");
    let _ = model.apply(&start);
    while ops.len() < steps {
        let op = (0..64)
            .map(|_| random_op(&mut rng))
            .find(|op| resolve(&model, op).is_some())
            .unwrap_or(Op::Advance);
        let Some(action) = resolve(&model, &op) else {
            break;
        };
        // A refused action leaves the model as it was: it is part of the sequence all the same.
        let _ = model.apply(&action);
        ops.push(op);
    }
    ops
}

fn token(choice: u8) -> Token {
    match choice % 4 {
        0 | 1 => Token::Lite,
        2 => Token::Standard,
        _ => Token::Max,
    }
}

fn mtok(choice: u8, token: Token) -> i64 {
    let most = match token {
        Token::Lite => 60,
        Token::Standard => 30,
        Token::Max => 6,
    };
    1 + i64::from(choice) % most
}

/// The next renewal of the active subscriptions.
fn boundary(model: &Model) -> Option<At> {
    model
        .subs
        .iter()
        .filter(|s| s.active)
        .map(|s| midnight(s.period_end))
        .min()
}

/// An instant at `hour`, `gap` days on, after the present and the day before the next renewal.
fn slot(model: &Model, gap: u8, hour: u32) -> Option<At> {
    let now = model.now();
    let time = NaiveTime::from_hms_opt(hour, 0, 0).expect("an hour");
    let mut at = (now.date_naive() + Duration::days(i64::from(gap % 4)))
        .and_time(time)
        .and_utc();
    if at <= now {
        at += Duration::days(1);
    }
    if let Some(bound) = boundary(model) {
        let last = (bound - Duration::days(1))
            .date_naive()
            .and_time(time)
            .and_utc();
        at = at.min(last);
    }
    (at > now).then_some(at)
}

fn change_day(model: &Model, sub: usize, gap: u8) -> Option<NaiveDate> {
    let s = &model.subs[sub];
    let day = model.now().date_naive() + Duration::days(1 + i64::from(gap % 5));
    (midnight(day) > model.now() && day > s.period_start && day < s.period_end).then_some(day)
}

fn late(choice: u8, token_choice: u8, mtok_choice: u8) -> Option<LateUse> {
    let when = match choice % 4 {
        2 => Late::WithinGrace,
        3 => Late::AfterGrace,
        _ => return None,
    };
    let token = token(token_choice);
    Some(LateUse {
        when,
        token,
        mtok: mtok(mtok_choice, token) / 2 + 1,
    })
}

fn pick<T: Copy>(candidates: &[T], choice: u8) -> Option<T> {
    (!candidates.is_empty()).then(|| candidates[usize::from(choice) % candidates.len()])
}

/// Pack credit may end with a change or a cancellation only once its invoice paid it (R15).
fn pack_settled(model: &Model, sub: usize) -> bool {
    let s = &model.subs[sub];
    s.plan == Plan::Payg
        || s.pack_invoice
            .is_some_and(|i| model.invoices[i].funded && !model.invoices[i].void)
}

pub fn resolve(model: &Model, op: &Op) -> Option<Action> {
    if let Op::Start { scale, day } = op {
        return model.now.is_none().then(|| Action::Start {
            day: NaiveDate::from_ymd_opt(2025, 8, 1 + u32::from(*day) % 28).expect("a day"),
            plan: if *scale { Plan::Scale } else { Plan::Payg },
        });
    }
    model.now?;
    let subs = model.subs.len();
    let sub_of = |choice: u8| usize::from(choice) % subs;
    match *op {
        Op::Start { .. } => None,
        Op::Usage {
            sub,
            token: t,
            mtok: m,
            gap,
        } => {
            let s = sub_of(sub);
            if !model.subs[s].active {
                return None;
            }
            let at = slot(model, gap, 14)?;
            let token = token(t);
            Some(Action::Usage {
                sub: s,
                at,
                dated: at,
                token,
                mtok: mtok(m, token),
            })
        }
        Op::LateUsage {
            sub,
            token: t,
            mtok: m,
        } => {
            let s = sub_of(sub);
            let state = &model.subs[s];
            if state.period_start == state.start {
                return None;
            }
            let at = slot(model, 0, 14)?;
            let token = token(t);
            Some(Action::Usage {
                sub: s,
                at,
                dated: midnight(state.period_start) - Duration::hours(2),
                token,
                mtok: mtok(m, token),
            })
        }
        Op::Advance => {
            let at = boundary(model)?;
            (at > model.now()).then(|| Action::Renew {
                at,
                subs: (0..subs)
                    .filter(|&s| model.subs[s].active && midnight(model.subs[s].period_end) == at)
                    .collect(),
            })
        }
        Op::TopUp { amount, pay } => Some(Action::TopUp {
            at: slot(model, 0, 14)?,
            amount: [5_000, 10_000, 20_000][usize::from(amount) % 3],
            pay,
        }),
        Op::Pay { pick: p, how } => {
            let due: Vec<usize> = (0..model.invoices.len())
                .filter(|&i| !model.invoices[i].void && model.invoices[i].due() > 0)
                .collect();
            let invoice = pick(&due, p)?;
            Some(Action::Pay {
                at: slot(model, 0, 14)?,
                invoice,
                how: match how % 4 {
                    0 => How::Manual,
                    1 => How::Provider(0),
                    2 => How::Provider(1_000),
                    _ => How::Provider(5_000),
                },
            })
        }
        Op::Fail { pick: p } => {
            let due: Vec<usize> = (0..model.invoices.len())
                .filter(|&i| !model.invoices[i].void && model.invoices[i].due() > 0)
                .collect();
            Some(Action::Fail {
                invoice: pick(&due, p)?,
                at: slot(model, 0, 14)?,
            })
        }
        Op::Chargeback { pick: p, late } => {
            let payments: Vec<(usize, usize)> = model
                .invoices
                .iter()
                .enumerate()
                .flat_map(|(i, inv)| {
                    inv.payments
                        .iter()
                        .enumerate()
                        // A refunded payment may be taken back too: only what it holds goes.
                        .filter(|(_, p)| p.provider && !p.reversed && !p.late_pack)
                        .map(move |(p, _)| (i, p))
                })
                .collect();
            let (invoice, payment) = pick(&payments, p)?;
            Some(Action::Chargeback {
                at: slot(model, 0, 14)?,
                invoice,
                payment,
                disputed: late % 3 != 0,
            })
        }
        Op::RefundNote { pick: p, part } => {
            let bought: Vec<(usize, usize, i64)> = model
                .invoices
                .iter()
                .enumerate()
                .filter(|(_, inv)| matches!(inv.sale, Some(Sale::TopUp(_))) && inv.funded)
                .filter_map(|(i, inv)| {
                    let unspent = model.grants[*inv.minted.first()?].remaining;
                    let payment = inv.payments.iter().position(|p| !p.reversed)?;
                    (unspent > 0).then_some((i, payment, unspent))
                })
                .collect();
            let (invoice, payment, unspent) = pick(&bought, p)?;
            Some(Action::RefundNote {
                at: slot(model, 0, 14)?,
                invoice,
                payment,
                amount: [unspent, unspent / 2 + 1, unspent + 1_000][usize::from(part) % 3],
            })
        }
        Op::RefundNoNote { pick: p, part } => {
            let overpaid: Vec<(usize, usize, i64)> = model
                .invoices
                .iter()
                .enumerate()
                .flat_map(|(i, inv)| {
                    let kept = model.x_remaining(i);
                    inv.payments
                        .iter()
                        .enumerate()
                        .filter(move |(_, p)| p.provider && !p.reversed && kept > 0)
                        .map(move |(p, _)| (i, p, kept))
                })
                .collect();
            let (invoice, payment, kept) = pick(&overpaid, p)?;
            Some(Action::RefundNoNote {
                at: slot(model, 0, 14)?,
                invoice,
                payment,
                amount: [kept, kept / 2 + 1, kept + 500][usize::from(part) % 3],
            })
        }
        Op::Void { pick: p } => {
            let open: Vec<usize> = (0..model.invoices.len())
                .filter(|&i| {
                    let inv = &model.invoices[i];
                    !inv.void && inv.payments.is_empty() && inv.due() > 0 && inv.notes.is_empty()
                })
                .collect();
            Some(Action::Void {
                invoice: pick(&open, p)?,
                at: slot(model, 0, 14)?,
            })
        }
        Op::Redeem => {
            // One subscription only: a customer-wide promotion lapsing over two is not modelled.
            (subs == 1 && model.subs[0].active).then_some(())?;
            Some(Action::Redeem {
                at: slot(model, 0, 13)?,
            })
        }
        Op::ChangeNow {
            sub,
            gap,
            late: l,
            token: t,
            mtok: m,
        } => {
            let s = sub_of(sub);
            let state = &model.subs[s];
            if !state.active
                || state.changed
                || state.scheduled.is_some()
                || !pack_settled(model, s)
            {
                return None;
            }
            Some(Action::ChangeNow {
                sub: s,
                day: change_day(model, s, gap)?,
                to: state.plan.other(),
                late: late(l, t, m),
            })
        }
        Op::ChangeAtEnd { sub } => {
            let s = sub_of(sub);
            let state = &model.subs[s];
            if !state.active || state.scheduled == Some(Scheduled::Change(state.plan.other())) {
                return None;
            }
            Some(Action::ChangeAtEnd {
                at: slot(model, 0, 14)?,
                sub: s,
                to: state.plan.other(),
            })
        }
        Op::CancelNow {
            sub,
            gap,
            late: l,
            token: t,
            mtok: m,
        } => {
            let s = sub_of(sub);
            let state = &model.subs[s];
            if !state.active || state.changed || !pack_settled(model, s) {
                return None;
            }
            Some(Action::CancelNow {
                sub: s,
                day: change_day(model, s, gap)?,
                late: late(l, t, m),
            })
        }
        Op::CancelAtEnd { sub } => {
            let s = sub_of(sub);
            let state = &model.subs[s];
            if !state.active || state.scheduled == Some(Scheduled::Cancel) {
                return None;
            }
            Some(Action::CancelAtEnd {
                at: slot(model, 0, 14)?,
                sub: s,
                end: state.period_end,
            })
        }
        Op::Second { scale } => {
            if subs != 1 || model.promo_redeemed {
                return None;
            }
            let day = model.now().date_naive() + Duration::days(1);
            Some(Action::Start {
                day,
                plan: if scale { Plan::Scale } else { Plan::Payg },
            })
        }
    }
}
