//! What issued documents billed, line by line (R-077).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::DayFraction;
use crate::allocation::largest_remainder;
use crate::charge::ChargeKey;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IssuedKind {
    /// A fixed fee, with the plan component it names and whether an add-on billed it.
    Fee {
        price_component: Option<Uuid>,
        add_on: bool,
    },
    /// A minimum shortfall, and whether it was charged up front for its period.
    Shortfall {
        up_front: bool,
    },
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedLine {
    pub id: String,
    /// The charge the line billed (R-075).
    pub key: ChargeKey,
    pub kind: IssuedKind,
    /// The lineage root of the component or add-on that billed it, when known.
    pub lineage: Option<String>,
    pub quantity: Option<Decimal>,
    pub unit_priced: bool,
    pub from: NaiveDate,
    pub until: NaiveDate,
    pub subtotal: i64,
    /// What a tax-inclusive line was quoted at, VAT included: the figures prices are in.
    pub quoted: Option<i64>,
    /// What credit notes already took off the line.
    pub already_credited: i64,
    /// What of it the engine's own notes took: corrections of what the line billed, where an
    /// operator's notes only cap what is left (R-082).
    pub corrected: i64,
    /// What an earlier ending of its fee credited from it, when one did.
    pub settled: Option<i64>,
}

impl IssuedLine {
    fn covers(&self, at: NaiveDate) -> bool {
        self.from <= at && at < self.until
    }

    fn left(&self) -> i64 {
        (self.subtotal - self.already_credited).max(0)
    }

    /// `amount`, in the figures the line was priced in, as the net share of the line it is
    /// (R-081).
    pub fn net_of(&self, amount: i64) -> i64 {
        match self.quoted {
            Some(quoted) if quoted != 0 => crate::scale(amount, self.subtotal, quoted),
            _ => amount,
        }
    }

    /// `amount`, a net share of the line, in the figures the line was priced in.
    pub fn quoted_of(&self, amount: i64) -> i64 {
        match self.quoted {
            Some(quoted) if self.subtotal != 0 => crate::scale(amount, quoted, self.subtotal),
            _ => amount,
        }
    }
}

/// What the shortfalls charged up front for a period covering `at` give back: the period's days
/// left, prorated once over the lines a product-scoped minimum split it into, never above what a
/// line has left.
pub fn shortfall_credits(at: NaiveDate, lines: &[IssuedLine]) -> Vec<(String, i64)> {
    let current: Vec<&IssuedLine> = lines
        .iter()
        .filter(|line| {
            matches!(line.kind, IssuedKind::Shortfall { up_front: true })
                && line.covers(at)
                && line.settled.is_none()
        })
        .collect();
    let mut periods: Vec<(NaiveDate, NaiveDate)> =
        current.iter().map(|line| (line.from, line.until)).collect();
    periods.sort();
    periods.dedup();
    let mut credits = Vec::new();
    for (from, until) in periods {
        let period: Vec<&&IssuedLine> = current
            .iter()
            .filter(|line| (line.from, line.until) == (from, until))
            .collect();
        let billed: i64 = period.iter().map(|line| line.subtotal).sum();
        if billed <= 0 {
            continue;
        }
        let credited = DayFraction::remaining(from, until, at).prorate(billed);
        let exact: Vec<Decimal> = period
            .iter()
            .map(|line| {
                Decimal::from(credited) * Decimal::from(line.subtotal) / Decimal::from(billed)
            })
            .collect();
        for (line, share) in period.iter().zip(largest_remainder(&exact)) {
            let amount = share.min(line.left());
            if amount > 0 {
                credits.push((line.id.clone(), amount));
            }
        }
    }
    credits
}
