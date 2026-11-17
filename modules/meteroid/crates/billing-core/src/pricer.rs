//! What a fee costs, as `evaluate` asks it: the store's fee type is read only here.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{BillableMetricId, ProductId};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::charge::{ChargeKind, Denomination, Fee};
use crate::facts::Billing;
use crate::pricing::{PricingError, SubLine};
use crate::proration::DayFraction;

/// A fee row billing over a segment, with the seats in force for it.
#[derive(Clone, Debug, PartialEq)]
pub struct InForce<'a, F> {
    pub lineage: &'a Fee,
    pub row: Uuid,
    pub billing: Billing,
    pub cadence: u32,
    pub seats: Option<u64>,
    pub fee: &'a F,
    /// The share of a whole period's allowance (a capacity's included units) its usage counts:
    /// whole unless its fixed amount is prorated over the window (R-002).
    pub allowance: DayFraction,
}

/// `[from, until)` over which every fact is constant, and the fees billing over it, priced
/// together.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment<'a, F> {
    pub from: NaiveDate,
    pub until: Option<NaiveDate>,
    pub fees: Vec<InForce<'a, F>>,
}

/// The credit a pack's charge funds once paid, in minor units of `denomination`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CreditGrant {
    pub amount: i64,
    pub denomination: Denomination,
    /// Granted by the same share of the period as the charge's amount.
    pub prorated: bool,
}

/// How a rate was priced, as its line shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    /// The price of one unit: for a fixed fee over a whole period.
    pub unit_price: Option<Decimal>,
    /// The tiers, packages or cells usage went through.
    pub sub_lines: Vec<SubLine>,
    /// The group of dimensions usage was read in, absent when not grouped.
    pub dimensions: Option<HashMap<String, String>>,
}

/// What a fee bills of one kind: for a whole period of its cadence when `prorated` (a part of
/// the period bills its share, R-079), else whatever part of it is billed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rate {
    pub fee: Fee,
    pub kind: ChargeKind,
    pub quantity: Option<Decimal>,
    /// Minor units of `denomination`, rounded once from the exact price (R-001).
    pub amount: i64,
    pub denomination: Denomination,
    pub prorated: bool,
    /// For a pack's charge: the credit it grants for a whole period, or whatever part is billed.
    pub credit_grant: Option<CreditGrant>,
    /// Billed once for the subscription's product, whichever fee bills it first: the issued
    /// documents decide whether it is due, as a minimum's shortfall.
    pub once_per_product: Option<ProductId>,
    pub shown: Shown,
}

/// The instants a usage window reads metering over, `[from, until)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UsageWindow {
    pub from: DateTime<Utc>,
    pub until: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UsageRead {
    pub metric: BillableMetricId,
    pub window: UsageWindow,
}

/// What a metric measured over a window, per group of dimensions.
pub type Usage = Vec<(BTreeMap<String, String>, Decimal)>;

/// Metering read at a pinned instant, keyed by metric and window: `evaluate` only asks whether a
/// read is there, the pricer reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageReads(BTreeMap<UsageRead, Usage>);

impl UsageReads {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, read: UsageRead, usage: Usage) {
        self.0.insert(read, usage);
    }

    pub fn get(&self, read: &UsageRead) -> Option<&Usage> {
        self.0.get(read)
    }

    pub fn contains(&self, read: &UsageRead) -> bool {
        self.0.contains_key(read)
    }
}

impl FromIterator<(UsageRead, Usage)> for UsageReads {
    fn from_iter<I: IntoIterator<Item = (UsageRead, Usage)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// Prices the store's fees. Every method prices a set of fees at once, so fees priced together
/// stay expressible.
pub trait Pricer<F> {
    /// The metrics `fee` bills the usage of; none for a fixed fee.
    fn metrics(&self, fee: &F) -> Vec<BillableMetricId>;

    /// Whether `fee`'s usage prices each part of a period on its own, at that part's row (per
    /// unit); else the period's usage is priced once, by the row in force at its end, its
    /// allowance and tiers counted over the whole period (R-097).
    fn per_part(&self, _fee: &F) -> bool {
        true
    }

    /// What each fee of `segment` bills that is not usage: one rate per fee and kind.
    fn rates(&self, segment: &Segment<'_, F>) -> Result<Vec<Rate>, PricingError>;

    /// What the fees of `segment` bill for the usage `reads` hold over `window`, one rate per fee
    /// and `ChargeKind::Usage`; never prorated (R-002).
    fn usage(
        &self,
        segment: &Segment<'_, F>,
        window: &UsageWindow,
        reads: &UsageReads,
    ) -> Result<Vec<Rate>, PricingError>;
}
