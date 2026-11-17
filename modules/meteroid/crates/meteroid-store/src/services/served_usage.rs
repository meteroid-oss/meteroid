//! Metering read before a transaction takes its row locks. A pass is served the reads made before
//! it and records the others (answered empty); one that recorded a read never commits: the reads run
//! unlocked and the next pass, re-reading every row under its locks, is served them by exact query.
//! The last pass, or the one after a read failed, fails the reads it is not served, so the work
//! handles them where it handles a failing metering read.

use crate::StoreResult;
use crate::domain::{BillableMetric, UsagePeriod, UsageScope};
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::clients::usage::{
    EventSearchOptions, EventSearchResult, IngestEventsRequest, IngestEventsResult, UsageClient,
    UsageData, WindowedUsageData,
};
use crate::store::PgConn;
use chrono::NaiveDateTime;
use common_domain::ids::{BillableMetricId, CustomerId, TenantId};
use error_stack::Report;
use rust_decimal::Decimal;
use scoped_futures::{ScopedBoxFuture, ScopedFutureExt};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The passes a transaction reading metering took, by its call site: work whose usage is planned
/// before the transaction takes one.
static USAGE_PASSES: std::sync::LazyLock<opentelemetry::metrics::Counter<u64>> =
    std::sync::LazyLock::new(|| {
        common_logging::GLOBAL_METER
            .u64_counter("billing_usage_passes")
            .with_description("Transactions reading metering, by call site and passes taken")
            .build()
    });

fn record_passes(entry: &std::panic::Location<'static>, passes: usize) {
    USAGE_PASSES.add(
        1,
        &[
            common_logging::KeyValue::new("entry", format!("{}:{}", entry.file(), entry.line())),
            common_logging::KeyValue::new("passes", passes as i64),
        ],
    );
}

/// Passes before giving up on work that keeps asking for metering it has not read.
const MAX_USAGE_PASSES: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct UsageRead {
    tenant_id: TenantId,
    customer_id: CustomerId,
    metric_id: BillableMetricId,
    metric_updated_at: Option<NaiveDateTime>,
    scope: UsageScope,
    period: UsagePeriod,
    group_by: Option<String>,
}

impl UsageRead {
    fn of(
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: &UsagePeriod,
        group_by: Option<&str>,
    ) -> Self {
        Self {
            tenant_id: *tenant_id,
            customer_id: *customer_id,
            metric_id: metric.id,
            metric_updated_at: metric.updated_at,
            scope: metric.usage_scope.clone(),
            period: period.clone(),
            group_by: group_by.map(str::to_owned),
        }
    }
}

enum Missed {
    Usage(UsageRead, Box<BillableMetric>),
    Events(TenantId, EventSearchOptions),
}

/// The metering reads made so far, by query.
#[derive(Default, Clone)]
pub(crate) struct UsageReads {
    usage: HashMap<UsageRead, UsageData>,
    events: Vec<(TenantId, EventSearchOptions, EventSearchResult)>,
}

impl UsageReads {
    async fn read(&mut self, client: &dyn UsageClient, missed: Vec<Missed>) -> StoreResult<()> {
        for missed in missed {
            match missed {
                Missed::Usage(read, metric) => {
                    if self.usage.contains_key(&read) {
                        continue;
                    }
                    let data = match &read.group_by {
                        None => {
                            client
                                .fetch_usage(
                                    &read.tenant_id,
                                    &read.customer_id,
                                    &metric,
                                    read.period.clone(),
                                )
                                .await?
                        }
                        Some(group_by) => {
                            client
                                .fetch_usage_grouped_by(
                                    &read.tenant_id,
                                    &read.customer_id,
                                    &metric,
                                    group_by,
                                    read.period.clone(),
                                )
                                .await?
                        }
                    };
                    self.usage.insert(read, data);
                }
                Missed::Events(tenant_id, mut options) => {
                    // An annex pages through the events: the pages after the one missed are read
                    // with it, else each pass would discover one more page.
                    while self.events_of(&tenant_id, &options).is_none() {
                        let found = client.search_events(&tenant_id, options.clone()).await?;
                        let last = options.limit == 0
                            || (found.events.len() as u32) < options.limit
                            || options.offset.saturating_add(options.limit)
                                >= crate::services::annexes::MAX_EVENTS;
                        self.events.push((tenant_id, options.clone(), found));
                        if last {
                            break;
                        }
                        options.offset = options.offset.saturating_add(options.limit);
                    }
                }
            }
        }
        Ok(())
    }

