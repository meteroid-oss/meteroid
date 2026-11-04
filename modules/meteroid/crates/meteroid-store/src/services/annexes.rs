use crate::domain::DetailedInvoice;
use crate::domain::invoice_annex_templates::{
    AnnexColumn, AnnexDefinition, ColumnAggregation, ColumnSource, ColumnTransform,
    EventTableDefinition,
};
use crate::domain::invoice_lines::{LineItem, SubLineAttributes};
use crate::domain::invoices::{
    InvoiceAnnex, InvoiceAnnexCell, InvoiceAnnexGroup, InvoiceAnnexRow, InvoiceAnnexTable,
};
use crate::errors::StoreError;
use crate::repositories::subscriptions::usage_attribution::usage_scopes_of;
use crate::services::Services;
use crate::services::clients::usage::EventSearchOptions;
use crate::store::PgConn;
use crate::{StoreResult, domain};
use chrono::{DateTime, NaiveDateTime, NaiveTime, Utc};
use common_domain::ids::{CustomerId, InvoiceId, PlanVersionId, SubscriptionId, TenantId};
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::invoice_annex_templates::InvoiceAnnexTemplateRow;
use diesel_models::invoices::InvoiceRow;
use metering_grpc::meteroid::metering::v1::Event;
use metering_grpc::meteroid::metering::v1::query_raw_events_request::SortOrder;
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;
use std::collections::{HashMap, HashSet};

/// One page of events per `search_events` call; we paginate until exhausted.
const EVENT_PAGE_SIZE: u32 = 500;
/// Hard cap so a misconfigured annex can never page forever.
pub(crate) const MAX_EVENTS: u32 = 50_000;

