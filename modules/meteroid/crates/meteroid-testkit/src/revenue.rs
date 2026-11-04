//! Revenue rows as an edition deriving revenue from billing writes them, for tests that assert
//! revenue beside billing. A test binary of such an edition registers its probe at startup;
//! without one every revenue check is skipped (None).
use std::sync::{Arc, OnceLock};

use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{CreditGrantId, CustomerId, InvoiceId, PlanId};
use meteroid_store::Store;

#[derive(Debug, Clone)]
pub struct RevenueRow {
    pub id: String,
    pub customer_id: CustomerId,
    pub plan_id: Option<PlanId>,
    pub invoice_id: Option<InvoiceId>,
    pub grant_id: Option<CreditGrantId>,
    pub charge_type: String,
    pub amount_subtotal_cents: i64,
    pub recognized_subtotal_cents: i64,
    pub billed_at: NaiveDateTime,
    /// End-exclusive.
    pub recognition_start: NaiveDate,
    pub recognition_end: NaiveDate,
    /// The first day recognized: before it the window's share comes due on this day at once.
    pub recognition_from: NaiveDate,
}

#[async_trait::async_trait]
pub trait RevenueProbe: Send + Sync {
    /// Every revenue row of `customer`, each as last written under its id, once every write
    /// running now has committed.
    async fn rows(&self, store: &Store, customer: CustomerId) -> Vec<RevenueRow>;
}

static PROBE: OnceLock<Arc<dyn RevenueProbe>> = OnceLock::new();

pub fn register(probe: Arc<dyn RevenueProbe>) {
    if PROBE.set(probe).is_err() {
        panic!("a revenue probe is already registered");
    }
}

pub async fn rows(store: &Store, customer: CustomerId) -> Option<Vec<RevenueRow>> {
    match PROBE.get() {
        Some(probe) => Some(probe.rows(store, customer).await),
        None => None,
    }
}

/// Cumulative revenue of one customer as of a day: billed, recognized (spread over each row's
/// window), and the breakage among it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Revenue {
    pub billed: i64,
    pub recognized: i64,
    pub breakage: i64,
}

impl Revenue {
    pub fn deferred(&self) -> i64 {
        self.billed - self.recognized
    }
}

/// The customer's revenue as of `as_of`: billed by then, and recognized by the whole days elapsed.
pub async fn revenue(store: &Store, customer: CustomerId, as_of: NaiveDate) -> Option<Revenue> {
    let mut revenue = Revenue {
        billed: 0,
        recognized: 0,
        breakage: 0,
    };
    for row in rows(store, customer).await? {
        eprintln!(
            "revenue row {} {} billed {} at {}, recognized {} over [{}, {}) from {}",
            row.id,
            row.charge_type,
            row.amount_subtotal_cents,
            row.billed_at,
            row.recognized_subtotal_cents,
            row.recognition_start,
            row.recognition_end,
            row.recognition_from
        );
        if row.billed_at.date() <= as_of {
            revenue.billed += row.amount_subtotal_cents;
        }
        // A one-day window is recognized at an instant of its day (a draw, a lapse): it counts
        // on that day. A longer one counts by the whole days elapsed before `as_of`.
        let window = (row.recognition_end - row.recognition_start).num_days();
        let elapsed = if as_of < row.recognition_from {
            0
        } else if window <= 1 {
            if row.recognition_start <= as_of {
                window
            } else {
                0
            }
        } else {
            (as_of.min(row.recognition_end) - row.recognition_start)
                .num_days()
                .clamp(0, window)
        };
        let recognized = if window == 0 || elapsed == window {
            row.recognized_subtotal_cents
        } else {
            row.recognized_subtotal_cents * elapsed / window
        };
        revenue.recognized += recognized;
        if row.charge_type == "breakage" {
            revenue.breakage += recognized;
        }
    }
    Some(revenue)
}

/// Asserts nothing is recognized below zero and never more than was billed; returns the breakage
/// recognized.
pub async fn assert_sane(store: &Store, customer: CustomerId) -> Option<i64> {
    let rows = rows(store, customer).await?;
    let billed: i64 = rows.iter().map(|r| r.amount_subtotal_cents).sum();
    let recognized: i64 = rows.iter().map(|r| r.recognized_subtotal_cents).sum();
    let detail: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r.charge_type.as_str(),
                r.amount_subtotal_cents,
                r.recognized_subtotal_cents,
            )
        })
        .collect();
    assert!(
        recognized >= 0 && billed >= recognized,
        "billed {billed}, recognized {recognized}: {detail:?}"
    );
    Some(
        rows.iter()
            .filter(|r| r.charge_type == "breakage")
            .map(|r| r.recognized_subtotal_cents)
            .sum(),
    )
}