    fn events_of(
        &self,
        tenant_id: &TenantId,
        options: &EventSearchOptions,
    ) -> Option<&EventSearchResult> {
        self.events
            .iter()
            .find(|(tenant, read, _)| tenant == tenant_id && read == options)
            .map(|(_, _, found)| found)
    }
}

/// What a pass answers for a read it was not served.
enum Unserved {
    Record,
    /// The first unserved read gets the failure that ended the reads, if any.
    Fail(Mutex<Option<Report<StoreError>>>),
}

/// Serves the reads made before a pass; records the others, answering them empty, or fails them.
pub(crate) struct ServedUsage {
    reads: UsageReads,
    unserved: Unserved,
    missed: Mutex<Vec<Missed>>,
}

impl ServedUsage {
    fn new(reads: UsageReads, unserved: Unserved) -> Self {
        Self {
            reads,
            unserved,
            missed: Mutex::default(),
        }
    }

    fn record(&self, missed: Missed) -> StoreResult<()> {
        let failure = match &self.unserved {
            Unserved::Record => {
                self.missed
                    .lock()
                    .map_err(|_| Report::new(StoreError::InvoiceComputationError))?
                    .push(missed);
                return Ok(());
            }
            Unserved::Fail(failure) => failure.lock().ok().and_then(|mut failure| failure.take()),
        };
        Err(failure.unwrap_or_else(|| {
            Report::new(StoreError::InvoiceComputationError)
                .attach("work under row locks kept reading metering it had not read")
        }))
    }

    fn serve(&self, read: UsageRead, metric: &BillableMetric) -> StoreResult<UsageData> {
        if let Some(data) = self.reads.usage.get(&read) {
            return Ok(data.clone());
        }
        let period = read.period.clone();
        self.record(Missed::Usage(read, Box::new(metric.clone())))?;
        Ok(UsageData {
            data: Vec::new(),
            period,
        })
    }

    fn take_missed(&self) -> StoreResult<Vec<Missed>> {
        self.missed
            .lock()
            .map(|mut missed| std::mem::take(&mut *missed))
            .map_err(|_| Report::new(StoreError::InvoiceComputationError))
    }

    /// Fails when the pass read metering not read before it: what it computed must not commit.
    pub(crate) fn ensure_served(&self) -> StoreResult<()> {
        let missed = self
            .missed
            .lock()
            .map_err(|_| Report::new(StoreError::InvoiceComputationError))?
            .len();
        if missed == 0 {
            Ok(())
        } else {
            Err(Report::new(StoreError::InvoiceComputationError)
                .attach(format!("{missed} metering reads this pass was not served")))
        }
    }
}

#[cfg(feature = "facts-shadow")]
impl ServedUsage {
    /// A client served what this pass is, recording any other read and answering it empty; none
    /// once the pass missed a read, as it is redone.
    pub(crate) fn peek(&self) -> Option<Arc<ServedUsage>> {
        let missed = self.missed.lock().ok()?.len();
        (missed == 0).then(|| Arc::new(ServedUsage::new(self.reads.clone(), Unserved::Record)))
    }

    /// Whether reads this client was served, the same but for their periods, tile `read`'s.
    fn served_in_parts(&self, read: &UsageRead) -> bool {
        let mut parts: Vec<&UsagePeriod> = self
            .reads
            .usage
            .keys()
            .filter(|served| {
                UsageRead {
                    period: read.period.clone(),
                    ..(*served).clone()
                } == *read
                    && read.period.start <= served.period.start
                    && served.period.end <= read.period.end
            })
            .map(|served| &served.period)
            .collect();
        parts.sort_by_key(|period| period.start);
        parts.len() > 1
            && parts
                .first()
                .is_some_and(|first| first.start == read.period.start)
            && parts.last().is_some_and(|last| last.end == read.period.end)
            && parts.windows(2).all(|pair| pair[0].end == pair[1].start)
    }