impl Services {
    /// Re-derives a correction's appendix from the event stream, on explicit operator demand.
    ///
    /// A correction otherwise carries its parent's frozen snapshot for good (see
    /// `create_corrected_invoice_from_tx`). This is the escape hatch for the case where that
    /// snapshot is empty or stale — a parent issued before its plan version had an annex bound.
    pub async fn regenerate_invoice_annexes(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<DetailedInvoice> {
        let mut conn = self.store.get_conn().await?;
        self.transaction_reading_usage_first(&mut conn, |services, conn| {
            async move {
                let invoice: domain::Invoice =
                    InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                        .await?
                        .invoice
                        .try_into()?;

                if !invoice.can_edit() {
                    return Err(StoreError::InvalidArgument(
                        "Can only regenerate annexes on a draft invoice".into(),
                    )
                    .into());
                }

                if !invoice.is_correction() {
                    return Err(StoreError::InvalidArgument(
                        "Annexes are derived automatically outside a correction invoice".into(),
                    )
                    .into());
                }

                let annexes = services
                    .resolve_invoice_annexes(
                        conn,
                        tenant_id,
                        invoice.plan_version_id,
                        invoice.customer_id,
                        invoice.subscription_id,
                        &invoice.line_items,
                    )
                    .await?;

                let serialized = serde_json::to_value(&annexes).map_err(|e| {
                    StoreError::SerdeError("Failed to serialize annexes".to_string(), e)
                })?;

                InvoiceRow::update_annexes(conn, invoice_id, tenant_id, serialized).await?;

                InvoiceRow::find_detailed_by_id(conn, tenant_id, invoice_id)
                    .await
                    .map_err(Into::into)
                    .and_then(std::convert::TryInto::try_into)
            }
            .scope_boxed()
        })
        .await
    }

    /// Resolves the annexes (appendices) to attach to an invoice for the given plan version,
    /// customer and billed period. Reads the raw event stream directly — independent of any
    /// metric's aggregation. Returns the presentation snapshot to be frozen on the invoice.
    pub(in crate::services) async fn resolve_invoice_annexes(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        plan_version_id: Option<PlanVersionId>,
        customer_id: CustomerId,
        subscription_id: Option<SubscriptionId>,
        line_items: &[LineItem],
    ) -> StoreResult<Vec<InvoiceAnnex>> {
        let Some(plan_version_id) = plan_version_id else {
            return Ok(vec![]);
        };

        let template_rows =
            InvoiceAnnexTemplateRow::list_for_plan_version(conn, tenant_id, plan_version_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        if template_rows.is_empty() {
            return Ok(vec![]);
        }

        let period = line_period(line_items);
        let attribution = match subscription_id {
            Some(subscription_id) => {
                Some(annex_attribution(conn, tenant_id, subscription_id, line_items).await?)
            }
            None => None,
        };

        let mut annexes = Vec::new();
        for row in template_rows {
            let template: domain::AnnexTemplate = row.try_into()?;
            match template.definition {
                AnnexDefinition::StaticText(def) => {
                    annexes.push(InvoiceAnnex {
                        title: def.title,
                        description: def.description,
                        table: InvoiceAnnexTable {
                            columns: vec![],
                            groups: vec![],
                        },
                    });
                }
                AnnexDefinition::EventTable(def) => {
                    let Some((from, to)) = period else {
                        // No billed period to scope events to — skip silently.
                        continue;
                    };
                    if let Some(annex) = self
                        .resolve_event_table_annex(
                            tenant_id,
                            customer_id,
                            from,
                            to,
                            &def,
                            line_items,
                            attribution.as_ref(),
                        )
                        .await?
                    {
                        annexes.push(annex);
                    }
                }
            }
        }

        Ok(annexes)
    }

    async fn resolve_event_table_annex(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        from: NaiveDateTime,
        to: NaiveDateTime,
        def: &EventTableDefinition,
        line_items: &[LineItem],
        attribution: Option<&AnnexAttribution>,
    ) -> StoreResult<Option<InvoiceAnnex>> {
        if def.columns.is_empty() {
            return Ok(None);
        }

        let mut events = self
            .fetch_all_events(tenant_id, customer_id, from, to, &def.source.event_codes)
            .await?;
        if let Some(attribution) = attribution {
            events.retain(|event| attribution.bills(event));
        }

        Ok(build_event_annex(def, &events, &billed_rates(line_items)))
    }

    async fn fetch_all_events(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        from: NaiveDateTime,
        to: NaiveDateTime,
        event_codes: &[String],
    ) -> StoreResult<Vec<Event>> {
        let from_ts = to_timestamp(from);
        let to_ts = to_timestamp(to);

        let mut events: Vec<Event> = Vec::new();
        let mut offset = 0u32;
        loop {
            let result = self
                .usage_client
                .search_events(
                    &tenant_id,
                    EventSearchOptions {
                        from: Some(from_ts),
                        to: Some(to_ts),
                        limit: EVENT_PAGE_SIZE,
                        offset,
                        search: None,
                        event_codes: event_codes.to_vec(),
                        customer_ids: vec![customer_id.to_string()],
                        filters: vec![],
                        sort_order: SortOrder::TimestampAsc as i32,
                    },
                )
                .await?;

            let fetched = result.events.len() as u32;
            events.extend(result.events);

            if fetched < EVENT_PAGE_SIZE || events.len() as u32 >= MAX_EVENTS {
                break;
            }
            offset += EVENT_PAGE_SIZE;
        }

        Ok(events)
    }
}

/// Which of the customer's events the invoice's subscription bills, by event code: the scopes of
/// its metrics of that code, else its own usage filter.
pub(in crate::services) struct AnnexAttribution {
    by_code: HashMap<String, Vec<domain::UsageScope>>,
    otherwise: domain::UsageScope,
}

impl AnnexAttribution {
    fn bills(&self, event: &Event) -> bool {
        let Ok(at) = DateTime::parse_from_rfc3339(&event.timestamp) else {
            return false;
        };
        let at = at.naive_utc();
        match self.by_code.get(&event.code) {
            Some(scopes) => scopes
                .iter()
                .any(|scope| scope.contains(&event.properties, at)),
            None => self.otherwise.contains(&event.properties, at),
        }
    }
}

async fn annex_attribution(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    line_items: &[LineItem],
) -> StoreResult<AnnexAttribution> {
    let scopes = usage_scopes_of(conn, tenant_id, subscription_id).await?;
    let metric_ids = line_items
        .iter()
        .filter_map(|line| line.metric_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut by_code: HashMap<String, Vec<domain::UsageScope>> = HashMap::new();
    if !metric_ids.is_empty() {
        for row in BillableMetricRow::get_by_ids(conn, &metric_ids, &tenant_id)
            .await
            .map_err(Into::<error_stack::Report<StoreError>>::into)?
        {
            by_code
                .entry(row.code.clone())
                .or_default()
                .push(scopes.scope_of(row.id));
        }
    }
    Ok(AnnexAttribution {
        by_code,
        otherwise: scopes.own_scope(),
    })
}

struct ProjectedRow {
    group_key: Vec<String>,
    sort_key: Vec<String>,
    cells: Vec<InvoiceAnnexCell>,
}

/// Pure projection: turns raw events into the annex snapshot for an event-table definition.
/// Returns `None` when there is nothing to render. In aggregated mode (`aggregate_by` set)
/// events collapse into one row per key with per-column aggregations, optional per-group
/// subtotals and a grand total; otherwise every event is listed individually.
fn build_event_annex(
    def: &EventTableDefinition,
    events: &[Event],
    rates: &BilledRates,
) -> Option<InvoiceAnnex> {
    if def.columns.is_empty() || events.is_empty() {
        return None;
    }

    let groups = if def.aggregate_by.is_empty() {
        build_listed_groups(def, events)
    } else {
        build_aggregated_groups(def, events, rates)
    };

    Some(InvoiceAnnex {
        title: def.title.clone(),
        description: def.description.clone(),
        table: InvoiceAnnexTable {
            columns: def.columns.iter().map(|c| c.label.clone()).collect(),
            groups,
        },
    })
}

/// Per-event listing: one row per event, with optional heading rows per `group_by`.
fn build_listed_groups(def: &EventTableDefinition, events: &[Event]) -> Vec<InvoiceAnnexGroup> {
    let mut projected: Vec<ProjectedRow> = events
        .iter()
        .map(|event| ProjectedRow {
            group_key: def.group_by.iter().map(|p| property(event, p)).collect(),
            sort_key: def.sort_by.iter().map(|p| property(event, p)).collect(),
            cells: def.columns.iter().map(|c| cell_value(event, c)).collect(),
        })
        .collect();

    // Sort by group first (so groups stay contiguous) then by the configured sort keys.
    projected.sort_by(|a, b| {
        a.group_key
            .cmp(&b.group_key)
            .then_with(|| a.sort_key.cmp(&b.sort_key))
    });

    let grouped = !def.group_by.is_empty();
    let mut groups: Vec<InvoiceAnnexGroup> = Vec::new();
    for row in projected {
        let heading = if grouped {
            Some(row.group_key.join(" — "))
        } else {
            None
        };
        let cells = row.cells;
        match groups.last_mut() {
            Some(g) if g.heading == heading => g.rows.push(InvoiceAnnexRow {
                cells,
                emphasis: false,
            }),
            _ => groups.push(InvoiceAnnexGroup {
                heading,
                rows: vec![InvoiceAnnexRow {
                    cells,
                    emphasis: false,
                }],
            }),
        }
    }
    groups
}

/// One numeric accumulator per column (`None` for non-summable columns).
type Sums = Vec<Option<Decimal>>;

/// Aggregated mode: collapse events sharing `aggregate_by` into one row, lay them out with
/// optional per-`group_by` subtotals and a grand total. Returns a single heading-less group —
/// dimension values are repeated in each row, matching the customer appendix layout.
fn build_aggregated_groups(
    def: &EventTableDefinition,
    events: &[Event],
    rates: &BilledRates,
) -> Vec<InvoiceAnnexGroup> {
    // 1. Collapse events into aggregate buckets, preserving first-seen order.
    let mut order: Vec<Vec<String>> = Vec::new();
    let mut buckets: HashMap<Vec<String>, Vec<&Event>> = HashMap::new();
    for event in events {
        let key: Vec<String> = def
            .aggregate_by
            .iter()
            .map(|p| property(event, p))
            .collect();
        buckets
            .entry(key.clone())
            .or_insert_with(|| {
                order.push(key);
                Vec::new()
            })
            .push(event);
    }

    // 2. One output row per bucket, with parallel numeric accumulators for subtotaling.
    struct AggRow {
        group_key: Vec<String>,
        cells: Vec<InvoiceAnnexCell>,
        sums: Sums,
    }
    let mut rows: Vec<AggRow> = order
        .into_iter()
        .map(|key| {
            let bucket = &buckets[&key];
            let mut cells = Vec::with_capacity(def.columns.len());
            let mut sums: Sums = Vec::with_capacity(def.columns.len());
            for col in &def.columns {
                let (text, num) = aggregate_cell(col, bucket, rates);
                cells.push(text);
                sums.push(num);
            }
            AggRow {
                group_key: def
                    .group_by
                    .iter()
                    .map(|p| property(bucket[0], p))
                    .collect(),
                cells,
                sums,
            }
        })
        .collect();

    // 3. Order by display group then by the row cells for stable, contiguous groups.
    rows.sort_by(|a, b| {
        a.group_key
            .cmp(&b.group_key)
            .then_with(|| a.cells.cmp(&b.cells))
    });

    let label_col = subtotal_label_column(def);
    let with_subtotals = !def.group_by.is_empty() && def.show_group_subtotals;

    let mut out_rows: Vec<InvoiceAnnexRow> = Vec::new();
    let mut group_sums: Sums = empty_sums(def);
    let mut total_sums: Sums = empty_sums(def);
    let mut current: Option<Vec<String>> = None;

    for row in rows {
        if with_subtotals {
            if let Some(k) = &current
                && *k != row.group_key
            {
                out_rows.push(summary_row(
                    def,
                    &group_sums,
                    label_col,
                    &format!("Total {}", k.join(" — ")),
                ));
                group_sums = empty_sums(def);
            }
            current = Some(row.group_key.clone());
            add_sums(&mut group_sums, &row.sums);
        }
        add_sums(&mut total_sums, &row.sums);
        out_rows.push(InvoiceAnnexRow {
            cells: row.cells,
            emphasis: false,
        });
    }

    if with_subtotals && let Some(k) = &current {
        out_rows.push(summary_row(
            def,
            &group_sums,
            label_col,
            &format!("Total {}", k.join(" — ")),
        ));
    }
    if def.show_total {
        out_rows.push(summary_row(def, &total_sums, label_col, "TOTAL"));
    }

    vec![InvoiceAnnexGroup {
        heading: None,
        rows: out_rows,
    }]
}

/// Computes one column's value for an aggregate bucket: the typed cell plus, for summable
/// columns, the numeric value used to build subtotal/total rows.
fn aggregate_cell(
    col: &AnnexColumn,
    events: &[&Event],
    rates: &BilledRates,
) -> (InvoiceAnnexCell, Option<Decimal>) {
    let agg = col.aggregation.unwrap_or(ColumnAggregation::First);
    match agg {
        ColumnAggregation::First => (
            events
                .first()
                .map(|e| cell_value(e, col))
                .unwrap_or(InvoiceAnnexCell::Empty),
            None,
        ),
        ColumnAggregation::Count => {
            let n = Decimal::from(events.len() as u64);
            (InvoiceAnnexCell::Number(n), Some(n))
        }
        ColumnAggregation::Sum => {
            let sum: Decimal = events.iter().filter_map(|e| sum_value(e, col)).sum();
            (InvoiceAnnexCell::Number(sum), Some(sum))
        }
        ColumnAggregation::PeriodRange => (period_range(events, col), None),
        ColumnAggregation::BilledAmount => {
            let units = Decimal::from(events.len() as u64);
            // Match this bucket to a billed rate via the property values it carries (so a
            // matrix-priced metric bills each dimension — e.g. per client — at its own rate).
            let value_set: HashSet<String> = events
                .iter()
                .flat_map(|e| e.properties.values().cloned())
                .collect();
            match rates.rate_for(&value_set) {
                // Carried in minor units so subtotals stay exact; the renderer scales to major
                // units using the invoice currency.
                Some(rate) => (InvoiceAnnexCell::Money(units * rate), Some(units * rate)),
                None => (InvoiceAnnexCell::Empty, Some(Decimal::ZERO)),
            }
        }
    }
}

fn empty_sums(def: &EventTableDefinition) -> Sums {
    def.columns
        .iter()
        .map(|c| match c.aggregation {
            Some(a) if a.is_summable() => Some(Decimal::ZERO),
            _ => None,
        })
        .collect()
}

fn add_sums(acc: &mut Sums, row: &Sums) {
    for (a, r) in acc.iter_mut().zip(row.iter()) {
        if let (Some(av), Some(rv)) = (a.as_mut(), r) {
            *av += *rv;
        }
    }
}

/// The column carrying the subtotal/total label: the one immediately left of the first summable
/// column (so "Total TDF" sits just before the unit count, as in the customer appendix).
fn subtotal_label_column(def: &EventTableDefinition) -> usize {
    def.columns
        .iter()
        .position(|c| matches!(c.aggregation, Some(a) if a.is_summable()))
        .map(|i| i.saturating_sub(1))
        .unwrap_or(0)
}

fn summary_row(
    def: &EventTableDefinition,
    sums: &Sums,
    label_col: usize,
    label: &str,
) -> InvoiceAnnexRow {
    let cells = def
        .columns
        .iter()
        .enumerate()
        .map(|(i, col)| match sums[i] {
            Some(v) => match col.aggregation.unwrap_or(ColumnAggregation::Sum) {
                ColumnAggregation::BilledAmount => InvoiceAnnexCell::Money(v),
                _ => InvoiceAnnexCell::Number(v),
            },
            None if i == label_col => InvoiceAnnexCell::Text(label.to_string()),
            None => InvoiceAnnexCell::Empty,
        })
        .collect();
    InvoiceAnnexRow {
        cells,
        emphasis: true,
    }
}

/// The numeric value a Sum column contributes for one event (its property parsed as a number).
fn sum_value(event: &Event, col: &AnnexColumn) -> Option<Decimal> {
    match &col.source {
        ColumnSource::Property(name) => parse_decimal(&property(event, name)),
        ColumnSource::Timestamp | ColumnSource::BilledPrice => None,
    }
}

fn parse_decimal(s: &str) -> Option<Decimal> {
    s.trim().replace(' ', "").parse::<Decimal>().ok()
}

fn parse_ts(raw: &str) -> Option<NaiveDateTime> {
    raw.parse::<DateTime<Utc>>().ok().map(|dt| dt.naive_utc())
}

/// The strftime pattern for a date column (its `timestamp_format` transform, default `%b %Y`).
fn column_fmt(col: &AnnexColumn) -> String {
    match &col.transform {
        Some(ColumnTransform::TimestampFormat(f)) => f.clone(),
        None => "%b %Y".to_string(),
    }
}

/// `min … max` of the bucket's event timestamps; the renderer formats & collapses when the ends
/// coincide under the chosen format.
fn period_range(events: &[&Event], col: &AnnexColumn) -> InvoiceAnnexCell {
    let mut times: Vec<NaiveDateTime> = events
        .iter()
        .filter_map(|e| parse_ts(&e.timestamp))
        .collect();
    if times.is_empty() {
        return InvoiceAnnexCell::Empty;
    }
    times.sort();
    InvoiceAnnexCell::DateRange {
        min: *times.first().unwrap(),
        max: *times.last().unwrap(),
        fmt: column_fmt(col),
    }
}

/// Per-unit billed rates (minor currency units per unit) derived from the invoice's usage lines.
/// A segmented (matrix) metric prices each dimension separately, so its rate lives on the
/// sub-lines, not the parent line; `per_dimension` captures those, with a blended `fallback`.
struct BilledRates {
    /// (matrix dimension values, rate). Matched when every value appears among a bucket's event
    /// property values (e.g. `["France", "TDF"]`).
    per_dimension: Vec<(Vec<String>, Decimal)>,
    /// Blended Σtotal / Σquantity across all usage; used when no dimension matches. `None` when
    /// nothing usage-billed.
    fallback: Option<Decimal>,
}

impl BilledRates {
    #[cfg(test)]
    fn none() -> Self {
        Self {
            per_dimension: Vec::new(),
            fallback: None,
        }
    }

    /// Most specific matrix rate whose dimension values are all present in the bucket's property
    /// values, else the blended fallback.
    fn rate_for(&self, value_set: &HashSet<String>) -> Option<Decimal> {
        self.per_dimension
            .iter()
            .filter(|(vals, _)| !vals.is_empty() && vals.iter().all(|v| value_set.contains(v)))
            .max_by_key(|(vals, _)| vals.len())
            .map(|(_, rate)| *rate)
            .or(self.fallback)
    }
}

/// Builds [`BilledRates`] from the invoice line items. Sums the sub-line quantities/amounts for
/// segmented metrics (whose parent line has `quantity: None`) — the reason a naive
/// Σ`line.quantity` yielded 0 and blanked the amount column.
fn billed_rates(lines: &[LineItem]) -> BilledRates {
    let mut per_dimension: Vec<(Vec<String>, Decimal)> = Vec::new();
    let mut total_amount: i64 = 0;
    let mut total_qty = Decimal::ZERO;

    for line in lines.iter().filter(|l| l.metric_id.is_some()) {
        if line.sub_lines.is_empty() {
            total_amount += line.amount_subtotal;
            total_qty += line.quantity.unwrap_or(Decimal::ZERO);
        } else {
            for sl in &line.sub_lines {
                total_amount += sl.total;
                total_qty += sl.quantity;
                if sl.quantity > Decimal::ZERO
                    && let Some(SubLineAttributes::Matrix {
                        dimension1_value,
                        dimension2_value,
                        ..
                    }) = &sl.attributes
                {
                    let mut vals = vec![dimension1_value.clone()];
                    if let Some(v2) = dimension2_value {
                        vals.push(v2.clone());
                    }
                    per_dimension.push((vals, Decimal::from(sl.total) / sl.quantity));
                }
            }
        }
    }

    let fallback = if total_qty > Decimal::ZERO {
        Some(Decimal::from(total_amount) / total_qty)
    } else {
        None
    };
    BilledRates {
        per_dimension,
        fallback,
    }
}

fn property(event: &Event, name: &str) -> String {
    event.properties.get(name).cloned().unwrap_or_default()
}

fn cell_value(event: &Event, column: &AnnexColumn) -> InvoiceAnnexCell {
    match &column.source {
        ColumnSource::Property(name) => {
            let v = property(event, name);
            if v.is_empty() {
                InvoiceAnnexCell::Empty
            } else {
                InvoiceAnnexCell::Text(v)
            }
        }
        ColumnSource::Timestamp => match parse_ts(&event.timestamp) {
            Some(ts) => InvoiceAnnexCell::Date {
                ts,
                fmt: column_fmt(column),
            },
            None => InvoiceAnnexCell::Empty,
        },
        // Per-event billed price is not attributable for tiered/volume pricing; use a
        // BilledAmount column (aggregated mode) for amounts instead.
        ColumnSource::BilledPrice => InvoiceAnnexCell::Empty,
    }
}

/// Billed period derived from the invoice line items: earliest start to (exclusive) day after
/// the latest end, so the whole final day is covered regardless of the store's bound semantics.
fn line_period(lines: &[LineItem]) -> Option<(NaiveDateTime, NaiveDateTime)> {
    let start = lines.iter().map(|l| l.start_date).min()?;
    let end = lines.iter().map(|l| l.end_date).max()?;

    let from = start.and_time(NaiveTime::MIN);
    let to = end.succ_opt().unwrap_or(end).and_time(NaiveTime::MIN);
    Some((from, to))
}

fn to_timestamp(dt: NaiveDateTime) -> prost_types::Timestamp {
    let utc = dt.and_utc();
    prost_types::Timestamp {
        seconds: utc.timestamp(),
        nanos: utc.timestamp_subsec_nanos() as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::invoice_annex_templates::EventSource;
    use std::collections::HashMap;

    fn event(props: &[(&str, &str)], ts: &str) -> Event {
        Event {
            id: "evt".to_string(),
            code: "project_billed".to_string(),
            customer_id: None,
            timestamp: ts.to_string(),
            properties: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<HashMap<_, _>>(),
        }
    }

    fn def() -> EventTableDefinition {
        EventTableDefinition {
            title: "Detail".to_string(),
            description: None,
            source: EventSource {
                event_codes: vec!["project_billed".to_string()],
            },
            columns: vec![
                AnnexColumn {
                    label: "Project".to_string(),
                    source: ColumnSource::Property("project_id".to_string()),
                    transform: None,
                    aggregation: None,
                },
                AnnexColumn {
                    label: "Period".to_string(),
                    source: ColumnSource::Timestamp,
                    transform: Some(ColumnTransform::TimestampFormat("%Y/%m".to_string())),
                    aggregation: None,
                },
            ],
            aggregate_by: vec![],
            group_by: vec!["client".to_string()],
            sort_by: vec!["project_id".to_string()],
            show_group_subtotals: false,
            show_total: false,
        }
    }

    fn ndt(s: &str) -> NaiveDateTime {
        s.parse::<DateTime<Utc>>().unwrap().naive_utc()
    }

    fn col(label: &str, source: ColumnSource, agg: ColumnAggregation) -> AnnexColumn {
        AnnexColumn {
            label: label.to_string(),
            source,
            transform: None,
            aggregation: Some(agg),
        }
    }

    #[test]
    fn groups_rows_by_client_and_projects_typed_cells() {
        let events = vec![
            event(
                &[("client", "Globex"), ("project_id", "PRJ-9")],
                "2025-03-10T00:00:00Z",
            ),
            event(
                &[("client", "Acme"), ("project_id", "PRJ-1")],
                "2025-01-05T00:00:00Z",
            ),
            event(
                &[("client", "Acme"), ("project_id", "PRJ-1")],
                "2025-02-05T00:00:00Z",
            ),
        ];

        let annex = build_event_annex(&def(), &events, &BilledRates::none()).expect("annex");

        assert_eq!(annex.table.columns, vec!["Project", "Period"]);
        // Two groups, alphabetically ordered, Acme first with its two events.
        assert_eq!(annex.table.groups.len(), 2);
        assert_eq!(annex.table.groups[0].heading.as_deref(), Some("Acme"));
        assert_eq!(annex.table.groups[0].rows.len(), 2);
        // Property projected as text; timestamp carried as a typed date + its format (the renderer
        // localizes it).
        assert_eq!(
            annex.table.groups[0].rows[0].cells,
            vec![
                InvoiceAnnexCell::Text("PRJ-1".to_string()),
                InvoiceAnnexCell::Date {
                    ts: ndt("2025-01-05T00:00:00Z"),
                    fmt: "%Y/%m".to_string(),
                },
            ]
        );
        assert_eq!(annex.table.groups[1].heading.as_deref(), Some("Globex"));
        assert_eq!(
            annex.table.groups[1].rows[0].cells,
            vec![
                InvoiceAnnexCell::Text("PRJ-9".to_string()),
                InvoiceAnnexCell::Date {
                    ts: ndt("2025-03-10T00:00:00Z"),
                    fmt: "%Y/%m".to_string(),
                },
            ]
        );
    }

    #[test]
    fn ungrouped_when_no_group_by() {
        let mut d = def();
        d.group_by = vec![];
        let events = vec![event(&[("project_id", "PRJ-1")], "2025-01-05T00:00:00Z")];

        let annex = build_event_annex(&d, &events, &BilledRates::none()).expect("annex");
        assert_eq!(annex.table.groups.len(), 1);
        assert!(annex.table.groups[0].heading.is_none());
    }

    #[test]
    fn none_when_no_events_or_columns() {
        assert!(build_event_annex(&def(), &[], &BilledRates::none()).is_none());
        let mut d = def();
        d.columns = vec![];
        assert!(
            build_event_annex(
                &d,
                &[event(&[], "2025-01-05T00:00:00Z")],
                &BilledRates::none()
            )
            .is_none()
        );
    }

    /// Mirrors the customer appendix: aggregate per mission, count units, sum a billed amount,
    /// show a period range, with per-client subtotals and a grand total.
    #[test]
    fn aggregates_with_subtotals_and_total() {
        let d = EventTableDefinition {
            title: "Invoice appendix".to_string(),
            description: None,
            source: EventSource {
                event_codes: vec!["mission_active".to_string()],
            },
            columns: vec![
                col(
                    "Mission ID",
                    ColumnSource::Property("mission_id".to_string()),
                    ColumnAggregation::First,
                ),
                col(
                    "Client",
                    ColumnSource::Property("client".to_string()),
                    ColumnAggregation::First,
                ),
                AnnexColumn {
                    label: "Billing Period".to_string(),
                    source: ColumnSource::Timestamp,
                    transform: Some(ColumnTransform::TimestampFormat("%b %Y".to_string())),
                    aggregation: Some(ColumnAggregation::PeriodRange),
                },
                col("Units", ColumnSource::Timestamp, ColumnAggregation::Count),
                col(
                    "Amount",
                    ColumnSource::Timestamp,
                    ColumnAggregation::BilledAmount,
                ),
            ],
            aggregate_by: vec!["client".to_string(), "mission_id".to_string()],
            group_by: vec!["client".to_string()],
            sort_by: vec![],
            show_group_subtotals: true,
            show_total: true,
        };

        // TDF/M1 active 3 months, TDF/M2 active 1 month → 4 units total. Rate = 22.00 / unit.
        let ev = |client: &str, mission: &str, ts: &str| {
            event(&[("client", client), ("mission_id", mission)], ts)
        };
        let events = vec![
            ev("TDF", "M1", "2025-10-01T00:00:00Z"),
            ev("TDF", "M1", "2025-11-01T00:00:00Z"),
            ev("TDF", "M1", "2025-12-01T00:00:00Z"),
            ev("TDF", "M2", "2025-10-01T00:00:00Z"),
        ];
        // Blended fallback rate: 22.00 / unit (in minor units).
        let rates = BilledRates {
            per_dimension: vec![],
            fallback: Some(Decimal::from(2200)),
        };

        let annex = build_event_annex(&d, &events, &rates).expect("annex");
        let rows = &annex.table.groups[0].rows;

        // 2 data rows + 1 subtotal + 1 grand total.
        assert_eq!(rows.len(), 4);

        // M1: 3 units, Oct→Dec range (typed), amount 3 × 2200 = 6600 minor units.
        assert_eq!(rows[0].cells[0], InvoiceAnnexCell::Text("M1".to_string()));
        assert_eq!(
            rows[0].cells[2],
            InvoiceAnnexCell::DateRange {
                min: ndt("2025-10-01T00:00:00Z"),
                max: ndt("2025-12-01T00:00:00Z"),
                fmt: "%b %Y".to_string(),
            }
        );
        assert_eq!(rows[0].cells[3], InvoiceAnnexCell::Number(Decimal::from(3)));
        assert_eq!(
            rows[0].cells[4],
            InvoiceAnnexCell::Money(Decimal::from(6600))
        );
        assert!(!rows[0].emphasis);
        // M2: single month → collapsed range, 1 unit, 2200 minor units.
        assert_eq!(rows[1].cells[3], InvoiceAnnexCell::Number(Decimal::from(1)));
        assert_eq!(
            rows[1].cells[4],
            InvoiceAnnexCell::Money(Decimal::from(2200))
        );

        // Subtotal row (emphasized): label in the column left of Units, 4 units, 8800 minor.
        assert!(rows[2].emphasis);
        assert_eq!(
            rows[2].cells[2],
            InvoiceAnnexCell::Text("Total TDF".to_string())
        );
        assert_eq!(rows[2].cells[3], InvoiceAnnexCell::Number(Decimal::from(4)));
        assert_eq!(
            rows[2].cells[4],
            InvoiceAnnexCell::Money(Decimal::from(8800))
        );

        // Grand total
        assert!(rows[3].emphasis);
        assert_eq!(
            rows[3].cells[2],
            InvoiceAnnexCell::Text("TOTAL".to_string())
        );
        assert_eq!(rows[3].cells[3], InvoiceAnnexCell::Number(Decimal::from(4)));
        assert_eq!(
            rows[3].cells[4],
            InvoiceAnnexCell::Money(Decimal::from(8800))
        );
    }

    fn value_set(values: &[&str]) -> HashSet<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    /// Locks the root-cause fix: a segmented (matrix) metric prices on its sub-lines and the
    /// parent line has `quantity: None`, so rates must come from the sub-lines.
    #[test]
    fn billed_rates_reads_matrix_sub_lines() {
        use crate::domain::invoice_lines::{LineItemKind, SubLineItem};
        use chrono::NaiveDate;
        use common_domain::ids::BaseId;

        let sub = |client: &str, qty: i64, unit_cents: i64| SubLineItem {
            local_id: "s".to_string(),
            name: format!("France,{client}"),
            total: qty * unit_cents,
            quantity: Decimal::from(qty),
            unit_price: Decimal::from(unit_cents) / Decimal::from(100),
            attributes: Some(SubLineAttributes::Matrix {
                dimension1_key: "country".to_string(),
                dimension1_value: "France".to_string(),
                dimension2_key: Some("client".to_string()),
                dimension2_value: Some(client.to_string()),
            }),
        };
        let line = LineItem {
            local_id: "l".to_string(),
            name: "Missions".to_string(),
            charge_type: None,
            amount_subtotal: 14 * 100 + 15 * 200,
            tax_rate: Decimal::ZERO,
            taxable_amount: 0,
            tax_amount: 0,
            amount_total: 0,
            tax_details: vec![],
            quantity: None, // matrix parent line: quantity lives on the sub-lines
            unit_price: None,
            start_date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            end_date: NaiveDate::from_ymd_opt(2026, 6, 30).unwrap(),
            sub_lines: vec![sub("TDF", 14, 100), sub("Boulanger", 15, 200)],
            is_prorated: false,
            price_component_id: None,
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: None,
            metric_id: Some(common_domain::ids::BillableMetricId::new()),
            description: None,
            group_by_dimensions: None,
            kind: LineItemKind::Regular,
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
        };

        let rates = billed_rates(&[line]);
        // Per-dimension rates: TDF 1.00, Boulanger 2.00 (minor units).
        assert_eq!(
            rates.rate_for(&value_set(&["France", "TDF"])),
            Some(Decimal::from(100))
        );
        assert_eq!(
            rates.rate_for(&value_set(&["France", "Boulanger"])),
            Some(Decimal::from(200))
        );
        // Fallback blends over the summed sub-line quantities (29), NOT zero.
        assert_eq!(
            rates.fallback,
            Some(Decimal::from(4400) / Decimal::from(29))
        );
    }

    /// End-to-end: each client bills at its own matrix rate (TDF 1€, Boulanger 2€).
    #[test]
    fn per_dimension_rates_bill_each_client_separately() {
        let d = EventTableDefinition {
            title: "A".to_string(),
            description: None,
            source: EventSource {
                event_codes: vec!["mission_active".to_string()],
            },
            columns: vec![
                col(
                    "Client",
                    ColumnSource::Property("client".to_string()),
                    ColumnAggregation::First,
                ),
                col("Units", ColumnSource::Timestamp, ColumnAggregation::Count),
                col(
                    "Amount",
                    ColumnSource::Timestamp,
                    ColumnAggregation::BilledAmount,
                ),
            ],
            aggregate_by: vec!["client".to_string()],
            group_by: vec![],
            sort_by: vec![],
            show_group_subtotals: false,
            show_total: true,
        };
        let evc = |client: &str, ts: &str| event(&[("client", client), ("country", "France")], ts);
        let events = vec![
            evc("TDF", "2026-04-01T00:00:00Z"),
            evc("TDF", "2026-05-01T00:00:00Z"),
            evc("TDF", "2026-06-01T00:00:00Z"),
            evc("Boulanger", "2026-04-01T00:00:00Z"),
            evc("Boulanger", "2026-05-01T00:00:00Z"),
        ];
        let rates = BilledRates {
            per_dimension: vec![
                (
                    value_set(&["France", "TDF"]).into_iter().collect(),
                    Decimal::from(100),
                ),
                (
                    value_set(&["France", "Boulanger"]).into_iter().collect(),
                    Decimal::from(200),
                ),
            ],
            fallback: None,
        };

        let annex = build_event_annex(&d, &events, &rates).expect("annex");
        let rows = &annex.table.groups[0].rows;
        let find = |client: &str| {
            rows.iter()
                .find(|r| r.cells[0] == InvoiceAnnexCell::Text(client.to_string()))
                .unwrap()
        };

        // Amounts are carried in minor units (rate 100 = 1.00 / unit); the renderer scales them.
        assert_eq!(
            find("TDF").cells[1],
            InvoiceAnnexCell::Number(Decimal::from(3))
        );
        assert_eq!(
            find("TDF").cells[2],
            InvoiceAnnexCell::Money(Decimal::from(300))
        ); // 3 × 100
        assert_eq!(
            find("Boulanger").cells[1],
            InvoiceAnnexCell::Number(Decimal::from(2))
        );
        assert_eq!(
            find("Boulanger").cells[2],
            InvoiceAnnexCell::Money(Decimal::from(400))
        ); // 2 × 200
        // Grand total amount = 300 + 400 minor units.
        let total = rows.iter().find(|r| r.emphasis).unwrap();
        assert_eq!(total.cells[2], InvoiceAnnexCell::Money(Decimal::from(700)));
    }
}
