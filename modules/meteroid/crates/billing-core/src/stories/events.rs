//! A story's metered usage: a few units on some whole hours, the same for every run of a story,
//! so a read of any window is the units it holds and nothing else.

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::story::{At, metric};
use crate::evaluate::UsageNeed;
use crate::pricer::UsageReads;

pub struct Events {
    from: At,
    /// Units used before each hour since `from`.
    before: Vec<i64>,
}

fn units_at(hour: i64) -> i64 {
    let x = (hour as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 59;
    if x < 6 { x as i64 + 1 } else { 0 }
}

impl Events {
    pub fn new(from: At, until: At) -> Self {
        let hours = (until - from).num_hours().max(0) + 1;
        let first = from.timestamp() / 3_600;
        let mut before = Vec::with_capacity(usize::try_from(hours + 1).expect("hours"));
        let mut total = 0;
        before.push(0);
        for hour in 0..hours {
            total += units_at(first + hour);
            before.push(total);
        }
        Events { from, before }
    }

    fn index(&self, at: At) -> usize {
        let hours = (at - self.from)
            .num_hours()
            .clamp(0, self.before.len() as i64 - 1);
        usize::try_from(hours).expect("an index")
    }

    /// Units used over `[from, until)`, both on whole hours.
    pub fn units(&self, from: At, until: At) -> i64 {
        if until <= from {
            return 0;
        }
        self.before[self.index(until)] - self.before[self.index(from)]
    }

    /// The reads `needed` that ended by `now`, each the units of its window.
    pub fn reads(&self, needed: &[UsageNeed], now: At) -> UsageReads {
        needed
            .iter()
            .filter(|need| need.read.window.until <= now && need.read.metric == metric())
            .map(|need| {
                let units = self.units(need.read.window.from, need.read.window.until);
                (need.read, vec![(BTreeMap::new(), Decimal::from(units))])
            })
            .collect()
    }
}