    /// Whether this client was served the same read for a window starting before `read`'s and
    /// ending at its instant.
    fn tail_of_served(&self, read: &UsageRead) -> bool {
        self.reads.usage.keys().any(|served| {
            UsageRead {
                period: read.period.clone(),
                ..served.clone()
            } == *read
                && is_tail(&read.period, &served.period)
        })
    }

    /// The usage reads this client answered empty, each metric with its window; an events search
    /// it was not served has no metric.
    pub(crate) fn unserved_reads(&self) -> Vec<UnservedRead> {
        let Ok(missed) = self.missed.lock() else {
            return Vec::new();
        };
        missed
            .iter()
            .map(|missed| match missed {
                Missed::Usage(read, _) => UnservedRead {
                    metric: Some(read.metric_id),
                    from: read.period.start.date(),
                    to: read.period.end.date(),
                    in_parts: self.served_in_parts(read),
                    tail_of_served: self.tail_of_served(read),
                },
                Missed::Events(_, options) => {
                    let day = |at: &Option<prost_types::Timestamp>| {
                        at.as_ref()
                            .and_then(|at| {
                                chrono::DateTime::from_timestamp(at.seconds, at.nanos as u32)
                            })
                            .map_or(chrono::NaiveDate::MIN, |at| at.date_naive())
                    };
                    UnservedRead {
                        metric: None,
                        from: day(&options.from),
                        to: day(&options.to),
                        in_parts: false,
                        tail_of_served: false,
                    }
                }
            })
            .collect()
    }
}

/// A metering read a client was not served: its metric (none for an events search) and days.
#[cfg(feature = "facts-shadow")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnservedRead {
    pub metric: Option<BillableMetricId>,
    pub from: chrono::NaiveDate,
    pub to: chrono::NaiveDate,
    /// The same read was served for parts of the window, which together tile it.
    pub in_parts: bool,
    /// The same read was served for a window starting earlier and ending at the same instant.
    pub tail_of_served: bool,
}

/// Whether `part` is the end of `whole`: starting after it does, ending at its instant.
#[cfg(feature = "facts-shadow")]
pub(crate) fn is_tail(part: &UsagePeriod, whole: &UsagePeriod) -> bool {
    whole.start < part.start && part.end == whole.end
}

fn unserved() -> Report<StoreError> {
    Report::new(StoreError::InvoiceComputationError)
        .attach("work under row locks reads metering only as usage totals or events")
}

#[async_trait::async_trait]
impl UsageClient for ServedUsage {
    async fn fetch_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.serve(
            UsageRead::of(tenant_id, customer_id, metric, &period, None),
            metric,
        )
    }

    async fn fetch_usage_grouped_by(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: &str,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.serve(
            UsageRead::of(tenant_id, customer_id, metric, &period, Some(group_by)),
            metric,
        )
    }

    async fn fetch_total_usage(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        _metric: &BillableMetric,
        _period: UsagePeriod,
    ) -> StoreResult<Decimal> {
        Err(unserved())
    }

    async fn fetch_windowed_usage(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        _metric: &BillableMetric,
        _period: UsagePeriod,
    ) -> StoreResult<WindowedUsageData> {
        Err(unserved())
    }

    async fn fetch_usage_summary(
        &self,
        _tenant_id: &TenantId,
        _customer_id: Option<&CustomerId>,
        _metric: &BillableMetric,
        _period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        Err(unserved())
    }

    async fn search_events(
        &self,
        tenant_id: &TenantId,
        options: EventSearchOptions,
    ) -> StoreResult<EventSearchResult> {
        if let Some(found) = self.reads.events_of(tenant_id, &options) {
            return Ok(found.clone());
        }
        self.record(Missed::Events(*tenant_id, options))?;
        Ok(EventSearchResult { events: Vec::new() })
    }

    async fn ingest_events(
        &self,
        _tenant_id: &TenantId,
        _request: IngestEventsRequest,
    ) -> StoreResult<IngestEventsResult> {
        Err(unserved())
    }
}

/// The passes of work under row locks: each is served the metering the previous ones missed.
#[derive(Default)]
pub(crate) struct UsagePasses {
    reads: UsageReads,
    passes: usize,
    failure: Option<Report<StoreError>>,
    failing: bool,
}

