//! What issued documents billed, line by line (R-077).

use chrono::NaiveDate;
use common_domain::ids::PriceComponentId;
use rust_decimal::Decimal;

use crate::charge::{ChargeKey, Fee};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IssuedKind {
    /// A fixed fee, with the plan component it names and whether an add-on billed it.
    Fee {
        price_component: Option<PriceComponentId>,
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
    /// On a voided invoice: what it billed counts as billed and forgiven (R-096).
    pub voided: bool,
}

impl IssuedLine {
    /// Whether the line billed the charge `key`: the same subscription, fee, kind and
    /// denomination, over part of the key's period. A line of no days (an ending's usage on its
    /// day, R-016) bills only the key of that day, never the period ending on it.
    pub fn bills(&self, key: &ChargeKey) -> bool {
        self.key.subscription == key.subscription
            && self.key.fee == key.fee
            && self.key.kind == key.kind
            && self.key.denomination == key.denomination
            && key.from <= self.from
            && self.until <= key.until
            && (self.from < self.until || key.from == key.until)
    }

    /// Whether the line bills a period of its fee the reconcile watermark settled, one of
    /// `settled` (`Evaluation::settled`): no charge's, however long a period holds it (R-094).
    pub fn settled_by(&self, settled: &[(Fee, (NaiveDate, NaiveDate))]) -> bool {
        let key = &self.key;
        settled.iter().any(|(fee, (from, until))| {
            *fee == key.fee
                && *from <= key.from
                && key.until <= *until
                && (key.from < *until || from == until)
        })
    }

    pub(crate) fn covers(&self, at: NaiveDate) -> bool {
        self.from <= at && at < self.until
    }

    pub(crate) fn left(&self) -> i64 {
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
