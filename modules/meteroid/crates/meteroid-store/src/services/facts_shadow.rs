//! Each period invoice billed from the facts, checked against what the per-operation code would
//! have computed for it, and the state the facts derive against the stored row, in tests only
//! (feature `facts-shadow`): each difference not in [`SKIPS`] fails the test, as `timeline_shadow`
//! does.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use billing_core::evaluate::{DerivedState, Status};
use chrono::NaiveDate;
use common_domain::ids::{BaseId, BillableMetricId, SubscriptionId};
use diesel_models::invoices::InvoiceRow;
use diesel_models::subscription_facts::SubscriptionPlanWindowRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;

use crate::StoreResult;
use crate::domain::enums::{InvoiceType, SubscriptionStatusEnum};
use crate::domain::{Invoice, LineItem, Subscription, SubscriptionDetails};
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::evaluate::bill::{FromFacts, midnight};
use crate::services::invoice_lines::invoice_lines::PricedInvoice;
use crate::services::invoice_lines::minimum::PeriodTrueUp;
use crate::services::lifecycle::derived::stored_as;
use crate::services::served_usage::{ServedUsage, UnservedRead};
use crate::store::PgConn;

static MISMATCHES: Mutex<Vec<String>> = Mutex::new(Vec::new());

static SPENT: AtomicU64 = AtomicU64::new(0);

/// The time this process spent in the checks, to profile a test run.
pub fn time_spent() -> Duration {
    Duration::from_nanos(SPENT.load(Ordering::Relaxed))
}

fn spend(since: Instant) {
    let nanos = u64::try_from(since.elapsed().as_nanos()).unwrap_or(u64::MAX);
    SPENT.fetch_add(nanos, Ordering::Relaxed);
}

/// What the comparison found different since the last call.
pub fn take_mismatches() -> Vec<String> {
    MISMATCHES
        .lock()
        .map(|mut found| std::mem::take(&mut *found))
        .unwrap_or_default()
}

/// A known divergence: a difference of `class` whose text holds `contains`.
struct Skip {
    class: &'static str,
    contains: &'static str,
    reason: &'static str,
}

/// Known, explained divergences; each entry says in one line why it is expected.
const SKIPS: &[Skip] = &[
    Skip {
        class: "id",
        contains: "",
        reason: "R-075: a line of a closed row is keyed by its lineage's root; the legacy path keyed it by the row",
    },
    Skip {
        class: "watermark",
        contains: "",
        reason: "R-080: a period ending by the reconcile watermark was settled before; the legacy path, blind to it, bills it again",
    },
    Skip {
        class: "dated price",
        contains: "",
        reason: "R-097: a price succeeded inside the document's span bills each part at its own rates; the legacy path prices the span at today's",
    },
    Skip {
        class: "unserved watermark",
        contains: "",
        reason: "R-080: the legacy path, blind to the reconcile watermark, reads the usage of a window ending by it, which the facts settled before and never read",
    },
    Skip {
        class: "unserved backdated",
        contains: "",
        reason: "R-016, R-077: the legacy path reads usage past a backdated ending, which the facts cut at it",
    },
    Skip {
        class: "unserved dated price",
        contains: "",
        reason: "R-097: the legacy path reads whole the usage of a window a dated price splits, which the facts read per part",
    },
    Skip {
        class: "unserved billed",
        contains: "",
        reason: "R-080: the legacy path reads the usage of days another document bills, which the facts bill once",
    },
    Skip {
        class: "unserved parts",
        contains: "",
        reason: "R-016, R-097: the facts read a window by the parts its rows or prices split it in, the legacy path whole",
    },
    Skip {
        class: "unserved cut",
        contains: "",
        reason: "R-016: an ending cuts a usage row's period, its usage read in one window to the ending's instant; the legacy path reads only the ending's day of it",
    },
    Skip {
        class: "unserved settled",
        contains: "",
        reason: "R-011: a closed row's last arrears are read on the first document from its end; the legacy path reads them again on every later invoice while no line billed them up to their end",
    },
    Skip {
        class: "unserved run",
        contains: "",
        reason: "R-084, R-011: the legacy path reads the usage of a closed row whose run goes on in a row of its cadence and billing mode, billed at the run's period end",
    },
    Skip {
        class: "unserved own grid",
        contains: "",
        reason: "R-084: the legacy path reads a usage row of another cadence up to the subscription's renewal, a day on no boundary of the row's grid; the facts read the row's own periods",
    },
    Skip {
        class: "deferred one-time",
        contains: "facts only: ",
        reason: "R-093: a one-time fee added in a free trial bills once, on the first invoice of the days it lives into, or on the final invoice of a backdated ending closing the trial first, at that ending; the legacy path billed it on none, or on a renewal past the ending, which gave it back",
    },
    Skip {
        class: "settled row",
        contains: "facts only: ",
        reason: "R-011: the legacy path reads a closed row another document billed up to its end as settled and drops its arrears (retain_unsettled), its own period's too: a draft finalized after later renewals, a capacity whose fee billed its last period in advance",
    },
    Skip {
        class: "legacy one-time",
        contains: "",
        reason: "R-075: a one-time fee bills once, on its day's document; the legacy path bills it again on another invoice of that day, an ending's",
    },
    Skip {
        class: "amendment cadence",
        contains: "",
        reason: "R-084: an amendment's cadence is no plan switch, its grid kept; the legacy path reads grids from plan switches alone",
    },
    Skip {
        class: "rider",
        contains: "",
        reason: "R-011: a row's arrears ended off its grid ride the next renewal, after the row of its fee that replaced it; the legacy path drops them",
    },
    Skip {
        class: "late rider",
        contains: "",
        reason: "R-011: arrears of a row ended off its grid bill on the renewal ending the period that holds them; the legacy path bills them one renewal later",
    },
    Skip {
        class: "name range",
        contains: "",
        reason: "a fee billed by several lines on one invoice is named with each line's dates; the other path billed it by one",
    },
    Skip {
        class: "term end",
        contains: "",
        reason: "R-084, R-002: a fixed term's end cuts its last period as a cancellation that day would, a capacity's allowance with its fee; the legacy path billed that period whole",
    },
    Skip {
        class: "capacity allowance",
        contains: "",
        reason: "R-002: a capacity's allowance is prorated with its fee where the fee is, its overage over the same days and rate higher, or billed alone on a backdated final where the whole allowance left none; the legacy path counted the allowance whole",
    },
    Skip {
        class: "first partial capacity",
        contains: "",
        reason: "R-002: a capacity's fee is prorated in a first partial period, as any fixed fee is; the legacy path billed it whole",
    },
    Skip {
        class: "backdated",
        contains: "legacy only",
        reason: "R-077: a cancellation backdated into periods already billed takes their unused days back from their lines; the legacy path bills the days it kept again on the cancellation's own invoice",
    },
    Skip {
        class: "backdated final",
        contains: "true-up legacy None facts Some",
        reason: "R-016: a backdated cancellation's final invoice trues up the period its ending cut, as the recurring invoice of the ending's day would; the legacy path trues up no final invoice",
    },
    Skip {
        class: "backdated final",
        contains: "facts only",
        reason: "R-102, R-016: a backdated cancellation's charges go on the final invoice issued the day it is decided, never on a document dated in the past; the legacy path billed them on the recurring invoice of the ending's day (a stub over its own period, a run row by row, R-084)",
    },
    Skip {
        class: "written day final",
        contains: "facts only",
        reason: "R-016: a cancellation decided after the recurring invoice of its day was written bills what it cuts on a final invoice of its own; the legacy path left it unbilled",
    },
    Skip {
        class: "backdated draft",
        contains: "legacy only",
        reason: "R-016: a draft of a period after a backdated cancellation's ending is closed empty, its charges on the final invoice issued the day it is decided; the legacy path keeps its lines",
    },
    Skip {
        class: "backdated draft",
        contains: "legacy stored lines",
        reason: "R-016: a draft of a period after a backdated cancellation's ending is priced again, to close empty; the legacy path keeps its stored lines",
    },
    Skip {
        class: "backdated draft",
        contains: "}) facts None",
        reason: "R-016: a draft of a period after a backdated cancellation's ending is closed empty, the true-up of the period the ending cut on the final invoice; the legacy path trues it up on the draft",
    },
    Skip {
        class: "adjusted",
        contains: "legacy only",
        reason: "R-080: a change on a boundary of its new grid billed that period on its adjustment; the period's own invoice owes nothing, the legacy path bills it again",
    },
    Skip {
        class: "stub",
        contains: "",
        reason: "R-084: the days between a cadence change off the old grid and the new grid's origin price over the new grid's period before that origin; the legacy path prices them over a period off that grid (a monthly stub of 3 days at 999: x 3/31, the legacy path x 3/35)",
    },
    Skip {
        class: "unseen switch",
        contains: "",
        reason: "R-084: a change of cadence on the day an amendment's row opened and was closed by it is a plan window of the facts; the legacy path reads that row, which never lived, as opened then and misses the switch, its grid after it off",
    },
    Skip {
        class: "tiling",
        contains: "",
        reason: "R-079: a prorated part is what is left at its start less what is left at its end, each rounded once; the legacy path rounds the share once, a minor unit apart at a half, its line named with its dates or not",
    },
    Skip {
        class: "repriced rest",
        contains: "facts only: ",
        reason: "R-080, R-078: a fee repriced on a renewal inside its own longer period bills the new price's rest less the old one's from that day; the legacy path bills that period only on the invoice of its start",
    },
    Skip {
        class: "repriced arrears",
        contains: "legacy only: ",
        reason: "R-084, R-011: a fee in arrears repriced at its cadence and billing mode goes on in one run, billed at its period's end; the legacy path bills the replaced row's days on the next renewal, as if the fee had ended",
    },
    Skip {
        class: "carried run",
        contains: "legacy only: ",
        reason: "R-084: a fee whose row a change replaced at its cadence and billing mode goes on in its run, on its own grid; the legacy path bills a new whole period from the renewal, inside one an issued line bills",
    },
    Skip {
        class: "caught up",
        contains: "facts only: ",
        reason: "R-084: a renewal bills the whole periods of a fee's own cadence no recurring invoice was written for; the legacy path bills only the invoice's own period",
    },
    Skip {
        class: "run grid",
        contains: "",
        reason: "R-084: a fee carried at its cadence into a new plan version bills its run's periods on their own grid; the legacy path bills the same length from the change's grid",
    },
    Skip {
        class: "voided",
        contains: "legacy only: ",
        reason: "R-096: a void forgives what its invoice billed, never billed again; the legacy path bills a voided line's charge again on a later invoice",
    },
    Skip {
        class: "backdated arrears",
        contains: "facts only: ",
        reason: "R-016, R-011: a backdated cancellation's final invoice bills the arrears it cuts at the row in force on their days; the legacy path, reading the rows in force when it is decided, drops those of a row a later change closed",
    },
    Skip {
        class: "split charge",
        contains: "",
        reason: "R-080, R-084: rows of one cadence on both sides of another cadence's row in a period are one charge, billed by one line; the legacy path bills each row's part on a line of its own, around the other cadence's line, the same money",
    },
];

fn skip_of(class: &str, detail: &str) -> Option<&'static Skip> {
    SKIPS
        .iter()
        .find(|skip| skip.class == class && detail.contains(skip.contains))
}

fn record(subscription: SubscriptionId, class: &'static str, detail: &str) {
    if let Some(skip) = skip_of(class, detail) {
        log::debug!("facts shadow: skipped ({}): {detail}", skip.reason);
        if std::env::var_os("FACTS_SHADOW_SHOW_SKIPS").is_some()
            && let Ok(mut found) = MISMATCHES.lock()
        {
            found.push(format!("{subscription} [skipped {class}] {detail}"));
        }
        return;
    }
    let difference = format!("{subscription} [{class}] {detail}");
    log::warn!("facts shadow: {difference}");
    if let Ok(mut found) = MISMATCHES.lock() {
        found.push(difference);
    }
}

/// What the facts owe that a written document keeps unbilled (R-077): a difference of its own.
pub(crate) fn record_unbilled(subscription: SubscriptionId, detail: &str) {
    record(subscription, "unbilled", detail);
}

/// A period invoice billed from the facts, as the per-operation code is asked for it.
pub(crate) struct Compared<'a> {
    pub invoice_date: NaiveDate,
    pub details: &'a SubscriptionDetails,
    pub prepaid_amount: Option<u64>,
    pub invoice: Option<&'a Invoice>,
    pub invoice_type: InvoiceType,
}

/// A fee row as the facts bill it, for the comparison.
#[derive(Clone, Debug)]
pub(crate) struct ShadowFee {
    pub row: uuid::Uuid,
    pub arrears: bool,
    /// A fixed fee's price for a whole period of its own, in minor units.
    pub price: Option<Decimal>,
    /// A capacity's allowance.
    pub included: Option<Decimal>,
    pub metric: Option<BillableMetricId>,
}

/// A difference of the two documents, with the lines it compares on each side.
#[derive(Clone, Debug)]
struct Difference {
    class: &'static str,
    detail: String,
    legacy: Vec<LineItem>,
    facts: Vec<LineItem>,
}

impl Difference {
    fn new(class: &'static str, detail: String, legacy: &[&LineItem], facts: &[&LineItem]) -> Self {
        Self {
            class,
            detail,
            legacy: legacy.iter().map(|line| (*line).clone()).collect(),
            facts: facts.iter().map(|line| (*line).clone()).collect(),
        }
    }

    fn of_document(class: &'static str, detail: String) -> Self {
        Self::new(class, detail, &[], &[])
    }

    fn legacy_only(&self) -> Option<&LineItem> {
        match (self.legacy.as_slice(), self.facts.as_slice()) {
            ([line], []) => Some(line),
            _ => None,
        }
    }

    fn facts_only(&self) -> Option<&LineItem> {
        match (self.legacy.as_slice(), self.facts.as_slice()) {
            ([], [line]) => Some(line),
            _ => None,
        }
    }

    fn pair(&self) -> Option<(&LineItem, &LineItem)> {
        match (self.legacy.as_slice(), self.facts.as_slice()) {
            ([a], [b]) => Some((a, b)),
            _ => None,
        }
    }

    fn lines(&self) -> impl Iterator<Item = &LineItem> {
        self.legacy.iter().chain(&self.facts)
    }
}

type Differences = Vec<Difference>;

fn row_of(line: &LineItem) -> Option<uuid::Uuid> {
    line.sub_component_id
        .map(|id| id.as_uuid())
        .or_else(|| line.sub_add_on_id.map(|id| id.as_uuid()))
}

/// A line of another document of the subscription, with its document's kind and period start
/// (an adjustment's is its date); a voided one billed what its void forgave (R-096).
#[derive(Clone, Debug)]
struct Other {
    line: LineItem,
    kind: InvoiceType,
    on: NaiveDate,
    issued: bool,
    void: bool,
    /// What finalized credit notes took off the line.
    credited: i64,
}

/// What the narrowing reads besides the two documents.
#[derive(Clone, Debug, Default)]
struct Context {
    invoice_date: NaiveDate,
    final_invoice: bool,
    draft: bool,
    billing_start: NaiveDate,
    /// Where a free trial ends, as the facts read it.
    trial_end: Option<NaiveDate>,
    term_end: Option<NaiveDate>,
    watermark: Option<NaiveDate>,
    /// The day the subscription ends, as its ending fact records it.
    ending: Option<NaiveDate>,
    /// The day a cancellation decided after it ends at.
    backdated_ending: Option<NaiveDate>,
    /// A final invoice of a cancellation decided after its day's recurring invoice was written.
    written_day_final: bool,
    /// The invoice of a cancellation backdated into periods already billed, whose row has caught up.
    cancelled_here: bool,
    facts_empty: bool,
    fees: Vec<ShadowFee>,
    lives: Vec<FeeLife>,
    /// The facts' lines on the compared document.
    facts_lines: Vec<LineItem>,
    others: Vec<Other>,
    /// The days an amendment changed the cadence on.
    amendment_days: Vec<NaiveDate>,
    /// A window's start and its cadence's origin, after it.
    stubs: Vec<(NaiveDate, NaiveDate)>,
    /// The starts of windows switching cadence on the day a row opened and closed.
    unseen: Vec<NaiveDate>,
    /// Lineages, by root, whose price a successor dates inside the document's span.
    dated: HashSet<uuid::Uuid>,
    roots: HashMap<uuid::Uuid, uuid::Uuid>,
    /// The period starts of the subscription's recurring, final and adjustment documents, void
    /// ones included.
    slots: Vec<NaiveDate>,
    /// The period starts of its recurring documents, void ones included.
    renewals: Vec<NaiveDate>,
    /// The billing start and each plan window's cadence origin: what fee grids are anchored on.
    origins: Vec<NaiveDate>,
    /// The currency's minor-unit exponent.
    precision: u32,
    /// The reads the legacy path was not served, each with the class explaining it.
    unserved: Vec<(UnservedRead, Option<&'static str>)>,
    /// The legacy path's recurring invoice of the ending's day, for a final invoice.
    at_ending: Option<(Vec<LineItem>, Option<PeriodTrueUp>)>,
    facts_true_up: Option<PeriodTrueUp>,
}

impl Context {
    fn life_of(&self, line: &LineItem) -> Option<&FeeLife> {
        let row = row_of(line)?;
        self.lives.iter().find(|life| life.row == row)
    }

    fn lineage_of(&self, line: &LineItem) -> Option<uuid::Uuid> {
        self.life_of(line).map(|life| life.lineage)
    }

    fn fee(&self, row: uuid::Uuid) -> Option<&ShadowFee> {
        self.fees.iter().find(|fee| fee.row == row)
    }

    fn price_of(&self, line: &LineItem) -> Option<Decimal> {
        self.fee(row_of(line)?)?.price
    }

    /// The lines of other documents billing `line`'s lineage over its very days.
    fn billed_elsewhere<'s>(&'s self, line: &'s LineItem) -> impl Iterator<Item = &'s Other> + 's {
        let lineage = self.lineage_of(line);
        self.others.iter().filter(move |other| {
            lineage.is_some()
                && self.lineage_of(&other.line) == lineage
                && (other.line.start_date, other.line.end_date) == (line.start_date, line.end_date)
        })
    }
}

/// `whole` over `days` of a period of `period` days, rounded once, half away from zero (R-002).
fn prorated(whole: Decimal, days: i64, period: i64) -> Option<i64> {
    use rust_decimal::prelude::ToPrimitive;
    (period > 0).then_some(())?;
    (whole * Decimal::from(days) / Decimal::from(period))
        .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
        .to_i64()
}

fn days(from: NaiveDate, until: NaiveDate) -> i64 {
    (until - from).num_days()
}

fn months_after(day: NaiveDate, months: u32) -> Option<NaiveDate> {
    day.checked_add_months(chrono::Months::new(months))
}

fn months_before(day: NaiveDate, months: u32) -> Option<NaiveDate> {
    day.checked_sub_months(chrono::Months::new(months))
}

impl Services {
    /// Checks `billed` against what `price_invoice_lines` computes for the same invoice, reading
    /// in a savepoint the metering the facts read; reports and never fails.
    pub(crate) async fn compare_with_legacy(
        &self,
        conn: &mut PgConn,
        compared: Compared<'_>,
        billed: &FromFacts,
    ) {
        let started = Instant::now();
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(self.compare_with_legacy_unboxed(conn, compared, billed)).await;
        spend(started);
    }

    async fn compare_with_legacy_unboxed(
        &self,
        conn: &mut PgConn,
        compared: Compared<'_>,
        billed: &FromFacts,
    ) {
        let subscription = compared.details.subscription.id;
        // A served pass answers only what it read: the legacy reads go to what the facts read,
        // the others answered empty and recorded.
        let (legacy, served) = match &self.served {
            Some(served) => match served.peek() {
                Some(peek) => (self.reading_usage(peek.clone()), Some(peek)),
                None => return,
            },
            None => (self.reading_usage(self.usage_client.clone()), None),
        };
        let checked = self
            .store
            .transaction_with(conn, |conn| {
                async move { compare(&legacy, served.as_deref(), conn, &compared, billed).await }
                    .scope_boxed()
            })
            .await;
        match checked {
            Ok(differences) => {
                for difference in differences {
                    record(subscription, difference.class, &difference.detail);
                }
            }
            Err(error) => record(subscription, "legacy", &format!("{error:?}")),
        }
    }
}