impl UsagePasses {
    /// The usage the next pass reads: the last of [`MAX_USAGE_PASSES`], or the one after a read
    /// failed, fails the reads it is not served instead of recording them.
    pub(crate) fn next_pass(&mut self) -> StoreResult<Arc<ServedUsage>> {
        if self.failing {
            return Err(Report::new(StoreError::InvoiceComputationError)
                .attach("a failing pass of work under row locks recorded a metering read"));
        }
        self.passes += 1;
        self.failing = self.failure.is_some() || self.passes == MAX_USAGE_PASSES;
        let unserved = if self.failing {
            Unserved::Fail(Mutex::new(self.failure.take()))
        } else {
            Unserved::Record
        };
        Ok(Arc::new(ServedUsage::new(self.reads.clone(), unserved)))
    }

    /// True when the pass read only served metering; otherwise reads, on `client`, what it missed.
    /// A read failing is kept for the next pass, which fails on it.
    pub(crate) async fn served(
        &mut self,
        client: &dyn UsageClient,
        usage: &ServedUsage,
    ) -> StoreResult<bool> {
        let missed = usage.take_missed()?;
        if missed.is_empty() {
            return Ok(true);
        }
        if let Err(failure) = self.reads.read(client, missed).await {
            self.failure = Some(failure);
        }
        Ok(false)
    }
}

impl Services {
    /// Fails when work running on metering read before its row locks read some it was not
    /// served: called before it acts on what it computed, where acting has effects (finalizing).
    pub(in crate::services) fn ensure_usage_served(&self) -> StoreResult<()> {
        self.served
            .as_ref()
            .map_or(Ok(()), |usage| usage.ensure_served())
    }

    /// A transaction on `conn` whose work reads metering before taking its row locks (see the
    /// module). Nested in a caller's transaction it stays correct, but its reads then run under
    /// the caller's locks: it belongs where the transaction is the outermost.
    #[track_caller]
    pub(in crate::services) fn transaction_reading_usage_first<'s, 'c, 'a, R, F>(
        &'s self,
        conn: &'c mut PgConn,
        work: F,
    ) -> impl Future<Output = StoreResult<R>> + use<'s, 'c, 'a, R, F>
    where
        R: Send + 'a,
        F: for<'r> Fn(&'r Services, &'r mut PgConn) -> ScopedBoxFuture<'a, 'r, StoreResult<R>>
            + Send
            + Sync
            + 'a,
    {
        let passes = self.transaction_keeping_usage_reads(conn, work);
        async move { passes.await.map(|(result, _)| result)? }
    }

    /// [`Self::transaction_reading_usage_first`], also returning the metering its last pass was
    /// served, for later work under locks to be served the same (see [`Self::served_only`]).
    #[track_caller]
    pub(in crate::services) fn transaction_keeping_usage_reads<'s, 'c, 'a, R, F>(
        &'s self,
        conn: &'c mut PgConn,
        work: F,
    ) -> impl Future<Output = StoreResult<(StoreResult<R>, UsageReads)>> + use<'s, 'c, 'a, R, F>
    where
        R: Send + 'a,
        F: for<'r> Fn(&'r Services, &'r mut PgConn) -> ScopedBoxFuture<'a, 'r, StoreResult<R>>
            + Send
            + Sync
            + 'a,
    {
        let entry = std::panic::Location::caller();
        Box::pin(async move {
            let mut passes = UsagePasses::default();
            loop {
                let usage = passes.next_pass()?;
                let services = Services {
                    served: Some(usage.clone()),
                    ..self.reading_usage(usage.clone())
                };
                let result = self
                    .store
                    .transaction_with(conn, |conn| {
                        let (work, services, usage) = (&work, &services, &usage);
                        async move {
                            let done = work(services, conn).await;
                            usage.ensure_served()?;
                            done
                        }
                        .scope_boxed()
                    })
                    .await;
                if passes.served(self.usage_client.as_ref(), &usage).await? {
                    record_passes(entry, passes.passes);
                    return Ok((result, passes.reads));
                }
            }
        })
    }

    /// Services whose metering is only `reads`, made before the caller's locks: a read they do
    /// not hold fails rather than reaching metering under those locks.
    pub(in crate::services) fn served_only(&self, reads: UsageReads) -> Services {
        let usage = Arc::new(ServedUsage::new(reads, Unserved::Fail(Mutex::new(None))));
        Services {
            served: Some(usage.clone()),
            ..self.reading_usage(usage)
        }
    }
}