async fn compare(
    legacy: &Services,
    served: Option<&ServedUsage>,
    conn: &mut PgConn,
    compared: &Compared<'_>,
    billed: &FromFacts,
) -> StoreResult<Differences> {
    // The termination instant and the ending fact are stored apart, a microsecond apart at most:
    // the legacy path reads the ending's.
    let mut details = compared.details.clone();
    if let (Some(at), Some(ending)) = (details.subscription.terminated_at, billed.ending)
        && (at.and_utc() - ending).abs() <= chrono::Duration::microseconds(1)
    {
        details.subscription.terminated_at = Some(ending.naive_utc());
    }
    let computed = legacy
        .price_invoice_lines(
            conn,
            &compared.invoice_date,
            &details,
            compared.prepaid_amount,
            compared.invoice,
            compared.invoice_type.clone(),
            None,
        )
        .await?;
    let unserved = served.map(ServedUsage::unserved_reads).unwrap_or_default();
    let slot = format!(
        "{:?} {}{}",
        compared.invoice_type,
        compared.invoice_date,
        compared
            .invoice
            .map(|invoice| format!(" (draft {})", invoice.id))
            .unwrap_or_default()
    );
    let mut differences = document_differences(&slot, &computed, &billed.priced);
    if !differences.is_empty() || !unserved.is_empty() {
        let mut context = load_context(conn, compared, billed, &differences).await?;
        context.unserved = explain_unserved(conn, compared, billed, &context, unserved).await?;
        let ends_here = billed.ending.map(|at| at.date_naive());
        if context.final_invoice
            && let Some(ending) = ends_here
            && !differences.is_empty()
        {
            // The legacy path billed a cancellation's last charges on the recurring invoice of
            // its ending's day; its reads there are served only where the facts read the same.
            let at_ending = at_ending_day(legacy, conn, &details, ending, compared.invoice).await;
            context.at_ending = at_ending.ok().map(|priced| match priced {
                PricedInvoice::Lines(lines) => (lines.lines, lines.true_up),
                PricedInvoice::Stored(content) => (content.invoice_lines, None),
            });
        }
        narrow(&slot, &mut differences, &context);
    }

    let details = compared.details;
    let clock = common_clock::now();
    if midnight(compared.invoice_date) <= clock {
        let stored: Subscription = SubscriptionRow::get_subscription_by_id(
            conn,
            &details.subscription.tenant_id,
            details.subscription.id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .try_into()?;
        differences.extend(state_differences(
            &stored,
            &billed.state,
            clock.date_naive(),
        ));
        if let Some(difference) = mrr_difference(legacy, conn, &stored, clock.date_naive()).await? {
            differences.push(Difference::of_document("mrr", difference));
        }
    }
    Ok(differences)
}

/// The legacy path's recurring invoice of the ending's day, the final draft compared aside: voided
/// in a savepoint rolled back, so the legacy path reads none of its lines as billed.
async fn at_ending_day(
    legacy: &Services,
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    ending: NaiveDate,
    draft: Option<&Invoice>,
) -> StoreResult<PricedInvoice> {
    let Some(draft) = draft else {
        return legacy
            .price_invoice_lines(
                conn,
                &ending,
                details,
                None,
                None,
                InvoiceType::Recurring,
                None,
            )
            .await;
    };
    let kept = Mutex::new(None);
    let _ = legacy
        .store
        .transaction_with(conn, |conn| {
            async {
                use diesel::{ExpressionMethods, QueryDsl};
                use diesel_models::schema::invoice::dsl as i;
                diesel_async::RunQueryDsl::execute(
                    diesel::update(i::invoice.filter(i::id.eq(draft.id)))
                        .set(i::status.eq(diesel_models::enums::InvoiceStatusEnum::Void)),
                    conn,
                )
                .await
                .map_err(|error| -> Report<StoreError> {
                    crate::errors::StoreErrorContainer::from(error).into()
                })?;
                let priced = legacy
                    .price_invoice_lines(
                        conn,
                        &ending,
                        details,
                        None,
                        None,
                        InvoiceType::Recurring,
                        None,
                    )
                    .await;
                if let Ok(mut kept) = kept.lock() {
                    *kept = Some(priced);
                }
                Err::<(), _>(Report::new(StoreError::InvoiceComputationError).attach("rolled back"))
            }
            .scope_boxed()
        })
        .await;
    kept.into_inner()
        .ok()
        .flatten()
        .unwrap_or_else(|| Err(Report::new(StoreError::InvoiceComputationError)))
}

/// Reads what the narrowing needs of the subscription.
async fn load_context(
    conn: &mut PgConn,
    compared: &Compared<'_>,
    billed: &FromFacts,
    differences: &Differences,
) -> StoreResult<Context> {
    let sub = &compared.details.subscription;
    let windows = SubscriptionPlanWindowRow::list_by_subscription(conn, sub.id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let amendment_days = windows
        .windows(2)
        .filter(|pair| {
            pair[0].plan_version_id == pair[1].plan_version_id
                && format!("{:?}", pair[0].cadence) != format!("{:?}", pair[1].cadence)
        })
        .map(|pair| pair[1].from_date)
        .collect();
    let stubs = windows
        .iter()
        .filter(|window| window.cadence_origin > window.from_date)
        .map(|window| (window.from_date, window.cadence_origin))
        .collect();
    let endings = diesel_models::subscription_facts::SubscriptionEndingRow::list_of(conn, sub.id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let backdated_ending = endings
        .iter()
        .find(|row| row.is_backdated_cancellation())
        .map(|row| row.at.date());
    let final_invoice = compared.invoice_type == InvoiceType::Final;
    let written_day_final = final_invoice
        && backdated_ending.is_none()
        && crate::services::subscriptions::terminate::ending_day_invoiced(
            conn,
            sub.tenant_id,
            sub.id,
            &endings,
        )
        .await?;
    let arrears: Vec<uuid::Uuid> = billed
        .fees
        .iter()
        .filter(|fee| fee.arrears)
        .map(|fee| fee.row)
        .collect();
    let lives = fee_lives(conn, sub.id, &arrears).await?;
    let written = InvoiceRow::written_slots_of(conn, sub.tenant_id, sub.id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let slots = written.iter().map(|(_, on)| *on).collect();
    let renewals = written
        .iter()
        .filter(|(kind, _)| matches!(kind, diesel_models::enums::InvoiceType::Recurring))
        .map(|(_, on)| *on)
        .collect();
    let billing_start = sub.billing_start_date.unwrap_or(sub.start_date);
    let origins = std::iter::once(billing_start)
        .chain(windows.iter().map(|window| window.cadence_origin))
        .collect();
    let precision = u32::from(
        crate::constants::Currencies::resolve_currency_precision(&sub.currency).unwrap_or(2),
    );
    let others = other_lines(conn, compared).await?;
    let unseen = unseen_switches(conn, sub.id, compared.invoice_date).await?;
    let (dated, roots) = match span_of(&billed.priced) {
        Some(span) if differences.iter().any(|d| d.pair().is_some()) => (
            succeeded_within(conn, compared.details, span).await?,
            lineage_roots(conn, compared.details).await?,
        ),
        _ => Default::default(),
    };
    let clock = common_clock::now();
    let cancelled_here = if compared.invoice_date < clock.date_naive() {
        let stored: Subscription =
            SubscriptionRow::get_subscription_by_id(conn, &sub.tenant_id, sub.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .try_into()?;
        stored.status == SubscriptionStatusEnum::Cancelled
            && stored.current_period_start == compared.invoice_date
    } else {
        false
    };
    let (facts_empty, facts_true_up) = match &billed.priced {
        PricedInvoice::Lines(lines) => (lines.lines.is_empty(), lines.true_up.clone()),
        PricedInvoice::Stored(content) => (content.invoice_lines.is_empty(), None),
    };
    Ok(Context {
        invoice_date: compared.invoice_date,
        final_invoice,
        draft: compared.invoice.is_some(),
        billing_start: sub.billing_start_date.unwrap_or(sub.start_date),
        trial_end: billed.trial_end,
        term_end: sub.end_date,
        watermark: billed.watermark,
        ending: billed.ending.map(|at| at.date_naive()),
        backdated_ending,
        written_day_final,
        cancelled_here,
        facts_empty,
        fees: billed.fees.clone(),
        lives,
        facts_lines: lines_of(&billed.priced).1.to_vec(),
        others,
        slots,
        renewals,
        origins,
        precision,
        amendment_days,
        stubs,
        unseen,
        dated,
        roots,
        unserved: Vec::new(),
        at_ending: None,
        facts_true_up,
    })
}

/// The lines of the subscription's other documents that bill or will bill, void ones aside.
async fn other_lines(conn: &mut PgConn, compared: &Compared<'_>) -> StoreResult<Vec<Other>> {
    let sub = &compared.details.subscription;
    let rows = InvoiceRow::list_period_spend_invoices(
        conn,
        sub.tenant_id,
        sub.id,
        NaiveDate::from_ymd_opt(1970, 1, 1).unwrap_or(compared.invoice_date),
        NaiveDate::from_ymd_opt(9999, 12, 31).unwrap_or(compared.invoice_date),
        compared.invoice.map(|invoice| invoice.id),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    let voided: Vec<InvoiceRow> = {
        use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
        use diesel_models::schema::invoice::dsl as i;
        diesel_async::RunQueryDsl::load(
            i::invoice
                .filter(i::tenant_id.eq(sub.tenant_id))
                .filter(i::subscription_id.eq(sub.id))
                .filter(i::status.eq(diesel_models::enums::InvoiceStatusEnum::Void))
                .select(InvoiceRow::as_select()),
            conn,
        )
        .await
        .map_err(|error| -> Report<StoreError> {
            crate::errors::StoreErrorContainer::from(error).into()
        })?
    };
    let mut others = Vec::new();
    for row in rows.into_iter().chain(voided) {
        let consolidated = row.subscription_id != Some(sub.id);
        let on = row.billing_period_start;
        let invoice = Invoice::try_from(row)?;
        // Computed afresh, the compared document is not yet the one written for its slot.
        if compared.invoice.is_none()
            && invoice.invoice_type == compared.invoice_type
            && (on == compared.invoice_date || invoice.invoice_date == compared.invoice_date)
        {
            continue;
        }
        let issued = matches!(
            invoice.status,
            crate::domain::enums::InvoiceStatusEnum::Finalized
                | crate::domain::enums::InvoiceStatusEnum::Uncollectible
        );
        let void = invoice.status == crate::domain::enums::InvoiceStatusEnum::Void;
        let credited = if issued {
            credited_lines(conn, sub.tenant_id, invoice.id).await?
        } else {
            HashMap::new()
        };
        for line in invoice.line_items {
            if consolidated && line.subscription_id != Some(sub.id) {
                continue;
            }
            others.push(Other {
                credited: credited.get(&line.local_id).copied().unwrap_or(0),
                line,
                kind: invoice.invoice_type.clone(),
                on,
                issued,
                void,
            });
        }
    }
    Ok(others)
}

/// What the finalized credit notes of `invoice` took off each of its lines, by line id.
async fn credited_lines(
    conn: &mut PgConn,
    tenant: common_domain::ids::TenantId,
    invoice: common_domain::ids::InvoiceId,
) -> StoreResult<HashMap<String, i64>> {
    use diesel_models::credit_notes::CreditNoteRow;
    use diesel_models::enums::CreditNoteStatus;
    let mut credited = HashMap::new();
    let notes = CreditNoteRow::list_by_invoice_id(conn, tenant, invoice)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    for note in notes {
        if note.status != CreditNoteStatus::Finalized {
            continue;
        }
        let lines: Vec<LineItem> = serde_json::from_value(note.line_items).map_err(|e| {
            Report::new(StoreError::SerdeError(
                "Failed to deserialize credit note line items".to_string(),
                e,
            ))
        })?;
        for line in lines {
            *credited.entry(line.local_id).or_insert(0) += line.amount_subtotal.saturating_neg();
        }
    }
    Ok(credited)
}

/// The starts of the windows, up to `until`, switching cadence on the day a row of the
/// subscription opened and closed: a switch the legacy path cannot see.
async fn unseen_switches(
    conn: &mut PgConn,
    subscription: SubscriptionId,
    until: NaiveDate,
) -> StoreResult<Vec<NaiveDate>> {
    #[derive(diesel::QueryableByName)]
    struct Unseen {
        #[diesel(sql_type = diesel::sql_types::Date)]
        from_date: NaiveDate,
    }
    let unseen: Vec<Unseen> = diesel_async::RunQueryDsl::load(
        diesel::sql_query(
            "SELECT DISTINCT w.from_date
             FROM subscription_plan_window w
             JOIN subscription_plan_window p
               ON p.subscription_id = w.subscription_id AND p.to_date = w.from_date
             JOIN subscription_component c
               ON c.subscription_id = w.subscription_id
              AND c.effective_from = w.from_date AND c.effective_to = w.from_date
             WHERE w.subscription_id = $1 AND w.from_date <= $2
               AND p.cadence <> w.cadence",
        )
        .bind::<diesel::sql_types::Uuid, _>(subscription)
        .bind::<diesel::sql_types::Date, _>(until),
        conn,
    )
    .await
    .map_err(|error| -> Report<StoreError> {
        crate::errors::StoreErrorContainer::from(error).into()
    })?;
    Ok(unseen.into_iter().map(|row| row.from_date).collect())
}

/// Each metering read the legacy path was not served, with what explains it: a window the
/// watermark settled, one past a backdated ending, or one a dated price splits.
async fn explain_unserved(
    conn: &mut PgConn,
    compared: &Compared<'_>,
    billed: &FromFacts,
    context: &Context,
    unserved: Vec<UnservedRead>,
) -> StoreResult<Vec<(UnservedRead, Option<&'static str>)>> {
    let mut explained = Vec::new();
    for read in unserved {
        let class = match (read.metric, explained_by(&read, context)) {
            (None, _) => None,
            (Some(_), Some(class)) => Some(class),
            (Some(metric), None) => {
                let dated = succeeded_within(conn, compared.details, (read.from, read.to)).await?;
                let roots = lineage_roots(conn, compared.details).await?;
                let lines = lines_of(&billed.priced).1;
                let of_metric = lines
                    .iter()
                    .filter(|line| line.metric_id == Some(metric))
                    .filter_map(row_of)
                    .chain(
                        context
                            .fees
                            .iter()
                            .filter(|fee| fee.metric == Some(metric))
                            .map(|fee| fee.row),
                    );
                of_metric
                    .filter_map(|row| roots.get(&row))
                    .any(|root| dated.contains(root))
                    .then_some("unserved dated price")
            }
        };
        explained.push((read, class));
    }
    Ok(explained)
}

/// What explains a read the facts never made: a window read in parts, settled by the watermark,
/// past a backdated ending, billed elsewhere, a closed row's read on the first document from its
/// end or whose run goes on, or one ending on no boundary of its row's grid.
fn explained_by(read: &UnservedRead, context: &Context) -> Option<&'static str> {
    let metric = read.metric?;
    let closed_draft = context.draft
        && !context.final_invoice
        && context.facts_empty
        && context
            .backdated_ending
            .is_some_and(|on| context.invoice_date >= on);
    let rows = || {
        context
            .fees
            .iter()
            .filter(move |fee| fee.metric == Some(metric))
            .filter_map(|fee| context.lives.iter().find(|life| life.row == fee.row))
    };
    let run_going_on = rows().any(|life| {
        run_goes_on(
            life.row,
            (read.from, read.to),
            context.invoice_date,
            &context.lives,
            &context.facts_lines,
        )
    });
    // A window of a row running past it, ending on no boundary of the row's grid: the facts read
    // the row's own periods (R-084).
    let off_grid = ![context.term_end, context.ending].contains(&Some(read.to))
        && rows().any(|life| {
            life.from <= read.from
                && life.to.is_none_or(|to| to > read.to)
                && life
                    .months
                    .is_some_and(|months| !on_a_grid(read.to, months, &context.origins))
        });
    let billed = context.others.iter().any(|other| {
        other.line.metric_id == Some(metric)
            && other.line.start_date <= read.from
            && read.to <= other.line.end_date
    });
    // A closed row's last arrears, read on the first document from its end, compared there.
    let settled = rows().any(|life| life.to == Some(read.to) && life.from <= read.from)
        && context
            .slots
            .iter()
            .any(|on| read.to <= *on && *on < context.invoice_date);
    if read.in_parts {
        Some("unserved parts")
    } else if read.tail_of_served && context.ending == Some(read.to) {
        Some("unserved cut")
    } else if context.watermark.is_some_and(|w| read.to <= w) {
        Some("unserved watermark")
    } else if context.backdated_ending.is_some_and(|on| on < read.to) || closed_draft {
        Some("unserved backdated")
    } else if billed {
        Some("unserved billed")
    } else if settled {
        Some("unserved settled")
    } else if run_going_on {
        Some("unserved run")
    } else if off_grid {
        Some("unserved own grid")
    } else {
        None
    }
}

/// Whether `day` is a boundary of a grid of `months` anchored on one of `origins`.
fn on_a_grid(day: NaiveDate, months: u32, origins: &[NaiveDate]) -> bool {
    origins.iter().any(|origin| {
        (0..=40).any(|k| {
            months_after(*origin, k * months) == Some(day)
                || months_before(*origin, k * months) == Some(day)
        })
    })
}

/// Whether `line` bills usage of `read`'s metric over some of its days.
fn reads(line: &LineItem, read: &UnservedRead) -> bool {
    read.metric.is_some()
        && line.metric_id == read.metric
        && line.start_date <= read.to
        && read.from <= line.end_date
        && (line.start_date < read.to || line.start_date == line.end_date)
}

/// Gives each difference a known divergence's class only where its own condition holds, and
/// takes back a provisional class `document_differences` gave whose condition does not.
fn narrow(slot: &str, differences: &mut Differences, context: &Context) {
    for difference in differences.iter_mut() {
        if !holds(difference, context) {
            difference.class = "line";
        }
    }
    for difference in differences.iter_mut() {
        if (difference.class == "line" || difference.class == "document")
            && let Some(class) = explain(difference, context)
        {
            difference.class = class;
        }
    }
    for (read, class) in &context.unserved {
        if class.is_none() {
            differences.push(Difference::of_document(
                "unserved",
                format!(
                    "{slot}: the legacy path read metering the facts did not: {:?} {}..{}",
                    read.metric, read.from, read.to
                ),
            ));
        }
    }
}

/// Whether the provisional class `document_differences` gave holds.
fn holds(difference: &Difference, context: &Context) -> bool {
    match difference.class {
        "rider" => difference
            .facts_only()
            .is_some_and(|line| rides(line, context)),
        "late rider" | "legacy one-time" => difference
            .legacy_only()
            .is_some_and(|line| context.billed_elsewhere(line).next().is_some()),
        "first partial capacity" => difference
            .pair()
            .is_some_and(|(a, b)| first_partial(a, b, context)),
        "capacity allowance" => difference
            .pair()
            .is_some_and(|(a, b)| allowance_prorated(a, b, context)),
        "term end" => match difference.pair() {
            Some((a, b)) => cut_by_term_end(a, b, context),
            None => difference.legacy.split_last().is_some_and(|(last, _)| {
                let ends = difference.facts.first().map(|line| line.end_date);
                ends.is_some()
                    && ends == context.term_end
                    && context
                        .price_of(last)
                        .and_then(|price| prorated(price, 1, 1))
                        .is_some_and(|whole| whole == last.amount_subtotal)
            }),
        },
        _ => true,
    }
}

/// Whether a difference is of a known divergence, read against the subscription.
type Check = fn(&Difference, &Context) -> bool;

/// The class of a known divergence a difference is, if any.
fn explain(difference: &Difference, context: &Context) -> Option<&'static str> {
    if let Some((_, class)) = context
        .unserved
        .iter()
        .find(|(read, class)| class.is_some() && difference.lines().any(|line| reads(line, read)))
    {
        return *class;
    }
    if difference.class == "document" {
        return explain_document(difference, context);
    }
    let checks: [(&'static str, Check); 21] = [
        ("watermark", settled_by_watermark),
        ("backdated final", backdated_final),
        ("written day final", written_day_final),
        ("backdated draft", closed_past_backdated_ending),
        ("amendment cadence", by_an_amendments_cadence),
        ("adjusted", billed_by_adjustment),
        ("settled row", dropped_as_settled),
        ("unseen switch", past_an_unseen_switch),
        ("stub", priced_as_stub),
        ("repriced rest", repriced_rest),
        ("repriced arrears", repriced_arrears),
        ("carried run", carried_run),
        ("dated price", dated_price),
        ("term end", term_end),
        ("backdated", billed_before_backdated_ending),
        ("deferred one-time", deferred_one_time),
        ("caught up", caught_up),
        ("run grid", on_its_runs_grid),
        ("voided", forgiven_by_void),
        ("backdated arrears", backdated_arrears),
        ("capacity allowance", overage_of_a_cut_allowance),
    ];
    checks
        .iter()
        .find(|(_, check)| check(difference, context))
        .map(|(class, _)| *class)
}

/// A document-wide difference: the true-up or the kind of lines.
fn explain_document(difference: &Difference, context: &Context) -> Option<&'static str> {
    let closed = context.draft
        && !context.final_invoice
        && context.facts_empty
        && context
            .backdated_ending
            .is_some_and(|on| context.invoice_date >= on);
    if closed {
        return Some("backdated draft");
    }
    // A final invoice trues up the period the legacy path's recurring invoice of the ending's
    // day does, to the same floor; the spends each counts are read apart.
    let unpriced =
        |true_up: Option<PeriodTrueUp>| true_up.map(|t| (t.start, t.end, t.minimum, t.true_up));
    let at_ending = context.at_ending.as_ref()?.1.clone();
    let trued_up_alike = difference.detail.contains("true-up legacy None facts Some")
        && unpriced(at_ending) == unpriced(context.facts_true_up.clone());
    match (context.final_invoice, trued_up_alike) {
        (true, true) if context.backdated_ending.is_some() => Some("backdated final"),
        (true, true) if context.written_day_final => Some("written day final"),
        _ => None,
    }
}

/// Arrears of a row ended at the line's end, which no other document billed any day of.
fn rides(line: &LineItem, context: &Context) -> bool {
    let Some(life) = context.life_of(line) else {
        return false;
    };
    let billed = context.others.iter().any(|other| {
        context.lineage_of(&other.line) == Some(life.lineage)
            && other.line.start_date < line.end_date
            && line.start_date < other.line.end_date
    });
    life.arrears
        && life.to == Some(line.end_date)
        && line.end_date <= context.invoice_date
        && line.start_date < line.end_date
        && !billed
}

/// A fee the legacy path billed whole and the facts prorate, starting on the billing start in a
/// short first period: `whole × days / the period's days` (R-002).
fn first_partial(legacy: &LineItem, facts: &LineItem, context: &Context) -> bool {
    let Some(months) = context.life_of(facts).and_then(|life| life.months) else {
        return false;
    };
    let Some(period_start) = months_before(facts.end_date, months) else {
        return false;
    };
    let whole = Decimal::from(legacy.amount_subtotal);
    // A period cut at its end, by the term's end or an ending, is no first period's.
    let cut = [context.term_end, context.ending].contains(&Some(facts.end_date));
    facts.start_date == context.billing_start
        && !cut
        && period_start < facts.start_date
        && context
            .price_of(legacy)
            .is_none_or(|price| prorated(price, 1, 1) == Some(legacy.amount_subtotal))
        && prorated(
            whole,
            days(facts.start_date, facts.end_date),
            days(period_start, facts.end_date),
        ) == Some(facts.amount_subtotal)
}

/// A capacity whose allowance the facts prorate with its fee, by the days of its period it
/// bills: the same usage, its overage at the same rate, past the prorated allowance (R-002).
fn allowance_prorated(legacy: &LineItem, facts: &LineItem, context: &Context) -> bool {
    let Some(months) = context.life_of(facts).and_then(|life| life.months) else {
        return false;
    };
    let included = row_of(facts)
        .and_then(|row| context.fee(row))
        .and_then(|fee| fee.included);
    let (Some(included), [legacy_overage], [facts_overage]) = (
        included,
        legacy.sub_lines.as_slice(),
        facts.sub_lines.as_slice(),
    ) else {
        return false;
    };
    let rate = facts_overage.unit_price;
    let minor = Decimal::from(10_i64.pow(context.precision));
    let billed = |units: Decimal| prorated(units * rate * minor, 1, 1);
    let used_days = days(facts.start_date, facts.end_date);
    let periods = [
        months_before(facts.end_date, months).map(|start| days(start, facts.end_date)),
        months_after(facts.start_date, months).map(|end| days(facts.start_date, end)),
    ];
    let same_charge = (legacy.start_date, legacy.end_date, legacy.metric_id)
        == (facts.start_date, facts.end_date, facts.metric_id)
        && legacy_overage.unit_price == rate
        && facts_overage.total == facts.amount_subtotal;
    same_charge
        && periods.into_iter().flatten().any(|period| {
            let left = included - included * Decimal::from(used_days) / Decimal::from(period);
            if legacy_overage.quantity > Decimal::ZERO {
                billed(legacy_overage.quantity + left) == Some(facts_overage.total)
            } else {
                billed(left).is_some_and(|most| (0..=most).contains(&facts_overage.total))
            }
        })
}

/// A line both paths bill up to the fixed term's end: the legacy path the whole period, the facts
/// its days, what is left of the whole at its start less what is left at the term's end (R-084,
/// R-002, R-079).
fn cut_by_term_end(legacy: &LineItem, facts: &LineItem, context: &Context) -> bool {
    let Some(months) = context.life_of(facts).and_then(|life| life.months) else {
        return false;
    };
    let Some(period_end) = months_after(facts.start_date, months) else {
        return false;
    };
    context.term_end == Some(facts.end_date)
        && (legacy.start_date, legacy.end_date) == (facts.start_date, facts.end_date)
        && base_name(&legacy.name) == base_name(&facts.name)
        && legacy.quantity == facts.quantity
        && context
            .price_of(legacy)
            .is_none_or(|price| prorated(price, 1, 1) == Some(legacy.amount_subtotal))
        && billing_core::proration::DaySpan {
            period: (facts.start_date, period_end),
            from: facts.start_date,
            until: facts.end_date,
        }
        .prorate(legacy.amount_subtotal)
            == facts.amount_subtotal
}

fn term_end(difference: &Difference, context: &Context) -> bool {
    difference
        .pair()
        .is_some_and(|(a, b)| cut_by_term_end(a, b, context))
}

/// A legacy line ending on the watermark that another document billed: a settled period.
fn settled_by_watermark(difference: &Difference, context: &Context) -> bool {
    difference.legacy_only().is_some_and(|line| {
        context.watermark == Some(line.end_date) && context.billed_elsewhere(line).next().is_some()
    })
}

/// What a final invoice bills of the legacy path's recurring invoice of the ending's day, less
/// what issued documents billed of each of its charges; every line ending at the ending.
fn as_at_ending(difference: &Difference, context: &Context) -> bool {
    let (Some(line), Some(ending), Some(at_ending)) = (
        difference.facts_only(),
        context.ending,
        context.at_ending.as_ref(),
    ) else {
        return false;
    };
    let issued: i64 = context
        .billed_elsewhere(line)
        .filter(|other| other.issued)
        .map(|other| other.line.amount_subtotal)
        .sum();
    context.final_invoice
        && line.end_date <= ending
        && at_ending.0.iter().any(|legacy| {
            let owed = legacy.amount_subtotal - issued;
            // A prorated part each path rounds its own way (R-079), or a capacity whose allowance
            // the facts prorate (R-002); never with an issued line of the charge.
            let alike = owed == line.amount_subtotal
                || issued == 0
                    && ((legacy.is_prorated
                        && line.is_prorated
                        && (owed - line.amount_subtotal).abs() == 1)
                        || allowance_prorated(legacy, line, context)
                        || stub_alike(legacy, line, context));
            (base_name(&legacy.name), legacy.start_date, legacy.end_date)
                == (base_name(&line.name), line.start_date, line.end_date)
                && alike
        })
        || issued == 0 && split_at_ending(line, &at_ending.0, context)
}

/// A run of one cadence the facts bill by one line, which the legacy path's invoice of the
/// ending's day bills row by row around a row of another cadence: its rows' lines and the other
/// cadence's tile the line's days, the run's add up to it, and the facts bill the other cadence's
/// days alike on this document (R-084, R-080).
fn split_at_ending(line: &LineItem, legacy: &[LineItem], context: &Context) -> bool {
    let (Some(lineage), Some(months)) = (
        context.lineage_of(line),
        context.life_of(line).and_then(|life| life.months),
    ) else {
        return false;
    };
    let mut parts: Vec<&LineItem> = legacy
        .iter()
        .filter(|part| {
            context.lineage_of(part) == Some(lineage)
                && line.start_date <= part.start_date
                && part.end_date <= line.end_date
        })
        .collect();
    parts.sort_by_key(|part| part.start_date);
    let of_run =
        |part: &LineItem| context.life_of(part).and_then(|life| life.months) == Some(months);
    let (run, other): (Vec<&LineItem>, Vec<&LineItem>) =
        parts.iter().copied().partition(|part| of_run(part));
    let billed_alike = |part: &&LineItem| {
        context.facts_lines.iter().any(|facts| {
            context.lineage_of(facts) == Some(lineage)
                && (facts.start_date, facts.end_date, facts.amount_subtotal)
                    == (part.start_date, part.end_date, part.amount_subtotal)
        })
    };
    run.len() > 1
        && !other.is_empty()
        && other.iter().all(billed_alike)
        && parts
            .first()
            .is_some_and(|first| first.start_date == line.start_date)
        && parts
            .last()
            .is_some_and(|last| last.end_date == line.end_date)
        && parts
            .windows(2)
            .all(|pair| pair[0].end_date == pair[1].start_date)
        && run.iter().map(|part| part.amount_subtotal).sum::<i64>() == line.amount_subtotal
}

fn backdated_final(difference: &Difference, context: &Context) -> bool {
    context.backdated_ending.is_some() && as_at_ending(difference, context)
}

fn written_day_final(difference: &Difference, context: &Context) -> bool {
    context.written_day_final && as_at_ending(difference, context)
}

/// A line the legacy path keeps on a draft past a backdated ending, which the facts close empty.
fn closed_past_backdated_ending(difference: &Difference, context: &Context) -> bool {
    context.draft
        && !context.final_invoice
        && context.facts_empty
        && difference.legacy_only().is_some()
        && context
            .backdated_ending
            .is_some_and(|on| context.invoice_date >= on)
}

/// A line of a lineage an amendment moved to another cadence, in the periods meeting its day.
fn by_an_amendments_cadence(difference: &Difference, context: &Context) -> bool {
    difference.lines().any(|line| {
        let Some(lineage) = context.lineage_of(line) else {
            return false;
        };
        context.amendment_days.iter().any(|day| {
            let amended = context.lives.iter().any(|life| {
                life.lineage == lineage && (life.from == *day || life.to == Some(*day))
            });
            amended && line.start_date <= *day && *day <= line.end_date
        })
    })
}

/// A legacy line from the invoice's day that the adjustment of that day billed (R-080).
fn billed_by_adjustment(difference: &Difference, context: &Context) -> bool {
    !context.draft
        && !context.final_invoice
        && context.facts_empty
        && difference.legacy_only().is_some_and(|line| {
            line.start_date == context.invoice_date
                && context.billed_elsewhere(line).any(|other| {
                    other.kind == InvoiceType::Adjustment && other.on == context.invoice_date
                })
        })
}

/// Arrears of a closed row another document billed up to its end, no other line of the row and
/// metric billing their days: the legacy path reads the row as settled and drops them (R-011).
fn dropped_as_settled(difference: &Difference, context: &Context) -> bool {
    let Some(line) = difference.facts_only() else {
        return false;
    };
    let Some(life) = context.life_of(line) else {
        return false;
    };
    let Some(to) = life.to else {
        return false;
    };
    let of_row = |other: &&Other| !other.void && row_of(&other.line) == Some(life.row);
    let settled = context
        .others
        .iter()
        .filter(of_row)
        .any(|other| other.line.end_date == to);
    let billed = context.others.iter().filter(of_row).any(|other| {
        other.line.metric_id == line.metric_id
            && other.line.start_date < line.end_date
            && line.start_date < other.line.end_date
    });
    settled
        && !billed
        && line.start_date < line.end_date
        && line.end_date <= to.min(context.invoice_date)
}

/// A line of a row opened on a window's start the legacy path cannot see, on a document from that
/// day before the window's first renewal.
fn past_an_unseen_switch(difference: &Difference, context: &Context) -> bool {
    context.unseen.iter().any(|from| {
        let renewal = context
            .others
            .iter()
            .filter(|other| other.kind == InvoiceType::Recurring && other.on > *from)
            .map(|other| other.on)
            .min();
        *from <= context.invoice_date
            && renewal.is_none_or(|renewal| context.invoice_date < renewal)
            && difference
                .lines()
                .any(|line| context.life_of(line).is_some_and(|life| life.from == *from))
    })
}

/// A fee's days between a cadence change and its new grid's origin: the facts price them over
/// the new grid's period before the origin, the legacy path the same price over a longer one.
fn priced_as_stub(difference: &Difference, context: &Context) -> bool {
    difference
        .pair()
        .is_some_and(|(legacy, facts)| stub_alike(legacy, facts, context))
}

/// `facts` prices a stub over the new grid's period before its origin, `legacy` the same days at
/// the same price over a longer one (R-084).
fn stub_alike(legacy: &LineItem, facts: &LineItem, context: &Context) -> bool {
    let (Some(price), Some(months)) = (
        context.price_of(facts),
        context.life_of(facts).and_then(|life| life.months),
    ) else {
        return false;
    };
    let stub = days(facts.start_date, facts.end_date);
    context.stubs.iter().any(|(from, origin)| {
        let Some(period_start) = months_before(*origin, months) else {
            return false;
        };
        // What is left at the stub's start less at its end (R-079): its share when it ends at
        // the origin, a minor unit apart at most when an ending cuts it first.
        let facts_priced = prorated(price, 1, 1).is_some_and(|whole| {
            billing_core::proration::DaySpan {
                period: (period_start, *origin),
                from: facts.start_date,
                until: facts.end_date,
            }
            .prorate(whole)
                == facts.amount_subtotal
        });
        let legacy_priced = (stub.max(1)..=days(period_start, *origin).max(stub) + 31)
            .any(|period| prorated(price, stub, period) == Some(legacy.amount_subtotal));
        facts.start_date >= *from
            && facts.end_date <= *origin
            && (legacy.start_date, legacy.end_date) == (facts.start_date, facts.end_date)
            && legacy.quantity == facts.quantity
            && facts_priced
            && legacy_priced
    })
}

/// The rest of a period of a fee repriced on the invoice's day, the new price's rest less the
/// old one's (R-080, R-078).
fn repriced_rest(difference: &Difference, context: &Context) -> bool {
    let Some(line) = difference.facts_only() else {
        return false;
    };
    if !bills_a_repriced_rest(line, context.invoice_date, &context.lives) {
        return false;
    }
    let Some(life) = context.life_of(line) else {
        return false;
    };
    let replaced = context.lives.iter().find(|earlier| {
        earlier.lineage == life.lineage && earlier.to == Some(context.invoice_date)
    });
    let price = |row: uuid::Uuid| context.fee(row).and_then(|fee| fee.price);
    let (Some(months), Some(replaced)) = (life.months, replaced) else {
        return false;
    };
    let (Some(new), Some(old), Some(period_start)) = (
        price(life.row),
        price(replaced.row),
        months_before(line.end_date, months),
    ) else {
        return false;
    };
    let left = days(context.invoice_date, line.end_date);
    let period = days(period_start, line.end_date);
    match (prorated(new, left, period), prorated(old, left, period)) {
        (Some(new), Some(old)) => new - old == line.amount_subtotal,
        _ => false,
    }
}

fn repriced_arrears(difference: &Difference, context: &Context) -> bool {
    difference.legacy_only().is_some_and(|line| {
        bills_a_run_going_on(
            line,
            context.invoice_date,
            &context.lives,
            &context.facts_lines,
        )
    })
}

fn carried_run(difference: &Difference, context: &Context) -> bool {
    let issued: Vec<LineItem> = context
        .others
        .iter()
        .filter(|other| !other.void && other.on < context.invoice_date)
        .map(|other| other.line.clone())
        .collect();
    difference.legacy_only().is_some_and(|line| {
        bills_inside_a_billed_run(line, &context.lives, &issued, &context.facts_lines)
    })
}

/// A line of a lineage a dated price splits, billed over the same quantity at other rates (R-097).
fn dated_price(difference: &Difference, context: &Context) -> bool {
    let Some((a, b)) = difference.pair() else {
        return false;
    };
    let dated = [a, b].iter().any(|line| {
        row_of(line)
            .and_then(|row| context.roots.get(&row))
            .is_some_and(|root| context.dated.contains(root))
    });
    let rated = |line: &LineItem| {
        let mut line = line.clone();
        line.local_id.clear();
        line.unit_price = None;
        line.amount_subtotal = 0;
        line.taxable_amount = 0;
        line.tax_amount = 0;
        line.amount_total = 0;
        line.tax_details.clear();
        for sub_line in &mut line.sub_lines {
            sub_line.local_id.clear();
            sub_line.unit_price = Decimal::ZERO;
            sub_line.total = 0;
        }
        serde_json::to_value(line).unwrap_or_default()
    };
    dated && rated(a) == rated(b)
}

/// A legacy line on the invoice of a cancellation backdated into billed periods whose days an
/// issued line of its lineage billed: given back by its notes, billed again by the legacy path.
fn billed_before_backdated_ending(difference: &Difference, context: &Context) -> bool {
    context.cancelled_here
        && difference.legacy_only().is_some_and(|line| {
            let lineage = context.lineage_of(line);
            lineage.is_some()
                && context.others.iter().any(|other| {
                    other.issued
                        && context.lineage_of(&other.line) == lineage
                        && other.line.start_date <= line.start_date
                        && line.end_date <= other.line.end_date
                })
        })
}

/// A whole period of a fee's own cadence before the invoice's, whose own recurring invoice was
/// never written, at its whole price, no other document billing any of its days: caught up by
/// this renewal (R-084).
fn caught_up(difference: &Difference, context: &Context) -> bool {
    let Some(line) = difference.facts_only() else {
        return false;
    };
    let Some(life) = context.life_of(line) else {
        return false;
    };
    let lineage = Some(life.lineage);
    let billed = context.others.iter().any(|other| {
        context.lineage_of(&other.line) == lineage
            && other.line.start_date < line.end_date
            && line.start_date < other.line.end_date
    });
    life.months
        .is_some_and(|months| months_after(line.start_date, months) == Some(line.end_date))
        && line.end_date <= context.invoice_date
        && life.from <= line.start_date
        && life.to.is_none_or(|to| line.end_date <= to)
        && !context.renewals.contains(&line.start_date)
        && !billed
        && context
            .price_of(line)
            .is_some_and(|price| prorated(price, 1, 1) == Some(line.amount_subtotal))
}

/// The same charge billed over a period of the run's own grid, which an earlier line of its
/// lineage ends at, where the legacy path bills the same length from the change's grid (R-084).
fn on_its_runs_grid(difference: &Difference, context: &Context) -> bool {
    let Some((legacy, facts)) = difference.pair() else {
        return false;
    };
    let Some(life) = context.life_of(facts) else {
        return false;
    };
    let Some(months) = life.months else {
        return false;
    };
    let carried = context.lives.iter().any(|earlier| {
        earlier.lineage == life.lineage
            && earlier.row != life.row
            && earlier.to == Some(life.from)
            && earlier.months == life.months
            && earlier.arrears == life.arrears
    });
    let continues = context.others.iter().any(|other| {
        !other.void
            && context.lineage_of(&other.line) == Some(life.lineage)
            && other.line.end_date == facts.start_date
    });
    let whole = |line: &LineItem| months_after(line.start_date, months) == Some(line.end_date);
    row_of(legacy) == row_of(facts)
        && (legacy.quantity, legacy.amount_subtotal) == (facts.quantity, facts.amount_subtotal)
        && whole(legacy)
        && whole(facts)
        && legacy.start_date != facts.start_date
        && carried
        && continues
}

/// A one-time fee added before the day it bills (in a free trial), billed once, here (R-093).
/// A one-time fee added in a free trial, billed on the invoice of the day it lives into and on no
/// other document; or on a backdated final at the ending that closed the trial first, its row
/// alive that day, every other line of it voided, credited whole, or issued for days wholly after
/// the ending, which gives them back whole (R-093, R-016).
fn deferred_one_time(difference: &Difference, context: &Context) -> bool {
    difference.facts_only().is_some_and(|line| {
        let lineage = context.lineage_of(line);
        let one_time = line.charge_type == Some(crate::domain::invoice_lines::ChargeType::OneTime)
            && lineage.is_some();
        let of_lineage = || {
            context
                .others
                .iter()
                .filter(move |other| context.lineage_of(&other.line) == lineage)
        };
        let deferred = (line.start_date, line.end_date)
            == (context.invoice_date, context.invoice_date)
            && context
                .life_of(line)
                .is_some_and(|life| life.from < context.invoice_date)
            && of_lineage().next().is_none();
        let day = line.start_date;
        let closed_trial = context.final_invoice
            && line.end_date == day
            && context.backdated_ending == Some(day)
            && context.trial_end.is_some_and(|end| day < end)
            && context
                .life_of(line)
                .is_some_and(|life| life.from < day && life.to.is_none_or(|to| to >= day))
            && of_lineage().all(|other| {
                other.void
                    || other.credited >= other.line.amount_subtotal
                    || (other.issued && other.line.start_date > day)
            });
        one_time && (deferred || closed_trial)
    })
}

/// A line the legacy path bills of a charge a voided invoice billed over the same days, for as
/// much or more, no live invoice billing that voided slot again (R-096).
fn forgiven_by_void(difference: &Difference, context: &Context) -> bool {
    difference.legacy_only().is_some_and(|line| {
        context.billed_elsewhere(line).any(|voided| {
            voided.void
                && voided.line.amount_subtotal >= line.amount_subtotal
                && !context
                    .others
                    .iter()
                    .any(|other| !other.void && other.kind == voided.kind && other.on == voided.on)
        })
    })
}

/// The periods of a grid of `months` anchored on one of `origins` holding `day`.
fn grid_periods(
    day: NaiveDate,
    months: u32,
    origins: &[NaiveDate],
) -> impl Iterator<Item = (NaiveDate, NaiveDate)> + '_ {
    origins.iter().flat_map(move |origin| {
        (0..=40).flat_map(move |k| {
            [
                months_after(*origin, k * months),
                months_before(*origin, k * months),
            ]
            .into_iter()
            .flatten()
            .filter_map(move |start| Some((start, months_after(start, months)?)))
            .filter(move |(start, end)| *start <= day && day < *end)
        })
    })
}

/// Arrears a backdated ending cuts, of a row in force on their days that a change closed after
/// the ending, billed on its final invoice and by no other document: the row's price over its
/// grid period, what is left at the line's start less what is left at the ending (R-016, R-011,
/// R-079).
fn backdated_arrears(difference: &Difference, context: &Context) -> bool {
    let Some(line) = difference.facts_only() else {
        return false;
    };
    let Some(life) = context.life_of(line) else {
        return false;
    };
    let (Some(months), Some(whole)) = (
        life.months,
        context
            .price_of(line)
            .and_then(|price| prorated(price, 1, 1)),
    ) else {
        return false;
    };
    let billed = context.others.iter().any(|other| {
        context.lineage_of(&other.line) == Some(life.lineage)
            && other.line.start_date < line.end_date
            && line.start_date < other.line.end_date
    });
    context.final_invoice
        && context.backdated_ending == Some(line.end_date)
        && life.arrears
        && life.from <= line.start_date
        && life.to.is_some_and(|to| to > line.end_date)
        && line.start_date < line.end_date
        && !billed
        && grid_periods(line.start_date, months, &context.origins).any(|(start, end)| {
            line.end_date <= end
                && billing_core::proration::DaySpan {
                    period: (start, end),
                    from: line.start_date,
                    until: line.end_date,
                }
                .prorate(whole)
                    == line.amount_subtotal
        })
}

/// A capacity's overage a backdated final bills alone for the days its ending cut, no other
/// document billing any of its usage then (the whole allowance's line bills 0): units priced
/// exactly to its total at the currency's precision, no more
/// than the allowance the cut took off, `included × (1 − days / period)`, which the legacy path
/// counted whole (R-002, R-016).
fn overage_of_a_cut_allowance(difference: &Difference, context: &Context) -> bool {
    let Some(line) = difference.facts_only() else {
        return false;
    };
    let (Some(life), [overage]) = (context.life_of(line), line.sub_lines.as_slice()) else {
        return false;
    };
    let (Some(months), Some(included)) = (
        life.months,
        row_of(line)
            .and_then(|row| context.fee(row))
            .and_then(|fee| fee.included),
    ) else {
        return false;
    };
    let billed = context.others.iter().any(|other| {
        context.lineage_of(&other.line) == Some(life.lineage)
            && other.line.metric_id == line.metric_id
            && other.line.amount_subtotal != 0
            && other.line.start_date < line.end_date
            && line.start_date < other.line.end_date
    });
    let minor = Decimal::from(10_i64.pow(context.precision));
    let used_days = days(line.start_date, line.end_date);
    let periods = [
        months_before(line.end_date, months).map(|start| days(start, line.end_date)),
        months_after(line.start_date, months).map(|end| days(line.start_date, end)),
    ];
    context.final_invoice
        && context.backdated_ending == Some(line.end_date)
        && line.metric_id.is_some()
        && used_days > 0
        && !billed
        && overage.total == line.amount_subtotal
        && overage.quantity > Decimal::ZERO
        && prorated(overage.quantity * overage.unit_price * minor, 1, 1) == Some(overage.total)
        && periods.into_iter().flatten().any(|period| {
            used_days < period
                && overage.quantity
                    <= included - included * Decimal::from(used_days) / Decimal::from(period)
        })
}

/// The stored MRR against the MRR of the fees in force in the stored period, once the row has
/// caught up and its creation's New Business is booked (the writer's own guard).
async fn mrr_difference(
    services: &Services,
    conn: &mut PgConn,
    stored: &Subscription,
    today: NaiveDate,
) -> StoreResult<Option<String>> {
    use diesel_models::enums::SubscriptionEventType;
    use diesel_models::subscription_events::SubscriptionEventRow;

    let Some(end) = stored
        .current_period_end
        .filter(|end| *end > today && stored.status == SubscriptionStatusEnum::Active)
    else {
        return Ok(None);
    };
    let created = SubscriptionEventRow::fetch_by_subscription_id_and_event_type(
        conn,
        stored.id,
        SubscriptionEventType::Created,
        stored.start_date,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    if created.is_some_and(|event| {
        event.mrr_delta.is_some_and(|delta| delta != 0) && event.bi_mrr_movement_log_id.is_none()
    }) {
        return Ok(None);
    }
    let day = end.pred_opt().map_or(today, |last| today.min(last));
    let details = services
        .store
        .subscription_details_as_of(conn, stored.tenant_id, stored.id, day)
        .await?;
    let precision =
        crate::constants::Currencies::resolve_currency_precision(&details.subscription.currency)
            .unwrap_or(2);
    let lines = crate::services::subscriptions::plan_change::calculate_components_mrr_with_slots(
        conn,
        stored.tenant_id,
        stored.id,
        &details.price_components,
        precision,
    )
    .await?;
    let lines =
        crate::services::subscriptions::utils::with_add_on_lines(&details, lines, precision);
    let restated = crate::services::subscriptions::utils::subscription_mrr(
        &lines,
        details
            .minimum
            .as_ref()
            .map(|minimum| (minimum.amount_subunit, &minimum.scope)),
        details.subscription.period,
        details.subscription.tax_inclusive_rate,
    );
    let stored_mrr = stored.mrr_cents as i64;
    Ok((restated != stored_mrr).then(|| format!("restated {restated} stored {stored_mrr}")))
}

/// A fee row's life: its lineage, its cadence in months (none for a one-time fee) and whether it
/// bills in arrears.
#[derive(Clone, Copy, Debug)]
struct FeeLife {
    row: uuid::Uuid,
    lineage: uuid::Uuid,
    from: NaiveDate,
    to: Option<NaiveDate>,
    months: Option<u32>,
    arrears: bool,
}

async fn fee_lives(
    conn: &mut PgConn,
    subscription: SubscriptionId,
    arrears: &[uuid::Uuid],
) -> StoreResult<Vec<FeeLife>> {
    use crate::domain::enums::SubscriptionFeeBillingPeriod;
    let months = |period: diesel_models::enums::SubscriptionFeeBillingPeriod| {
        SubscriptionFeeBillingPeriod::from(period)
            .as_billing_period_opt()
            .map(|period| period.as_months())
    };
    let components =
        diesel_models::subscription_components::SubscriptionComponentRow::list_fact_rows(
            conn,
            subscription,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let add_ons = diesel_models::subscription_add_ons::SubscriptionAddOnRow::list_fact_rows(
        conn,
        subscription,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    Ok(components
        .into_iter()
        .map(|c| FeeLife {
            row: c.id.as_uuid(),
            lineage: c.lineage_id.unwrap_or(c.id).as_uuid(),
            from: c.effective_from,
            to: c.effective_to,
            months: months(c.period),
            arrears: arrears.contains(&c.id.as_uuid()),
        })
        .chain(add_ons.into_iter().map(|a| FeeLife {
            row: a.id.as_uuid(),
            lineage: a.lineage_id.unwrap_or(a.id).as_uuid(),
            from: a.effective_from,
            to: a.effective_to,
            months: months(a.period),
            arrears: arrears.contains(&a.id.as_uuid()),
        }))
        .collect())
}

/// Whether `line` bills, from the invoice's day, the rest of a period of its fee begun before
/// that day, the fee's row of that day replacing one of the same cadence ending then (R-080).
fn bills_a_repriced_rest(line: &LineItem, invoice_date: NaiveDate, lives: &[FeeLife]) -> bool {
    let Some(row) = line
        .sub_component_id
        .map(|id| id.as_uuid())
        .or_else(|| line.sub_add_on_id.map(|id| id.as_uuid()))
    else {
        return false;
    };
    let Some(lineage) = lives.iter().find(|life| life.row == row).map(|l| l.lineage) else {
        return false;
    };
    let of_lineage = || lives.iter().filter(move |life| life.lineage == lineage);
    let Some(months) = of_lineage()
        .find(|life| life.from == invoice_date)
        .and_then(|life| life.months)
    else {
        return false;
    };
    let replaced = of_lineage().any(|life| {
        life.from < invoice_date && life.to == Some(invoice_date) && life.months == Some(months)
    });
    let within = line
        .start_date
        .checked_add_months(chrono::Months::new(months))
        .is_some_and(|whole| line.end_date < whole);
    line.start_date == invoice_date && line.end_date > line.start_date && replaced && within
}

/// Whether `line`, billed by the legacy path alone, bills part of a period of a row in arrears that
/// ended by the invoice's day, replaced then by a row of its lineage at its cadence and in arrears:
/// one run going on (R-084), of which the facts bill none of those days on this invoice.
fn bills_a_run_going_on(
    line: &LineItem,
    invoice_date: NaiveDate,
    lives: &[FeeLife],
    facts_lines: &[LineItem],
) -> bool {
    row_of(line).is_some_and(|row| {
        run_goes_on(
            row,
            (line.start_date, line.end_date),
            invoice_date,
            lives,
            facts_lines,
        )
    })
}

/// [`bills_a_run_going_on`] for the days `[start, end)` of `row`.
fn run_goes_on(
    row: uuid::Uuid,
    (start, end_date): (NaiveDate, NaiveDate),
    invoice_date: NaiveDate,
    lives: &[FeeLife],
    facts_lines: &[LineItem],
) -> bool {
    let Some(life) = lives.iter().find(|life| life.row == row) else {
        return false;
    };
    let (Some(months), Some(end)) = (life.months, life.to) else {
        return false;
    };
    let succeeded = lives.iter().any(|next| {
        next.lineage == life.lineage
            && next.row != life.row
            && next.from == end
            && next.months == Some(months)
            && next.arrears
    });
    let within = start
        .checked_add_months(chrono::Months::new(months))
        .is_some_and(|whole| end_date < whole);
    let billed_by_facts = facts_lines.iter().any(|other| {
        row_of(other)
            .and_then(|row| lives.iter().find(|l| l.row == row))
            .is_some_and(|other_life| other_life.lineage == life.lineage)
            && other.start_date < end_date
            && start < other.end_date
    });
    life.arrears
        && succeeded
        && within
        && !billed_by_facts
        && end <= invoice_date
        && end_date == end
        && life.from <= start
        && start < end
}

/// Whether `line`, billed by the legacy path alone, bills a period of a row that replaced one of
/// its lineage at its cadence and billing mode, from a day inside a period an issued line of that
/// lineage bills: one run going on on its own grid (R-084), the facts billing none of it here.
fn bills_inside_a_billed_run(
    line: &LineItem,
    lives: &[FeeLife],
    issued: &[LineItem],
    facts_lines: &[LineItem],
) -> bool {
    let life_of = |line: &LineItem| {
        line.sub_component_id
            .map(|id| id.as_uuid())
            .or_else(|| line.sub_add_on_id.map(|id| id.as_uuid()))
            .and_then(|row| lives.iter().find(|life| life.row == row))
    };
    let Some(life) = life_of(line) else {
        return false;
    };
    let Some(months) = life.months else {
        return false;
    };
    let replaced = lives.iter().any(|earlier| {
        earlier.lineage == life.lineage
            && earlier.row != life.row
            && earlier.to == Some(life.from)
            && earlier.months == Some(months)
            && earlier.arrears == life.arrears
    });
    let of_lineage = |other: &LineItem| life_of(other).is_some_and(|l| l.lineage == life.lineage);
    let inside_billed = issued.iter().any(|other| {
        of_lineage(other) && other.start_date < line.start_date && line.start_date < other.end_date
    });
    let billed_by_facts = facts_lines.iter().any(|other| {
        of_lineage(other) && other.start_date < line.end_date && line.start_date < other.end_date
    });
    replaced && inside_billed && !billed_by_facts && life.from <= line.start_date
}

/// The days a document's lines bill, `[first start, last end)`.
fn span_of(priced: &PricedInvoice) -> Option<(NaiveDate, NaiveDate)> {
    let (_, lines) = lines_of(priced);
    let from = lines.iter().map(|line| line.start_date).min()?;
    let until = lines.iter().map(|line| line.end_date).max()?;
    Some((from, until))
}

/// Whether a price the subscription's fee rows name, closed ones too, has a successor taking
/// over inside `span`.
/// Each fee row of the subscription, component or add-on, by id: its lineage's root.
async fn lineage_roots(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
) -> StoreResult<std::collections::HashMap<uuid::Uuid, uuid::Uuid>> {
    let subscription = details.subscription.id;
    let components =
        diesel_models::subscription_components::SubscriptionComponentRow::list_fact_rows(
            conn,
            subscription,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let add_ons = diesel_models::subscription_add_ons::SubscriptionAddOnRow::list_fact_rows(
        conn,
        subscription,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    Ok(components
        .iter()
        .map(|c| (c.id.as_uuid(), c.lineage_id.unwrap_or(c.id).as_uuid()))
        .chain(
            add_ons
                .iter()
                .map(|a| (a.id.as_uuid(), a.lineage_id.unwrap_or(a.id).as_uuid())),
        )
        .collect())
}

/// The lineages, by root, whose price a dated successor changes strictly inside `(from, until)`.
async fn succeeded_within(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    (from, until): (NaiveDate, NaiveDate),
) -> StoreResult<std::collections::HashSet<uuid::Uuid>> {
    let subscription = details.subscription.id;
    let components =
        diesel_models::subscription_components::SubscriptionComponentRow::list_fact_rows(
            conn,
            subscription,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let add_ons = diesel_models::subscription_add_ons::SubscriptionAddOnRow::list_fact_rows(
        conn,
        subscription,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    let priced: Vec<(common_domain::ids::PriceId, uuid::Uuid)> = components
        .iter()
        .filter_map(|c| Some((c.price_id?, c.lineage_id.unwrap_or(c.id).as_uuid())))
        .chain(
            add_ons
                .iter()
                .filter_map(|a| Some((a.price_id?, a.lineage_id.unwrap_or(a.id).as_uuid()))),
        )
        .collect();
    let roots: Vec<_> = priced.iter().map(|(price, _)| *price).collect();
    let succeeded: Vec<_> = diesel_models::prices::PriceRow::successors_of(
        conn,
        &roots,
        details.subscription.tenant_id,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?
    .into_iter()
    .filter(|row| row.effective_from.is_some_and(|at| from < at && at < until))
    .filter_map(|row| row.supersedes)
    .collect();
    Ok(priced
        .into_iter()
        .filter(|(price, _)| succeeded.contains(price))
        .map(|(_, lineage)| lineage)
        .collect())
}

/// The lines a document holds, each as its stored JSON: every field it is written with.
fn lines_of(priced: &PricedInvoice) -> (&'static str, &[LineItem]) {
    match priced {
        PricedInvoice::Stored(content) => ("stored", &content.invoice_lines),
        PricedInvoice::Lines(priced) => ("priced", &priced.lines),
    }
}

/// A line's name without the date ranges an invoice billing its fee by several lines adds.
fn base_name(name: &str) -> &str {
    let mut name = name;
    while let Some(open) = name.rfind(" (")
        && name.ends_with(')')
        && name[open + 2..name.len() - 1]
            .split(" - ")
            .all(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").is_ok())
    {
        name = &name[..open];
    }
    name
}

/// A line as a difference names it.
fn describe(line: &LineItem) -> String {
    format!(
        "{} {}..{} qty {:?} x {:?} = {}",
        line.name,
        line.start_date,
        line.end_date,
        line.quantity,
        line.unit_price,
        line.amount_subtotal
    )
}

/// The documents' differences, each with the lines it compares. Some get a provisional class
/// [`narrow`] checks against the subscription.
fn document_differences(slot: &str, legacy: &PricedInvoice, core: &PricedInvoice) -> Differences {
    let mut differences = Differences::new();
    // Both keep the written lines; only the credit they would draw is estimated again.
    if let (PricedInvoice::Stored(_), PricedInvoice::Stored(_)) = (legacy, core) {
        return differences;
    }
    let ((legacy_kind, legacy_lines), (core_kind, core_lines)) = (lines_of(legacy), lines_of(core));
    if legacy_kind != core_kind {
        differences.push(Difference::of_document(
            "document",
            format!("{slot}: legacy {legacy_kind} lines, facts {core_kind} lines"),
        ));
    }
    // Only the facts know the whole period a prorated line is priced over.
    let unpriced = |true_up: &Option<PeriodTrueUp>| {
        true_up.clone().map(|mut true_up| {
            for spend in &mut true_up.prior {
                spend.nominal = None;
            }
            true_up
        })
    };
    if let (PricedInvoice::Lines(a), PricedInvoice::Lines(b)) = (legacy, core)
        && unpriced(&a.true_up) != unpriced(&b.true_up)
    {
        differences.push(Difference::of_document(
            "document",
            format!(
                "{slot}: true-up legacy {:?} facts {:?}",
                a.true_up, b.true_up
            ),
        ));
    }
    // A one-time fee the legacy path bills again, its own document having billed it.
    let once = |line: &LineItem| {
        line.charge_type == Some(crate::domain::invoice_lines::ChargeType::OneTime)
            && !core_lines.iter().any(|core| {
                (&core.name, core.start_date, core.end_date)
                    == (&line.name, line.start_date, line.end_date)
            })
    };
    for line in legacy_lines.iter().filter(|line| once(line)) {
        differences.push(Difference::new(
            "legacy one-time",
            format!("{slot}: legacy only: {}", describe(line)),
            &[line],
            &[],
        ));
    }
    let legacy_lines: Vec<&LineItem> = legacy_lines.iter().filter(|line| !once(line)).collect();
    // Rows of one fee in turn over a period: the legacy path bills each row's part on a line of
    // its own, its share rounded once; the facts one line, each part a difference of what is left.
    // A part may sit on either side of another charge's line both paths bill alike.
    let billed_alike = |line: &LineItem, legacy: &LineItem| {
        core_lines.iter().any(|core| {
            !std::ptr::eq(core, line)
                && (base_name(&core.name), core.start_date, core.end_date)
                    == (base_name(&legacy.name), legacy.start_date, legacy.end_date)
        })
    };
    let parts = |line: &LineItem| -> (Vec<&LineItem>, &'static str) {
        let within: Vec<&LineItem> = legacy_lines
            .iter()
            .copied()
            .filter(|legacy| {
                base_name(&legacy.name) == base_name(&line.name)
                    && line.start_date <= legacy.start_date
                    && legacy.end_date <= line.end_date
            })
            .collect();
        let (mut others, mut parts): (Vec<&LineItem>, Vec<&LineItem>) = within
            .into_iter()
            .partition(|legacy| billed_alike(line, legacy));
        let around = !others.is_empty();
        parts.sort_by_key(|part| part.start_date);
        others.extend(parts.iter().copied());
        others.sort_by_key(|part| part.start_date);
        let tiled = parts.len() > 1
            && others
                .first()
                .is_some_and(|p| p.start_date == line.start_date)
            && others.last().is_some_and(|p| p.end_date == line.end_date)
            && others.windows(2).all(|w| w[0].end_date == w[1].start_date);
        let sum: i64 = parts.iter().map(|part| part.amount_subtotal).sum();
        let apart = (sum - line.amount_subtotal).unsigned_abs();
        // Its last part cut by the fixed term's end, which the legacy path bills whole; `narrow`
        // checks the term ends there and that part is the whole period's price.
        let whole_at_term_end = parts.split_last().is_some_and(|(last, before)| {
            let left =
                line.amount_subtotal - before.iter().map(|part| part.amount_subtotal).sum::<i64>();
            left > 0 && last.amount_subtotal >= left
        });
        match (tiled, apart < parts.len() as u64, around) {
            (true, true, true) => (parts, "split charge"),
            (true, true, false) => (parts, "tiling"),
            (true, false, _) if whole_at_term_end => (parts, "term end"),
            _ => (Vec::new(), ""),
        }
    };
    let mut split: Vec<&LineItem> = Vec::new();
    for line in core_lines.iter() {
        let (parts, class) = parts(line);
        if parts.is_empty() {
            continue;
        }
        differences.push(Difference::new(
            class,
            format!(
                "{slot}: {} billed by {} legacy lines",
                describe(line),
                parts.len()
            ),
            &parts,
            &[line],
        ));
        split.extend(parts);
    }
    let core_lines: Vec<&LineItem> = core_lines
        .iter()
        .filter(|line| parts(line).0.is_empty())
        .collect();
    let legacy_lines: Vec<&LineItem> = legacy_lines
        .iter()
        .copied()
        .filter(|line| !split.iter().any(|part| std::ptr::eq(*part, *line)))
        .collect();
    // Arrears of a row ended off its grid riding the first renewal from its end (R-011), after
    // the invoice's own periods began, which the legacy path never bills.
    let rider = |line: &LineItem| {
        !legacy_lines.iter().any(|legacy| {
            (base_name(&legacy.name), legacy.start_date, legacy.end_date)
                == (base_name(&line.name), line.start_date, line.end_date)
        }) && core_lines
            .iter()
            .any(|other| other.start_date >= line.end_date && other.end_date > line.end_date)
    };
    for line in core_lines.iter().filter(|line| rider(line)) {
        differences.push(Difference::new(
            "rider",
            format!("{slot}: facts only: {}", describe(line)),
            &[],
            &[line],
        ));
    }
    let core_lines: Vec<&LineItem> = core_lines
        .iter()
        .copied()
        .filter(|line| !rider(line))
        .collect();
    // A legacy line of days before every fee period the facts put on this document, which no
    // facts line of the document bills: arrears the facts billed on an earlier renewal. A
    // document of usage alone reads its usage periods.
    let starts = |usage: bool| {
        core_lines
            .iter()
            .filter(move |line| {
                (line.charge_type == Some(crate::domain::invoice_lines::ChargeType::Usage)) == usage
            })
            .map(|line| line.start_date)
            .min()
    };
    let first = starts(false).or_else(|| starts(true));
    let late = |line: &LineItem| {
        first.is_some_and(|first| line.end_date <= first)
            && !core_lines.iter().any(|core| {
                (base_name(&core.name), core.start_date, core.end_date)
                    == (base_name(&line.name), line.start_date, line.end_date)
            })
    };
    for line in legacy_lines.iter().filter(|line| late(line)) {
        differences.push(Difference::new(
            "late rider",
            format!("{slot}: legacy only: {}", describe(line)),
            &[line],
            &[],
        ));
    }
    let legacy_lines: Vec<&LineItem> = legacy_lines
        .iter()
        .copied()
        .filter(|line| !late(line))
        .collect();
    // Lines of the same charge (name and dates) are compared, whatever their order; the others
    // in order, so a line one side lacks does not shift every one after it.
    let same = |a: &LineItem, b: &LineItem| {
        (&a.name, a.start_date, a.end_date) == (&b.name, b.start_date, b.end_date)
    };
    let mut paired: Vec<(Option<&LineItem>, Option<&LineItem>)> = Vec::new();
    let mut core_left: Vec<&LineItem> = core_lines.clone();
    let mut legacy_left: Vec<&LineItem> = Vec::new();
    for a in &legacy_lines {
        match core_left.iter().position(|b| same(a, b)) {
            Some(at) => paired.push((Some(*a), Some(core_left.remove(at)))),
            None => legacy_left.push(*a),
        }
    }
    for i in 0..legacy_left.len().max(core_left.len()) {
        paired.push((legacy_left.get(i).copied(), core_left.get(i).copied()));
    }
    for (i, pair) in paired.into_iter().enumerate() {
        match pair {
            (Some(a), Some(b)) => {
                let (va, vb) = (
                    serde_json::to_value(a).unwrap_or_default(),
                    serde_json::to_value(b).unwrap_or_default(),
                );
                if va == vb {
                    continue;
                }
                let mut fields: Vec<String> = match (&va, &vb) {
                    (serde_json::Value::Object(ma), serde_json::Value::Object(mb)) => ma
                        .keys()
                        .chain(mb.keys())
                        .filter(|k| ma.get(*k) != mb.get(*k))
                        .map(|k| {
                            format!(
                                "{k}: legacy {} facts {}",
                                ma.get(k).map(ToString::to_string).unwrap_or_default(),
                                mb.get(k).map(ToString::to_string).unwrap_or_default()
                            )
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                fields.sort();
                fields.dedup();
                let unkeyed = |line: &LineItem| {
                    let mut line = line.clone();
                    line.local_id.clear();
                    for sub_line in &mut line.sub_lines {
                        sub_line.local_id.clear();
                    }
                    serde_json::to_value(line).unwrap_or_default()
                };
                // The same prorated line but for a minor unit of its amounts.
                let tiling =
                    a.is_prorated && (a.amount_subtotal - b.amount_subtotal).abs() == 1 && {
                        let mut priced = a.clone();
                        if base_name(&a.name) == base_name(&b.name) {
                            priced.name = b.name.clone();
                        }
                        priced.amount_subtotal = b.amount_subtotal;
                        priced.taxable_amount = b.taxable_amount;
                        priced.tax_amount = b.tax_amount;
                        priced.amount_total = b.amount_total;
                        priced.unit_price = b.unit_price;
                        priced.tax_details = b.tax_details.clone();
                        unkeyed(&priced) == unkeyed(b)
                    };
                // A capacity's overage over the same days and rate, its allowance prorated by the
                // facts with its fee where the legacy path counted it whole.
                let overage = |line: &LineItem| match line.sub_lines.as_slice() {
                    [only] if only.name == "Overage" => Some(only.unit_price),
                    _ => None,
                };
                let allowance = a.metric_id.is_some()
                    && (a.start_date, a.end_date, a.metric_id)
                        == (b.start_date, b.end_date, b.metric_id)
                    && overage(a).is_some()
                    && overage(a) == overage(b)
                    && b.amount_subtotal > a.amount_subtotal;
                // A fixed fee the legacy path billed whole over the same days, the facts prorated.
                let partial_capacity = a.metric_id.is_none()
                    && b.metric_id.is_none()
                    && (a.start_date, a.end_date, a.sub_component_id, a.quantity)
                        == (b.start_date, b.end_date, b.sub_component_id, b.quantity)
                    && !a.is_prorated
                    && b.is_prorated
                    && (0..a.amount_subtotal).contains(&b.amount_subtotal);
                differences.push(Difference::new(
                    if unkeyed(a) == unkeyed(b) {
                        "id"
                    } else if allowance {
                        "capacity allowance"
                    } else if partial_capacity {
                        "first partial capacity"
                    } else if fields
                        .iter()
                        .all(|f| f.starts_with("name:") || f.starts_with("local_id:"))
                        && base_name(&a.name) == base_name(&b.name)
                    {
                        "name range"
                    } else if tiling {
                        "tiling"
                    } else {
                        "line"
                    },
                    format!("{slot}: line {i} {}: {}", describe(a), fields.join("; ")),
                    &[a],
                    &[b],
                ));
            }
            (Some(a), None) => {
                differences.push(Difference::new(
                    "line",
                    format!("{slot}: legacy only: {}", describe(a)),
                    &[a],
                    &[],
                ));
            }
            (None, Some(b)) => {
                differences.push(Difference::new(
                    "line",
                    format!("{slot}: facts only: {}", describe(b)),
                    &[],
                    &[b],
                ));
            }
            (None, None) => {}
        }
    }
    differences
}

/// The derived status, current period and plan version against the stored row, once the row
/// has caught up with `today` (a renewal still to run leaves it a period behind).
fn state_differences(stored: &Subscription, state: &DerivedState, today: NaiveDate) -> Differences {
    let mut differences = Differences::new();
    // A cancellation backdated before the stored period is settled before its row is patched:
    // the facts end it already, the row still runs the period it gives back.
    let backdated = state.status == Status::Cancelled
        && state
            .current_period
            .is_some_and(|(from, to)| to.is_none() && from < stored.current_period_start);
    if stored.current_period_end.is_some_and(|end| end <= today) || backdated {
        return differences;
    }
    if let Some(status) = stored_as(&stored.status)
        && status != state.status
    {
        differences.push(Difference::of_document(
            "status",
            format!("derived {:?} stored {:?}", state.status, stored.status),
        ));
    }
    let period = Some((stored.current_period_start, stored.current_period_end));
    if state.current_period != period {
        differences.push(Difference::of_document(
            "current_period",
            format!("derived {:?} stored {period:?}", state.current_period),
        ));
    }
    if state.plan_version != Some(stored.plan_version_id) {
        differences.push(Difference::of_document(
            "plan_version",
            format!(
                "derived {:?} stored {}",
                state.plan_version, stored.plan_version_id
            ),
        ));
    }
    differences
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::subscription_minimums::SubscriptionMinimum;
    use crate::services::invoice_lines::invoice_lines::{BilledSpan, PricedLines};
    use rust_decimal::Decimal;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2025, m, day).unwrap()
    }

    fn fee(from: NaiveDate, until: NaiveDate, amount: i64) -> LineItem {
        LineItem {
            local_id: format!("{from}{until}"),
            name: format!("A fee ({from} - {until})"),
            charge_type: None,
            amount_subtotal: amount,
            tax_rate: Decimal::ZERO,
            taxable_amount: amount,
            tax_amount: 0,
            amount_total: amount,
            tax_details: vec![],
            quantity: None,
            unit_price: None,
            start_date: from,
            end_date: until,
            sub_lines: vec![],
            is_prorated: true,
            price_component_id: None,
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: None,
            metric_id: None,
            description: None,
            group_by_dimensions: None,
            kind: crate::domain::LineItemKind::Regular,
            credit_grant_amount: None,
            credit_grant_expiry: None,
            credit_grant_included: false,
            taxed_at_purchase: false,
            tax_inclusive_quote: None,
            credit_unit: None,
            credit_grant_unit: None,
            credit_grant_rollover: None,
            promotional_credit: None,
            subscription_id: None,
            usage_ends_at: None,
            usage_breakdown: None,
        }
    }

    fn priced(lines: Vec<LineItem>) -> PricedInvoice {
        PricedInvoice::Lines(PricedLines {
            lines,
            invoice_date: d(2, 18),
            currency_precision: 2,
            tax_inclusive: false,
            invoice_type: InvoiceType::Recurring,
            prepaid_amount: None,
            invoice_id: None,
            true_up: None,
            reopened: Box::default(),
            span: BilledSpan::Period,
        })
    }

    fn classes(legacy: Vec<LineItem>, facts: Vec<LineItem>) -> Vec<&'static str> {
        document_differences("slot", &priced(legacy), &priced(facts))
            .into_iter()
            .map(|difference| difference.class)
            .collect()
    }

    /// 29.00 a month in arrears from Jan 18, amended on Jan 22 to 500.00 a year and on Jan 24
    /// back to 29.00 a month: the month's charge bills 2900 − 2526 + 2339 = 2713 on one line,
    /// beside the year's stub of 274; the legacy path bills 374 and 2339 around that stub.
    /// Rules: R-080, R-084
    #[test]
    fn a_charge_billed_by_one_line_around_another_cadences_stub_is_a_split_charge() {
        let stub = fee(d(1, 22), d(1, 24), 274);
        let legacy = vec![
            fee(d(1, 18), d(1, 22), 374),
            stub.clone(),
            fee(d(1, 24), d(2, 18), 2_339),
        ];
        let facts = vec![fee(d(1, 18), d(2, 18), 2_713), stub.clone()];
        assert_eq!(classes(legacy.clone(), facts), vec!["split charge"]);

        // A part the facts do not bill is no split: 374 + 2339 against 374.
        let dropped = vec![fee(d(1, 18), d(2, 18), 374), stub];
        assert!(classes(legacy, dropped).contains(&"line"));
    }

    /// The same around a fixed term ending on Feb 10, whose last part the legacy path bills
    /// whole (2900) where the facts prorate it: 374 + 1497 = 1871, under the term's end. Without
    /// a term ending there, or with a last part that is not the month's price, it is a line.
    /// Rules: R-080, R-084, R-002
    #[test]
    fn a_split_charge_cut_by_the_term_end_is_the_term_end() {
        let stub = fee(d(1, 22), d(1, 24), 274);
        let legacy = |last: i64| {
            vec![
                of_row(1, fee(d(1, 18), d(1, 22), 374)),
                stub.clone(),
                of_row(1, fee(d(1, 24), d(2, 10), last)),
            ]
        };
        let facts = vec![of_row(1, fee(d(1, 18), d(2, 10), 1_871)), stub.clone()];
        let cut = Context {
            term_end: Some(d(2, 10)),
            fees: vec![priced_at(1, 2_900)],
            lives: vec![life(1, 1, d(1, 18), None, 1)],
            ..on(d(2, 10))
        };
        assert_eq!(classes(legacy(2_900), facts.clone()), vec!["term end"]);
        assert_eq!(
            narrowed(legacy(2_900), facts.clone(), &cut),
            vec!["term end"]
        );

        let open = Context {
            term_end: None,
            ..cut.clone()
        };
        assert_eq!(narrowed(legacy(2_900), facts.clone(), &open), vec!["line"]);
        assert_eq!(narrowed(legacy(3_000), facts, &cut), vec!["line"]);
    }

    /// 49.99 a year from Jan 11, 2025 on a subscription renewing monthly, repriced to 123.45 a
    /// year from the Feb 9 renewal: Feb 9 bills 11297 − 4574 = 6723 of the year to Jan 9, 2026,
    /// which the legacy path bills on no invoice. A line of the same shape the rule does not
    /// explain is still a line: a fee added that day, a change of cadence, a whole period.
    /// Rules: R-080, R-078
    #[test]
    fn the_rest_of_a_year_repriced_at_a_monthly_renewal_is_a_repriced_rest() {
        let year_end = NaiveDate::from_ymd_opt(2026, 1, 9).unwrap();
        let life = |row: u128, from, to, months| FeeLife {
            row: uuid::Uuid::from_u128(row),
            lineage: uuid::Uuid::from_u128(1),
            from,
            to,
            months: Some(months),
            arrears: false,
        };
        let line = |from, until, amount| LineItem {
            sub_component_id: Some(uuid::Uuid::from_u128(2).into()),
            ..fee(from, until, amount)
        };
        let lives = [
            life(1, d(1, 11), Some(d(2, 9)), 12),
            life(2, d(2, 9), None, 12),
        ];
        let rest = line(d(2, 9), year_end, 6_723);
        assert!(bills_a_repriced_rest(&rest, d(2, 9), &lives));
        let detail = format!("Recurring 2025-02-09: facts only: {}", describe(&rest));
        assert!(skip_of("repriced rest", &detail).is_some());
        assert!(skip_of("line", &detail).is_none());

        let added = [life(2, d(2, 9), None, 12)];
        assert!(!bills_a_repriced_rest(&rest, d(2, 9), &added));
        let recadenced = [
            life(1, d(1, 11), Some(d(2, 9)), 1),
            life(2, d(2, 9), None, 12),
        ];
        assert!(!bills_a_repriced_rest(&rest, d(2, 9), &recadenced));
        let whole = line(
            d(2, 9),
            NaiveDate::from_ymd_opt(2026, 2, 9).unwrap(),
            12_345,
        );
        assert!(!bills_a_repriced_rest(&whole, d(2, 9), &lives));
        assert!(!bills_a_repriced_rest(&rest, d(3, 9), &lives));
    }

    /// 123.45 a month in arrears to Feb 7, then 29.00 a year in arrears, repriced to 9.99 a year
    /// from Mar 4: the yearly rows are one run, billed at the year's end. The legacy path bills
    /// the 29.00 row's 199 for Feb 7–Mar 4 on Mar 4's renewal, which the facts do not. A line
    /// of the same shape the rule does not explain stays a line: a row ended for good, replaced
    /// at another cadence or in advance, a whole period, or days the facts bill on the invoice.
    /// Rules: R-084, R-011
    #[test]
    fn the_days_of_a_yearly_row_in_arrears_repriced_in_its_run_are_repriced_arrears() {
        let life = |row: u128, from, to, months, arrears| FeeLife {
            row: uuid::Uuid::from_u128(row),
            lineage: uuid::Uuid::from_u128(1),
            from,
            to,
            months: Some(months),
            arrears,
        };
        let row = |id: u128, line: LineItem| LineItem {
            sub_component_id: Some(uuid::Uuid::from_u128(id).into()),
            ..line
        };
        let lives = [
            life(1, d(2, 4), Some(d(2, 7)), 1, true),
            life(2, d(2, 7), Some(d(3, 4)), 12, true),
            life(3, d(3, 4), None, 12, true),
        ];
        let month = row(1, fee(d(2, 4), d(2, 7), 1_323));
        let days = row(2, fee(d(2, 7), d(3, 4), 199));
        let facts = [month.clone()];
        assert!(bills_a_run_going_on(&days, d(3, 4), &lives, &facts));
        let detail = format!("Recurring 2025-03-04: legacy only: {}", describe(&days));
        assert!(skip_of("repriced arrears", &detail).is_some());
        assert!(skip_of("line", &detail).is_none());
        // The facts bill the month the monthly row ended: no run goes on past it.
        assert!(!bills_a_run_going_on(&month, d(3, 4), &lives, &facts));

        let ended = [lives[0], lives[1]];
        assert!(!bills_a_run_going_on(&days, d(3, 4), &ended, &facts));
        let monthly = [lives[0], lives[1], life(3, d(3, 4), None, 1, true)];
        assert!(!bills_a_run_going_on(&days, d(3, 4), &monthly, &facts));
        let in_advance = [lives[0], lives[1], life(3, d(3, 4), None, 12, false)];
        assert!(!bills_a_run_going_on(&days, d(3, 4), &in_advance, &facts));
        let not_arrears = [
            lives[0],
            life(2, d(2, 7), Some(d(3, 4)), 12, false),
            lives[2],
        ];
        assert!(!bills_a_run_going_on(&days, d(3, 4), &not_arrears, &facts));
        let whole = row(
            2,
            fee(d(2, 7), NaiveDate::from_ymd_opt(2026, 2, 7).unwrap(), 2_900),
        );
        let year = [
            lives[0],
            life(
                2,
                d(2, 7),
                Some(NaiveDate::from_ymd_opt(2026, 2, 7).unwrap()),
                12,
                true,
            ),
            life(
                3,
                NaiveDate::from_ymd_opt(2026, 2, 7).unwrap(),
                None,
                12,
                true,
            ),
        ];
        assert!(!bills_a_run_going_on(
            &whole,
            NaiveDate::from_ymd_opt(2026, 2, 7).unwrap(),
            &year,
            &facts
        ));
        let billed = [month.clone(), row(3, fee(d(2, 7), d(3, 4), 199))];
        assert!(!bills_a_run_going_on(&days, d(3, 4), &lives, &billed));
        assert!(!bills_a_run_going_on(&days, d(3, 3), &lives, &facts));
        let early = row(2, fee(d(2, 7), d(3, 1), 175));
        assert!(!bills_a_run_going_on(&early, d(3, 4), &lives, &facts));
    }

    /// A yearly 99.00 add-on, three from Jan 26 billed to Jan 23, 2026 (29456), set to one on
    /// Feb 4 after a change to a monthly plan: its run keeps its year (R-084), and Feb 23's
    /// renewal bills none of it. The legacy path bills the new row a year from Feb 23, 9900. A
    /// line of the same shape the rule does not explain stays a line: a row no earlier one of its
    /// cadence preceded, a period starting where the billed one ends, days the facts bill.
    /// Rules: R-084
    #[test]
    fn a_year_billed_again_from_a_renewal_inside_its_run_is_a_carried_run() {
        let year_end = NaiveDate::from_ymd_opt(2026, 1, 23).unwrap();
        let life = |row: u128, from, to, months| FeeLife {
            row: uuid::Uuid::from_u128(row),
            lineage: uuid::Uuid::from_u128(1),
            from,
            to,
            months: Some(months),
            arrears: false,
        };
        let row = |id: u128, line: LineItem| LineItem {
            sub_add_on_id: Some(uuid::Uuid::from_u128(id).into()),
            ..line
        };
        let lives = [
            life(1, d(1, 26), Some(d(2, 4)), 12),
            life(2, d(2, 4), None, 12),
        ];
        let stub = row(1, fee(d(1, 26), year_end, 29_456));
        let again = row(
            2,
            fee(
                d(2, 23),
                NaiveDate::from_ymd_opt(2026, 2, 23).unwrap(),
                9_900,
            ),
        );
        let issued = [stub.clone()];
        assert!(bills_inside_a_billed_run(&again, &lives, &issued, &[]));
        let detail = format!("Recurring 2025-02-23: legacy only: {}", describe(&again));
        assert!(skip_of("carried run", &detail).is_some());
        assert!(skip_of("line", &detail).is_none());

        let fresh = [life(2, d(2, 4), None, 12)];
        assert!(!bills_inside_a_billed_run(&again, &fresh, &issued, &[]));
        let monthly = [
            life(1, d(1, 26), Some(d(2, 4)), 1),
            life(2, d(2, 4), None, 12),
        ];
        assert!(!bills_inside_a_billed_run(&again, &monthly, &issued, &[]));
        let renewed = row(
            2,
            fee(
                year_end,
                NaiveDate::from_ymd_opt(2027, 1, 23).unwrap(),
                9_900,
            ),
        );
        assert!(!bills_inside_a_billed_run(&renewed, &lives, &issued, &[]));
        assert!(!bills_inside_a_billed_run(&again, &lives, &[], &[]));
        let billed = [row(2, fee(d(2, 23), year_end, 9_000))];
        assert!(!bills_inside_a_billed_run(&again, &lives, &issued, &billed));
    }

    fn units(name: &str, from: NaiveDate, until: NaiveDate, amount: i64) -> LineItem {
        LineItem {
            name: name.to_owned(),
            charge_type: Some(crate::domain::invoice_lines::ChargeType::Usage),
            is_prorated: false,
            ..fee(from, until, amount)
        }
    }

    /// The year's usage from Feb 10, 2025 closes it alone; the legacy path bills there too the
    /// arrears of a usage row a change ended on Feb 10, which the facts billed on that day's
    /// renewal. A usage line of the year the facts lack is still a line.
    /// Rules: R-011
    #[test]
    fn arrears_before_a_document_of_usage_alone_are_a_late_rider() {
        let next = NaiveDate::from_ymd_opt(2026, 2, 10).unwrap();
        let year = units("B units", d(2, 10), next, 1_920);
        let ridden = of_row(2, units("A units", d(1, 13), d(2, 10), 3_600));
        assert_eq!(
            classes(vec![ridden.clone(), year.clone()], vec![year.clone()]),
            vec!["late rider"]
        );

        let dropped = units("C units", d(2, 10), next, 500);
        assert_eq!(
            classes(
                vec![ridden.clone(), year.clone(), dropped],
                vec![year.clone()]
            ),
            vec!["late rider", "line"]
        );

        // Only arrears another document billed, the same lineage over the same days.
        let billed = Context {
            lives: vec![life(2, 2, d(1, 13), Some(d(2, 10)), 1)],
            others: vec![other(ridden.clone(), d(2, 10))],
            ..on(next)
        };
        assert_eq!(
            narrowed(
                vec![ridden.clone(), year.clone()],
                vec![year.clone()],
                &billed
            ),
            vec!["late rider"]
        );
        let voided = Context {
            others: vec![Other {
                issued: false,
                void: true,
                ..other(ridden.clone(), d(2, 10))
            }],
            ..billed.clone()
        };
        assert_eq!(
            narrowed(
                vec![ridden.clone(), year.clone()],
                vec![year.clone()],
                &voided
            ),
            vec!["late rider"]
        );
        let unbilled = Context {
            others: vec![],
            ..billed
        };
        assert_eq!(
            narrowed(vec![ridden, year.clone()], vec![year], &unbilled),
            vec!["line"]
        );
    }

    /// A draft after a backdated ending closes empty, the period's true-up on the final invoice:
    /// the legacy path's true-up there is skipped, a true-up the facts alone compute or a line is
    /// still a difference.
    /// Rules: R-016, R-022
    #[test]
    fn a_true_up_only_the_legacy_path_writes_on_a_draft_past_a_backdated_ending_is_skipped() {
        let minimum = SubscriptionMinimum {
            subscription_id: SubscriptionId::new(),
            amount_subunit: 5_000,
            scope: crate::domain::plan_minimums::MinimumScope::AllComponents,
            is_override: true,
        };
        let true_up = PeriodTrueUp {
            start: d(1, 5),
            end: d(2, 5),
            minimum,
            true_up: billing_core::minimum::TrueUp {
                start: d(1, 5),
                end: d(2, 5),
                floor: 5_000,
                scope: billing_core::minimum::Scope::AllComponents,
                cut: Some(d(2, 5)),
                prepaid: vec![],
                trued_up: 0,
                carried: vec![],
            },
            prior: vec![],
            up_front: None,
            provisional: vec![],
            queue: Default::default(),
        };
        let with = |true_up: Option<PeriodTrueUp>| match priced(vec![]) {
            PricedInvoice::Lines(lines) => PricedInvoice::Lines(PricedLines { true_up, ..lines }),
            stored => stored,
        };
        let detail = |legacy: Option<PeriodTrueUp>, facts: Option<PeriodTrueUp>| {
            document_differences("draft", &with(legacy), &with(facts))
                .into_iter()
                .map(|difference| difference.detail)
                .next()
                .unwrap()
        };
        let legacy_only = detail(Some(true_up.clone()), None);
        assert!(skip_of("backdated draft", &legacy_only).is_some());
        assert!(skip_of("line", &legacy_only).is_none());

        let facts_only = detail(None, Some(true_up));
        assert!(skip_of("backdated draft", &facts_only).is_none());

        let missing = "Recurring 2025-02-05 (draft x): facts only: A fee 2025-02-05..2025-03-05";
        assert!(skip_of("backdated draft", missing).is_none());
    }

    fn y(year: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, m, day).unwrap()
    }

    fn on(invoice_date: NaiveDate) -> Context {
        Context {
            invoice_date,
            ..Context::default()
        }
    }

    fn of_row(row: u128, line: LineItem) -> LineItem {
        LineItem {
            sub_component_id: Some(uuid::Uuid::from_u128(row).into()),
            ..line
        }
    }

    fn priced_at(row: u128, minor: i64) -> ShadowFee {
        ShadowFee {
            row: uuid::Uuid::from_u128(row),
            arrears: false,
            price: Some(Decimal::from(minor)),
            included: None,
            metric: None,
        }
    }

    fn life(
        row: u128,
        lineage: u128,
        from: NaiveDate,
        to: Option<NaiveDate>,
        months: u32,
    ) -> FeeLife {
        FeeLife {
            row: uuid::Uuid::from_u128(row),
            lineage: uuid::Uuid::from_u128(lineage),
            from,
            to,
            months: Some(months),
            arrears: false,
        }
    }

    fn other(line: LineItem, on: NaiveDate) -> Other {
        Other {
            line,
            kind: InvoiceType::Recurring,
            on,
            issued: true,
            void: false,
            credited: 0,
        }
    }

    fn narrowed(
        legacy: Vec<LineItem>,
        facts: Vec<LineItem>,
        context: &Context,
    ) -> Vec<&'static str> {
        let mut differences = document_differences("slot", &priced(legacy), &priced(facts));
        narrow("slot", &mut differences, context);
        differences.into_iter().map(|d| d.class).collect()
    }

    /// The legacy path, blind to the watermark of Feb 1, reads January's usage the facts settled
    /// before: the differences of that metric's lines in January are the watermark's, the others
    /// still differences, and a read nothing explains is a difference of its own.
    /// Rules: R-080, R-016, R-097
    #[test]
    fn only_the_lines_of_an_explained_unserved_read_are_skipped() {
        let metric = BillableMetricId::new();
        let usage = |from, until, amount| LineItem {
            metric_id: Some(metric),
            ..units("A units", from, until, amount)
        };
        let read = UnservedRead {
            metric: Some(metric),
            from: d(1, 1),
            to: d(2, 1),
            in_parts: false,
            tail_of_served: false,
        };
        let split = UnservedRead {
            in_parts: true,
            ..read.clone()
        };
        assert_eq!(explained_by(&split, &on(d(2, 1))), Some("unserved parts"));
        let at = |watermark, backdated_ending| Context {
            watermark,
            backdated_ending,
            ..on(d(2, 1))
        };
        assert_eq!(
            explained_by(&read, &at(Some(d(2, 1)), None)),
            Some("unserved watermark")
        );
        assert_eq!(explained_by(&read, &at(Some(d(1, 31)), None)), None);
        assert_eq!(
            explained_by(&read, &at(None, Some(d(1, 20)))),
            Some("unserved backdated")
        );
        assert_eq!(explained_by(&read, &at(None, Some(d(2, 1)))), None);
        // A draft from the ending the facts close empty reads nothing.
        let closed = Context {
            draft: true,
            facts_empty: true,
            ..at(None, Some(d(2, 1)))
        };
        assert_eq!(explained_by(&read, &closed), Some("unserved backdated"));
        // Another document billed the window.
        let billed = Context {
            others: vec![other(usage(d(1, 1), d(2, 1), 500), d(2, 1))],
            ..at(None, None)
        };
        assert_eq!(explained_by(&read, &billed), Some("unserved billed"));
        // Days a later line of another document bills too.
        let later = Context {
            others: vec![other(usage(d(1, 1), d(3, 1), 900), d(3, 1))],
            ..at(None, None)
        };
        assert_eq!(explained_by(&read, &later), Some("unserved billed"));
        let part = Context {
            others: vec![other(usage(d(1, 10), d(3, 1), 900), d(3, 1))],
            ..at(None, None)
        };
        assert_eq!(explained_by(&read, &part), None);
        // A row closed on Feb 1, read on Mar 1 after the renewal of Feb 1; not before it.
        let closed_row = Context {
            fees: vec![ShadowFee {
                metric: Some(metric),
                price: None,
                ..priced_at(1, 0)
            }],
            lives: vec![life(1, 1, d(1, 1), Some(d(2, 1)), 1)],
            slots: vec![d(1, 1), d(2, 1)],
            ..on(d(3, 1))
        };
        assert_eq!(explained_by(&read, &closed_row), Some("unserved settled"));
        let unread = Context {
            slots: vec![d(1, 1)],
            ..closed_row
        };
        assert_eq!(explained_by(&read, &unread), None);
        // A month read for a usage row billed yearly; a year's row read on its year is not.
        let fee = ShadowFee {
            metric: Some(metric),
            price: None,
            ..priced_at(1, 0)
        };
        let yearly = Context {
            fees: vec![fee.clone()],
            lives: vec![life(1, 1, d(1, 1), None, 12)],
            origins: vec![d(1, 1)],
            ..at(None, None)
        };
        assert_eq!(explained_by(&read, &yearly), Some("unserved own grid"));
        let monthly = Context {
            lives: vec![life(1, 1, d(1, 1), None, 1)],
            ..yearly.clone()
        };
        assert_eq!(explained_by(&read, &monthly), None);
        // A closed yearly row in arrears whose run a row of its cadence goes on.
        let cut = UnservedRead {
            from: d(1, 29),
            to: d(2, 2),
            ..read.clone()
        };
        let arrears = |row, from, to| FeeLife {
            arrears: true,
            ..life(row, 1, from, to, 12)
        };
        let run = Context {
            fees: vec![fee],
            lives: vec![
                arrears(1, d(1, 26), Some(d(2, 2))),
                arrears(2, d(2, 2), None),
            ],
            ..on(d(2, 26))
        };
        assert_eq!(explained_by(&cut, &run), Some("unserved run"));
        let ended = Context {
            lives: vec![arrears(1, d(1, 26), Some(d(2, 2)))],
            ..run
        };
        assert_eq!(explained_by(&cut, &ended), None);

        let settled = Context {
            unserved: vec![(read.clone(), Some("unserved watermark"))],
            ..on(d(2, 1))
        };
        assert_eq!(
            narrowed(vec![usage(d(1, 1), d(2, 1), 0)], vec![], &settled),
            vec!["unserved watermark"]
        );
        let february = usage(d(2, 1), d(3, 1), 500);
        assert_eq!(narrowed(vec![february], vec![], &settled), vec!["line"]);

        let unexplained = Context {
            unserved: vec![(read, None)],
            ..on(d(2, 1))
        };
        assert_eq!(
            narrowed(vec![usage(d(1, 1), d(2, 1), 0)], vec![], &unexplained),
            vec!["line", "unserved"]
        );
        assert_eq!(narrowed(vec![], vec![], &unexplained), vec!["unserved"]);
    }

    /// A legacy line ending on the watermark is a settled period only where another document
    /// billed its lineage over its days.
    /// Rules: R-080
    #[test]
    fn a_line_ending_on_the_watermark_is_settled_only_where_billed() {
        let line = of_row(1, fee(d(1, 1), d(2, 1), 2_900));
        let settled = Context {
            watermark: Some(d(2, 1)),
            lives: vec![life(1, 1, d(1, 1), None, 1)],
            others: vec![other(line.clone(), d(1, 1))],
            ..on(d(2, 1))
        };
        assert_eq!(
            narrowed(vec![line.clone()], vec![], &settled),
            vec!["watermark"]
        );
        let unbilled = Context {
            others: vec![],
            ..settled.clone()
        };
        assert_eq!(
            narrowed(vec![line.clone()], vec![], &unbilled),
            vec!["line"]
        );
        let later = Context {
            watermark: Some(d(1, 15)),
            ..settled
        };
        assert_eq!(narrowed(vec![line], vec![], &later), vec!["line"]);
    }

    /// 500.00 a month in arrears from Jan 7, cancelled on Feb 9 from Feb 7: the final invoice
    /// bills January's 50000, what the legacy path's recurring invoice of Feb 7 billed. Twice
    /// that, a January an issued invoice already billed, or a line past the ending is a line; so
    /// is a written day's final billing what its recurring invoice issued.
    /// Rules: R-016, R-102
    #[test]
    fn a_final_invoice_bills_what_the_ending_days_invoice_would_less_what_was_issued() {
        let january = of_row(1, fee(d(1, 7), d(2, 7), 50_000));
        let backdated = Context {
            final_invoice: true,
            ending: Some(d(2, 7)),
            backdated_ending: Some(d(2, 7)),
            lives: vec![life(1, 1, d(1, 7), Some(d(2, 7)), 1)],
            at_ending: Some((vec![january.clone()], None)),
            ..on(d(2, 9))
        };
        assert_eq!(
            narrowed(vec![], vec![january.clone()], &backdated),
            vec!["backdated final"]
        );
        let twice = of_row(1, fee(d(1, 7), d(2, 7), 100_000));
        assert_eq!(narrowed(vec![], vec![twice], &backdated), vec!["line"]);
        let issued = Context {
            others: vec![other(january.clone(), d(2, 7))],
            ..backdated.clone()
        };
        assert_eq!(
            narrowed(vec![], vec![january.clone()], &issued),
            vec!["line"]
        );
        let past = Context {
            ending: Some(d(2, 1)),
            ..backdated.clone()
        };
        assert_eq!(narrowed(vec![], vec![january.clone()], &past), vec!["line"]);

        let written = Context {
            backdated_ending: None,
            written_day_final: true,
            ..backdated.clone()
        };
        assert_eq!(
            narrowed(vec![], vec![january.clone()], &written),
            vec!["written day final"]
        );
        // A closed row's arrears ending before the ending ride the final invoice too; a prorated
        // part a minor unit apart is R-079's rounding, but not beside an issued line.
        let closed = of_row(1, fee(d(1, 7), d(2, 1), 40_323));
        let before = Context {
            at_ending: Some((vec![of_row(1, fee(d(1, 7), d(2, 1), 40_324))], None)),
            ..backdated.clone()
        };
        assert_eq!(
            narrowed(vec![], vec![closed.clone()], &before),
            vec!["backdated final"]
        );
        let apart = of_row(1, fee(d(1, 7), d(2, 1), 40_321));
        assert_eq!(narrowed(vec![], vec![apart], &before), vec!["line"]);
        let partly_issued = Context {
            others: vec![other(of_row(1, fee(d(1, 7), d(2, 1), 2)), d(1, 7))],
            ..before
        };
        assert_eq!(narrowed(vec![], vec![closed], &partly_issued), vec!["line"]);

        let written_issued = Context {
            others: vec![other(january.clone(), d(2, 7))],
            ..written
        };
        assert_eq!(
            narrowed(vec![], vec![january], &written_issued),
            vec!["line"]
        );
    }

    /// Cancelled on Feb 2 from Jan 25 after a change on Jan 6 to a yearly fee in arrears whose
    /// grid starts on Feb 2: the facts' final bills its stub over the year before that origin,
    /// 369 − 109 = 260, where the legacy path's invoice of the ending's day priced it over a
    /// longer period, 244. Cut from Feb 25 before an origin of Mar 2, it bills 329 − 68 = 261,
    /// where the share rounded once reads 260. 261 on the first, or a stub off the change's
    /// window, is a line.
    /// Rules: R-016, R-084
    #[test]
    fn a_backdated_finals_stub_is_priced_over_its_grids_period() {
        let stub = |amount| LineItem {
            quantity: Some(Decimal::ONE),
            is_prorated: true,
            ..of_row(1, fee(d(1, 6), d(1, 25), amount))
        };
        let backdated = Context {
            final_invoice: true,
            ending: Some(d(1, 25)),
            backdated_ending: Some(d(1, 25)),
            fees: vec![priced_at(1, 4_999)],
            lives: vec![FeeLife {
                arrears: true,
                ..life(1, 1, d(1, 6), None, 12)
            }],
            stubs: vec![(d(1, 6), d(2, 2))],
            at_ending: Some((vec![stub(244)], None)),
            ..on(d(2, 2))
        };
        assert_eq!(
            narrowed(vec![], vec![stub(260)], &backdated),
            vec!["backdated final"]
        );
        assert_eq!(narrowed(vec![], vec![stub(261)], &backdated), vec!["line"]);
        let off = Context {
            stubs: vec![],
            ..backdated.clone()
        };
        assert_eq!(narrowed(vec![], vec![stub(260)], &off), vec!["line"]);
        let cut = |amount| LineItem {
            quantity: Some(Decimal::ONE),
            is_prorated: true,
            ..of_row(1, fee(d(2, 6), d(2, 25), amount))
        };
        let before_origin = Context {
            ending: Some(d(2, 25)),
            backdated_ending: Some(d(2, 25)),
            lives: vec![FeeLife {
                arrears: true,
                ..life(1, 1, d(2, 6), None, 12)
            }],
            stubs: vec![(d(2, 6), d(3, 2))],
            at_ending: Some((vec![cut(244)], None)),
            ..backdated
        };
        assert_eq!(
            narrowed(vec![], vec![cut(261)], &before_origin),
            vec!["backdated final"]
        );
        assert_eq!(
            narrowed(vec![], vec![cut(260)], &before_origin),
            vec!["line"]
        );
    }

    /// 49.99 a month in arrears from Jan 13, amended to 9.99 a year on Jan 18 and to 9.99 a month
    /// on Jan 21, cancelled from Feb 3: the facts' final bills the monthly run by one line, 806 +
    /// 419 = 1225, beside the yearly row's 8; the legacy path's invoice of the ending's day bills
    /// 806, 8 and 419. 1226, or the yearly row's 8 missing from the final, is a line.
    /// Rules: R-016, R-084, R-080
    #[test]
    fn a_backdated_finals_run_the_legacy_path_split_around_another_cadence_is_the_same_charge() {
        let part = |row, from, until, amount| LineItem {
            quantity: Some(Decimal::ONE),
            is_prorated: true,
            ..of_row(row, fee(from, until, amount))
        };
        let yearly = part(2, d(1, 18), d(1, 21), 8);
        let run = |amount| part(3, d(1, 13), d(2, 3), amount);
        let backdated = Context {
            final_invoice: true,
            ending: Some(d(2, 3)),
            backdated_ending: Some(d(2, 3)),
            lives: vec![
                life(1, 1, d(1, 13), Some(d(1, 18)), 1),
                life(2, 1, d(1, 18), Some(d(1, 21)), 12),
                life(3, 1, d(1, 21), None, 1),
            ],
            facts_lines: vec![run(1_225), yearly.clone()],
            at_ending: Some((
                vec![
                    part(3, d(1, 21), d(2, 3), 419),
                    part(1, d(1, 13), d(1, 18), 806),
                    yearly.clone(),
                ],
                None,
            )),
            ..on(d(2, 13))
        };
        assert_eq!(
            narrowed(vec![], vec![run(1_225)], &backdated),
            vec!["backdated final"]
        );
        assert_eq!(narrowed(vec![], vec![run(1_226)], &backdated), vec!["line"]);
        let alone = Context {
            facts_lines: vec![run(1_225)],
            ..backdated
        };
        assert_eq!(narrowed(vec![], vec![run(1_225)], &alone), vec!["line"]);
    }

    /// A prorated line a minor unit apart is R-079's rounding whether or not one path names it
    /// with its dates; another base name, or two units apart, is a line.
    /// Rules: R-079
    #[test]
    fn a_tiling_difference_holds_across_a_date_range_in_the_name() {
        let named = |name: &str, amount| LineItem {
            name: name.to_owned(),
            is_prorated: true,
            quantity: Some(Decimal::ONE),
            ..of_row(1, fee(d(9, 3), d(10, 1), amount))
        };
        assert_eq!(
            classes(
                vec![named("B fee (2025-09-03 - 2025-10-01)", 383)],
                vec![named("B fee", 384)]
            ),
            vec!["tiling"]
        );
        assert_eq!(
            classes(
                vec![named("B fee (2025-09-03 - 2025-10-01)", 382)],
                vec![named("B fee", 384)]
            ),
            vec!["line"]
        );
        assert_eq!(
            classes(vec![named("A fee", 383)], vec![named("B fee", 384)]),
            vec!["line"]
        );
    }

    /// A draft past a backdated ending is closed empty: its legacy lines are skipped only when
    /// the facts write none on it.
    /// Rules: R-016
    #[test]
    fn a_draft_past_a_backdated_ending_is_skipped_only_when_closed_empty() {
        let february = of_row(1, fee(d(2, 1), d(3, 1), 2_900));
        let closed = Context {
            draft: true,
            facts_empty: true,
            backdated_ending: Some(d(1, 20)),
            ..on(d(2, 1))
        };
        assert_eq!(
            narrowed(vec![february.clone()], vec![], &closed),
            vec!["backdated draft"]
        );
        let kept = Context {
            facts_empty: false,
            ..closed.clone()
        };
        assert_eq!(
            narrowed(vec![february.clone()], vec![], &kept),
            vec!["line"]
        );
        let before = Context {
            backdated_ending: Some(d(2, 5)),
            ..closed
        };
        assert_eq!(narrowed(vec![february], vec![], &before), vec!["line"]);
    }

    /// Bandwidth amended on Feb 11 from monthly to yearly: its line from Feb 11 is the
    /// amendment's; another lineage's, or the same lineage's line a month later, is a line.
    /// Rules: R-084
    #[test]
    fn only_the_amended_lineage_next_to_the_amendment_is_its_cadence() {
        let amended = Context {
            amendment_days: vec![d(2, 11)],
            lives: vec![
                life(1, 1, d(1, 6), Some(d(2, 11)), 1),
                life(2, 1, d(2, 11), None, 12),
                life(3, 3, d(1, 6), None, 1),
            ],
            ..on(d(2, 15))
        };
        let line = of_row(2, units("Bandwidth", d(2, 11), d(2, 15), 200));
        assert_eq!(
            narrowed(vec![], vec![line], &amended),
            vec!["amendment cadence"]
        );
        let other_fee = of_row(3, units("Seats", d(2, 11), d(2, 15), 200));
        assert_eq!(narrowed(vec![], vec![other_fee], &amended), vec!["line"]);
        let later = of_row(2, units("Bandwidth", d(3, 11), d(3, 15), 200));
        assert_eq!(narrowed(vec![], vec![later], &amended), vec!["line"]);
    }

    /// A change of cadence on Jan 21 billed the month from Jan 21 on its adjustment: the
    /// renewal's legacy line of those days is skipped, the elapsed period's arrears are not.
    /// Rules: R-080
    #[test]
    fn only_what_the_adjustment_of_the_day_billed_is_adjusted() {
        let month = of_row(1, fee(d(1, 21), d(2, 21), 50_000));
        let adjusted = Context {
            facts_empty: true,
            lives: vec![life(1, 1, d(1, 21), None, 1)],
            others: vec![Other {
                kind: InvoiceType::Adjustment,
                ..other(month.clone(), d(1, 21))
            }],
            ..on(d(1, 21))
        };
        assert_eq!(
            narrowed(vec![month.clone()], vec![], &adjusted),
            vec!["adjusted"]
        );
        let arrears = of_row(1, fee(y(2024, 12, 21), d(1, 21), 2_900));
        assert_eq!(narrowed(vec![arrears], vec![], &adjusted), vec!["line"]);
        let recurring = Context {
            others: vec![other(month.clone(), d(1, 21))],
            ..adjusted
        };
        assert_eq!(narrowed(vec![month], vec![], &recurring), vec!["line"]);
    }

    /// A draft of Feb 1 finalized after Mar 1's renewal billed a row closed on Mar 1 up to its
    /// end: the facts' January of that row is a settled row; billed by another document too, or
    /// of a row still open, it is a line.
    /// Rules: R-011
    #[test]
    fn a_settled_row_is_a_closed_row_another_document_billed_to_its_end() {
        let line = of_row(1, fee(d(1, 1), d(2, 1), 2_900));
        let settled = Context {
            draft: true,
            lives: vec![life(1, 1, d(1, 1), Some(d(3, 1)), 1)],
            others: vec![other(of_row(1, fee(d(2, 1), d(3, 1), 2_900)), d(3, 1))],
            ..on(d(2, 1))
        };
        assert_eq!(
            narrowed(vec![], vec![line.clone()], &settled),
            vec!["settled row"]
        );
        let twice = Context {
            others: vec![
                other(of_row(1, fee(d(2, 1), d(3, 1), 2_900)), d(3, 1)),
                other(of_row(1, fee(d(1, 1), d(2, 1), 2_900)), d(1, 15)),
            ],
            ..settled.clone()
        };
        assert_eq!(narrowed(vec![], vec![line.clone()], &twice), vec!["line"]);
        let open = Context {
            lives: vec![life(1, 1, d(1, 1), None, 1)],
            ..settled
        };
        assert_eq!(narrowed(vec![], vec![line], &open), vec!["line"]);
    }

    /// A cadence switched on Jan 10, the day a row opened and closed: the lines of rows opened
    /// that day are skipped up to the window's first renewal, not after it.
    /// Rules: R-084
    #[test]
    fn an_unseen_switch_holds_only_until_its_first_renewal() {
        let line = of_row(1, fee(d(1, 10), d(2, 10), 2_900));
        let unseen = Context {
            unseen: vec![d(1, 10)],
            lives: vec![life(1, 1, d(1, 10), None, 1)],
            others: vec![other(of_row(9, fee(d(2, 10), d(3, 10), 1)), d(2, 10))],
            ..on(d(1, 10))
        };
        assert_eq!(
            narrowed(vec![line.clone()], vec![], &unseen),
            vec!["unseen switch"]
        );
        let renewed = Context {
            invoice_date: d(2, 10),
            ..unseen.clone()
        };
        assert_eq!(narrowed(vec![line.clone()], vec![], &renewed), vec!["line"]);
        let older = Context {
            lives: vec![life(1, 1, d(1, 1), None, 1)],
            ..unseen
        };
        assert_eq!(narrowed(vec![line], vec![], &older), vec!["line"]);
    }

    /// 9.99 a month, three days to the new grid's origin of Jan 30, 2026: the facts bill
    /// 999 × 3/31 = 97 over the month before the origin, the legacy path 999 × 3/35 = 86.
    /// Another amount on either side is a line.
    /// Rules: R-084
    #[test]
    fn a_stub_is_priced_over_the_grids_period_before_its_origin() {
        let line = |amount| of_row(1, fee(y(2026, 1, 27), y(2026, 1, 30), amount));
        let stub = Context {
            stubs: vec![(y(2026, 1, 27), y(2026, 1, 30))],
            fees: vec![priced_at(1, 999)],
            lives: vec![life(1, 1, y(2026, 1, 27), None, 1)],
            ..on(y(2026, 1, 30))
        };
        assert_eq!(
            narrowed(vec![line(86)], vec![line(97)], &stub),
            vec!["stub"]
        );
        assert_eq!(
            narrowed(vec![line(86)], vec![line(98)], &stub),
            vec!["line"]
        );
        assert_eq!(
            narrowed(vec![line(1_500)], vec![line(97)], &stub),
            vec!["line"]
        );
        // A yearly 49.99 from Feb 13 to its origin of Mar 19: 4999 × 34/365 = 466, the legacy
        // path the year whole.
        let year = |amount| of_row(2, fee(d(2, 13), d(3, 19), amount));
        let yearly = Context {
            stubs: vec![(d(2, 13), d(3, 19))],
            fees: vec![priced_at(2, 4_999)],
            lives: vec![life(2, 2, d(2, 13), None, 12)],
            ..on(d(3, 19))
        };
        assert_eq!(
            narrowed(vec![year(4_999)], vec![year(466)], &yearly),
            vec!["stub"]
        );
        assert_eq!(
            narrowed(vec![year(4_999)], vec![year(4_999)], &yearly),
            Vec::<&str>::new()
        );
        assert_eq!(
            narrowed(vec![year(4_999)], vec![year(500)], &yearly),
            vec!["line"]
        );
    }

    /// The repriced rest of the year is 11297 − 4574 = 6723; another amount is a line.
    /// Rules: R-080, R-078
    #[test]
    fn a_repriced_rest_bills_the_new_rest_less_the_old() {
        let year_end = y(2026, 1, 9);
        let rest = |amount| of_row(2, fee(d(2, 9), year_end, amount));
        let repriced = Context {
            fees: vec![priced_at(1, 4_999), priced_at(2, 12_345)],
            lives: vec![
                life(1, 1, d(1, 11), Some(d(2, 9)), 12),
                life(2, 1, d(2, 9), None, 12),
            ],
            ..on(d(2, 9))
        };
        assert_eq!(
            narrowed(vec![], vec![rest(6_723)], &repriced),
            vec!["repriced rest"]
        );
        assert_eq!(
            narrowed(vec![], vec![rest(11_297)], &repriced),
            vec!["line"]
        );
    }

    /// A matrix line of a lineage whose price a successor dates inside the document: the same
    /// usage at other rates is skipped, another quantity is a line.
    /// Rules: R-097
    #[test]
    fn a_dated_price_differs_only_in_its_rates() {
        let line = |quantity: i64, rate: Decimal, amount| LineItem {
            quantity: Some(Decimal::from(quantity)),
            unit_price: Some(rate),
            ..of_row(1, units("Model usage", d(1, 1), d(2, 1), amount))
        };
        let dated = Context {
            dated: [uuid::Uuid::from_u128(7)].into_iter().collect(),
            roots: [(uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(7))]
                .into_iter()
                .collect(),
            ..on(d(2, 1))
        };
        assert_eq!(
            narrowed(
                vec![line(100, Decimal::new(10, 2), 1_000)],
                vec![line(100, Decimal::new(20, 2), 2_000)],
                &dated
            ),
            vec!["dated price"]
        );
        assert_eq!(
            narrowed(
                vec![line(100, Decimal::new(10, 2), 1_000)],
                vec![line(200, Decimal::new(10, 2), 2_000)],
                &dated
            ),
            vec!["line"]
        );
    }

    /// Cancelled now on Feb 18 at 15:01, the facts read a yearly row in arrears from Jan 19 to the
    /// instant, the period the ending cuts; the legacy path reads Feb 18 alone. A read of the
    /// end of a window the facts read, ending at the ending, is skipped with the lines reading
    /// it. One ending at another instant or before the window's end, or on a day other than the
    /// ending's, is a difference.
    /// Rules: R-016
    #[test]
    fn the_ending_day_of_a_period_the_facts_read_to_the_ending_is_the_cut() {
        use crate::domain::UsagePeriod;
        use crate::services::served_usage::is_tail;
        let at = |m, day, h, min| d(m, day).and_hms_opt(h, min, 0).unwrap();
        let period = |start, end| UsagePeriod { start, end };
        let cut = period(at(1, 19, 0, 0), at(2, 18, 15, 1));
        assert!(is_tail(&period(at(2, 18, 0, 0), at(2, 18, 15, 1)), &cut));
        assert!(!is_tail(&period(at(2, 18, 0, 0), at(2, 18, 16, 1)), &cut));
        assert!(!is_tail(&period(at(2, 18, 0, 0), at(2, 18, 0, 0)), &cut));
        assert!(!is_tail(&cut, &cut));

        let metric = BillableMetricId::new();
        let used = |amount| LineItem {
            metric_id: Some(metric),
            ..units("A units", d(1, 19), d(2, 18), amount)
        };
        let read = UnservedRead {
            metric: Some(metric),
            from: d(2, 18),
            to: d(2, 18),
            in_parts: false,
            tail_of_served: true,
        };
        let ending = Context {
            ending: Some(d(2, 18)),
            ..on(d(2, 18))
        };
        assert_eq!(explained_by(&read, &ending), Some("unserved cut"));
        let explained = Context {
            unserved: vec![(read.clone(), explained_by(&read, &ending))],
            ..ending.clone()
        };
        assert_eq!(
            narrowed(vec![used(0)], vec![used(450)], &explained),
            vec!["unserved cut"]
        );
        let elsewhere = Context {
            ending: Some(d(3, 18)),
            ..ending.clone()
        };
        assert_eq!(explained_by(&read, &elsewhere), None);
        let unflagged = UnservedRead {
            tail_of_served: false,
            ..read
        };
        assert_eq!(explained_by(&unflagged, &ending), None);
        let reported = Context {
            unserved: vec![(unflagged, None)],
            ..ending
        };
        assert_eq!(
            narrowed(vec![used(0)], vec![used(450)], &reported),
            vec!["line", "unserved"]
        );
    }

    /// A fixed term from Jan 1 to Jan 17, 2024: 31.00 in arrears bills 3100 × 16/31 = 1600 where
    /// the legacy path billed 3100. Another amount, or a legacy amount not the whole month's,
    /// is a line.
    /// Rules: R-084, R-002
    #[test]
    fn a_term_end_cuts_its_period_by_its_days() {
        let line = |amount, prorated| LineItem {
            is_prorated: prorated,
            quantity: Some(Decimal::ONE),
            ..of_row(1, fee(y(2024, 1, 1), y(2024, 1, 17), amount))
        };
        let term = Context {
            term_end: Some(y(2024, 1, 17)),
            billing_start: y(2024, 1, 1),
            fees: vec![priced_at(1, 3_100)],
            lives: vec![life(1, 1, y(2024, 1, 1), Some(y(2024, 1, 17)), 1)],
            ..on(y(2024, 1, 17))
        };
        assert_eq!(
            narrowed(vec![line(3_100, false)], vec![line(1_600, true)], &term),
            vec!["term end"]
        );
        assert_eq!(
            narrowed(vec![line(3_100, false)], vec![line(1_700, true)], &term),
            vec!["line"]
        );
        let dearer = Context {
            fees: vec![priced_at(1, 3_000)],
            ..term
        };
        assert_eq!(
            narrowed(vec![line(3_100, false)], vec![line(1_600, true)], &dearer),
            vec!["line"]
        );
    }

    /// A fixed term from Feb 19 to Mar 5, 2025: 49.99 a month in arrears bills what is left of
    /// 4999 on Feb 19 less what is left on Mar 5, 4999 − 2500 = 2499, where the share rounded
    /// once reads 2499.5, 2500. The share's 2500 is a line.
    /// Rules: R-084, R-079
    #[test]
    fn a_term_end_at_a_half_bills_the_period_less_what_is_left() {
        let line = |amount, prorated| LineItem {
            is_prorated: prorated,
            quantity: Some(Decimal::ONE),
            ..of_row(1, fee(y(2025, 2, 19), y(2025, 3, 5), amount))
        };
        let term = Context {
            term_end: Some(y(2025, 3, 5)),
            billing_start: y(2025, 1, 19),
            fees: vec![priced_at(1, 4_999)],
            lives: vec![life(1, 1, y(2025, 1, 19), Some(y(2025, 3, 5)), 1)],
            ..on(y(2025, 3, 5))
        };
        assert_eq!(
            narrowed(vec![line(4_999, false)], vec![line(2_499, true)], &term),
            vec!["term end"]
        );
        assert_eq!(
            narrowed(vec![line(4_999, false)], vec![line(2_500, true)], &term),
            vec!["line"]
        );
    }

    /// Started Jan 10 on anchor 1, 31.00 bills its first period 3100 × 22/31 = 2200. The same on
    /// a full first period, or another amount, is a line.
    /// Rules: R-002
    #[test]
    fn a_first_partial_period_prorates_from_the_billing_start() {
        let line = |amount, prorated| LineItem {
            is_prorated: prorated,
            quantity: Some(Decimal::ONE),
            ..of_row(1, fee(y(2024, 1, 10), y(2024, 2, 1), amount))
        };
        let first = Context {
            billing_start: y(2024, 1, 10),
            fees: vec![priced_at(1, 3_100)],
            lives: vec![life(1, 1, y(2024, 1, 10), None, 1)],
            ..on(y(2024, 2, 1))
        };
        assert_eq!(
            narrowed(vec![line(3_100, false)], vec![line(2_200, true)], &first),
            vec!["first partial capacity"]
        );
        assert_eq!(
            narrowed(vec![line(3_100, false)], vec![line(2_300, true)], &first),
            vec!["line"]
        );
        let later = Context {
            billing_start: y(2024, 1, 1),
            ..first
        };
        assert_eq!(
            narrowed(vec![line(3_100, false)], vec![line(2_200, true)], &later),
            vec!["line"]
        );
    }

    /// 310 units included from Jan 10 on anchor 1 are 220 for the first period: 240 used owe 20
    /// at 0.10, 200, where the legacy path counted 310 and billed 0; 400 used owe 180 against 90.
    /// More overage than the prorated allowance leaves is a line.
    /// Rules: R-002
    #[test]
    fn a_capacitys_allowance_is_prorated_with_its_fee() {
        let metric = BillableMetricId::new();
        let line = |over: i64| {
            let rate = Decimal::new(10, 2);
            let quantity = Decimal::from(over);
            LineItem {
                metric_id: Some(metric),
                sub_lines: vec![crate::domain::SubLineItem {
                    local_id: String::new(),
                    name: "Overage".to_owned(),
                    total: over * 10,
                    quantity,
                    unit_price: rate,
                    attributes: None,
                }],
                ..of_row(
                    1,
                    units("Bandwidth", y(2024, 1, 10), y(2024, 2, 1), over * 10),
                )
            }
        };
        let allowance = Context {
            fees: vec![ShadowFee {
                included: Some(Decimal::from(310)),
                ..priced_at(1, 3_100)
            }],
            lives: vec![life(1, 1, y(2024, 1, 10), None, 1)],
            precision: 2,
            ..on(y(2024, 2, 1))
        };
        assert_eq!(
            narrowed(vec![line(0)], vec![line(20)], &allowance),
            vec!["capacity allowance"]
        );
        assert_eq!(
            narrowed(vec![line(90)], vec![line(180)], &allowance),
            vec!["capacity allowance"]
        );
        assert_eq!(
            narrowed(vec![line(0)], vec![line(100)], &allowance),
            vec!["line"]
        );
        assert_eq!(
            narrowed(vec![line(90)], vec![line(190)], &allowance),
            vec!["line"]
        );
    }

    /// 100 units a year included from Jan 14, 2025, 71 used, cancelled on Jan 16, 2026 from Aug 8:
    /// the backdated final bills the overage alone, 71 − 100 × 206/365 at 0.25, 364, where the
    /// legacy path counted the 100 whole and billed nothing, its renewal's line of the year at 0.
    /// 365, usage of those days another document billed, or an invoice other than the final, is
    /// a line.
    /// Rules: R-002, R-016
    #[test]
    fn a_backdated_finals_overage_of_a_cut_allowance_is_the_capacitys() {
        let metric = BillableMetricId::new();
        let (from, ending) = (y(2025, 1, 14), y(2025, 8, 8));
        let over = Decimal::from(71) - Decimal::from(100) * Decimal::from(206) / Decimal::from(365);
        let line = |total: i64| LineItem {
            metric_id: Some(metric),
            sub_lines: vec![crate::domain::SubLineItem {
                local_id: String::new(),
                name: "Overage".to_owned(),
                total,
                quantity: over,
                unit_price: Decimal::new(25, 2),
                attributes: None,
            }],
            ..of_row(1, units("A capacity", from, ending, total))
        };
        let cut = Context {
            final_invoice: true,
            backdated_ending: Some(ending),
            ending: Some(ending),
            fees: vec![ShadowFee {
                included: Some(Decimal::from(100)),
                ..priced_at(1, 9_900)
            }],
            lives: vec![life(1, 1, from, Some(ending), 12)],
            precision: 2,
            ..on(y(2026, 1, 16))
        };
        assert_eq!(
            narrowed(vec![], vec![line(364)], &cut),
            vec!["capacity allowance"]
        );
        assert_eq!(narrowed(vec![], vec![line(365)], &cut), vec!["line"]);
        let whole = LineItem {
            metric_id: Some(metric),
            ..of_row(1, units("A capacity", from, y(2026, 1, 14), 0))
        };
        let fee = Context {
            others: vec![
                other(of_row(1, fee(from, y(2026, 1, 14), 9_900)), from),
                other(whole, y(2026, 1, 14)),
            ],
            ..cut.clone()
        };
        assert_eq!(
            narrowed(vec![], vec![line(364)], &fee),
            vec!["capacity allowance"]
        );
        let billed = Context {
            others: vec![other(line(364), y(2025, 8, 8))],
            ..cut.clone()
        };
        assert_eq!(narrowed(vec![], vec![line(364)], &billed), vec!["line"]);
        let recurring = Context {
            final_invoice: false,
            ..cut
        };
        assert_eq!(narrowed(vec![], vec![line(364)], &recurring), vec!["line"]);
    }

    /// A cancellation backdated into billed periods: its invoice's legacy line is skipped only
    /// where an issued line of its lineage billed its days.
    /// Rules: R-077
    #[test]
    fn a_backdated_cancellations_invoice_skips_only_days_already_billed() {
        let month = of_row(1, fee(d(2, 1), d(3, 1), 2_900));
        let billed = Context {
            cancelled_here: true,
            lives: vec![life(1, 1, d(1, 1), Some(d(2, 15)), 1)],
            others: vec![other(month.clone(), d(2, 1))],
            ..on(d(2, 1))
        };
        assert_eq!(
            narrowed(vec![month.clone()], vec![], &billed),
            vec!["backdated"]
        );
        let draft = Context {
            others: vec![Other {
                issued: false,
                ..other(month.clone(), d(2, 1))
            }],
            ..billed.clone()
        };
        assert_eq!(narrowed(vec![month.clone()], vec![], &draft), vec!["line"]);
        let unbilled = Context {
            others: vec![],
            ..billed
        };
        assert_eq!(narrowed(vec![month], vec![], &unbilled), vec!["line"]);
    }

    /// Arrears of a row ended off its grid on Feb 6 ride the next renewal; the same line where
    /// another document billed some of those days, or of a row billed in advance, is a line.
    /// Rules: R-011
    #[test]
    fn a_rider_is_arrears_no_other_document_billed() {
        let ridden = of_row(1, fee(d(1, 20), d(2, 6), 1_600));
        let next = of_row(2, fee(d(2, 20), d(3, 20), 2_900));
        let arrears = |arrears| FeeLife {
            arrears,
            ..life(1, 1, d(1, 6), Some(d(2, 6)), 1)
        };
        let rider = Context {
            lives: vec![arrears(true), life(2, 2, d(2, 6), None, 1)],
            ..on(d(2, 20))
        };
        assert_eq!(
            narrowed(
                vec![next.clone()],
                vec![ridden.clone(), next.clone()],
                &rider
            ),
            vec!["rider"]
        );
        let billed = Context {
            others: vec![other(of_row(1, fee(d(1, 20), d(2, 20), 2_900)), d(1, 20))],
            ..rider.clone()
        };
        assert_eq!(
            narrowed(
                vec![next.clone()],
                vec![ridden.clone(), next.clone()],
                &billed
            ),
            vec!["line"]
        );
        let advance = Context {
            lives: vec![arrears(false), life(2, 2, d(2, 6), None, 1)],
            ..rider
        };
        assert_eq!(
            narrowed(vec![next.clone()], vec![ridden, next], &advance),
            vec!["line"]
        );
    }

    /// A one-time fee the legacy path bills again is skipped only where another document billed
    /// it; one the facts alone bill, added in a trial before the day it bills, only where no
    /// other document did.
    /// Rules: R-075, R-093
    #[test]
    fn a_one_time_fee_bills_once() {
        let setup = LineItem {
            charge_type: Some(crate::domain::invoice_lines::ChargeType::OneTime),
            ..of_row(1, fee(d(1, 15), d(1, 15), 20_000))
        };
        let billed = Context {
            lives: vec![FeeLife {
                months: None,
                ..life(1, 1, d(1, 5), None, 1)
            }],
            others: vec![other(setup.clone(), d(1, 15))],
            ..on(d(1, 15))
        };
        assert_eq!(
            narrowed(vec![setup.clone()], vec![], &billed),
            vec!["legacy one-time"]
        );
        let unbilled = Context {
            others: vec![],
            ..billed.clone()
        };
        assert_eq!(
            narrowed(vec![setup.clone()], vec![], &unbilled),
            vec!["line"]
        );

        assert_eq!(
            narrowed(vec![], vec![setup.clone()], &unbilled),
            vec!["deferred one-time"]
        );
        assert_eq!(narrowed(vec![], vec![setup.clone()], &billed), vec!["line"]);
        let same_day = Context {
            lives: vec![FeeLife {
                months: None,
                ..life(1, 1, d(1, 15), None, 1)
            }],
            ..unbilled
        };
        assert_eq!(narrowed(vec![], vec![setup], &same_day), vec!["line"]);
    }

    /// 99.00 added on Jan 8 in a free trial to Feb 5, billed on the renewal of Feb 5, then a
    /// cancellation decided that day from Jan 25: the renewal's line, past the ending, is given
    /// back whole, and the final invoice of Feb 5 bills the fee on Jan 25, the day the ending
    /// closed the trial. A line of it on or before that day credited in part or not at all, the
    /// ending after the trial, the row ended before that day or an invoice other than the final,
    /// it is a line.
    /// Rules: R-093, R-016
    #[test]
    fn a_one_time_fee_of_a_trial_a_backdated_ending_closed_bills_on_its_final() {
        let setup = |on: NaiveDate| LineItem {
            charge_type: Some(crate::domain::invoice_lines::ChargeType::OneTime),
            ..of_row(1, fee(on, on, 9_900))
        };
        let given_back = Other {
            credited: 9_900,
            ..other(setup(d(2, 5)), d(2, 5))
        };
        let closed = Context {
            final_invoice: true,
            backdated_ending: Some(d(1, 25)),
            ending: Some(d(1, 25)),
            trial_end: Some(d(2, 5)),
            lives: vec![FeeLife {
                months: None,
                ..life(1, 1, d(1, 8), Some(d(1, 25)), 1)
            }],
            others: vec![given_back.clone()],
            ..on(d(2, 5))
        };
        let classes = |context: &Context| narrowed(vec![], vec![setup(d(1, 25))], context);
        assert_eq!(classes(&closed), vec!["deferred one-time"]);
        let unnoted = Context {
            others: vec![other(setup(d(2, 5)), d(2, 5))],
            ..closed.clone()
        };
        assert_eq!(classes(&unnoted), vec!["deferred one-time"]);
        let draft = Context {
            others: vec![Other {
                issued: false,
                ..other(setup(d(2, 5)), d(2, 5))
            }],
            ..closed.clone()
        };
        assert_eq!(classes(&draft), vec!["line"]);
        let kept = |credited| Context {
            others: vec![Other {
                credited,
                ..other(setup(d(1, 25)), d(1, 25))
            }],
            ..closed.clone()
        };
        assert_eq!(classes(&kept(9_900)), vec!["deferred one-time"]);
        assert_eq!(classes(&kept(0)), vec!["line"]);
        assert_eq!(classes(&kept(9_899)), vec!["line"]);
        let after_the_trial = Context {
            trial_end: Some(d(1, 20)),
            ..closed.clone()
        };
        assert_eq!(classes(&after_the_trial), vec!["line"]);
        let no_trial = Context {
            trial_end: None,
            ..closed.clone()
        };
        assert_eq!(classes(&no_trial), vec!["line"]);
        let ended_before = Context {
            lives: vec![FeeLife {
                months: None,
                ..life(1, 1, d(1, 8), Some(d(1, 20)), 1)
            }],
            ..closed.clone()
        };
        assert_eq!(classes(&ended_before), vec!["line"]);
        let recurring = Context {
            final_invoice: false,
            ..closed
        };
        assert_eq!(classes(&recurring), vec!["line"]);
    }

    /// 500.00 a year in arrears, its 1-day stub Feb 10–11 billed 137 on the renewal of Mar 10,
    /// which is voided: a later invoice billing the stub again is the legacy path's. The void's
    /// slot billed again by a live invoice, a voided line for less, or no void, it is a line.
    /// Rules: R-096
    #[test]
    fn a_charge_a_void_forgave_billed_again_is_the_legacy_paths() {
        let stub = of_row(1, fee(y(2026, 2, 10), y(2026, 2, 11), 137));
        let voided = Other {
            issued: false,
            void: true,
            ..other(stub.clone(), y(2026, 3, 10))
        };
        let forgiven = Context {
            lives: vec![life(1, 1, y(2025, 2, 10), Some(y(2026, 2, 11)), 12)],
            others: vec![voided.clone()],
            ..on(y(2026, 5, 14))
        };
        assert_eq!(
            narrowed(vec![stub.clone()], vec![], &forgiven),
            vec!["voided"]
        );
        let regenerated = Context {
            others: vec![
                voided.clone(),
                other(
                    of_row(2, fee(y(2026, 3, 10), y(2026, 4, 10), 999)),
                    y(2026, 3, 10),
                ),
            ],
            ..forgiven.clone()
        };
        assert_eq!(
            narrowed(vec![stub.clone()], vec![], &regenerated),
            vec!["line"]
        );
        let less = Context {
            others: vec![Other {
                line: of_row(1, fee(y(2026, 2, 10), y(2026, 2, 11), 136)),
                ..voided
            }],
            ..forgiven.clone()
        };
        assert_eq!(narrowed(vec![stub.clone()], vec![], &less), vec!["line"]);
        let live = Context {
            others: vec![other(stub.clone(), y(2026, 3, 10))],
            ..forgiven
        };
        assert_eq!(narrowed(vec![stub], vec![], &live), vec!["line"]);
    }

    /// 500.00 a month in arrears from Jan 9, 2025 on a grid from Jan 5, its row closed by an
    /// amendment on Jan 14, then a cancellation decided that day from Jan 12: the final invoice
    /// bills 43548 − 38710 = 4838 for Jan 9–12. Another amount, a row ended at the ending or
    /// open, days another document billed, or an invoice other than the final, is a line.
    /// Rules: R-016, R-011, R-079
    #[test]
    fn arrears_a_backdated_ending_cuts_of_a_row_closed_after_it_bill_on_its_final() {
        let cut = |amount| of_row(1, fee(y(2025, 1, 9), y(2025, 1, 12), amount));
        let arrears = |to| FeeLife {
            arrears: true,
            ..life(1, 1, y(2025, 1, 9), to, 1)
        };
        let closed = Context {
            final_invoice: true,
            backdated_ending: Some(y(2025, 1, 12)),
            ending: Some(y(2025, 1, 12)),
            billing_start: y(2025, 1, 5),
            origins: vec![y(2025, 1, 5)],
            fees: vec![priced_at(1, 50_000)],
            lives: vec![arrears(Some(y(2025, 1, 14)))],
            ..on(y(2025, 1, 14))
        };
        let classes = |amount, context: &Context| narrowed(vec![], vec![cut(amount)], context);
        assert_eq!(classes(4_838, &closed), vec!["backdated arrears"]);
        assert_eq!(classes(4_839, &closed), vec!["line"]);
        for to in [Some(y(2025, 1, 12)), None] {
            let lived = Context {
                lives: vec![arrears(to)],
                ..closed.clone()
            };
            assert_eq!(classes(4_838, &lived), vec!["line"]);
        }
        let billed = Context {
            others: vec![other(
                of_row(1, fee(y(2025, 1, 5), y(2025, 2, 5), 50_000)),
                y(2025, 2, 5),
            )],
            ..closed.clone()
        };
        assert_eq!(classes(4_838, &billed), vec!["line"]);
        let recurring = Context {
            final_invoice: false,
            ..closed
        };
        assert_eq!(classes(4_838, &recurring), vec!["line"]);
    }

    /// A monthly 123.45 on a subscription renewing yearly: the renewal of May 24, 2026 bills the
    /// months no recurring invoice was written for, each whole. A month another document billed,
    /// one with its own recurring invoice, or a part of a month is a line.
    /// Rules: R-084
    #[test]
    fn a_renewal_catches_up_the_whole_periods_no_invoice_was_written_for() {
        let month = of_row(1, fee(y(2025, 6, 24), y(2025, 7, 24), 12_345));
        let caught = Context {
            fees: vec![priced_at(1, 12_345)],
            lives: vec![life(1, 1, y(2025, 5, 3), None, 1)],
            renewals: vec![y(2025, 5, 24)],
            ..on(y(2026, 5, 24))
        };
        assert_eq!(
            narrowed(vec![], vec![month.clone()], &caught),
            vec!["caught up"]
        );
        let billed = Context {
            others: vec![other(
                of_row(1, fee(y(2025, 7, 1), y(2025, 7, 24), 9_000)),
                y(2025, 7, 1),
            )],
            ..caught.clone()
        };
        assert_eq!(narrowed(vec![], vec![month.clone()], &billed), vec!["line"]);
        let renewed = Context {
            renewals: vec![y(2025, 5, 24), y(2025, 6, 24)],
            ..caught.clone()
        };
        assert_eq!(narrowed(vec![], vec![month], &renewed), vec!["line"]);
        let part = of_row(1, fee(y(2025, 6, 24), y(2025, 7, 10), 6_371));
        assert_eq!(narrowed(vec![], vec![part], &caught), vec!["line"]);
    }

    /// Yearly seats billed Jan 12, 2026 to Jan 12, 2027, carried into a plan renewing Feb 12: the
    /// run bills Jan 12, 2027 to Jan 12, 2028 where the legacy path bills from Feb 12. Another
    /// amount, or seats no earlier line of the run ends at, is a line.
    /// Rules: R-084
    #[test]
    fn a_carried_run_bills_its_own_grid() {
        let seats = |from: NaiveDate, amount| {
            of_row(
                2,
                fee(
                    from,
                    from.checked_add_months(chrono::Months::new(12)).unwrap(),
                    amount,
                ),
            )
        };
        let run = Context {
            lives: vec![
                life(1, 1, y(2026, 1, 12), Some(y(2026, 2, 3)), 12),
                life(2, 1, y(2026, 2, 3), None, 12),
            ],
            others: vec![other(
                of_row(1, fee(y(2026, 1, 12), y(2027, 1, 12), 500)),
                y(2026, 1, 12),
            )],
            ..on(y(2027, 2, 12))
        };
        assert_eq!(
            narrowed(
                vec![seats(y(2027, 2, 12), 500)],
                vec![seats(y(2027, 1, 12), 500)],
                &run
            ),
            vec!["run grid"]
        );
        assert_eq!(
            narrowed(
                vec![seats(y(2027, 2, 12), 500)],
                vec![seats(y(2027, 1, 12), 600)],
                &run
            ),
            vec!["line"]
        );
        let unbilled = Context {
            others: vec![],
            ..run
        };
        assert_eq!(
            narrowed(
                vec![seats(y(2027, 2, 12), 500)],
                vec![seats(y(2027, 1, 12), 500)],
                &unbilled
            ),
            vec!["line"]
        );
    }
}
