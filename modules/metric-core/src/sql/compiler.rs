//! Compiler from the engine-agnostic metric model to ClickHouse SQL.

use crate::domain::WindowSize;
use crate::metric::{
    AggregateOp, AggregateSpec, BinOp, CohortDef, CohortPredicate, CohortReduction, CrossPeriod,
    Dimension, DimensionFilter, Expr, FactSource, FilterOp, GaugeOp, GaugeSpec, MetricDef,
    MetricError, MetricKind, MetricQuery, MetricRegistry, MetricSource, PropertyOp,
    PropertyValueType, ThresholdOp, TimeWeight, TimeWeightedSpec, UnaryOp,
};
use crate::sql::{
    BindValue, EventDedup, Fragment, LATEST_EVENT_ORDER, PropertyColumn, SafeQuery, dimension_alias,
};
use chrono::Utc;
use common_domain::ids::BaseId;

/// The ClickHouse dictionary mapping `customer_id` → customer attributes (country,
/// plan tier, custom metadata). Backed by `dim_customer`; loaded in memory so a
/// `dictGet` during `GROUP BY` is an O(1) lookup, not a join — the scalable way to
/// slice any metric by any customer attribute.
const CUSTOMER_DICT: &str = "dim_customer_dict";

/// The ClickHouse dictionary mapping `plan_id` → plan attributes (name). Backed by
/// `dim_plan`; a `dictGet` during `GROUP BY` turns the raw `plan_id` split into a
/// readable plan name without a join — the customer-dict pattern applied to plans.
const PLAN_DICT: &str = "dim_plan_dict";

/// The ClickHouse dictionary mapping `product_id` → product attributes (name). Backed
/// by `dim_product`; a `dictGet` during `GROUP BY` turns the raw `product_id` split
/// into a readable product name without a join — the plan-dict pattern applied to
/// products, so revenue can be collected per individual product, not just per plan.
const PRODUCT_DICT: &str = "dim_product_dict";

/// The all-zero UUID stand-in for a NULL plan/product key. `dictGet` rejects a
/// Nullable key argument outright (`CANNOT_INSERT_NULL_IN_ORDINARY_COLUMN`), so a
/// nullable id is coalesced to this sentinel — absent from every dictionary, it
/// resolves to the default below.
const NIL_UUID: &str = "toUUID('00000000-0000-0000-0000-000000000000')";

/// The breakdown label for rows whose plan/product id is NULL (e.g. one-time or
/// usage revenue lines with no plan/product lineage).
const NONE_LABEL: &str = "(none)";

/// The per-source context a [`Dimension`] resolves against.
struct DimCtx<'a> {
    /// The ClickHouse database holding the dimension dictionaries. dictGet resolves an
    /// unqualified dictionary name against the *session* database, which resets to `default`
    /// on the remote hop of a Distributed read — so every dictionary reference must be
    /// database-qualified or it fails there with `Dictionary not found`.
    database: &'a str,
    /// Table/sub-query qualifier prefix applied to native columns (e.g. `"f."`, or `""`).
    qualifier: &'a str,
    /// The customer-id column on this source (drives `CustomerAttr` lookups).
    customer_column: &'a str,
    /// The plan-id column on this source (drives `PlanAttr` lookups). Only valid on
    /// fact sources that carry it (the same constraint as the legacy `plan_id` split).
    plan_column: &'a str,
    /// The product-id column on this source (drives `ProductAttr` lookups). Only valid
    /// on fact sources that carry it (the revenue-item fact).
    product_column: &'a str,
    /// `true` for a structured fact source, `false` for the raw-events table.
    is_fact: bool,
}

/// The value SQL for a [`Dimension`] (no alias). Open-ended names are bound as
/// values (`properties[?]`) or allowlist-validated before inlining, so an
/// arbitrary user-chosen dimension can never inject SQL.
fn dim_value_sql(
    dim: &Dimension,
    ctx: &DimCtx,
    binds: &mut Vec<BindValue>,
) -> Result<String, MetricError> {
    match dim {
        // Source default: a native column on a fact, an event property on raw events.
        Dimension::Auto(n) if ctx.is_fact => {
            ensure_inlinable(n)?;
            Ok(format!("{}{n}", ctx.qualifier))
        }
        Dimension::Auto(n) | Dimension::Property(n) => {
            if matches!(dim, Dimension::Property(_)) && ctx.is_fact {
                return Err(MetricError::Invalid(
                    n.clone(),
                    "property dimensions need a usage (raw-event) metric".to_string(),
                ));
            }
            binds.push(BindValue::String(n.clone()));
            Ok("properties[?]".to_string())
        }
        Dimension::Column(n) => {
            ensure_inlinable(n)?;
            Ok(format!("{}{n}", ctx.qualifier))
        }
        Dimension::CustomerAttr(n) => {
            // dictGet's attribute name must be a constant string literal, so it is
            // inlined — allowlist-validated so it can't break out of the quotes. The
            // dictionary has a complex (UUID) key, so the id is passed as a tuple.
            ensure_inlinable(n)?;
            Ok(format!(
                "dictGet('{}.{CUSTOMER_DICT}', '{n}', tuple({}{}))",
                ctx.database, ctx.qualifier, ctx.customer_column
            ))
        }
        Dimension::PlanAttr(n) => {
            // The dim_plan dictionary exposes only `name`, so the dict attribute is
            // the constant 'name'; the carried `n` is the output dimension key/alias
            // (the breakdown key, e.g. "plan"), so the result column lands under the
            // key the caller asked for rather than the generic "name". Allowlist-
            // validated so it stays a safe SQL alias. plan_id is Nullable, so the key
            // is null-guarded (a raw Nullable key makes dictGet throw); NULL rows fall
            // into a labelled bucket via dictGetOrDefault.
            ensure_inlinable(n)?;
            Ok(format!(
                "dictGetOrDefault('{}.{PLAN_DICT}', 'name', tuple(ifNull({}{}, {NIL_UUID})), '{NONE_LABEL}')",
                ctx.database, ctx.qualifier, ctx.plan_column
            ))
        }
        Dimension::CustomerAttrMap { map_key, .. } => {
            // The customer `attributes` map is projected into the dictionary as a JSON
            // string (ClickHouse rejects Map as a dictionary attribute type), so a single
            // key is read with JSONExtractString over the constant 'attributes' attribute.
            // `map_key` is the attribute read from the map (inlined and allowlist-validated
            // so it can't break out of the string literal); the output column is aliased
            // from the dimension's `alias` elsewhere.
            ensure_inlinable(map_key)?;
            Ok(format!(
                "JSONExtractString(dictGet('{}.{CUSTOMER_DICT}', 'attributes', tuple({}{})), '{map_key}')",
                ctx.database, ctx.qualifier, ctx.customer_column
            ))
        }
        Dimension::CustomerProperty { key, .. } => {
            // Read as text whatever the stored JSON type: strings unquoted, other scalars
            // raw (`42`, `true`), and an absent or null key lands in the (none) bucket.
            let json = format!(
                "dictGet('{}.{CUSTOMER_DICT}', 'custom_properties', tuple({}{}))",
                ctx.database, ctx.qualifier, ctx.customer_column
            );
            binds.extend((0..4).map(|_| BindValue::String(key.clone())));
            Ok(format!(
                "multiIf(JSONType({json}, ?) = 'String', JSONExtractString({json}, ?), \
                 JSONType({json}, ?) = 'Null', '{NONE_LABEL}', JSONExtractRaw({json}, ?))"
            ))
        }
        Dimension::ProductAttr(n) => {
            // Same shape as PlanAttr but keyed on product_id, resolved via the
            // dim_product dictionary — collects a metric per individual product. The
            // dict exposes only `name`; `n` is the output key/alias (e.g. "product").
            // product_id is Nullable, so the key is null-guarded the same way.
            ensure_inlinable(n)?;
            Ok(format!(
                "dictGetOrDefault('{}.{PRODUCT_DICT}', 'name', tuple(ifNull({}{}, {NIL_UUID})), '{NONE_LABEL}')",
                ctx.database, ctx.qualifier, ctx.product_column
            ))
        }
        Dimension::CustomerName(n) => {
            // Per-customer split: the display value is the customer name from the
            // dim_customer dictionary, but grouping keys on customer_id (see
            // `entity_id_expr`), so distinct same-named customers stay distinct.
            ensure_inlinable(n)?;
            Ok(format!(
                "dictGetOrDefault('{}.{CUSTOMER_DICT}', 'name', tuple(ifNull({}{}, {NIL_UUID})), '{NONE_LABEL}')",
                ctx.database, ctx.qualifier, ctx.customer_column
            ))
        }
    }
}

/// The null-guarded id expression an [entity dimension](Dimension::is_entity) groups
/// on. Entity dimensions key on the id (plan/product/customer) so two entities that
/// share a display name remain distinct series; the name is only a label. `None` for
/// non-entity dimensions, which key on their own value.
fn entity_id_expr(dim: &Dimension, ctx: &DimCtx) -> Option<String> {
    let col = match dim {
        Dimension::PlanAttr(_) => ctx.plan_column,
        Dimension::ProductAttr(_) => ctx.product_column,
        Dimension::CustomerName(_) => ctx.customer_column,
        _ => return None,
    };
    Some(format!("ifNull({}{}, {NIL_UUID})", ctx.qualifier, col))
}

/// The GROUP BY expression for a dimension: the grouping id for entity dimensions,
/// else the dimension's own value.
fn dim_group_sql(
    dim: &Dimension,
    ctx: &DimCtx,
    binds: &mut Vec<BindValue>,
) -> Result<String, MetricError> {
    match entity_id_expr(dim, ctx) {
        Some(id) => Ok(id),
        None => dim_value_sql(dim, ctx, binds),
    }
}

/// The SELECT columns for a dimension: always its `dimension_alias` value column,
/// plus for entity dimensions a `_dis_` disambiguator column carrying the grouping id
/// so the backend can tell same-named entities apart.
fn dim_select_columns(
    dim: &Dimension,
    ctx: &DimCtx,
    binds: &mut Vec<BindValue>,
) -> Result<Vec<String>, MetricError> {
    let mut cols = vec![dim_select_sql(dim, ctx, binds)?];
    if let Some(id) = entity_id_expr(dim, ctx) {
        cols.push(format!(
            "toString({id}) AS {}",
            crate::sql::dimension_disambiguator_alias(dim)
        ));
    }
    Ok(cols)
}

/// `dim_value_sql` plus the stable `AS <alias>` the backend reads back.
fn dim_select_sql(
    dim: &Dimension,
    ctx: &DimCtx,
    binds: &mut Vec<BindValue>,
) -> Result<String, MetricError> {
    Ok(format!(
        "{} AS {}",
        dim_value_sql(dim, ctx, binds)?,
        dimension_alias(dim)
    ))
}

/// DoS cap: the time-weighted gauge compiles to a cross join, so an unbounded window count melts the cluster.
const MAX_GAUGE_WINDOWS: i64 = 100_000;

const MAX_WINDOWS: i64 = 100_000;

/// Only allow a conservative charset for inlined (not bound) identifiers; anything that could break out of a string literal is rejected.
fn ensure_inlinable(name: &str) -> Result<(), MetricError> {
    if name.is_empty() || name.len() > 128 {
        return Err(MetricError::Invalid(
            name.to_string(),
            "property name empty or too long".to_string(),
        ));
    }
    if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
    {
        Ok(())
    } else {
        Err(MetricError::Invalid(
            name.to_string(),
            "property name contains characters not allowed in an inlined identifier".to_string(),
        ))
    }
}

/// Stricter than ensure_inlinable: an unqualified identifier, so a `.`-qualified name (`system.tables`) can never be reached via a client-supplied source.
fn ensure_table_name(name: &str) -> Result<(), MetricError> {
    let mut chars = name.chars();
    let valid = name.len() <= 128
        && matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if valid {
        Ok(())
    } else {
        Err(MetricError::Invalid(
            name.to_string(),
            "table name must be an unqualified identifier (no schema, no '.')".to_string(),
        ))
    }
}

struct Series {
    sql: String,
    binds: Vec<BindValue>,
    dims: Vec<String>,
    /// Per-statement `SETTINGS`; applied to the outermost statement only (invalid inside the subquery a transform embeds `sql` as).
    settings: Vec<String>,
}

pub fn compile(
    registry: &MetricRegistry,
    query: &MetricQuery,
    events_table: &str,
    database: &str,
) -> Result<SafeQuery, MetricError> {
    registry.validate()?;
    let dedup = EventDedup::new(query.keep_last_events);
    let def = registry
        .get(&query.metric)
        .ok_or_else(|| MetricError::UnknownMetric(query.metric.clone()))?;

    if let Some(window) = &query.window {
        let span = (query.to.unwrap_or_else(Utc::now).timestamp() - query.from.timestamp()).max(0);
        let count = span / window_seconds(window) + 1;
        if count > MAX_WINDOWS {
            return Err(MetricError::Invalid(
                "window".to_string(),
                format!(
                    "window count {count} exceeds limit {MAX_WINDOWS}; use a coarser grain or shorter range"
                ),
            ));
        }
    }

    let cohort = query
        .cohort
        .as_ref()
        .map(|c| compile_cohort(registry, c, query, events_table, dedup))
        .transpose()?;

    // A carry-over gauge persists a balance: it must see windows before `from` to inherit an opening level, so it primes from epoch like a running total.
    let carry_over_gauge = matches!(
        &def.kind,
        MetricKind::Gauge(GaugeSpec {
            op: GaugeOp::Latest | GaugeOp::Max,
            carry_over: true,
            ..
        })
    ) && query.window.is_some();

    // Cross-period transforms read prior windows: build from an earlier `history_from`, then floor the output back to `>= from` after the window function.
    let history_from = query.window.as_ref().and_then(|w| match &query.transform {
        Some(CrossPeriod::RunningTotal) | Some(CrossPeriod::RunningTotalLagged) => {
            Some(chrono::DateTime::from_timestamp(0, 0).expect("epoch is valid"))
        }
        Some(CrossPeriod::PeriodOverPeriodDelta) | Some(CrossPeriod::PercentChange) => {
            Some(query.from - window_duration(w))
        }
        Some(CrossPeriod::MovingAverage { periods }) => {
            Some(query.from - window_duration(w) * periods.saturating_sub(1) as i32)
        }
        None if carry_over_gauge => {
            Some(chrono::DateTime::from_timestamp(0, 0).expect("epoch is valid"))
        }
        None => None,
    });

    let mut series = if let Some(history_from) = history_from {
        let mut history_query = query.clone();
        history_query.from = history_from;
        build_series(
            registry,
            def,
            &history_query,
            events_table,
            dedup,
            database,
            cohort.as_ref(),
        )?
    } else {
        build_series(
            registry,
            def,
            query,
            events_table,
            dedup,
            database,
            cohort.as_ref(),
        )?
    };

    // Forward-fill the gauge level across empty windows; the floor back to `>= from` happens here (a transform, if any, re-windows the already-filled series).
    if carry_over_gauge && let Some(w) = &query.window {
        series = carry_over_gauge_series(series, query, w);
    }

    let settings_suffix = if series.settings.is_empty() {
        String::new()
    } else {
        format!(" SETTINGS {}", series.settings.join(", "))
    };

    let mut final_query = match &query.transform {
        None => SafeQuery {
            sql: series.sql,
            binds: series.binds,
        },
        Some(transform) => {
            let floor_ts = history_from.map(|_| query.from.timestamp());
            // A running total is a continuous balance: gap-fill the visible range so the line carries the cumulative value across empty windows.
            let fill = match (transform, &query.window) {
                (CrossPeriod::RunningTotal, Some(w)) => Some(GapFill {
                    from_ts: query.from.timestamp(),
                    to_ts: query.to.unwrap_or_else(Utc::now).timestamp(),
                    interval: interval_sql(w).to_string(),
                    tz: tz_name(query),
                }),
                _ => None,
            };
            apply_cross_period(series, transform, floor_ts, fill)
        }
    };
    final_query.sql.push_str(&settings_suffix);
    Ok(final_query)
}

fn build_series(
    registry: &MetricRegistry,
    def: &MetricDef,
    query: &MetricQuery,
    events_table: &str,
    dedup: EventDedup,
    database: &str,
    cohort: Option<&CohortClause>,
) -> Result<Series, MetricError> {
    // Per-metric filters (DESIGN §6) AND into *this* metric's read only: clone the query with the
    // request-level filters extended by the def's own. A derived metric has no direct read, so its
    // filters (if any) don't apply here — each referenced sub-metric carries its own.
    let merged;
    let query = if def.filters.is_empty() || matches!(def.kind, MetricKind::Derived(_)) {
        query
    } else {
        let mut q = query.clone();
        q.filters.extend(def.filters.iter().cloned());
        merged = q;
        &merged
    };

    match &def.kind {
        MetricKind::Aggregate(spec) => match &def.source {
            MetricSource::RawEvents => simple_series(
                &def.event_code,
                ValueAgg::Aggregate(spec.clone()),
                query,
                events_table,
                dedup,
                database,
                cohort,
            ),
            MetricSource::Fact(fs) => fact_series(fs, spec, query, database, cohort),
        },
        MetricKind::Gauge(spec) => match &spec.op {
            GaugeOp::Latest | GaugeOp::Max => match &def.source {
                MetricSource::RawEvents => simple_series(
                    &def.event_code,
                    ValueAgg::Gauge(spec.clone()),
                    query,
                    events_table,
                    dedup,
                    database,
                    cohort,
                ),
                MetricSource::Fact(_) => Err(MetricError::Unsupported(
                    "gauge metrics require a raw-events source".to_string(),
                )),
            },
            GaugeOp::TimeWeighted(tw) => {
                time_weighted_series(&def.event_code, tw, query, events_table, dedup, cohort)
            }
        },
        MetricKind::Derived(expr) => {
            derived_series(registry, expr, query, events_table, dedup, database, cohort)
        }
    }
}

enum ValueAgg {
    Aggregate(AggregateSpec),
    Gauge(GaugeSpec),
}

fn interval_sql(window: &WindowSize) -> &'static str {
    match window {
        WindowSize::Minute => "toIntervalMinute(1)",
        WindowSize::Hour => "toIntervalHour(1)",
        WindowSize::Day => "toIntervalDay(1)",
        WindowSize::Week => "toIntervalWeek(1)",
    }
}

fn window_duration(window: &WindowSize) -> chrono::Duration {
    match window {
        WindowSize::Minute => chrono::Duration::minutes(1),
        WindowSize::Hour => chrono::Duration::hours(1),
        WindowSize::Day => chrono::Duration::days(1),
        WindowSize::Week => chrono::Duration::days(7),
    }
}

fn window_seconds(window: &WindowSize) -> i64 {
    match window {
        WindowSize::Minute => 60,
        WindowSize::Hour => 3_600,
        WindowSize::Day => 86_400,
        WindowSize::Week => 604_800,
    }
}

fn tz_name(query: &MetricQuery) -> String {
    query
        .window_time_zone
        .unwrap_or(chrono_tz::UTC)
        .name()
        .to_string()
}

fn dim_columns(query: &MetricQuery) -> Vec<String> {
    // A `customer_ids` scope is a WHERE filter only, never an output dimension —
    // otherwise a non-broken-down metric returns one row per (window × customer).
    query.group_by.iter().map(dimension_alias).collect()
}

/// The raw-events grouping context (no qualifier; properties live in the event map).
fn event_dim_ctx(database: &str) -> DimCtx<'_> {
    DimCtx {
        database,
        qualifier: "",
        customer_column: "customer_id",
        plan_column: "plan_id",
        product_column: "product_id",
        is_fact: false,
    }
}

fn simple_series(
    event_code: &str,
    agg: ValueAgg,
    query: &MetricQuery,
    events_table: &str,
    dedup: EventDedup,
    database: &str,
    cohort: Option<&CohortClause>,
) -> Result<Series, MetricError> {
    let mut select_binds: Vec<BindValue> = Vec::new();
    let mut where_binds: Vec<BindValue> = Vec::new();
    let mut group_binds: Vec<BindValue> = Vec::new();

    let tz = tz_name(query);
    let mut select_columns: Vec<String> = Vec::new();
    let mut group_columns: Vec<String> = Vec::new();

    if let Some(window) = &query.window {
        let interval = interval_sql(window);
        // `tumbleStart`/`tumbleEnd` return a `Date` (not `DateTime`) for week/month
        // intervals, which the JSON read-back can't parse as a timestamp — wrap in
        // `toDateTime(_, tz)` so every grain yields a `DateTime` uniformly.
        select_binds.push(BindValue::String(tz.clone()));
        select_binds.push(BindValue::String(tz.clone()));
        select_columns.push(format!(
            "toDateTime(tumbleStart(toDateTime(timestamp), {interval}, ?), ?) AS window_start"
        ));
        select_binds.push(BindValue::String(tz.clone()));
        select_binds.push(BindValue::String(tz.clone()));
        select_columns.push(format!(
            "toDateTime(tumbleEnd(toDateTime(timestamp), {interval}, ?), ?) AS window_end"
        ));
    } else {
        // Whole-range single bucket with constant bounds, so multiple metrics align.
        select_binds.push(BindValue::I64(query.from.timestamp()));
        select_columns.push("toDateTime(?) AS window_start".to_string());
        let end = query.to.unwrap_or_else(Utc::now).timestamp();
        select_binds.push(BindValue::I64(end));
        select_columns.push("toDateTime(?) AS window_end".to_string());
    }

    let value_column = match &agg {
        ValueAgg::Aggregate(spec) => aggregate_value_sql(spec, &mut select_binds)?,
        ValueAgg::Gauge(spec) => gauge_value_sql(spec, &mut select_binds)?,
    };
    select_columns.push(value_column);

    let dim_ctx = event_dim_ctx(database);
    for d in &query.group_by {
        select_columns.extend(dim_select_columns(d, &dim_ctx, &mut select_binds)?);
    }

    let mut conditions: Vec<String> = Vec::new();
    conditions.push("tenant_id = ?".to_string());
    where_binds.push(BindValue::Uuid(query.tenant_id.as_uuid()));
    conditions.push("code = ?".to_string());
    where_binds.push(BindValue::String(event_code.to_string()));
    conditions.push("timestamp >= toDateTime(?)".to_string());
    where_binds.push(BindValue::I64(query.from.timestamp()));
    if let Some(to) = query.to {
        conditions.push("timestamp <= toDateTime(?)".to_string());
        where_binds.push(BindValue::I64(to.timestamp()));
    }
    if !query.customer_ids.is_empty() {
        conditions.push("customer_id IN ?".to_string());
        where_binds.push(BindValue::Uuids(
            query.customer_ids.iter().map(|id| id.as_uuid()).collect(),
        ));
    }
    push_dimension_filters(&query.filters, &mut conditions, &mut where_binds);
    if let Some(value_prop) = value_property_of(&agg) {
        let col = PropertyColumn(value_prop);
        let p1 = col.path_sql(&mut where_binds);
        // CountDistinct counts distinct raw values, commonly non-numeric strings
        // (e.g. a user id), so it only requires the property to be present. Every
        // other value aggregation coerces through toFloat64, so it also guards on
        // numeric parseability — a guard that would wrongly drop every row here.
        if matches!(&agg, ValueAgg::Aggregate(s) if matches!(s.op, AggregateOp::CountDistinct)) {
            conditions.push(format!("{p1} != ''"));
        } else {
            let p2 = col.path_sql(&mut where_binds);
            conditions.push(format!("{p1} != '' AND isNotNull(toFloat64OrNull({p2}))"));
        }
    }
    if let Some(c) = cohort {
        conditions.push(format!("customer_id IN ( {} )", c.sql));
        where_binds.extend(c.binds.iter().cloned());
    }

    if let Some(window) = &query.window {
        let interval = interval_sql(window);
        group_binds.push(BindValue::String(tz.clone()));
        group_columns.push(format!("tumbleStart(toDateTime(timestamp), {interval}, ?)"));
        group_binds.push(BindValue::String(tz.clone()));
        group_columns.push(format!("tumbleEnd(toDateTime(timestamp), {interval}, ?)"));
    }
    for d in &query.group_by {
        group_columns.push(dim_group_sql(d, &dim_ctx, &mut group_binds)?);
    }

    let source = build_dedup_subquery(events_table, dedup, &conditions);

    let mut sql = format!("SELECT {}", select_columns.join(", "));
    sql.push_str(&format!(" FROM ( {source} )"));
    if !group_columns.is_empty() {
        sql.push_str(&format!(" GROUP BY {}", group_columns.join(", ")));
    }
    sql.push_str(" ORDER BY window_start");

    let mut binds = select_binds;
    binds.extend(where_binds);
    binds.extend(group_binds);

    Ok(Series {
        sql,
        binds,
        dims: dim_columns(query),
        settings: vec![],
    })
}

fn value_property_of(agg: &ValueAgg) -> Option<&str> {
    match agg {
        ValueAgg::Aggregate(spec) => {
            if matches!(spec.op, AggregateOp::Count) {
                None
            } else {
                spec.value_property.as_deref()
            }
        }
        ValueAgg::Gauge(spec) => spec.value_property.as_deref(),
    }
}

/// The ClickHouse function call for an [`AggregateOp`], with the function-name mapping in one
/// place. Call sites differ only in their expressions, passed explicitly:
/// - `count`: the full `Count` rendering (e.g. `toFloat64(count(*))` vs bare `count(*)`).
/// - `value`: the per-row numeric expression `sum`/`avg`/`min`/`max`/`argMax` aggregate over
///   (already wrapped in the call site's `toFloat64` vs `toFloat64OrZero` flavour).
/// - `latest_ts`: what `Latest` ranks by inside `argMax(value, ts)`.
/// - `distinct`: the `CountDistinct` argument for `uniq(...)` (often the raw column/path rather
///   than the float-coerced `value`).
fn aggregate_op_expr(
    op: AggregateOp,
    count: &str,
    value: &str,
    latest_ts: &str,
    distinct: &str,
) -> String {
    match op {
        AggregateOp::Count => count.to_string(),
        AggregateOp::Sum => format!("sum({value})"),
        AggregateOp::Avg => format!("avg({value})"),
        AggregateOp::Min => format!("min({value})"),
        AggregateOp::Max => format!("max({value})"),
        AggregateOp::Latest => format!("argMax({value}, {latest_ts})"),
        AggregateOp::CountDistinct => format!("uniq({distinct})"),
    }
}

/// The `*If`-suffixed form of [`aggregate_op_expr`]: each ClickHouse aggregate wrapped in
/// its conditional variant (`sumIf`/`countIf`/`argMaxIf`/`uniqIf`/…) so it reduces only the
/// rows where `predicate` holds. `metric_delta_set` uses it to compute the current- and
/// prior-window aggregates in one grouped scan. Args mirror `aggregate_op_expr`:
/// - `count_if`: the full `countIf(<predicate>)` rendering.
/// - `value_if`: the per-row numeric expression `sum`/`avg`/`min`/`max`/`argMax` aggregates.
/// - `ts_if`: what `argMaxIf` ranks by.
/// - `value_col`: the `uniqIf` distinct argument.
/// - `predicate`: the per-row window test each `*If` applies.
fn aggregate_op_if_expr(
    op: AggregateOp,
    count_if: &str,
    value_if: &str,
    ts_if: &str,
    value_col: &str,
    predicate: &str,
) -> String {
    match op {
        AggregateOp::Count => count_if.to_string(),
        AggregateOp::Sum => format!("sumIf({value_if}, {predicate})"),
        AggregateOp::Avg => format!("avgIf({value_if}, {predicate})"),
        AggregateOp::Min => format!("minIf({value_if}, {predicate})"),
        AggregateOp::Max => format!("maxIf({value_if}, {predicate})"),
        AggregateOp::Latest => format!("argMaxIf({value_if}, {ts_if}, {predicate})"),
        AggregateOp::CountDistinct => format!("uniqIf({value_col}, {predicate})"),
    }
}

fn aggregate_value_sql(
    spec: &AggregateSpec,
    binds: &mut Vec<BindValue>,
) -> Result<String, MetricError> {
    let value_expr = |binds: &mut Vec<BindValue>| -> Result<String, MetricError> {
        match &spec.value_property {
            Some(prop) => {
                let path = PropertyColumn(prop).path_sql(binds);
                Ok(format!("toFloat64OrZero({path})"))
            }
            None => Err(MetricError::Invalid(
                format!("{:?}", spec.op),
                "value_property is required".to_string(),
            )),
        }
    };

    // Count/CountDistinct yield an integer ClickHouse coerces to Float64 explicitly; the rest
    // already aggregate a Float64 expression. The `?`-pushing exprs are evaluated lazily so a
    // value-less op (Count) never binds a phantom property path.
    let expr = match spec.op {
        AggregateOp::Count => "toFloat64(count(*))".to_string(),
        AggregateOp::CountDistinct => {
            let prop = spec.value_property.as_deref().ok_or_else(|| {
                MetricError::Invalid(
                    "CountDistinct".to_string(),
                    "value_property is required".to_string(),
                )
            })?;
            let path = PropertyColumn(prop).path_sql(binds);
            format!("toFloat64(uniq({path}))")
        }
        _ => {
            let value = value_expr(binds)?;
            aggregate_op_expr(spec.op, "", &value, LATEST_EVENT_ORDER, "")
        }
    };
    Ok(format!("{} AS value", apply_unit_conversion(spec, expr)))
}

/// Divide an aggregate expression by the metric's unit-conversion factor, matching the
/// billing path (invoice line computation). A factor of `None` / ≤ 1 / non-finite is a
/// no-op — returns `expr` unchanged so unconverted metrics compile byte-identically. The
/// factor is inlined as a numeric literal (it is an `f64`, injection-safe) and pushes no
/// bind, so callers can wrap without disturbing bind order.
fn apply_unit_conversion(spec: &AggregateSpec, expr: String) -> String {
    match spec.unit_conversion_factor {
        Some(factor) if factor.is_finite() && factor > 1.0 => format!("({expr}) / {factor}"),
        _ => expr,
    }
}

fn gauge_value_sql(spec: &GaugeSpec, binds: &mut Vec<BindValue>) -> Result<String, MetricError> {
    let prop = spec.value_property.as_deref().ok_or_else(|| {
        MetricError::Invalid(
            "gauge".to_string(),
            "value_property is required".to_string(),
        )
    })?;
    let col = match spec.op {
        GaugeOp::Latest => {
            let path = PropertyColumn(prop).path_sql(binds);
            format!("argMax(toFloat64OrZero({path}), {LATEST_EVENT_ORDER}) AS value")
        }
        GaugeOp::Max => {
            let path = PropertyColumn(prop).path_sql(binds);
            format!("max(toFloat64OrZero({path})) AS value")
        }
        GaugeOp::TimeWeighted(_) => {
            return Err(MetricError::Unsupported(
                "time-weighted gauge is compiled by time_weighted_series".to_string(),
            ));
        }
    };
    Ok(col)
}

fn push_dimension_filters(
    filters: &[DimensionFilter],
    conditions: &mut Vec<String>,
    binds: &mut Vec<BindValue>,
) {
    for f in filters {
        if f.values.is_empty() {
            continue;
        }
        let col = PropertyColumn(&f.property);
        match f.op {
            FilterOp::In => {
                let ored = f
                    .values
                    .iter()
                    .map(|v| {
                        let path = col.path_sql(binds);
                        binds.push(BindValue::String(v.clone()));
                        format!("{path} = ?")
                    })
                    .collect::<Vec<_>>()
                    .join(" OR ");
                conditions.push(format!("({ored})"));
            }
            FilterOp::NotIn => {
                // Presence-required negation: the event must carry the property, and its
                // value must be none of `values`. Without mapContains, an absent key reads
                // as '' and would slip through every `!=`.
                binds.push(BindValue::String(f.property.clone()));
                let mut parts = vec!["mapContains(properties, ?)".to_string()];
                for v in &f.values {
                    let path = col.path_sql(binds);
                    binds.push(BindValue::String(v.clone()));
                    parts.push(format!("{path} != ?"));
                }
                conditions.push(format!("({})", parts.join(" AND ")));
            }
        }
    }
}

const METER_EVENT_COLUMNS: &str = "id, customer_id, timestamp, ingested_at, properties";

fn build_dedup_subquery(events_table: &str, dedup: EventDedup, conditions: &[String]) -> String {
    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };
    format!(
        "SELECT {METER_EVENT_COLUMNS} FROM {events_table} {where_clause}{}",
        dedup.limit_by_suffix()
    )
}

/// `FROM <events> WHERE <conditions>`, through the keep-last dedup subquery when it applies.
fn events_source(events_table: &str, dedup: EventDedup, conditions: &str) -> String {
    match dedup {
        EventDedup::LastIngested => format!(
            "FROM ( SELECT {METER_EVENT_COLUMNS} FROM {events_table} WHERE {conditions}{} )",
            dedup.limit_by_suffix()
        ),
        EventDedup::Off => format!("FROM {events_table} WHERE {conditions}"),
    }
}

fn time_weighted_series(
    event_code: &str,
    tw: &TimeWeightedSpec,
    query: &MetricQuery,
    events_table: &str,
    dedup: EventDedup,
    cohort: Option<&CohortClause>,
) -> Result<Series, MetricError> {
    let to = query.to.unwrap_or_else(Utc::now);
    let mut binds: Vec<BindValue> = Vec::new();

    // Validate every property name inlined (not bound) into the gauge SQL. The
    // time-weighted gauge reads from the raw-event property map, so only property
    // dimensions apply (column/customer-attribute splits aren't meaningful here).
    ensure_inlinable(&tw.identity_property)?;
    for d in &query.group_by {
        match d {
            Dimension::Auto(n) | Dimension::Property(n) => ensure_inlinable(n)?,
            Dimension::Column(_)
            | Dimension::CustomerAttr(_)
            | Dimension::PlanAttr(_)
            | Dimension::CustomerAttrMap { .. }
            | Dimension::CustomerProperty { .. }
            | Dimension::ProductAttr(_)
            | Dimension::CustomerName(_) => {
                return Err(MetricError::Unsupported(format!(
                    "dimension '{}' is not supported on time-weighted metrics",
                    d.name()
                )));
            }
        }
    }
    if let TimeWeight::ValueSeconds(prop) = &tw.weight {
        ensure_inlinable(prop)?;
    }

    let (window_function, seconds_in_interval) = match &query.window {
        Some(WindowSize::Minute) => ("toStartOfMinute", "60".to_string()),
        Some(WindowSize::Hour) => ("toStartOfHour", "3600".to_string()),
        Some(WindowSize::Day) => ("toStartOfDay", "86400".to_string()),
        Some(WindowSize::Week) => ("toStartOfWeek", "604800".to_string()),
        None => (
            "toStartOfSecond",
            (to.timestamp() - query.from.timestamp()).max(1).to_string(),
        ),
    };

    // Bound the cross-join blast radius (see MAX_GAUGE_WINDOWS).
    let interval_secs: i64 = seconds_in_interval.parse().unwrap_or(1).max(1);
    let span_secs = (to.timestamp() - query.from.timestamp()).max(0);
    let window_count = span_secs / interval_secs + 1;
    if window_count > MAX_GAUGE_WINDOWS {
        return Err(MetricError::Invalid(
            "time_weighted".to_string(),
            format!(
                "window count {window_count} exceeds limit {MAX_GAUGE_WINDOWS}; use a coarser grain or shorter range"
            ),
        ));
    }

    let identity_access = map_access(&tw.identity_property);
    let group_dim_selects = query
        .group_by
        .iter()
        .map(|d| format!(", {} AS {}", map_access(d.name()), dimension_alias(d)))
        .collect::<String>();
    let group_dim_passthrough = query
        .group_by
        .iter()
        .map(|d| format!(", el.{}", dimension_alias(d)))
        .collect::<String>();
    let group_dim_final_select = query
        .group_by
        .iter()
        .map(|d| format!(", ip.{0} AS {0}", dimension_alias(d)))
        .collect::<String>();
    let group_dim_final_groupby = query
        .group_by
        .iter()
        .map(|d| format!(", ip.{}", dimension_alias(d)))
        .collect::<String>();

    let (weight_value_select, weight_expr) = match &tw.weight {
        TimeWeight::Seconds => (String::new(), "1".to_string()),
        TimeWeight::ValueSeconds(prop) => (
            format!(", {} AS weight_value", map_access(prop)),
            "toFloat64OrZero(el.weight_value)".to_string(),
        ),
    };

    binds.push(BindValue::I64(query.from.timestamp()));
    binds.push(BindValue::I64(to.timestamp()));
    binds.push(BindValue::Uuid(query.tenant_id.as_uuid()));

    let (code_filter, code_bind) = if tw.lifecycle_codes.is_empty() {
        ("code = ?", BindValue::String(event_code.to_string()))
    } else {
        ("code IN ?", BindValue::Strings(tw.lifecycle_codes.clone()))
    };
    // Keep-last dedups before the window function, scoped to the tenant so the sort stays small.
    let lifecycle_source = match dedup {
        EventDedup::LastIngested => {
            binds.push(BindValue::Uuid(query.tenant_id.as_uuid()));
            binds.push(code_bind);
            format!(
                "( SELECT id, customer_id, tenant_id, timestamp, properties FROM {events_table} \
                 WHERE tenant_id = ? AND {code_filter}{} )",
                dedup.limit_by_suffix()
            )
        }
        EventDedup::Off => {
            binds.push(code_bind);
            format!("{events_table}\n        WHERE {code_filter}")
        }
    };

    let mut customer_param = if query.customer_ids.is_empty() {
        String::new()
    } else {
        binds.push(BindValue::Uuids(
            query.customer_ids.iter().map(|id| id.as_uuid()).collect(),
        ));
        " AND el.customer_id IN ?".to_string()
    };
    if let Some(c) = cohort {
        customer_param.push_str(&format!(" AND el.customer_id IN ( {} )", c.sql));
        binds.extend(c.binds.iter().cloned());
    }

    let sql = format!(
        r#"WITH
    params AS (
        SELECT toDateTime(?) AS window_start, toDateTime(?) AS window_end, ? AS param_tenant_id
    ),
    event_lifecycle AS (
        SELECT
            customer_id,
            tenant_id,
            {identity_access} AS identity,
            timestamp AS start_time,
            leadInFrame(toNullable(timestamp)) OVER (
                PARTITION BY customer_id, tenant_id, {identity_access}
                ORDER BY timestamp
                ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING
            ) AS end_time{group_dim_selects}{weight_value_select}
        FROM {lifecycle_source}
    ),
    instance_periods AS (
        SELECT
            el.customer_id,
            greatest(el.start_time, params.window_start) AS period_start,
            least(COALESCE(el.end_time, params.window_end), params.window_end) AS period_end,
            {weight_expr} AS w{group_dim_passthrough}
        FROM event_lifecycle el
        CROSS JOIN params
        WHERE el.tenant_id = params.param_tenant_id{customer_param}
          AND el.start_time < params.window_end
          AND (el.end_time > params.window_start OR el.end_time IS NULL)
    ),
    time_series AS (
        SELECT
            toDateTime(toDateTime({window_function}(window_start)) + number * {seconds_in_interval}) AS window_start,
            toDateTime(toDateTime({window_function}(window_start)) + (number + 1) * {seconds_in_interval}) AS window_end
        FROM params
        ARRAY JOIN range(toUInt32((toUnixTimestamp(window_end) - toUnixTimestamp({window_function}(window_start))) / {seconds_in_interval}) + 1) AS number
    )
SELECT
    ts.window_start AS window_start,
    ts.window_end AS window_end,
    sum(
        if(ip.period_start < ts.window_end AND ip.period_end > ts.window_start,
           dateDiff('second', greatest(ts.window_start, ip.period_start), least(ts.window_end, ip.period_end)) * ip.w,
           0)
    ) AS value,
    ip.customer_id AS customer_id{group_dim_final_select}
FROM time_series ts
CROSS JOIN instance_periods ip
WHERE ip.period_start < ts.window_end AND ip.period_end > ts.window_start
GROUP BY ts.window_start, ts.window_end, ip.customer_id{group_dim_final_groupby}
ORDER BY window_start"#,
    );

    let mut dims = vec!["customer_id".to_string()];
    for d in &query.group_by {
        dims.push(dimension_alias(d));
    }

    Ok(Series {
        sql,
        binds,
        dims,
        settings: vec![],
    })
}

fn map_access(prop: &str) -> String {
    let escaped = prop.replace('\'', "''");
    format!("properties['{escaped}']")
}

/// Render dimension filters as fact-column conditions, one string per filter. Shared by the
/// read's `WHERE` and by the per-operand `*If` predicates a fused read isolates its members with.
fn fact_filter_conditions(filters: &[DimensionFilter], binds: &mut Vec<BindValue>) -> Vec<String> {
    let mut out = Vec::new();
    for f in filters {
        if f.values.is_empty() {
            continue;
        }
        let (sep, cmp) = match f.op {
            FilterOp::In => (" OR ", "="),
            // Fact columns are always present, so negation is a plain per-value !=.
            FilterOp::NotIn => (" AND ", "!="),
        };
        let joined = f
            .values
            .iter()
            .map(|v| {
                binds.push(BindValue::String(v.clone()));
                format!("{} {cmp} ?", f.property)
            })
            .collect::<Vec<_>>()
            .join(sep);
        out.push(format!("({joined})"));
    }
    out
}

/// Whether filters actually restrict anything. A value-less filter is skipped when rendered, so
/// it must not count towards a member being "filtered" either.
fn has_effective_filters(filters: &[DimensionFilter]) -> bool {
    filters.iter().any(|f| !f.values.is_empty())
}

/// One operand of a fused fact read: the column it lands in and the per-metric filters that
/// isolate it. A lone member with no filters is the ordinary single-metric read, and emits
/// byte-identical SQL to the unfused path.
struct FusedMember<'a> {
    column: String,
    filters: &'a [DimensionFilter],
}

fn fact_series(
    fs: &FactSource,
    spec: &AggregateSpec,
    query: &MetricQuery,
    database: &str,
    cohort: Option<&CohortClause>,
) -> Result<Series, MetricError> {
    let lone = [FusedMember {
        column: "value".to_string(),
        filters: &[],
    }];
    fact_series_fused(fs, spec, query, database, cohort, &lone)
}

/// The fact read behind one or more operands. Operands sharing a fact source, aggregate and
/// request scope differ only by their per-metric filters, so one grouped scan can emit all of
/// them as `*If` aggregates — replacing N scans, N FX joins and N-1 outer joins with one.
fn fact_series_fused(
    fs: &FactSource,
    spec: &AggregateSpec,
    query: &MetricQuery,
    database: &str,
    cohort: Option<&CohortClause>,
    members: &[FusedMember<'_>],
) -> Result<Series, MetricError> {
    // Every identifier is inlined — allowlist-validate them all.
    ensure_table_name(&fs.table)?;
    for ident in [
        &fs.value_column,
        &fs.timestamp_column,
        &fs.customer_column,
        &fs.tenant_column,
    ] {
        ensure_inlinable(ident)?;
    }
    // group_by dimensions are validated as they're resolved (see dim_value_sql).
    for f in &query.filters {
        ensure_inlinable(&f.property)?;
    }

    let ts = &fs.timestamp_column;
    let cust = &fs.customer_column;
    let valcol = &fs.value_column;

    let mut select_binds: Vec<BindValue> = Vec::new();
    let mut where_binds: Vec<BindValue> = Vec::new();
    // FX build-side binds; emitted between the fact `WHERE` and the `GROUP BY`, so ordered there too.
    let mut fx_binds: Vec<BindValue> = Vec::new();
    let mut group_binds: Vec<BindValue> = Vec::new();

    // FX: convert each USD money row at its own date's historical rate BEFORE aggregating, so a running total accumulates already-converted deltas.
    const FX_TABLE: &str = "dim_fx_rate";
    let fx_active = matches!(
        spec.op,
        AggregateOp::Sum
            | AggregateOp::Avg
            | AggregateOp::Min
            | AggregateOp::Max
            | AggregateOp::Latest
    ) && valcol.ends_with("_usd")
        && query
            .reporting_currency
            .as_deref()
            .is_some_and(|c| !c.is_empty() && c != "USD");
    let q = if fx_active { "f." } else { "" };
    // Native-currency column paired with the USD one: a row already in the reporting currency is summed natively to avoid sub-cent FX drift.
    let native_col = valcol.strip_suffix("_usd").unwrap_or(valcol.as_str());
    // A daily-snapshot (stock) fact: a bucket must report its latest day's level, not
    // the sum of every day in it. Drives the two-level reduction at the assembly below.
    let snapshot = fs.snapshot;

    let tz = tz_name(query);
    let mut select_columns: Vec<String> = Vec::new();
    let mut group_columns: Vec<String> = Vec::new();

    if let Some(window) = &query.window {
        let interval = interval_sql(window);
        // `tumbleStart`/`tumbleEnd` return a `Date` (not `DateTime`) for week/month
        // intervals, which the JSON read-back can't parse as a timestamp — wrap in
        // `toDateTime(_, tz)` so every grain yields a `DateTime` uniformly.
        select_binds.push(BindValue::String(tz.clone()));
        select_binds.push(BindValue::String(tz.clone()));
        select_columns.push(format!(
            "toDateTime(tumbleStart(toDateTime({q}{ts}), {interval}, ?), ?) AS window_start"
        ));
        select_binds.push(BindValue::String(tz.clone()));
        select_binds.push(BindValue::String(tz.clone()));
        select_columns.push(format!(
            "toDateTime(tumbleEnd(toDateTime({q}{ts}), {interval}, ?), ?) AS window_end"
        ));
    } else {
        select_binds.push(BindValue::I64(query.from.timestamp()));
        select_columns.push("toDateTime(?) AS window_start".to_string());
        let end = query.to.unwrap_or_else(Utc::now).timestamp();
        select_binds.push(BindValue::I64(end));
        select_columns.push("toDateTime(?) AS window_end".to_string());
    }

    let value_expr = if fx_active {
        format!(
            "if(f.currency = f.fx_currency, toFloat64(f.{native_col}), toFloat64(f.{valcol}) * fx.rate_from_usd)"
        )
    } else {
        format!("toFloat64({valcol})")
    };
    // A filtered member reduces only its own rows via `*If` — the isolation the unfused path gets
    // from a per-metric `WHERE`. These predicates sit in the SELECT, so their binds go there too.
    for m in members {
        for f in m.filters {
            ensure_inlinable(&f.property)?;
        }
        let predicate = {
            let conds = fact_filter_conditions(m.filters, &mut select_binds);
            (!conds.is_empty()).then(|| conds.join(" AND "))
        };
        let col = &m.column;
        let value_col = match (&predicate, spec.op) {
            (None, AggregateOp::Count) => format!("toFloat64(count(*)) AS {col}"),
            (None, AggregateOp::CountDistinct) => format!("toFloat64(uniq({valcol})) AS {col}"),
            (None, _) => format!(
                "{} AS {col}",
                aggregate_op_expr(
                    spec.op,
                    "",
                    &value_expr,
                    &format!("toDateTime({q}{ts})"),
                    ""
                )
            ),
            (Some(p), AggregateOp::Count) => format!("toFloat64(countIf({p})) AS {col}"),
            (Some(p), AggregateOp::CountDistinct) => {
                format!("toFloat64(uniqIf({valcol}, {p})) AS {col}")
            }
            (Some(p), _) => format!(
                "{} AS {col}",
                aggregate_op_if_expr(
                    spec.op,
                    &format!("countIf({p})"),
                    &value_expr,
                    &format!("toDateTime({q}{ts})"),
                    valcol,
                    p,
                )
            ),
        };
        select_columns.push(value_col);
    }

    let fact_ctx = DimCtx {
        database,
        qualifier: q,
        customer_column: cust,
        plan_column: "plan_id",
        product_column: "product_id",
        is_fact: true,
    };
    for d in &query.group_by {
        select_columns.extend(dim_select_columns(d, &fact_ctx, &mut select_binds)?);
    }
    // Carry each row's snapshot day so the outer reduction can keep the bucket's latest.
    if snapshot {
        select_columns.push(format!("toDate({q}{ts}) AS _snap_day"));
    }

    let mut conditions: Vec<String> = Vec::new();
    conditions.push(format!("{} = ?", fs.tenant_column));
    where_binds.push(BindValue::Uuid(query.tenant_id.as_uuid()));
    conditions.push(format!("toDateTime({ts}) >= toDateTime(?)"));
    where_binds.push(BindValue::I64(query.from.timestamp()));
    if let Some(to) = query.to {
        conditions.push(format!("toDateTime({ts}) <= toDateTime(?)"));
        where_binds.push(BindValue::I64(to.timestamp()));
    }
    if !query.customer_ids.is_empty() {
        conditions.push(format!("{cust} IN ?"));
        where_binds.push(BindValue::Uuids(
            query.customer_ids.iter().map(|id| id.as_uuid()).collect(),
        ));
    }
    conditions.extend(fact_filter_conditions(&query.filters, &mut where_binds));
    // Narrow the read to the union of the members' rows; the `*If` predicates above split that
    // union back out per operand. Skipped when any member is unfiltered — its union is everything,
    // which is also what keeps the lone-member path byte-identical to the unfused read.
    if !members.is_empty() && members.iter().all(|m| has_effective_filters(m.filters)) {
        let mut ored = Vec::new();
        for m in members {
            let mut conds = fact_filter_conditions(m.filters, &mut where_binds);
            // Each condition is already parenthesised; only a multi-filter member needs grouping.
            ored.push(if conds.len() > 1 {
                format!("({})", conds.join(" AND "))
            } else {
                conds.remove(0)
            });
        }
        conditions.push(if ored.len() > 1 {
            format!("({})", ored.join(" OR "))
        } else {
            ored.remove(0)
        });
    }
    if let Some(c) = cohort {
        conditions.push(format!("{cust} IN ( {} )", c.sql));
        where_binds.extend(c.binds.iter().cloned());
    }

    if let Some(window) = &query.window {
        let interval = interval_sql(window);
        group_binds.push(BindValue::String(tz.clone()));
        group_binds.push(BindValue::String(tz.clone()));
        group_columns.push(format!(
            "toDateTime(tumbleStart(toDateTime({q}{ts}), {interval}, ?), ?)"
        ));
        group_binds.push(BindValue::String(tz.clone()));
        group_binds.push(BindValue::String(tz.clone()));
        group_columns.push(format!(
            "toDateTime(tumbleEnd(toDateTime({q}{ts}), {interval}, ?), ?)"
        ));
    }
    for d in &query.group_by {
        group_columns.push(dim_group_sql(d, &fact_ctx, &mut group_binds)?);
    }
    // Snapshot: total per (bucket, day) first, so the outer wrap can pick the latest day.
    if snapshot {
        group_columns.push(format!("toDate({q}{ts})"));
    }

    // FINAL: fact sources are ReplacingMergeTree(version); the read must dedup to the latest version or aggregates double-count.
    let mut sql = format!("SELECT {}", select_columns.join(", "));
    if fx_active {
        let mut inner_cols: Vec<&str> = vec![valcol.as_str(), native_col, "currency", ts.as_str()];
        // A customer-attribute split is computed via dictGet(customer_id) in the
        // outer query, so the dedup subquery must carry the customer column even
        // when no explicit customer scope was requested.
        let needs_customer = !query.customer_ids.is_empty()
            || query.group_by.iter().any(|d| {
                matches!(
                    d,
                    Dimension::CustomerAttr(_)
                        | Dimension::CustomerAttrMap { .. }
                        | Dimension::CustomerProperty { .. }
                        | Dimension::CustomerName(_)
                )
            });
        if needs_customer {
            inner_cols.push(cust.as_str());
        }
        for d in &query.group_by {
            // CustomerAttr derives from the customer column (already carried);
            // PlanAttr/ProductAttr derive from plan_id/product_id, so carry those
            // rather than the attribute name; native column/auto dimensions must
            // survive the dedup projection.
            match d {
                Dimension::CustomerAttr(_)
                | Dimension::CustomerAttrMap { .. }
                | Dimension::CustomerProperty { .. }
                | Dimension::CustomerName(_) => {}
                Dimension::PlanAttr(_) => inner_cols.push("plan_id"),
                Dimension::ProductAttr(_) => inner_cols.push("product_id"),
                _ => inner_cols.push(d.name()),
            }
        }
        // A fused member's `*If` predicate is evaluated in the OUTER select, above this dedup
        // projection — so the columns it tests have to survive it.
        for m in members {
            for f in m.filters {
                if !f.values.is_empty() {
                    inner_cols.push(f.property.as_str());
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        let inner_select = inner_cols
            .into_iter()
            .filter(|c| seen.insert(*c))
            .collect::<Vec<_>>()
            .join(", ");
        // Restrict the ASOF *build* side: `fx_currency` is constant but appears only in the join
        // condition, so the bare table hashes every currency and every date in history, per CTE.
        // No lower date bound — rates are sparse before ~2024 and history-primed operands read
        // from epoch, so clipping the low end could drop a row's match; the upper bound is safe
        // because `fx.date <= toDate(f.at) <= toDate(to)`. FINAL because duplicate (currency, date)
        // keys with conflicting rates otherwise let the ASOF match pick a stale one.
        let mut fx_conditions = vec!["currency = ?".to_string()];
        fx_binds.push(BindValue::String(
            query.reporting_currency.clone().unwrap_or_default(),
        ));
        if let Some(to) = query.to {
            fx_conditions.push("date <= toDate(toDateTime(?))".to_string());
            fx_binds.push(BindValue::I64(to.timestamp()));
        }
        sql.push_str(&format!(
            " FROM ( SELECT {}, toLowCardinality(?) AS fx_currency FROM {} FINAL WHERE {} ) AS f \
             ASOF LEFT JOIN ( SELECT currency, date, rate_from_usd FROM {} FINAL WHERE {} ) AS fx \
             ON f.fx_currency = fx.currency AND fx.date <= toDate(f.{})",
            inner_select,
            fs.table,
            conditions.join(" AND "),
            FX_TABLE,
            fx_conditions.join(" AND "),
            ts,
        ));
    } else {
        sql.push_str(&format!(
            " FROM {} FINAL WHERE {}",
            fs.table,
            conditions.join(" AND ")
        ));
    }
    if !group_columns.is_empty() {
        sql.push_str(&format!(" GROUP BY {}", group_columns.join(", ")));
    }

    // A snapshot (point-in-time daily level) must not sum a multi-day bucket's rows —
    // that multiplies the standing balance by the number of days. The read above already
    // reduces ACROSS entities per snapshot day (grouped by `_snap_day`); wrap it so each
    // bucket keeps only its LATEST day's total (`argMax` by that day). A one-day bucket has
    // a single day, so the argMax is a no-op there and day-grain values are unchanged.
    if snapshot {
        let mut carried: Vec<String> = Vec::new();
        for d in &query.group_by {
            carried.push(dimension_alias(d));
            if entity_id_expr(d, &fact_ctx).is_some() {
                carried.push(crate::sql::dimension_disambiguator_alias(d));
            }
        }
        let tail: String = carried.iter().map(|c| format!(", {c}")).collect();
        sql = format!(
            "SELECT window_start, window_end, argMax(value, _snap_day) AS value{tail} \
             FROM ( {sql} ) GROUP BY window_start, window_end{tail}"
        );
    }
    sql.push_str(" ORDER BY window_start");

    let mut binds = select_binds;
    if fx_active {
        binds.push(BindValue::String(
            query.reporting_currency.clone().unwrap_or_default(),
        ));
    }
    binds.extend(where_binds);
    binds.extend(fx_binds);
    binds.extend(group_binds);

    Ok(Series {
        sql,
        binds,
        dims: dim_columns(query),
        settings: vec![],
    })
}

struct CohortClause {
    sql: String,
    binds: Vec<BindValue>,
}

fn fmt_threshold(v: f64) -> Result<String, MetricError> {
    // Reject NaN/inf: they render as literals whose comparisons are always false, i.e. a silently empty cohort.
    if !v.is_finite() {
        return Err(MetricError::Invalid(
            "threshold".to_string(),
            format!("threshold must be a finite number, got {v}"),
        ));
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Ok(format!("{}", v as i64))
    } else {
        Ok(format!("{v}"))
    }
}

/// Compile a cohort to a `SELECT DISTINCT customer_id` over the INTERSECT of its
/// predicates — the *member set*, not a metric series filtered by it. Tenant scope and
/// the time window come from `query` (`tenant_id`/`from`/`to`); `metric`/`group_by`/etc.
/// are ignored. This is the engine half of `analytics::segment::resolve_segment_members`.
pub fn compile_cohort_members(
    registry: &MetricRegistry,
    cohort: &CohortDef,
    query: &MetricQuery,
    events_table: &str,
) -> Result<SafeQuery, MetricError> {
    registry.validate()?;
    let dedup = EventDedup::new(query.keep_last_events);
    if cohort.predicates.is_empty() {
        return Err(MetricError::Invalid(
            "cohort".to_string(),
            "a cohort needs at least one predicate".to_string(),
        ));
    }
    // Each predicate projects a single id column under varying names (`id`,
    // `customer_id`); alias every branch to `customer_id` so the INTERSECT and the
    // outer projection are name-stable for the backend's JSONEachRow read.
    let mut selects = Vec::new();
    let mut binds: Vec<BindValue> = Vec::new();
    for p in &cohort.predicates {
        let (sql, mut b) = compile_cohort_predicate(registry, p, query, events_table, dedup)?;
        selects.push(format!("( SELECT customer_id FROM ( {sql} ) AS p )"));
        binds.append(&mut b);
    }
    Ok(SafeQuery {
        sql: format!(
            "SELECT DISTINCT customer_id FROM ( {} ) AS members",
            selects.join(" INTERSECT ")
        ),
        binds,
    })
}

fn compile_cohort(
    registry: &MetricRegistry,
    cohort: &CohortDef,
    query: &MetricQuery,
    events_table: &str,
    dedup: EventDedup,
) -> Result<CohortClause, MetricError> {
    if cohort.predicates.is_empty() {
        return Err(MetricError::Invalid(
            "cohort".to_string(),
            "a cohort needs at least one predicate".to_string(),
        ));
    }
    let mut selects = Vec::new();
    let mut binds: Vec<BindValue> = Vec::new();
    for p in &cohort.predicates {
        let (sql, mut b) = compile_cohort_predicate(registry, p, query, events_table, dedup)?;
        selects.push(format!("( {sql} )"));
        binds.append(&mut b);
    }
    Ok(CohortClause {
        sql: selects.join(" INTERSECT "),
        binds,
    })
}

fn compile_cohort_predicate(
    registry: &MetricRegistry,
    predicate: &CohortPredicate,
    query: &MetricQuery,
    events_table: &str,
    dedup: EventDedup,
) -> Result<(String, Vec<BindValue>), MetricError> {
    match predicate {
        CohortPredicate::Dimension {
            table,
            id_column,
            column,
            values,
        } => {
            ensure_table_name(table)?;
            for ident in [id_column, column] {
                ensure_inlinable(ident)?;
            }
            let mut binds = vec![BindValue::Uuid(query.tenant_id.as_uuid())];
            binds.push(BindValue::Strings(values.clone()));
            let sql = format!(
                "SELECT {id_column} AS customer_id FROM {table} FINAL WHERE tenant_id = ? AND {column} IN ?"
            );
            Ok((sql, binds))
        }
        CohortPredicate::DimensionMap {
            table,
            id_column,
            map_column,
            key,
            values,
        } => {
            ensure_table_name(table)?;
            for ident in [id_column, map_column] {
                ensure_inlinable(ident)?;
            }
            let binds = vec![
                BindValue::Uuid(query.tenant_id.as_uuid()),
                BindValue::String(key.clone()),
                BindValue::Strings(values.clone()),
            ];
            // `key` is bound into the map subscript, so an arbitrary property name is
            // matched without ever being inlined as SQL.
            let sql = format!(
                "SELECT {id_column} AS customer_id FROM {table} FINAL WHERE tenant_id = ? AND {map_column}[?] IN ?"
            );
            Ok((sql, binds))
        }
        CohortPredicate::CustomProperty {
            table,
            id_column,
            json_column,
            key,
            op,
            value_type,
            values,
        } => {
            ensure_table_name(table)?;
            for ident in [id_column, json_column] {
                ensure_inlinable(ident)?;
            }
            let mut binds = vec![BindValue::Uuid(query.tenant_id.as_uuid())];
            let cond =
                custom_property_condition(json_column, key, *op, *value_type, values, &mut binds)?;
            let sql = format!(
                "SELECT {id_column} AS customer_id FROM {table} FINAL WHERE tenant_id = ? AND {cond}"
            );
            Ok((sql, binds))
        }
        CohortPredicate::ActivePlan { plan_ids } => {
            // Per subscription: the latest plan, and the lifecycle delta of its latest
            // state-changing event (+1 active / -1 left). Active on plan = delta is +1.
            let binds = vec![
                BindValue::Uuid(query.tenant_id.as_uuid()),
                BindValue::Strings(plan_ids.clone()),
            ];
            let sql = "SELECT customer_id FROM (\
                 SELECT subscription_id, any(customer_id) AS customer_id, \
                        argMax(plan_id, at) AS _plan, \
                        argMaxIf(lifecycle_delta, at, lifecycle_delta != 0) AS _state \
                   FROM fact_subscription_event FINAL WHERE tenant_id = ? \
                   GROUP BY subscription_id) \
                 WHERE _state = 1 AND toString(_plan) IN ?"
                .to_string();
            Ok((sql, binds))
        }
        CohortPredicate::MetricThreshold {
            metric,
            op,
            threshold,
        } => metric_active_set(
            registry,
            metric,
            *op,
            *threshold,
            query,
            events_table,
            dedup,
        ),
        CohortPredicate::MetricInactive { metric, max_active } => {
            // Anti-join the active set against the tenant customer universe: a metric
            // a customer never emits leaves no group row, so the active set can't see
            // who's missing — `dim_customer` MINUS active is the only way.
            let (active_sql, active_binds) = metric_active_set(
                registry,
                metric,
                ThresholdOp::Gt,
                max_active.unwrap_or(0.0),
                query,
                events_table,
                dedup,
            )?;
            let mut binds = vec![BindValue::Uuid(query.tenant_id.as_uuid())];
            binds.extend(active_binds);
            let sql = format!(
                "SELECT id AS customer_id FROM dim_customer FINAL WHERE tenant_id = ? \
                 AND id NOT IN ( {active_sql} )"
            );
            Ok((sql, binds))
        }
        CohortPredicate::MetricDelta {
            metric,
            op,
            pct,
            prior_window_days,
        } => metric_delta_set(
            registry,
            metric,
            *op,
            *pct,
            *prior_window_days,
            query,
            events_table,
            dedup,
        ),
        CohortPredicate::And { children } => {
            compile_cohort_set_op(registry, children, query, events_table, dedup, "INTERSECT")
        }
        CohortPredicate::Or { children } => compile_cohort_set_op(
            registry,
            children,
            query,
            events_table,
            dedup,
            "UNION DISTINCT",
        ),
        CohortPredicate::Not { child } => {
            let (child_sql, mut child_binds) =
                compile_cohort_predicate(registry, child, query, events_table, dedup)?;
            let mut binds = vec![BindValue::Uuid(query.tenant_id.as_uuid())];
            binds.append(&mut child_binds);
            let sql = format!(
                "SELECT id AS customer_id FROM dim_customer FINAL WHERE tenant_id = ? \
                 AND id NOT IN ( {child_sql} )"
            );
            Ok((sql, binds))
        }
    }
}

/// Build the typed `WHERE` condition for a [`CohortPredicate::CustomProperty`] leaf, pushing its
/// binds in placeholder order. `json_column` is already inlined-validated; `key` and the
/// comparison values are always bound.
///
/// The native `JSON` column is read via `toString(col)` + `JSONExtract*`: this is the
/// version-robust path for a *dynamic* (bound) key — native JSON path access (`col.key`) needs a
/// literal path. `value_type` picks the accessor so a NUMBER compares as `Float64` and a DATE as
/// an ISO string (lexically ordered), not the lossy stringified compare the legacy map leaf did.
/// Numeric literals are bound as strings and cast in-SQL (`toFloat64(?)`), avoiding a new
/// [`BindValue`] variant; the wire boundary validates they parse. Comparisons that a missing key
/// would spuriously satisfy (`= 0`, `!= x`, `< date`) are guarded with `JSONHas`.
fn custom_property_condition(
    json_column: &str,
    key: &str,
    op: PropertyOp,
    value_type: PropertyValueType,
    values: &[String],
    binds: &mut Vec<BindValue>,
) -> Result<String, MetricError> {
    let js = format!("toString({json_column})");
    // `JSONHas(json, key)` — presence guard; binds the key.
    let has = |binds: &mut Vec<BindValue>| {
        binds.push(BindValue::String(key.to_string()));
        format!("JSONHas({js}, ?)")
    };
    // The typed accessor for the property value; binds the key.
    let accessor = |binds: &mut Vec<BindValue>| {
        binds.push(BindValue::String(key.to_string()));
        match value_type {
            PropertyValueType::Number => format!("JSONExtractFloat({js}, ?)"),
            PropertyValueType::Boolean => format!("JSONExtractBool({js}, ?)"),
            PropertyValueType::Text | PropertyValueType::Date => {
                format!("JSONExtractString({js}, ?)")
            }
        }
    };
    let nth = |i: usize| -> Result<String, MetricError> {
        values.get(i).cloned().ok_or_else(|| {
            MetricError::Invalid(
                "custom property".to_string(),
                format!("operator {op:?} needs at least {} value(s)", i + 1),
            )
        })
    };
    // A numeric literal is bound as a string and cast in-SQL; text/date bind as-is.
    let cmp_operand = |binds: &mut Vec<BindValue>, raw: String| -> String {
        binds.push(BindValue::String(raw));
        match value_type {
            PropertyValueType::Number => "toFloat64(?)".to_string(),
            _ => "?".to_string(),
        }
    };

    let cond = match op {
        PropertyOp::IsSet => has(binds),
        PropertyOp::IsEmpty => format!("NOT {}", has(binds)),
        PropertyOp::IsTrue => {
            let a = accessor(binds);
            format!("{a} = 1")
        }
        PropertyOp::IsFalse => {
            let h = has(binds);
            let a = accessor(binds);
            format!("{h} AND {a} = 0")
        }
        PropertyOp::Equals => {
            let h = has(binds);
            let a = accessor(binds);
            let v = cmp_operand(binds, nth(0)?);
            format!("{h} AND {a} = {v}")
        }
        PropertyOp::NotEquals => {
            let h = has(binds);
            let a = accessor(binds);
            let v = cmp_operand(binds, nth(0)?);
            format!("{h} AND {a} != {v}")
        }
        PropertyOp::Gt | PropertyOp::After => {
            let h = has(binds);
            let a = accessor(binds);
            let v = cmp_operand(binds, nth(0)?);
            format!("{h} AND {a} > {v}")
        }
        PropertyOp::Gte => {
            let h = has(binds);
            let a = accessor(binds);
            let v = cmp_operand(binds, nth(0)?);
            format!("{h} AND {a} >= {v}")
        }
        PropertyOp::Lt | PropertyOp::Before => {
            let h = has(binds);
            let a = accessor(binds);
            let v = cmp_operand(binds, nth(0)?);
            format!("{h} AND {a} < {v}")
        }
        PropertyOp::Lte => {
            let h = has(binds);
            let a = accessor(binds);
            let v = cmp_operand(binds, nth(0)?);
            format!("{h} AND {a} <= {v}")
        }
        PropertyOp::Between => {
            let h = has(binds);
            let a = accessor(binds);
            let lo = cmp_operand(binds, nth(0)?);
            let hi = cmp_operand(binds, nth(1)?);
            format!("{h} AND {a} >= {lo} AND {a} <= {hi}")
        }
        PropertyOp::Contains => {
            let a = accessor(binds);
            binds.push(BindValue::String(nth(0)?));
            format!("positionCaseInsensitive({a}, ?) > 0")
        }
        PropertyOp::In => {
            if values.is_empty() {
                return Err(MetricError::Invalid(
                    "custom property".to_string(),
                    "IN needs at least one value".to_string(),
                ));
            }
            match value_type {
                PropertyValueType::Number => {
                    let h = has(binds);
                    let a = accessor(binds);
                    binds.push(BindValue::Strings(values.to_vec()));
                    format!("{h} AND has(arrayMap(v -> toFloat64(v), ?), {a})")
                }
                _ => {
                    let a = accessor(binds);
                    binds.push(BindValue::Strings(values.to_vec()));
                    format!("{a} IN ?")
                }
            }
        }
    };
    Ok(cond)
}

/// Combine child predicates' customer-id sets with a ClickHouse set operator
/// (`INTERSECT` for AND, `UNION DISTINCT` for OR). Each child is wrapped in a
/// derived table so its own set-ops (a nested Or inside an And) keep their
/// precedence; binds are appended in child order to match the placeholder order.
fn compile_cohort_set_op(
    registry: &MetricRegistry,
    children: &[CohortPredicate],
    query: &MetricQuery,
    events_table: &str,
    dedup: EventDedup,
    op: &str,
) -> Result<(String, Vec<BindValue>), MetricError> {
    if children.is_empty() {
        return Err(MetricError::Invalid(
            "cohort".to_string(),
            "a boolean group needs at least one child".to_string(),
        ));
    }
    let mut parts = Vec::new();
    let mut binds: Vec<BindValue> = Vec::new();
    for child in children {
        let (sql, mut b) = compile_cohort_predicate(registry, child, query, events_table, dedup)?;
        parts.push(format!("SELECT customer_id FROM ( {sql} ) AS c"));
        binds.append(&mut b);
    }
    Ok((parts.join(&format!(" {op} ")), binds))
}

/// `SELECT customer_id … GROUP BY customer_id HAVING <agg> <op> <lit>` — the set of
/// customers whose aggregate of `metric` over the query window satisfies the comparison.
/// Shared by the `MetricThreshold` predicate and the active-set anti-join of
/// `MetricInactive`. `metric` must be a non-derived aggregate.
fn metric_active_set(
    registry: &MetricRegistry,
    metric: &str,
    op: ThresholdOp,
    threshold: f64,
    query: &MetricQuery,
    events_table: &str,
    dedup: EventDedup,
) -> Result<(String, Vec<BindValue>), MetricError> {
    let def = registry
        .get(metric)
        .ok_or_else(|| MetricError::UnknownMetric(metric.to_string()))?;
    let spec = match &def.kind {
        MetricKind::Aggregate(s) => s,
        _ => {
            return Err(MetricError::Unsupported(
                "cohort thresholds require an aggregate metric".to_string(),
            ));
        }
    };
    let mut binds: Vec<BindValue> = Vec::new();
    let lit = fmt_threshold(threshold)?;
    let cmp = op.as_sql();

    match &def.source {
        MetricSource::RawEvents => {
            let mut conditions = vec!["tenant_id = ?".to_string()];
            binds.push(BindValue::Uuid(query.tenant_id.as_uuid()));
            conditions.push("code = ?".to_string());
            binds.push(BindValue::String(def.event_code.clone()));
            conditions.push("timestamp >= toDateTime(?)".to_string());
            binds.push(BindValue::I64(query.from.timestamp()));
            if let Some(to) = query.to {
                conditions.push("timestamp <= toDateTime(?)".to_string());
                binds.push(BindValue::I64(to.timestamp()));
            }
            // Divide by the unit-conversion factor so the threshold is compared in the
            // metric's converted (billed) unit, not raw event units.
            let having = apply_unit_conversion(spec, raw_having_expr(spec, &mut binds)?);
            let source = events_source(events_table, dedup, &conditions.join(" AND "));
            let sql = format!(
                "SELECT customer_id {source} GROUP BY customer_id HAVING {having} {cmp} {lit}"
            );
            Ok((sql, binds))
        }
        MetricSource::Fact(fs) => {
            ensure_table_name(&fs.table)?;
            for ident in [
                &fs.value_column,
                &fs.timestamp_column,
                &fs.customer_column,
                &fs.tenant_column,
            ] {
                ensure_inlinable(ident)?;
            }
            let mut conditions = vec![format!("{} = ?", fs.tenant_column)];
            binds.push(BindValue::Uuid(query.tenant_id.as_uuid()));
            // A STOCK reduction (AsOf) evaluates the level as-of `to`: drop the lower
            // time bound so a `Sum` accumulates every delta through `to` (a running
            // total = the MRR level) and a `Latest`/argMax picks the newest snapshot
            // on-or-before `to` (the AR level). A FLOW keeps `>= from` and sums the
            // window (net-new MRR, revenue booked in the period).
            if !matches!(spec.cohort_reduction, CohortReduction::AsOf) {
                conditions.push(format!(
                    "toDateTime({}) >= toDateTime(?)",
                    fs.timestamp_column
                ));
                binds.push(BindValue::I64(query.from.timestamp()));
            }
            if let Some(to) = query.to {
                conditions.push(format!(
                    "toDateTime({}) <= toDateTime(?)",
                    fs.timestamp_column
                ));
                binds.push(BindValue::I64(to.timestamp()));
            }
            let v = format!("toFloat64({})", fs.value_column);
            let having = aggregate_op_expr(
                spec.op,
                "count(*)",
                &v,
                &format!("toDateTime({})", fs.timestamp_column),
                &fs.value_column,
            );
            let sql = format!(
                "SELECT {0} AS customer_id FROM {1} FINAL WHERE {2} GROUP BY {0} HAVING {having} {cmp} {lit}",
                fs.customer_column,
                fs.table,
                conditions.join(" AND ")
            );
            Ok((sql, binds))
        }
    }
}

/// `SELECT customer_id … GROUP BY customer_id HAVING <prior> != 0 AND (<cur> - <prior>) /
/// <prior> * 100 <op> <pct>` — the set of customers whose metric aggregate changed, window
/// over window, by at least `pct` percent. One scan spans `[prior_from, to]`; `cur`/`prior`
/// are computed with conditional aggregates (`sumIf` etc.) splitting that scan into the
/// current window `[from, to]` and the prior window `[prior_from, from)`. The prior length
/// is `prior_window_days` days, defaulting to the current window length (`to - from`), a
/// symmetric period-over-period comparison. `metric` must be a non-derived aggregate.
///
/// `cur`/`prior` are aliased once in the inner SELECT and referenced by name in HAVING (each
/// heavy conditional aggregate is evaluated a single time), then the outer projection keeps
/// only `customer_id` so the predicate is the single-column shape the cohort set-ops expect.
fn metric_delta_set(
    registry: &MetricRegistry,
    metric: &str,
    op: ThresholdOp,
    pct: f64,
    prior_window_days: Option<u32>,
    query: &MetricQuery,
    events_table: &str,
    dedup: EventDedup,
) -> Result<(String, Vec<BindValue>), MetricError> {
    let def = registry
        .get(metric)
        .ok_or_else(|| MetricError::UnknownMetric(metric.to_string()))?;
    // The unit-conversion factor is deliberately NOT applied here: this predicate is a
    // window-over-window *percentage* change, and a constant divisor cancels between the
    // current and prior aggregates, so the result is identical with or without it.
    let spec = match &def.kind {
        MetricKind::Aggregate(s) => s,
        _ => {
            return Err(MetricError::Unsupported(
                "cohort deltas require an aggregate metric".to_string(),
            ));
        }
    };

    // Current window = the query's [from, to]; prior window = [prior_from, from). The prior
    // length is `prior_window_days` days, defaulting to the current window length so the two
    // windows are symmetric. `prior_from = from - W` is computed here (never client-supplied),
    // so the boundary timestamps are safe server-side i64s bound like every other time bound.
    let to = query.to.unwrap_or(query.from);
    let window_days = (to - query.from).num_days();
    let prior_days = prior_window_days.map(i64::from).unwrap_or(window_days);
    let prior_from = query.from - chrono::Duration::days(prior_days);
    let from_ts = query.from.timestamp();
    let prior_from_ts = prior_from.timestamp();
    let to_ts = to.timestamp();

    let pct_lit = fmt_threshold(pct)?;
    let cmp = op.as_sql();

    let mut binds: Vec<BindValue> = Vec::new();

    // Bind order follows textual emission: the SELECT's `cur` aggregate binds, then `prior`'s,
    // then WHERE (the scan bounds). The HAVING references the aliases only (no binds) and
    // inlines `pct` via `fmt_threshold`, exactly like the absolute-threshold predicates.
    let inner = match &def.source {
        MetricSource::RawEvents => {
            let cur_pred = "timestamp >= toDateTime(?)";
            let prior_pred = "timestamp < toDateTime(?) AND timestamp >= toDateTime(?)";
            let cur = raw_delta_agg(spec, cur_pred, &[from_ts], &mut binds)?;
            let prior = raw_delta_agg(spec, prior_pred, &[from_ts, prior_from_ts], &mut binds)?;
            let mut conditions = vec!["tenant_id = ?".to_string()];
            binds.push(BindValue::Uuid(query.tenant_id.as_uuid()));
            conditions.push("code = ?".to_string());
            binds.push(BindValue::String(def.event_code.clone()));
            conditions.push("timestamp >= toDateTime(?)".to_string());
            binds.push(BindValue::I64(prior_from_ts));
            conditions.push("timestamp <= toDateTime(?)".to_string());
            binds.push(BindValue::I64(to_ts));
            let source = events_source(events_table, dedup, &conditions.join(" AND "));
            format!(
                "SELECT customer_id, toFloat64({cur}) AS cur, toFloat64({prior}) AS prior \
                 {source} GROUP BY customer_id \
                 HAVING prior != 0 AND (cur - prior) / prior * 100 {cmp} {pct_lit}"
            )
        }
        MetricSource::Fact(fs) => {
            ensure_table_name(&fs.table)?;
            for ident in [
                &fs.value_column,
                &fs.timestamp_column,
                &fs.customer_column,
                &fs.tenant_column,
            ] {
                ensure_inlinable(ident)?;
            }
            let ts = format!("toDateTime({})", fs.timestamp_column);
            let cur_pred = format!("{ts} >= toDateTime(?)");
            let prior_pred = format!("{ts} < toDateTime(?) AND {ts} >= toDateTime(?)");
            let cur = fact_delta_agg(spec, fs, &cur_pred, &[from_ts], &mut binds);
            let prior =
                fact_delta_agg(spec, fs, &prior_pred, &[from_ts, prior_from_ts], &mut binds);
            let mut conditions = vec![format!("{} = ?", fs.tenant_column)];
            binds.push(BindValue::Uuid(query.tenant_id.as_uuid()));
            conditions.push(format!("{ts} >= toDateTime(?)"));
            binds.push(BindValue::I64(prior_from_ts));
            conditions.push(format!("{ts} <= toDateTime(?)"));
            binds.push(BindValue::I64(to_ts));
            format!(
                "SELECT {0} AS customer_id, toFloat64({cur}) AS cur, toFloat64({prior}) AS prior \
                 FROM {1} FINAL WHERE {2} GROUP BY {0} \
                 HAVING prior != 0 AND (cur - prior) / prior * 100 {cmp} {pct_lit}",
                fs.customer_column,
                fs.table,
                conditions.join(" AND ")
            )
        }
    };
    Ok((format!("SELECT customer_id FROM ( {inner} ) AS d"), binds))
}

/// One conditional aggregate (`cur` or `prior`) of a RAW-EVENTS delta. Pushes its binds in
/// textual order: the value/distinct property path first (it precedes the predicate in every
/// `*If` form), then the predicate's bound window bounds — so placeholder order matches
/// emission. `Count` binds no property path.
fn raw_delta_agg(
    spec: &AggregateSpec,
    predicate_sql: &str,
    window_bounds: &[i64],
    binds: &mut Vec<BindValue>,
) -> Result<String, MetricError> {
    let (value_if, distinct) = if matches!(spec.op, AggregateOp::Count) {
        (String::new(), String::new())
    } else {
        let prop = spec.value_property.as_deref().ok_or_else(|| {
            MetricError::Invalid(
                format!("{:?}", spec.op),
                "value_property required".to_string(),
            )
        })?;
        let path = PropertyColumn(prop).path_sql(binds);
        (format!("toFloat64OrZero({path})"), path)
    };
    for b in window_bounds {
        binds.push(BindValue::I64(*b));
    }
    Ok(aggregate_op_if_expr(
        spec.op,
        &format!("countIf({predicate_sql})"),
        &value_if,
        LATEST_EVENT_ORDER,
        &distinct,
        predicate_sql,
    ))
}

/// One conditional aggregate (`cur` or `prior`) of a FACT delta. The value/distinct columns
/// are inlined identifiers (allowlist-validated by the caller), so the only binds are the
/// predicate's window bounds, pushed in textual order.
fn fact_delta_agg(
    spec: &AggregateSpec,
    fs: &FactSource,
    predicate_sql: &str,
    window_bounds: &[i64],
    binds: &mut Vec<BindValue>,
) -> String {
    let v = format!("toFloat64({})", fs.value_column);
    for b in window_bounds {
        binds.push(BindValue::I64(*b));
    }
    aggregate_op_if_expr(
        spec.op,
        &format!("countIf({predicate_sql})"),
        &v,
        &format!("toDateTime({})", fs.timestamp_column),
        &fs.value_column,
        predicate_sql,
    )
}

fn raw_having_expr(
    spec: &AggregateSpec,
    binds: &mut Vec<BindValue>,
) -> Result<String, MetricError> {
    if matches!(spec.op, AggregateOp::Count) {
        return Ok("count(*)".to_string());
    }
    let prop = spec.value_property.as_deref().ok_or_else(|| {
        MetricError::Invalid(
            format!("{:?}", spec.op),
            "value_property required".to_string(),
        )
    })?;
    let path = PropertyColumn(prop).path_sql(binds);
    let v = format!("toFloat64OrZero({path})");
    // Count handled by the early return above; pass an unreachable marker for that arm.
    Ok(aggregate_op_expr(
        spec.op,
        "unreachable",
        &v,
        LATEST_EVENT_ORDER,
        &path,
    ))
}

/// A (metric key, optional transform) pair, as `collect_operands` yields it.
type OperandKey = (String, Option<CrossPeriod>);

/// Where an operand's value lands. Fusion puts several operands in one CTE, so the column is no
/// longer always `value`.
struct OperandRef {
    alias: String,
    column: String,
}

/// What makes two operands fusable: the same fact read under the same aggregate. Everything else
/// about them is already shared (tenant, range, window, dimensions, cohort).
#[derive(PartialEq)]
struct FusionKey {
    fs: FactSource,
    spec: AggregateSpec,
}

/// Operands fuse when one grouped scan can produce all of them — isolated only by their own
/// per-metric filters. A transformed operand reads a history-primed range, a snapshot needs a
/// second reduction keyed on `value`, and a unit-conversion divisor rescales the whole series;
/// none survive being folded into a shared read, so they keep their own CTE.
fn fusion_key(def: &MetricDef, transform: Option<&CrossPeriod>) -> Option<FusionKey> {
    if transform.is_some() || !has_effective_filters(&def.filters) {
        return None;
    }
    let (MetricKind::Aggregate(spec), MetricSource::Fact(fs)) = (&def.kind, &def.source) else {
        return None;
    };
    if fs.snapshot || spec.unit_conversion_factor.is_some() {
        return None;
    }
    Some(FusionKey {
        fs: fs.clone(),
        spec: spec.clone(),
    })
}

fn derived_series(
    registry: &MetricRegistry,
    expr: &Expr,
    query: &MetricQuery,
    events_table: &str,
    dedup: EventDedup,
    database: &str,
    cohort: Option<&CohortClause>,
) -> Result<Series, MetricError> {
    // Each operand is a (metric key, optional per-operand transform) pair, deduped while keeping
    // expression order. The same metric may appear both plain and transformed — they become two
    // distinct CTEs (see `operand_alias`), which is precisely how a retention ratio composes a
    // lagged-cumulative *opening balance* against a within-window flow.
    let operands = dedup_operands(collect_operands(expr));
    if operands.is_empty() {
        return Err(MetricError::Invalid(
            "derived".to_string(),
            "expression references no metrics".to_string(),
        ));
    }

    let dims = dim_columns(query);
    let mut ctes: Vec<String> = Vec::new();
    let mut binds: Vec<BindValue> = Vec::new();
    // True once any operand is history-primed (a lagged/running cumulative built from epoch): the
    // derived join then spans pre-`from` windows, so the final result must be floored.
    let mut history_primed = false;

    // Partition operands into fusion groups in first-appearance order. Operands that can't fuse
    // (or are alone) keep their own CTE and the exact SQL they had before.
    let mut groups: Vec<(Option<FusionKey>, Vec<usize>)> = Vec::new();
    for (i, (key, transform)) in operands.iter().enumerate() {
        let def = registry
            .get(key)
            .ok_or_else(|| MetricError::UnknownMetric(key.clone()))?;
        if matches!(def.kind, MetricKind::Derived(_)) {
            return Err(MetricError::Unsupported(
                "nested derived metrics are not yet supported".to_string(),
            ));
        }
        let fkey = fusion_key(def, transform.as_ref());
        let slot = fkey
            .as_ref()
            .and_then(|k| groups.iter_mut().find(|(g, _)| g.as_ref() == Some(k)));
        match slot {
            Some((_, members)) => members.push(i),
            None => groups.push((fkey, vec![i])),
        }
    }

    let mut refs: Vec<(OperandKey, OperandRef)> = Vec::new();
    let mut aliases: Vec<String> = Vec::new();
    // A fused CTE needs a synthetic alias that can't collide with a metric literally keyed
    // `fused_0`, whose own CTE would be `m_fused_0`.
    let mut used: std::collections::HashSet<String> = operands
        .iter()
        .map(|(k, t)| operand_alias(k, t.as_ref()))
        .collect();
    for (fkey, members) in &groups {
        // A fused group reads once and emits one column per operand; everything else is unchanged.
        if let (Some(FusionKey { fs, spec }), true) = (fkey.as_ref(), members.len() > 1) {
            let alias = (0..)
                .map(|n| format!("m_fused_{n}"))
                .find(|a| used.insert(a.clone()))
                .expect("an unused alias always exists");
            let defs: Vec<&MetricDef> = members
                .iter()
                .map(|&i| registry.get(&operands[i].0).expect("resolved above"))
                .collect();
            // Name each column after its metric so the outer expression reads back as the metrics
            // it fuses. `sanitize` is many-to-one, so a suffix breaks the rare key-to-key tie.
            let mut taken = std::collections::HashSet::new();
            let columns: Vec<String> = members
                .iter()
                .map(|&i| {
                    let base = format!("value_{}", sanitize(&operands[i].0));
                    (0..)
                        .map(|n| {
                            if n == 0 {
                                base.clone()
                            } else {
                                format!("{base}_{n}")
                            }
                        })
                        .find(|c| taken.insert(c.clone()))
                        .expect("an unused column always exists")
                })
                .collect();
            let fused: Vec<FusedMember<'_>> = defs
                .iter()
                .zip(&columns)
                .map(|(d, column)| FusedMember {
                    column: column.clone(),
                    filters: &d.filters,
                })
                .collect();
            let sub = fact_series_fused(fs, spec, query, database, cohort, &fused)?;
            ctes.push(format!("{alias} AS ( {} )", sub.sql));
            binds.extend(sub.binds);
            for (&i, column) in members.iter().zip(&columns) {
                refs.push((
                    operands[i].clone(),
                    OperandRef {
                        alias: alias.clone(),
                        column: column.clone(),
                    },
                ));
            }
            aliases.push(alias);
            continue;
        }

        let i = members[0];
        let (key, transform) = &operands[i];
        let def = registry.get(key).expect("resolved above");
        let alias = operand_alias(key, transform.as_ref());
        let sub = match transform {
            // Plain within-window operand: the existing sub-series.
            None => build_series(registry, def, query, events_table, dedup, database, cohort)?,
            // Transformed operand: build the sub-series over a history-primed range, then apply the
            // window function to it. Running/lagged cumulatives prime from epoch so the first
            // *displayed* window already carries a correct opening balance.
            Some(t) => {
                let primed = matches!(
                    t,
                    CrossPeriod::RunningTotal | CrossPeriod::RunningTotalLagged
                );
                let sub_query = if primed {
                    history_primed = true;
                    let mut q = query.clone();
                    q.from = chrono::DateTime::from_timestamp(0, 0).expect("epoch is valid");
                    q
                } else {
                    query.clone()
                };
                let base = build_series(
                    registry,
                    def,
                    &sub_query,
                    events_table,
                    dedup,
                    database,
                    cohort,
                )?;
                transformed_sub_series(base, t)
            }
        };
        ctes.push(format!("{alias} AS ( {} )", sub.sql));
        binds.extend(sub.binds);
        refs.push((
            operands[i].clone(),
            OperandRef {
                alias: alias.clone(),
                column: "value".to_string(),
            },
        ));
        aliases.push(alias);
    }

    // FULL OUTER JOIN chain over the emitted CTEs — one per fusion group, so a fully fused
    // expression has no join at all. Explicit ON conditions (never USING — bare USING-key
    // resolution is analyzer-dependent). Runs with `join_use_nulls = 1`.
    let mut key_cols = vec!["window_start".to_string(), "window_end".to_string()];
    key_cols.extend(dims.iter().cloned());

    let coalesce_key = |aliases: &[String], col: &str| -> String {
        if aliases.len() == 1 {
            format!("{}.{col}", aliases[0])
        } else {
            let qualified = aliases
                .iter()
                .map(|a| format!("{a}.{col}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("coalesce({qualified})")
        }
    };

    let mut from_clause = aliases[0].clone();
    for (i, alias) in aliases.iter().enumerate().skip(1) {
        let on = key_cols
            .iter()
            .map(|col| format!("{} = {alias}.{col}", coalesce_key(&aliases[..i], col)))
            .collect::<Vec<_>>()
            .join(" AND ");
        from_clause.push_str(&format!(" FULL OUTER JOIN {alias} ON {on}"));
    }

    let mut select_cols: Vec<String> = Vec::new();
    select_cols.push(format!(
        "{} AS window_start",
        coalesce_key(&aliases, "window_start")
    ));
    select_cols.push(format!(
        "{} AS window_end",
        coalesce_key(&aliases, "window_end")
    ));
    select_cols.push(format!("{} AS value", compile_expr(expr, &refs)));
    for d in &dims {
        select_cols.push(format!("{} AS {d}", coalesce_key(&aliases, d)));
    }

    let joined = format!(
        "WITH {} SELECT {} FROM {} ORDER BY window_start",
        ctes.join(", "),
        select_cols.join(", "),
        from_clause
    );

    // A history-primed operand makes the join span pre-`from` windows (so the lagged cumulative
    // carries a real opening balance into the first displayed window). Floor the *result* back to
    // `>= from` in an outer select — never inside the operand's WHERE, which would erase the very
    // history the window function needs.
    let sql = if history_primed {
        let dim_select = if dims.is_empty() {
            String::new()
        } else {
            format!(", {}", dims.join(", "))
        };
        binds.push(BindValue::I64(query.from.timestamp()));
        format!(
            "SELECT window_start, window_end, value{dim_select} FROM ( {joined} ) \
             WHERE window_start >= toDateTime(?) ORDER BY window_start"
        )
    } else {
        joined
    };

    // Force NULL (not type defaults) for missing-side columns so the key coalesce is correct under every server config.
    let settings = if aliases.len() > 1 {
        vec!["join_use_nulls = 1".to_string()]
    } else {
        vec![]
    };

    Ok(Series {
        sql,
        binds,
        dims,
        settings,
    })
}

/// Collect `(metric key, optional per-operand transform)` operands in expression order.
fn collect_operands(expr: &Expr) -> Vec<(String, Option<CrossPeriod>)> {
    let mut out = Vec::new();
    fn walk(e: &Expr, out: &mut Vec<(String, Option<CrossPeriod>)>) {
        match e {
            Expr::Metric(k) => out.push((k.clone(), None)),
            Expr::MetricTransformed { metric, transform } => {
                out.push((metric.clone(), Some(transform.clone())))
            }
            Expr::Const(_) => {}
            Expr::Binary(l, _, r) => {
                walk(l, out);
                walk(r, out);
            }
            Expr::Unary(_, e) => walk(e, out),
            Expr::Clamp { expr, min, max } => {
                walk(expr, out);
                if let Some(min) = min {
                    walk(min, out);
                }
                if let Some(max) = max {
                    walk(max, out);
                }
            }
            Expr::Coalesce(items) | Expr::Greatest(items) | Expr::Least(items) => {
                for item in items {
                    walk(item, out);
                }
            }
        }
    }
    walk(expr, &mut out);
    out
}

/// Order-preserving dedup. `CrossPeriod` isn't `Hash`, so a linear scan (operand counts are tiny).
fn dedup_operands(
    operands: Vec<(String, Option<CrossPeriod>)>,
) -> Vec<(String, Option<CrossPeriod>)> {
    let mut out: Vec<(String, Option<CrossPeriod>)> = Vec::new();
    for op in operands {
        if !out.contains(&op) {
            out.push(op);
        }
    }
    out
}

/// The window-function `value` expression for a [`CrossPeriod`] transform, given the already-built
/// `OVER (...)` clause body (`order`, including any `PARTITION BY`). Single source of truth shared
/// by [`transformed_sub_series`] (sub-series, no floor) and [`apply_cross_period`] (outer floor).
fn cross_period_value_expr(transform: &CrossPeriod, order: &str) -> String {
    match transform {
        CrossPeriod::RunningTotal => {
            format!("sum(value) OVER ({order} ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)")
        }
        // Opening balance: the cumulative total *through the previous window*.
        CrossPeriod::RunningTotalLagged => {
            format!("sum(value) OVER ({order} ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING)")
        }
        CrossPeriod::PeriodOverPeriodDelta => format!(
            "value - lagInFrame(value, 1, 0) OVER ({order} ROWS BETWEEN 1 PRECEDING AND CURRENT ROW)"
        ),
        CrossPeriod::PercentChange => format!(
            "(value - lagInFrame(value, 1, 0) OVER ({order} ROWS BETWEEN 1 PRECEDING AND CURRENT ROW)) / nullIf(lagInFrame(value, 1, 0) OVER ({order} ROWS BETWEEN 1 PRECEDING AND CURRENT ROW), 0)"
        ),
        CrossPeriod::MovingAverage { periods } => {
            let preceding = periods.saturating_sub(1);
            format!("avg(value) OVER ({order} ROWS BETWEEN {preceding} PRECEDING AND CURRENT ROW)")
        }
    }
}

/// Wrap a (history-primed, windowed) base sub-series in a [`CrossPeriod`] window function,
/// producing a CTE-ready `window_start, window_end, value, <dims>` series. Mirrors
/// [`apply_cross_period`]'s window-function SQL but emits a sub-series (no outer floor — the
/// derived join floors the composed result once, after all operands are joined).
fn transformed_sub_series(base: Series, transform: &CrossPeriod) -> Series {
    let order = if base.dims.is_empty() {
        "ORDER BY window_start".to_string()
    } else {
        format!(
            "PARTITION BY {} ORDER BY window_start",
            base.dims.join(", ")
        )
    };
    let value_expr = cross_period_value_expr(transform, &order);
    let dim_select = if base.dims.is_empty() {
        String::new()
    } else {
        format!(", {}", base.dims.join(", "))
    };
    Series {
        sql: format!(
            "SELECT window_start, window_end, {value_expr} AS value{dim_select} FROM ( {} )",
            base.sql
        ),
        binds: base.binds,
        dims: base.dims,
        settings: base.settings,
    }
}

/// The qualified column an operand reads. Falls back to the unfused naming so a reference that
/// somehow escaped `collect_operands` still resolves to its own CTE.
fn operand_value(
    refs: &[(OperandKey, OperandRef)],
    key: &str,
    transform: Option<&CrossPeriod>,
) -> String {
    refs.iter()
        .find(|((k, t), _)| k == key && t.as_ref() == transform)
        .map(|(_, r)| format!("{}.{}", r.alias, r.column))
        .unwrap_or_else(|| format!("{}.value", operand_alias(key, transform)))
}

fn compile_expr(expr: &Expr, refs: &[(OperandKey, OperandRef)]) -> String {
    match expr {
        // Zero-fill: under `join_use_nulls = 1` a window missing from one sub-series yields NULL.
        // A fused operand is already 0 there (its `*If` reduced no rows), so this is a no-op.
        Expr::Metric(k) => format!("coalesce({}, 0)", operand_value(refs, k, None)),
        // A transformed operand reads its own (lagged-cumulative / windowed) CTE. We keep the NULL
        // here (not coalesce-to-0): an *opening balance* of NULL must propagate to a NULL ratio,
        // not a guarded 0 that would read as "0% retention".
        Expr::MetricTransformed { metric, transform } => {
            operand_value(refs, metric, Some(transform))
        }
        Expr::Const(c) => format!("{c}"),
        Expr::Binary(l, op, r) => {
            let ls = compile_expr(l, refs);
            match op {
                BinOp::Add => format!("({ls} + {})", compile_expr(r, refs)),
                BinOp::Sub => format!("({ls} - {})", compile_expr(r, refs)),
                BinOp::Mul => format!("({ls} * {})", compile_expr(r, refs)),
                // Divide-by-zero guard.
                BinOp::Div => {
                    let denom = match r.as_ref() {
                        Expr::Metric(k) => operand_value(refs, k, None),
                        Expr::MetricTransformed { metric, transform } => {
                            operand_value(refs, metric, Some(transform))
                        }
                        _ => compile_expr(r, refs),
                    };
                    format!("({ls} / nullIf({denom}, 0))")
                }
            }
        }
        Expr::Unary(op, e) => {
            let es = compile_expr(e, refs);
            match op {
                UnaryOp::Neg => format!("(-({es}))"),
                UnaryOp::Abs => format!("abs({es})"),
            }
        }
        Expr::Clamp { expr, min, max } => {
            let mut out = compile_expr(expr, refs);
            // Lower-bound first so the result also respects the upper bound when both are present.
            if let Some(min) = min {
                out = format!("greatest({}, {out})", compile_expr(min, refs));
            }
            if let Some(max) = max {
                out = format!("least({}, {out})", compile_expr(max, refs));
            }
            out
        }
        Expr::Coalesce(items) => format!("coalesce({})", compile_expr_list(items, refs)),
        Expr::Greatest(items) => format!("greatest({})", compile_expr_list(items, refs)),
        Expr::Least(items) => format!("least({})", compile_expr_list(items, refs)),
    }
}

fn compile_expr_list(items: &[Expr], refs: &[(OperandKey, OperandRef)]) -> String {
    items
        .iter()
        .map(|i| compile_expr(i, refs))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The CTE alias for a (possibly transformed) operand. A plain reference reads `m_<key>`; a
/// transformed operand reads a distinct CTE so the same metric can appear both as a within-window
/// flow (`m_mrr_delta`) and as a lagged cumulative (`m_mrr_delta__running_total_lagged`) in one
/// expression — exactly what retention ratios need.
fn operand_alias(key: &str, transform: Option<&CrossPeriod>) -> String {
    match transform {
        None => format!("m_{}", sanitize(key)),
        Some(t) => format!("m_{}__{}", sanitize(key), transform_suffix(t)),
    }
}

fn transform_suffix(transform: &CrossPeriod) -> &'static str {
    match transform {
        CrossPeriod::RunningTotal => "running_total",
        CrossPeriod::RunningTotalLagged => "running_total_lagged",
        CrossPeriod::PeriodOverPeriodDelta => "delta",
        CrossPeriod::PercentChange => "pct_change",
        CrossPeriod::MovingAverage { .. } => "moving_avg",
    }
}

fn sanitize(key: &str) -> String {
    crate::sql::sanitize_identifier(key)
}

struct GapFill {
    from_ts: i64,
    to_ts: i64,
    interval: String,
    /// The window's timezone, so the fill start floors to the same bucket boundary the
    /// window function produces (else a non-aligned `from` — e.g. any weekday for a week
    /// grain — puts fill rows off the real buckets, doubling/misplacing points).
    tz: String,
}

fn apply_cross_period(
    series: Series,
    transform: &CrossPeriod,
    floor_ts: Option<i64>,
    fill: Option<GapFill>,
) -> SafeQuery {
    let partition = if series.dims.is_empty() {
        String::new()
    } else {
        format!("PARTITION BY {} ", series.dims.join(", "))
    };
    let order = format!("{partition}ORDER BY window_start");

    let value_expr = cross_period_value_expr(transform, &order);

    let dim_select = if series.dims.is_empty() {
        String::new()
    } else {
        format!(", {}", series.dims.join(", "))
    };

    let mut binds = series.binds;
    let mut windowed = format!(
        "SELECT window_start, window_end, {value_expr} AS value{dim_select} FROM ( {} )",
        series.sql
    );

    // Densify a dimensionless cumulative series via WITH FILL + INTERPOLATE, carrying `value` forward into empty windows.
    let filled = fill.as_ref().filter(|_| series.dims.is_empty());
    if let Some(g) = filled {
        // Floor the fill start to the window's own bucket boundary (`tumbleStart`, wrapped
        // in `toDateTime` since it yields a `Date` for week/month) so the fill grid lands
        // exactly on the real buckets — otherwise a non-aligned `from` interleaves off-grid
        // rows with the real ones.
        windowed.push_str(&format!(
            " ORDER BY window_start WITH FILL \
             FROM toDateTime(tumbleStart(toDateTime(?), {interval}, ?), ?) \
             TO toDateTime(?) STEP {interval} INTERPOLATE (value AS value)",
            interval = g.interval
        ));
        binds.push(BindValue::I64(g.from_ts));
        binds.push(BindValue::String(g.tz.clone()));
        binds.push(BindValue::String(g.tz.clone()));
        binds.push(BindValue::I64(g.to_ts));
    }

    match floor_ts {
        None => SafeQuery {
            sql: format!("{windowed} ORDER BY window_start"),
            binds,
        },
        // The window function must see pre-`from` rows for the opening balance, so floor in an outer select, never in its WHERE.
        Some(ts) => {
            let window_end_col = match filled {
                Some(g) => format!("window_start + {} AS window_end", g.interval),
                None => "window_end".to_string(),
            };
            binds.push(BindValue::I64(ts));
            SafeQuery {
                sql: format!(
                    "SELECT window_start, {window_end_col}, value{dim_select} FROM ( {windowed} ) \
                     WHERE window_start >= toDateTime(?) ORDER BY window_start"
                ),
                binds,
            }
        }
    }
}

/// Forward-fill a Latest/Max gauge so empty windows inherit the prior window's level (a persisting
/// balance). The input `series` is primed from epoch (so a pre-`from` event sets the opening level);
/// here we densify the visible range into a window grid, carry the last non-null level forward with a
/// window function, then floor the output back to `>= from`.
fn carry_over_gauge_series(series: Series, query: &MetricQuery, window: &WindowSize) -> Series {
    let interval = interval_sql(window);
    let step = window_seconds(window);
    let tz = tz_name(query);
    let from_ts = query.from.timestamp();
    let to_ts = query.to.unwrap_or_else(Utc::now).timestamp();

    let dims = series.dims.clone();
    let dim_list = if dims.is_empty() {
        String::new()
    } else {
        format!(", {}", dims.join(", "))
    };
    let partition = if dims.is_empty() {
        String::new()
    } else {
        format!("PARTITION BY {} ", dims.join(", "))
    };

    let mut binds = series.binds;

    // Densify so empty windows exist as rows the forward-fill can carry into. We only fabricate the
    // dense grid for the ungrouped (single-series balance) case: a scaffold can't know which
    // dimension tuples to densify, and emitting NULL-dimension rows would pollute the result with
    // junk groups. For grouped gauges we forward-fill over the rows the gauge already produced
    // (fully-empty windows for a given dimension stay absent — per-dimension densification is future
    // work, see DESIGN.md §3.2). The forward-fill itself applies in both cases.
    let inner = if dims.is_empty() {
        // Build the dense grid as a Fragment so each `?` carries its bind inline: the old code
        // pushed six binds in a separate block that had to mirror the SQL's `?` order by hand.
        let mut grid = Fragment::new();
        grid.push_bound(
            "SELECT toDateTime(tumbleStart(toDateTime(",
            BindValue::I64(from_ts),
            &format!("), {interval}, "),
        );
        grid.push_bound(
            "",
            BindValue::String(tz.clone()),
            &format!(") + number * {step}) AS window_start, toDateTime(tumbleStart(toDateTime("),
        );
        grid.push_bound("", BindValue::I64(from_ts), &format!("), {interval}, "));
        grid.push_bound(
            "",
            BindValue::String(tz.clone()),
            &format!(
                ") + (number + 1) * {step}) AS window_end FROM numbers(toUInt64((toUnixTimestamp(toDateTime("
            ),
        );
        grid.push_bound(
            "",
            BindValue::I64(to_ts),
            ")) - toUnixTimestamp(toDateTime(",
        );
        grid.push_bound("", BindValue::I64(from_ts), &format!("))) / {step}) + 1)"));
        binds.extend(grid.binds);
        let grid = grid.sql;
        // `max` collapses the (real, scaffold-NULL) pair per window to the real level, NULL when only
        // the scaffold is present; `last_value ... IGNORE NULLS` then carries the level forward.
        format!(
            "SELECT window_start, window_end, max(value) AS value FROM ( \
                 SELECT window_start, window_end, value FROM ( {} ) \
                 UNION ALL \
                 SELECT window_start, window_end, NULL AS value FROM ( {grid} ) \
             ) GROUP BY window_start, window_end",
            series.sql
        )
    } else {
        format!(
            "SELECT window_start, window_end, value{dim_list} FROM ( {} )",
            series.sql
        )
    };

    let sql = format!(
        "SELECT window_start, window_end, \
            coalesce(last_value(value) IGNORE NULLS OVER ({partition}ORDER BY window_start \
                ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW), 0) AS value{dim_list} \
         FROM ( {inner} ) \
         WHERE window_start >= toDateTime(?) \
         ORDER BY window_start"
    );
    binds.push(BindValue::I64(from_ts));

    Series {
        sql,
        binds,
        dims,
        settings: series.settings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metric::{AggregateOp, DimensionFilter, Expr, FilterOp, MetricDef, ThresholdOp};
    use chrono::{TimeZone, Utc};
    use common_domain::ids::TenantId;
    use rstest::rstest;

    fn normalize(sql: &str) -> String {
        sql.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn base_query(metric: &str) -> MetricQuery {
        MetricQuery {
            tenant_id: TenantId::default(),
            metric: metric.to_string(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            window: Some(WindowSize::Day),
            window_time_zone: Some(chrono_tz::UTC),
            group_by: vec![],
            customer_ids: vec![],
            filters: vec![],
            transform: None,
            cohort: None,
            reporting_currency: None,
            keep_last_events: false,
        }
    }

    #[test]
    fn aggregate_sum_matches_expected_shape() {
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "bytes",
            "usage",
            AggregateOp::Sum,
            Some("amount"),
        ));
        let q = base_query("bytes");
        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        let expected = r#"
            SELECT
                toDateTime(tumbleStart(toDateTime(timestamp), toIntervalDay(1), ?), ?) AS window_start,
                toDateTime(tumbleEnd(toDateTime(timestamp), toIntervalDay(1), ?), ?) AS window_end,
                sum(toFloat64OrZero(properties[?])) AS value
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties
                FROM raw_events_v2
                WHERE tenant_id = ? AND code = ? AND timestamp >= toDateTime(?)
                    AND timestamp <= toDateTime(?)
                    AND properties[?] != '' AND isNotNull(toFloat64OrNull(properties[?]))
            )
            GROUP BY
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), ?),
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), ?)
            ORDER BY window_start
        "#;
        assert_eq!(normalize(&out.sql), normalize(expected));
    }

    #[test]
    fn unit_conversion_factor_divides_the_aggregate() {
        // A factor > 1 divides the per-window aggregate, matching the billing path.
        let reg = MetricRegistry::new().with(
            MetricDef::aggregate("bytes", "usage", AggregateOp::Sum, Some("amount"))
                .with_unit_conversion_factor(Some(1024.0)),
        );
        let out = compile(&reg, &base_query("bytes"), "raw_events_v2", "meteroid").unwrap();
        assert!(
            normalize(&out.sql).contains("(sum(toFloat64OrZero(properties[?]))) / 1024 AS value"),
            "converted aggregate: {}",
            normalize(&out.sql)
        );
    }

    #[test]
    fn unit_conversion_noop_factor_is_byte_identical() {
        // A factor of None / <= 1 leaves the SQL exactly as the unconverted metric.
        let base = MetricDef::aggregate("bytes", "usage", AggregateOp::Sum, Some("amount"));
        let unconverted = compile(
            &MetricRegistry::new().with(base.clone()),
            &base_query("bytes"),
            "raw_events_v2",
            "meteroid",
        )
        .unwrap();
        for factor in [None, Some(1.0), Some(0.0)] {
            let reg = MetricRegistry::new().with(base.clone().with_unit_conversion_factor(factor));
            let out = compile(&reg, &base_query("bytes"), "raw_events_v2", "meteroid").unwrap();
            assert_eq!(
                normalize(&out.sql),
                normalize(&unconverted.sql),
                "factor {factor:?} must not alter SQL"
            );
        }
    }

    #[test]
    fn count_distinct_omits_numeric_guard() {
        // CountDistinct over a string property (e.g. user_id) must NOT filter on
        // numeric parseability, or every non-numeric row is dropped and the metric
        // collapses to 0. It keeps only the non-empty guard.
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "users",
            "session_resource_usage",
            AggregateOp::CountDistinct,
            Some("user_id"),
        ));
        let out = compile(&reg, &base_query("users"), "raw_events_v2", "meteroid").unwrap();
        let sql = normalize(&out.sql);
        assert!(
            sql.contains("toFloat64(uniq(properties[?])) AS value"),
            "distinct aggregate: {sql}"
        );
        assert!(
            sql.contains("properties[?] != ''"),
            "non-empty guard kept: {sql}"
        );
        assert!(
            !sql.contains("toFloat64OrNull"),
            "numeric guard dropped for CountDistinct: {sql}"
        );
    }

    #[test]
    fn dimension_filter_in_is_or_of_property_equals() {
        // An `In` metric filter narrows the read to matching property values (OR of
        // equals) and never touches presence — the analytics twin of the metering
        // meter path (query_raw.rs::test_query_meter_property_filter_in_is_where_only).
        let reg = MetricRegistry::new().with(
            MetricDef::aggregate("bytes", "usage", AggregateOp::Sum, Some("amount"))
                .with_filter("region", &["us", "eu"]),
        );
        let out = compile(&reg, &base_query("bytes"), "raw_events_v2", "meteroid").unwrap();
        let sql = normalize(&out.sql);
        assert!(
            sql.contains("(properties[?] = ? OR properties[?] = ?)"),
            "In filter compiles to an OR of equals: {sql}"
        );
        assert!(
            !sql.contains("mapContains"),
            "In must not require presence: {sql}"
        );
    }

    #[test]
    fn dimension_filter_not_in_is_presence_required() {
        // `NotIn` must require the property to be present (mapContains) before the `!=`,
        // else an event missing the key reads as '' and slips through the negation.
        let mut def = MetricDef::aggregate("bytes", "usage", AggregateOp::Sum, Some("amount"));
        def.filters.push(DimensionFilter {
            property: "region".to_string(),
            values: vec!["eu".to_string()],
            op: FilterOp::NotIn,
        });
        let out = compile(
            &MetricRegistry::new().with(def),
            &base_query("bytes"),
            "raw_events_v2",
            "meteroid",
        )
        .unwrap();
        let sql = normalize(&out.sql);
        assert!(
            sql.contains("(mapContains(properties, ?) AND properties[?] != ?)"),
            "NotIn filter compiles to a presence-guarded !=: {sql}"
        );
    }

    #[test]
    fn empty_dimension_filters_are_byte_identical() {
        // The safety invariant: a metric with no filters (or an all-empty-values filter,
        // which is skipped) compiles to exactly the unfiltered SQL.
        let base = MetricDef::aggregate("bytes", "usage", AggregateOp::Sum, Some("amount"));
        let unfiltered = compile(
            &MetricRegistry::new().with(base.clone()),
            &base_query("bytes"),
            "raw_events_v2",
            "meteroid",
        )
        .unwrap();
        let mut with_empty = base;
        with_empty.filters.push(DimensionFilter {
            property: "region".to_string(),
            values: vec![],
            op: FilterOp::In,
        });
        let out = compile(
            &MetricRegistry::new().with(with_empty),
            &base_query("bytes"),
            "raw_events_v2",
            "meteroid",
        )
        .unwrap();
        assert_eq!(normalize(&out.sql), normalize(&unfiltered.sql));
    }

    #[test]
    fn scoped_series_filters_customer_without_projecting_it() {
        // A `customer_ids` scope must narrow the read (WHERE customer_id IN ?) without
        // becoming an output dimension — otherwise a non-broken-down series returns one
        // row per (window × customer) instead of one summed row per window.
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "calls",
            "api_call",
            AggregateOp::Count,
            None,
        ));
        let def = reg.get("calls").unwrap();
        let mut q = base_query("calls");
        q.customer_ids = vec![common_domain::ids::CustomerId::from(uuid::Uuid::from_u128(
            7,
        ))];
        let series = build_series(
            &reg,
            def,
            &q,
            "raw_events_v2",
            EventDedup::Off,
            "meteroid",
            None,
        )
        .unwrap();

        let sql = normalize(&series.sql);
        assert!(
            sql.contains("customer_id IN ?"),
            "scope kept as a WHERE filter: {sql}"
        );
        let outer_select = sql.split("FROM (").next().unwrap();
        assert!(
            !outer_select.contains("customer_id"),
            "outer SELECT omits customer_id: {outer_select}"
        );
        let group_by = sql.rsplit("GROUP BY").next().unwrap();
        assert!(
            !group_by.contains("customer_id"),
            "GROUP BY omits customer_id: {group_by}"
        );
        assert!(
            !series.dims.iter().any(|d| d == "customer_id"),
            "series dims omit customer_id: {:?}",
            series.dims
        );
    }

    #[test]
    fn count_needs_no_value_property() {
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "calls",
            "api_call",
            AggregateOp::Count,
            None,
        ));
        let out = compile(&reg, &base_query("calls"), "raw_events_v2", "meteroid").unwrap();
        assert!(out.sql.contains("toFloat64(count(*)) AS value"));
        assert!(!out.sql.contains("properties[?] != ''"));
    }

    #[test]
    fn gauge_latest_uses_argmax() {
        let reg = MetricRegistry::new().with(MetricDef {
            key: "seats".into(),
            event_code: "seats.set".into(),
            kind: MetricKind::Gauge(GaugeSpec {
                op: GaugeOp::Latest,
                value_property: Some("count".into()),
                carry_over: false,
            }),
            source: MetricSource::RawEvents,
            filters: Vec::new(),
        });
        let out = compile(&reg, &base_query("seats"), "raw_events_v2", "meteroid").unwrap();
        assert!(out.sql.contains(
            "argMax(toFloat64OrZero(properties[?]), (timestamp, ingested_at, id)) AS value"
        ));
    }

    #[test]
    fn time_weighted_gauge_structure() {
        let reg = MetricRegistry::new().with(MetricDef {
            key: "uptime".into(),
            event_code: "compute.instance".into(),
            kind: MetricKind::Gauge(GaugeSpec {
                op: GaugeOp::TimeWeighted(TimeWeightedSpec {
                    identity_property: "instance_id".into(),
                    lifecycle_codes: vec![
                        "compute.instance.create.end".into(),
                        "compute.instance.delete.end".into(),
                    ],
                    weight: TimeWeight::Seconds,
                }),
                value_property: None,
                carry_over: false,
            }),
            source: MetricSource::RawEvents,
            filters: Vec::new(),
        });
        let mut q = base_query("uptime");
        q.customer_ids = vec![common_domain::ids::CustomerId::from(uuid::Uuid::from_u128(
            1,
        ))];
        q.window = Some(WindowSize::Hour);
        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        assert!(out.sql.contains("leadInFrame(toNullable(timestamp))"));
        assert!(out.sql.contains("toStartOfHour"));
        assert!(out.sql.contains("dateDiff('second'"));
        assert!(out.sql.contains("code IN ?"));
        assert!(out.sql.contains("ARRAY JOIN range"));
    }

    #[test]
    fn derived_ratio_joins_sub_series() {
        let reg = MetricRegistry::new()
            .with(MetricDef::aggregate(
                "revenue",
                "charge",
                AggregateOp::Sum,
                Some("amount"),
            ))
            .with(MetricDef::aggregate(
                "users",
                "login",
                AggregateOp::CountDistinct,
                Some("user_id"),
            ))
            .with(MetricDef::derived(
                "arpu",
                Expr::Binary(
                    Box::new(Expr::Metric("revenue".into())),
                    BinOp::Div,
                    Box::new(Expr::Metric("users".into())),
                ),
            ));
        let out = compile(&reg, &base_query("arpu"), "raw_events_v2", "meteroid").unwrap();
        assert!(out.sql.contains("m_revenue AS ("));
        assert!(out.sql.contains("m_users AS ("));
        assert!(out.sql.contains(
            "FULL OUTER JOIN m_users ON m_revenue.window_start = m_users.window_start \
             AND m_revenue.window_end = m_users.window_end"
        ));
        assert!(
            out.sql
                .contains("(coalesce(m_revenue.value, 0) / nullIf(m_users.value, 0)) AS value")
        );
    }

    /// A registry with two leaf metrics plus a derived metric wrapping `revenue` in `expr`.
    fn derived_over_two(expr: Expr) -> MetricRegistry {
        MetricRegistry::new()
            .with(MetricDef::aggregate(
                "revenue",
                "charge",
                AggregateOp::Sum,
                Some("amount"),
            ))
            .with(MetricDef::aggregate(
                "users",
                "login",
                AggregateOp::CountDistinct,
                Some("user_id"),
            ))
            .with(MetricDef::derived("custom", expr))
    }

    #[test]
    fn derived_unary_neg_and_abs_lower_to_sql() {
        let neg = derived_over_two(Expr::Unary(
            UnaryOp::Neg,
            Box::new(Expr::Metric("revenue".into())),
        ));
        let out = compile(&neg, &base_query("custom"), "raw_events_v2", "meteroid").unwrap();
        assert!(
            out.sql
                .contains("(-(coalesce(m_revenue.value, 0))) AS value")
        );

        let abs = derived_over_two(Expr::Unary(
            UnaryOp::Abs,
            Box::new(Expr::Metric("revenue".into())),
        ));
        let out = compile(&abs, &base_query("custom"), "raw_events_v2", "meteroid").unwrap();
        assert!(
            out.sql
                .contains("abs(coalesce(m_revenue.value, 0)) AS value")
        );
    }

    #[test]
    fn derived_clamp_emits_only_present_bounds() {
        // Both bounds present: greatest (lower) then least (upper).
        let both = derived_over_two(Expr::Clamp {
            expr: Box::new(Expr::Metric("revenue".into())),
            min: Some(Box::new(Expr::Const(0.0))),
            max: Some(Box::new(Expr::Const(100.0))),
        });
        let out = compile(&both, &base_query("custom"), "raw_events_v2", "meteroid").unwrap();
        assert!(out.sql.contains("greatest("));
        assert!(out.sql.contains("least("));

        // Only `min`: greatest, but never least.
        let min_only = derived_over_two(Expr::Clamp {
            expr: Box::new(Expr::Metric("revenue".into())),
            min: Some(Box::new(Expr::Const(0.0))),
            max: None,
        });
        let out = compile(
            &min_only,
            &base_query("custom"),
            "raw_events_v2",
            "meteroid",
        )
        .unwrap();
        assert!(out.sql.contains("greatest("));
        assert!(!out.sql.contains("least("));
    }

    #[test]
    fn derived_coalesce_greatest_least_lower_to_sql() {
        let operands = || vec![Expr::Metric("revenue".into()), Expr::Metric("users".into())];

        let out = compile(
            &derived_over_two(Expr::Coalesce(operands())),
            &base_query("custom"),
            "raw_events_v2",
            "meteroid",
        )
        .unwrap();
        assert!(out.sql.contains(
            "coalesce(coalesce(m_revenue.value, 0), coalesce(m_users.value, 0)) AS value"
        ));

        let out = compile(
            &derived_over_two(Expr::Greatest(operands())),
            &base_query("custom"),
            "raw_events_v2",
            "meteroid",
        )
        .unwrap();
        assert!(out.sql.contains(
            "greatest(coalesce(m_revenue.value, 0), coalesce(m_users.value, 0)) AS value"
        ));

        let out = compile(
            &derived_over_two(Expr::Least(operands())),
            &base_query("custom"),
            "raw_events_v2",
            "meteroid",
        )
        .unwrap();
        assert!(
            out.sql.contains(
                "least(coalesce(m_revenue.value, 0), coalesce(m_users.value, 0)) AS value"
            )
        );
    }

    fn balance_gauge(carry_over: bool) -> MetricRegistry {
        MetricRegistry::new().with(MetricDef {
            key: "balance".into(),
            event_code: "balance.set".into(),
            kind: MetricKind::Gauge(GaugeSpec {
                op: GaugeOp::Latest,
                value_property: Some("amount".into()),
                carry_over,
            }),
            source: MetricSource::RawEvents,
            filters: Vec::new(),
        })
    }

    #[test]
    fn carry_over_gauge_forward_fills_and_floors() {
        let out = compile(
            &balance_gauge(true),
            &base_query("balance"),
            "raw_events_v2",
            "meteroid",
        )
        .unwrap();
        assert!(
            out.sql.contains("last_value(value) IGNORE NULLS OVER ("),
            "carry-over gauge must forward-fill empty windows with a window function"
        );
        assert!(
            out.sql
                .contains("WHERE window_start >= toDateTime(?) ORDER BY window_start"),
            "carry-over gauge must floor its output back to `from`"
        );
        assert!(matches!(out.binds.last(), Some(BindValue::I64(_))));
    }

    #[test]
    fn non_carry_over_gauge_has_no_forward_fill() {
        let out = compile(
            &balance_gauge(false),
            &base_query("balance"),
            "raw_events_v2",
            "meteroid",
        )
        .unwrap();
        assert!(
            !out.sql.contains("last_value(value) IGNORE NULLS"),
            "default (carry_over=false) gauge must keep its old output"
        );
        assert!(out.sql.contains(
            "argMax(toFloat64OrZero(properties[?]), (timestamp, ingested_at, id)) AS value"
        ));
    }

    #[test]
    fn carry_over_gauge_binds_match_placeholders() {
        // Positional binds: a `?`/bind count or order mismatch is a runtime failure the
        // string-contains assertions can't see, so pin parity explicitly.
        let out = compile(
            &balance_gauge(true),
            &base_query("balance"),
            "raw_events_v2",
            "meteroid",
        )
        .unwrap();
        assert_eq!(
            out.sql.matches('?').count(),
            out.binds.len(),
            "every placeholder must have exactly one bind"
        );
    }

    #[test]
    fn carry_over_gauge_grouped_fills_per_dimension_without_null_scaffold() {
        let mut query = base_query("balance");
        query.group_by = vec!["region".into()];
        let out = compile(&balance_gauge(true), &query, "raw_events_v2", "meteroid").unwrap();
        assert!(
            out.sql.contains("PARTITION BY _prop_region"),
            "grouped carry-over forward-fills within each dimension partition"
        );
        assert!(
            !out.sql.contains("NULL AS value"),
            "grouped case must not fabricate NULL-dimension scaffold rows"
        );
        assert_eq!(
            out.sql.matches('?').count(),
            out.binds.len(),
            "binds stay aligned in the grouped path too"
        );
    }

    #[rstest]
    #[case(
        CrossPeriod::RunningTotal,
        "sum(value) OVER (ORDER BY window_start ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS value"
    )]
    #[case(
        CrossPeriod::MovingAverage { periods: 7 },
        "avg(value) OVER (ORDER BY window_start ROWS BETWEEN 6 PRECEDING AND CURRENT ROW)"
    )]
    fn transform_emits_expected_window_function(
        #[case] transform: CrossPeriod,
        #[case] expected: &str,
    ) {
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "rev",
            "charge",
            AggregateOp::Sum,
            Some("amount"),
        ));
        let mut q = base_query("rev");
        q.transform = Some(transform);
        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        assert!(out.sql.contains(expected));
    }

    #[test]
    fn running_total_carries_an_opening_balance() {
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "rev",
            "charge",
            AggregateOp::Sum,
            Some("amount"),
        ));
        let mut q = base_query("rev");
        q.transform = Some(CrossPeriod::RunningTotal);
        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        assert!(
            out.sql
                .contains("WHERE window_start >= toDateTime(?) ORDER BY window_start"),
            "running total must floor its output to `from`, not its window function"
        );
        assert!(matches!(out.binds.last(), Some(BindValue::I64(_))));
        assert!(
            out.sql
                .contains("WITH FILL FROM toDateTime(tumbleStart(toDateTime(?),"),
            "running total must densify the visible range, floored to the bucket boundary"
        );
        assert!(
            out.sql.contains("INTERPOLATE (value AS value)"),
            "running total must carry the cumulative value forward into empty windows"
        );
    }

    #[test]
    fn period_over_period_primes_one_preceding_window() {
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "rev",
            "charge",
            AggregateOp::Sum,
            Some("amount"),
        ));
        let mut q = base_query("rev");
        q.transform = Some(CrossPeriod::PeriodOverPeriodDelta);
        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        assert!(
            out.sql
                .contains("WHERE window_start >= toDateTime(?) ORDER BY window_start"),
            "cross-period delta must prime preceding windows and floor its output"
        );
        assert!(matches!(out.binds.last(), Some(BindValue::I64(_))));
    }

    #[test]
    fn running_total_lagged_operand_compiles_to_lagged_window_primed_from_history() {
        // A retention-shaped derived metric: a within-window flow ÷ the *opening* balance
        // (lagged running total) of the same delta. Proves the per-operand transform mechanism.
        let reg = MetricRegistry::new()
            .with(MetricDef::aggregate(
                "flow",
                "charge",
                AggregateOp::Sum,
                Some("amount"),
            ))
            .with(MetricDef::aggregate(
                "delta",
                "charge",
                AggregateOp::Sum,
                Some("amount"),
            ))
            .with(MetricDef::derived(
                "ratio",
                Expr::Binary(
                    Box::new(Expr::Metric("flow".into())),
                    BinOp::Div,
                    Box::new(Expr::MetricTransformed {
                        metric: "delta".into(),
                        transform: CrossPeriod::RunningTotalLagged,
                    }),
                ),
            ));
        let out = compile(&reg, &base_query("ratio"), "raw_events_v2", "meteroid").unwrap();

        // The opening-balance operand is its own CTE with the lagged window function.
        assert!(
            out.sql.contains("m_delta__running_total_lagged AS ("),
            "transformed operand gets a distinct CTE"
        );
        assert!(
            out.sql.contains(
                "sum(value) OVER (ORDER BY window_start \
                 ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING) AS value"
            ),
            "lagged running total = opening balance window function"
        );
        // The within-window flow operand stays a plain CTE.
        assert!(out.sql.contains("m_flow AS ("));
        // Divide guards the (NULL/0) opening balance.
        assert!(
            out.sql
                .contains("/ nullIf(m_delta__running_total_lagged.value, 0))"),
            "opening balance denominator is null-guarded"
        );
        // History priming: the lagged operand reads from epoch, and the result is floored to `from`.
        assert!(
            out.sql.contains("timestamp >= toDateTime(?)"),
            "operand reads events"
        );
        assert!(
            out.sql
                .contains("WHERE window_start >= toDateTime(?) ORDER BY window_start"),
            "history-primed derived result is floored back to `from`"
        );
        assert_eq!(
            out.sql.matches('?').count(),
            out.binds.len(),
            "every placeholder has exactly one bind"
        );
    }

    #[test]
    fn unknown_metric_errors() {
        let reg = MetricRegistry::new();
        let err = compile(&reg, &base_query("nope"), "raw_events_v2", "meteroid").unwrap_err();
        assert_eq!(err, MetricError::UnknownMetric("nope".into()));
    }

    fn uptime_registry(identity: &str) -> MetricRegistry {
        MetricRegistry::new().with(MetricDef {
            key: "uptime".into(),
            event_code: "compute".into(),
            kind: MetricKind::Gauge(GaugeSpec {
                op: GaugeOp::TimeWeighted(TimeWeightedSpec {
                    identity_property: identity.into(),
                    lifecycle_codes: vec![],
                    weight: TimeWeight::Seconds,
                }),
                value_property: None,
                carry_over: false,
            }),
            source: MetricSource::RawEvents,
            filters: Vec::new(),
        })
    }

    #[test]
    fn time_weighted_rejects_unsafe_property_name() {
        let reg = uptime_registry("instance']) OR 1=1 --");
        let err = compile(&reg, &base_query("uptime"), "raw_events_v2", "meteroid").unwrap_err();
        assert!(matches!(err, MetricError::Invalid(_, _)));
    }

    fn mrr_fact() -> FactSource {
        FactSource {
            table: "fact_mrr_movement".into(),
            value_column: "delta_cents".into(),
            timestamp_column: "at".into(),
            customer_column: "customer_id".into(),
            tenant_column: "tenant_id".into(),
            snapshot: false,
        }
    }

    #[test]
    fn fact_aggregate_uses_real_columns() {
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "net_new_mrr",
            AggregateOp::Sum,
            mrr_fact(),
        ));
        let out = compile(
            &reg,
            &base_query("net_new_mrr"),
            "raw_events_v2",
            "meteroid",
        )
        .unwrap();
        assert!(out.sql.contains("FROM fact_mrr_movement FINAL WHERE"));
        assert!(out.sql.contains("sum(toFloat64(delta_cents)) AS value"));
        assert!(out.sql.contains("tumbleStart(toDateTime(at)"));
        assert!(!out.sql.contains("properties["));
        assert!(!out.sql.contains("LIMIT 1 BY"));
    }

    fn ar_snapshot_fact() -> FactSource {
        FactSource {
            table: "fact_ar_aging".into(),
            value_column: "outstanding_cents_usd".into(),
            timestamp_column: "snapshot_date".into(),
            customer_column: "customer_id".into(),
            tenant_column: "tenant_id".into(),
            snapshot: true,
        }
    }

    #[test]
    fn snapshot_fact_reduces_a_multiday_bucket_to_its_latest_day() {
        // A daily-snapshot stock (AR aging) summed over a week bucket would multiply the
        // standing balance by the days in the bucket (the reported bug: ~14k → ~72k at a
        // coarse grain). The compiler must total per snapshot day (inner) and keep only
        // the bucket's LATEST day (outer argMax), so the level is reported as-of the bucket.
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "outstanding_ar",
            AggregateOp::Sum,
            ar_snapshot_fact(),
        ));
        let mut q = base_query("outstanding_ar");
        q.window = Some(WindowSize::Week);
        q.to = Some(Utc.with_ymd_and_hms(2024, 3, 1, 0, 0, 0).unwrap());
        let sql = normalize(&compile(&reg, &q, "raw_events_v2", "meteroid").unwrap().sql);
        // Inner: sum across entities per snapshot day, carrying the day.
        assert!(
            sql.contains("toDate(snapshot_date) AS _snap_day"),
            "carries the snapshot day: {sql}"
        );
        assert!(
            sql.contains("sum(toFloat64(outstanding_cents_usd)) AS value"),
            "inner totals across entities: {sql}"
        );
        assert!(
            sql.contains(
                "toDateTime(tumbleEnd(toDateTime(snapshot_date), toIntervalWeek(1), ?), ?), toDate(snapshot_date) )"
            ),
            "inner groups per (bucket, day): {sql}"
        );
        // Outer: keep only the bucket's latest day, then order.
        assert!(
            sql.contains(
                "SELECT window_start, window_end, argMax(value, _snap_day) AS value FROM ("
            ),
            "outer keeps the latest day: {sql}"
        );
        assert!(
            sql.trim_end()
                .ends_with("GROUP BY window_start, window_end ORDER BY window_start"),
            "outer groups by the bucket only: {sql}"
        );
    }

    #[test]
    fn flow_fact_still_sums_the_whole_bucket() {
        // A non-snapshot fact (MRR movements) is a flow: summing the whole bucket is
        // correct, so it must be left completely untouched — no latest-day wrap.
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "net_new_mrr",
            AggregateOp::Sum,
            mrr_fact(),
        ));
        let mut q = base_query("net_new_mrr");
        q.window = Some(WindowSize::Week);
        let sql = normalize(&compile(&reg, &q, "raw_events_v2", "meteroid").unwrap().sql);
        assert!(
            !sql.contains("_snap_day"),
            "flow facts are not reduced: {sql}"
        );
        assert!(!sql.contains("argMax(value,"), "no latest-day wrap: {sql}");
        assert!(
            sql.contains("sum(toFloat64(delta_cents)) AS value"),
            "{sql}"
        );
    }

    #[test]
    fn snapshot_breakdown_carries_both_the_dimension_and_its_disambiguator() {
        // A per-customer split emits a value column AND a `_dis_` id column; the outer
        // latest-day reduction must group by (and re-select) both so the breakdown survives.
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "outstanding_ar",
            AggregateOp::Sum,
            ar_snapshot_fact(),
        ));
        let mut q = base_query("outstanding_ar");
        q.window = Some(WindowSize::Week);
        q.group_by = vec![Dimension::CustomerName("customer".into())];
        let sql = normalize(&compile(&reg, &q, "raw_events_v2", "meteroid").unwrap().sql);
        assert!(
            sql.contains("argMax(value, _snap_day) AS value, _prop_customer, _dis_customer FROM ("),
            "outer carries the label and the id disambiguator: {sql}"
        );
        assert!(
            sql.trim_end().ends_with(
                "GROUP BY window_start, window_end, _prop_customer, _dis_customer ORDER BY window_start"
            ),
            "outer groups by the bucket and the split: {sql}"
        );
    }

    #[test]
    fn grainless_snapshot_is_the_latest_days_level_over_the_whole_range() {
        // A KPI (no grain) over a snapshot must be the balance as-of the range end — the
        // latest day's total — not the sum of every day's balance across the range.
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "outstanding_ar",
            AggregateOp::Sum,
            ar_snapshot_fact(),
        ));
        let mut q = base_query("outstanding_ar");
        q.window = None;
        let sql = normalize(&compile(&reg, &q, "raw_events_v2", "meteroid").unwrap().sql);
        assert!(
            !sql.contains("tumbleStart"),
            "single whole-range bucket: {sql}"
        );
        assert!(
            sql.contains("GROUP BY toDate(snapshot_date) )"),
            "inner still totals per snapshot day: {sql}"
        );
        assert!(sql.contains("argMax(value, _snap_day) AS value"), "{sql}");
    }

    #[test]
    fn snapshot_reduction_survives_fx_conversion() {
        // With a reporting currency the read is an FX dedup sub-query; the latest-day
        // reduction must wrap it and rank on the fact's own day (`f.snapshot_date`).
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "outstanding_ar",
            AggregateOp::Sum,
            ar_snapshot_fact(),
        ));
        let mut q = base_query("outstanding_ar");
        q.window = Some(WindowSize::Week);
        q.reporting_currency = Some("EUR".into());
        let sql = normalize(&compile(&reg, &q, "raw_events_v2", "meteroid").unwrap().sql);
        assert!(
            sql.contains("ASOF LEFT JOIN"),
            "FX conversion is active: {sql}"
        );
        assert!(
            sql.contains("toDate(f.snapshot_date) AS _snap_day"),
            "ranks on the fact's own snapshot day inside the FX sub-query: {sql}"
        );
        assert!(sql.contains("argMax(value, _snap_day) AS value"), "{sql}");
    }

    fn mrr_usd_fact() -> FactSource {
        FactSource {
            value_column: "delta_cents_usd".into(),
            ..mrr_fact()
        }
    }

    /// Substitute each bind into its `?` so a shifted bind list fails loudly. Asserting on the
    /// SQL text alone cannot tell a correct ordering from a misordered one.
    fn inline_binds(out: &SafeQuery) -> String {
        let mut sql = String::new();
        let mut binds = out.binds.iter();
        for ch in out.sql.chars() {
            if ch != '?' {
                sql.push(ch);
                continue;
            }
            let rendered = match binds.next().expect("a bind for every placeholder") {
                BindValue::String(s) => format!("'{s}'"),
                BindValue::I64(v) => v.to_string(),
                BindValue::U32(v) => v.to_string(),
                BindValue::Uuid(v) => format!("'{v}'"),
                BindValue::Strings(v) => format!("{v:?}"),
                BindValue::Uuids(v) => format!("{v:?}"),
            };
            sql.push_str(&rendered);
        }
        assert!(
            binds.next().is_none(),
            "every bind consumed by a placeholder"
        );
        normalize(&sql)
    }

    fn fx_series_sql(to: Option<chrono::DateTime<Utc>>) -> String {
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "mrr",
            AggregateOp::Sum,
            mrr_usd_fact(),
        ));
        let mut q = base_query("mrr");
        q.reporting_currency = Some("EUR".into());
        q.to = to;
        inline_binds(&compile(&reg, &q, "raw_events_v2", "meteroid").unwrap())
    }

    #[test]
    fn fx_join_builds_only_the_reporting_currency() {
        let sql = fx_series_sql(base_query("mrr").to);
        assert!(
            sql.contains(
                "ASOF LEFT JOIN ( SELECT currency, date, rate_from_usd FROM dim_fx_rate FINAL \
                 WHERE currency = 'EUR' AND date <= toDate(toDateTime(1704153600)) ) AS fx"
            ),
            "build side is restricted to the reporting currency, deduped, and bounded: {sql}"
        );
        // The fact read keeps its own binds — proof the FX binds landed after the fact WHERE.
        assert!(
            sql.contains("AND toDateTime(at) >= toDateTime(1704067200)"),
            "{sql}"
        );
    }

    #[test]
    fn fx_build_side_has_no_lower_date_bound() {
        // Rates are sparse before ~2024 and history-primed operands read from epoch, so a lower
        // bound could drop the ASOF match a row needs. Guard against one being added.
        let sql = fx_series_sql(base_query("mrr").to);
        let fx = sql.split("ASOF LEFT JOIN (").nth(1).expect("fx sub-query");
        let fx = fx.split(") AS fx").next().unwrap();
        assert!(
            !fx.contains("date >="),
            "no lower bound on the build side: {fx}"
        );
    }

    #[test]
    fn fx_build_side_is_unbounded_when_the_query_has_no_end() {
        let sql = fx_series_sql(None);
        assert!(
            sql.contains("FROM dim_fx_rate FINAL WHERE currency = 'EUR' ) AS fx"),
            "an open-ended query bounds the build side by currency alone: {sql}"
        );
    }

    /// A derived metric over N movement components isolated by `movement_type`, the shape every
    /// retention ratio in the catalog takes.
    fn movement_ratio_registry(fact: FactSource) -> MetricRegistry {
        MetricRegistry::new()
            .with(
                MetricDef::fact_aggregate("gained", AggregateOp::Sum, fact.clone())
                    .with_filter("movement_type", &["NewBusiness"]),
            )
            .with(
                MetricDef::fact_aggregate("lost", AggregateOp::Sum, fact)
                    .with_filter("movement_type", &["Churn"]),
            )
            .with(MetricDef::derived(
                "ratio",
                Expr::Binary(
                    Box::new(Expr::Metric("gained".into())),
                    BinOp::Div,
                    Box::new(Expr::Metric("lost".into())),
                ),
            ))
    }

    #[test]
    fn same_source_operands_fuse_into_one_scan() {
        let reg = movement_ratio_registry(mrr_fact());
        // Inlined, so the assertions pin each metric's column to its own filter value.
        let sql = normalize(&inline_binds(
            &compile(&reg, &base_query("ratio"), "raw_events_v2", "meteroid").unwrap(),
        ));
        assert_eq!(
            sql.matches("FROM fact_mrr_movement FINAL").count(),
            1,
            "one scan serves both operands: {sql}"
        );
        assert!(
            !sql.contains("FULL OUTER JOIN"),
            "nothing left to join: {sql}"
        );
        assert!(
            sql.contains(
                "sumIf(toFloat64(delta_cents), (movement_type = 'NewBusiness')) AS value_gained"
            ) && sql
                .contains("sumIf(toFloat64(delta_cents), (movement_type = 'Churn')) AS value_lost"),
            "each operand becomes its own conditional aggregate, named for its metric: {sql}"
        );
        // The read is narrowed to the union, so unrelated movement types never enter the group by.
        assert!(
            sql.contains("AND ((movement_type = 'NewBusiness') OR (movement_type = 'Churn'))"),
            "{sql}"
        );
        assert!(
            sql.contains("coalesce(m_fused_0.value_gained, 0) / nullIf(m_fused_0.value_lost, 0)"),
            "the expression reads both columns out of the one CTE: {sql}"
        );
    }

    #[test]
    fn fused_fx_read_projects_the_predicate_columns() {
        // The `*If` predicates are evaluated above the FX dedup projection, so the columns they
        // test must survive it — otherwise ClickHouse rejects the query with UNKNOWN_IDENTIFIER.
        let reg = movement_ratio_registry(mrr_usd_fact());
        let mut q = base_query("ratio");
        q.reporting_currency = Some("EUR".into());
        let sql = normalize(&compile(&reg, &q, "raw_events_v2", "meteroid").unwrap().sql);
        assert!(
            sql.contains(
                "SELECT delta_cents_usd, delta_cents, currency, at, movement_type, \
                 toLowCardinality(?) AS fx_currency"
            ),
            "the dedup sub-query carries movement_type: {sql}"
        );
    }

    #[test]
    fn operands_that_cannot_fuse_keep_their_own_cte() {
        // Different fact sources have nothing to share, so the join chain must survive.
        let other = FactSource {
            table: "fact_revenue_item".into(),
            value_column: "amount_subtotal_cents".into(),
            timestamp_column: "billed_at".into(),
            ..mrr_fact()
        };
        let reg = MetricRegistry::new()
            .with(
                MetricDef::fact_aggregate("gained", AggregateOp::Sum, mrr_fact())
                    .with_filter("movement_type", &["NewBusiness"]),
            )
            .with(
                MetricDef::fact_aggregate("billed", AggregateOp::Sum, other)
                    .with_filter("charge_type", &["Recurring"]),
            )
            .with(MetricDef::derived(
                "ratio",
                Expr::Binary(
                    Box::new(Expr::Metric("gained".into())),
                    BinOp::Div,
                    Box::new(Expr::Metric("billed".into())),
                ),
            ));
        let sql = normalize(
            &compile(&reg, &base_query("ratio"), "raw_events_v2", "meteroid")
                .unwrap()
                .sql,
        );
        assert!(sql.contains("m_gained AS ("), "{sql}");
        assert!(sql.contains("m_billed AS ("), "{sql}");
        assert!(sql.contains("FULL OUTER JOIN m_billed"), "{sql}");
        assert!(!sql.contains("m_fused"), "{sql}");
    }

    #[test]
    fn a_transformed_operand_never_fuses_with_its_plain_twin() {
        // The transformed side reads an epoch-primed range; folding it into the plain operand's
        // scan would silently change the window it aggregates.
        let reg = MetricRegistry::new()
            .with(
                MetricDef::fact_aggregate("churn", AggregateOp::Sum, mrr_fact())
                    .with_filter("movement_type", &["Churn"]),
            )
            .with(
                MetricDef::fact_aggregate("delta", AggregateOp::Sum, mrr_fact())
                    .with_filter("movement_type", &["NewBusiness"]),
            )
            .with(MetricDef::derived(
                "grr",
                Expr::Binary(
                    Box::new(Expr::Metric("churn".into())),
                    BinOp::Div,
                    Box::new(Expr::MetricTransformed {
                        metric: "delta".into(),
                        transform: CrossPeriod::RunningTotalLagged,
                    }),
                ),
            ));
        let sql = normalize(
            &compile(&reg, &base_query("grr"), "raw_events_v2", "meteroid")
                .unwrap()
                .sql,
        );
        assert!(sql.contains("m_churn AS ("), "{sql}");
        assert!(sql.contains("m_delta__running_total_lagged AS ("), "{sql}");
        assert!(!sql.contains("m_fused"), "{sql}");
        assert_eq!(
            sql.matches("FROM fact_mrr_movement FINAL").count(),
            2,
            "the primed operand keeps its own read: {sql}"
        );
    }

    #[test]
    fn event_property_dimension_is_param_bound() {
        // Grouping a usage metric by an arbitrary event property must read the
        // property map with the name BOUND (never inlined), so any property is safe.
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "calls",
            "api",
            AggregateOp::Count,
            None,
        ));
        let mut q = base_query("calls");
        q.group_by = vec![Dimension::Property("region".into())];
        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        let sql = normalize(&out.sql);
        assert!(sql.contains("properties[?] AS _prop_region"), "{sql}");
        assert!(sql.contains("GROUP BY"));
        // The property name travels as a bind value, not an inlined identifier.
        assert!(!out.sql.contains("'region'"));
        assert!(
            out.binds
                .iter()
                .any(|b| matches!(b, BindValue::String(s) if s == "region"))
        );
    }

    #[test]
    fn auto_dimension_is_a_column_on_facts_and_a_property_on_events() {
        // Same bare dimension, two sources: a fact groups by the native column,
        // a usage metric groups by the event property.
        let fact = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "net_new_mrr",
            AggregateOp::Sum,
            mrr_fact(),
        ));
        let mut fq = base_query("net_new_mrr");
        fq.group_by = vec![Dimension::Auto("movement_type".into())];
        let fsql = normalize(
            &compile(&fact, &fq, "raw_events_v2", "meteroid")
                .unwrap()
                .sql,
        );
        assert!(
            fsql.contains("movement_type AS _prop_movement_type"),
            "{fsql}"
        );
        assert!(
            !fsql.contains("properties["),
            "fact must not read the property map"
        );

        let ev = MetricRegistry::new().with(MetricDef::aggregate(
            "calls",
            "api",
            AggregateOp::Count,
            None,
        ));
        let mut eq = base_query("calls");
        eq.group_by = vec![Dimension::Auto("movement_type".into())];
        let esql = normalize(&compile(&ev, &eq, "raw_events_v2", "meteroid").unwrap().sql);
        assert!(
            esql.contains("properties[?] AS _prop_movement_type"),
            "{esql}"
        );
    }

    #[test]
    fn customer_attribute_dimension_compiles_to_dictget() {
        // Slicing a fact metric by a customer attribute resolves through the
        // dim_customer dictionary by customer id — no join, attribute inlined as a
        // constant (dictGet requires it), the lookup grouped on.
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "net_new_mrr",
            AggregateOp::Sum,
            mrr_fact(),
        ));
        let mut q = base_query("net_new_mrr");
        q.group_by = vec![Dimension::CustomerAttr("country".into())];
        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        let sql = normalize(&out.sql);
        assert!(
            sql.contains(
                "dictGet('meteroid.dim_customer_dict', 'country', tuple(customer_id)) AS _prop_country"
            ),
            "{sql}"
        );
        assert!(sql.contains("GROUP BY"));
    }

    #[test]
    fn plan_attribute_dimension_compiles_to_dictget() {
        // Slicing a fact metric by a plan attribute resolves through the dim_plan
        // dictionary by plan id — no join, attribute inlined as a constant (dictGet
        // requires it), the lookup grouped on. Mirrors the customer-attr case.
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "net_new_mrr",
            AggregateOp::Sum,
            mrr_fact(),
        ));
        let mut q = base_query("net_new_mrr");
        // The carried string is the output key ("plan"); the dict attribute is the
        // fixed 'name' column, so the series key under "plan" (what the FE requests).
        q.group_by = vec![Dimension::PlanAttr("plan".into())];
        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        let sql = normalize(&out.sql);
        // plan_id is Nullable, so the key is coalesced to the nil UUID before the
        // lookup — a raw Nullable key makes dictGet throw at query time.
        assert!(
            sql.contains("dictGetOrDefault('meteroid.dim_plan_dict', 'name', tuple(ifNull(plan_id, toUUID('00000000-0000-0000-0000-000000000000'))), '(none)') AS _prop_plan"),
            "{sql}"
        );
        assert!(sql.contains("GROUP BY"));
    }

    #[test]
    fn customer_attr_map_dimension_indexes_the_attributes_map() {
        // A derived/synced attribute (lifecycle_state, campaign id) lives inside the
        // dim_customer `attributes` map, read via dictGet of the constant 'attributes'
        // attribute then subscripted by the allowlist-validated, inlined key.
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "net_new_mrr",
            AggregateOp::Sum,
            mrr_fact(),
        ));
        // The map key read (lifecycle_state) is independent of the output alias
        // (customer_type) the caller looks results up by.
        let mut q = base_query("net_new_mrr");
        q.group_by = vec![Dimension::CustomerAttrMap {
            map_key: "lifecycle_state".into(),
            alias: "customer_type".into(),
        }];
        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        let sql = normalize(&out.sql);
        assert!(
            sql.contains(
                "JSONExtractString(dictGet('meteroid.dim_customer_dict', 'attributes', tuple(customer_id)), 'lifecycle_state') AS _prop_customer_type"
            ),
            "{sql}"
        );
        assert!(sql.contains("GROUP BY"));
    }

    #[test]
    fn customer_property_dimension_binds_the_key_and_reads_it_as_text() {
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "net_new_mrr",
            AggregateOp::Sum,
            mrr_fact(),
        ));
        let mut q = base_query("net_new_mrr");
        q.group_by = vec![Dimension::CustomerProperty {
            key: "parent's company".into(),
            alias: "cp:parent_company".into(),
        }];
        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        let sql = normalize(&out.sql);
        let json = "dictGet('meteroid.dim_customer_dict', 'custom_properties', tuple(customer_id))";
        assert!(
            sql.contains(&format!(
                "multiIf(JSONType({json}, ?) = 'String', JSONExtractString({json}, ?), JSONType({json}, ?) = 'Null', '(none)', JSONExtractRaw({json}, ?)) AS _prop_cp_parent_company"
            )),
            "{sql}"
        );
        assert!(!sql.contains("parent's company"), "{sql}");
        let key_binds = out
            .binds
            .iter()
            .filter(|b| matches!(b, BindValue::String(s) if s == "parent's company"))
            .count();
        assert_eq!(key_binds % 4, 0);
        assert!(key_binds >= 4);
        assert_eq!(sql.matches('?').count(), out.binds.len(), "{sql}");
    }

    #[test]
    fn product_attribute_dimension_compiles_to_dictget() {
        // Slicing a revenue-item metric by a product attribute resolves through the
        // dim_product dictionary by product id — no join, attribute inlined as a
        // constant, the lookup grouped on. This is what lets revenue be collected per
        // individual product, not just per plan. Mirrors the plan-attr case.
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "billed_revenue",
            AggregateOp::Sum,
            FactSource {
                table: "fact_revenue_item".into(),
                value_column: "amount_subtotal_cents_usd".into(),
                timestamp_column: "billed_at".into(),
                customer_column: "customer_id".into(),
                tenant_column: "tenant_id".into(),
                snapshot: false,
            },
        ));
        let mut q = base_query("billed_revenue");
        // Carried string "product" is the output key; the dict attribute is 'name'.
        q.group_by = vec![Dimension::ProductAttr("product".into())];
        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        let sql = normalize(&out.sql);
        // product_id is Nullable, so the key is null-guarded the same way as plan_id.
        assert!(
            sql.contains("dictGetOrDefault('meteroid.dim_product_dict', 'name', tuple(ifNull(product_id, toUUID('00000000-0000-0000-0000-000000000000'))), '(none)') AS _prop_product"),
            "{sql}"
        );
        assert!(sql.contains("GROUP BY"));
    }

    #[test]
    fn entity_breakdown_groups_by_id_not_the_resolved_name() {
        // Decision A: an entity dimension groups on the id so two products sharing a
        // name stay distinct series; the dict name is only the display value, and a
        // `_dis_` column carries the id for the backend to disambiguate on.
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "billed_revenue",
            AggregateOp::Sum,
            FactSource {
                table: "fact_revenue_item".into(),
                value_column: "amount_subtotal_cents_usd".into(),
                timestamp_column: "billed_at".into(),
                customer_column: "customer_id".into(),
                tenant_column: "tenant_id".into(),
                snapshot: false,
            },
        ));
        let mut q = base_query("billed_revenue");
        q.group_by = vec![Dimension::ProductAttr("product".into())];
        let sql = normalize(&compile(&reg, &q, "raw_events_v2", "meteroid").unwrap().sql);
        let group_by = sql.rsplit("GROUP BY").next().unwrap();
        assert!(
            group_by.contains("ifNull(product_id, toUUID('00000000-0000-0000-0000-000000000000'))"),
            "entity dim groups on the id: {sql}"
        );
        assert!(
            !group_by.contains("dictGet"),
            "entity dim must group on the id, not the resolved name: {sql}"
        );
        assert!(
            sql.contains(
                "toString(ifNull(product_id, toUUID('00000000-0000-0000-0000-000000000000'))) AS _dis_product"
            ),
            "emits the id disambiguator column: {sql}"
        );
    }

    #[test]
    fn customer_name_dimension_splits_per_customer_by_id() {
        // A per-customer breakdown resolves the name via dim_customer but groups on
        // customer_id (distinct same-named customers stay distinct).
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "net_new_mrr",
            AggregateOp::Sum,
            mrr_fact(),
        ));
        let mut q = base_query("net_new_mrr");
        q.group_by = vec![Dimension::CustomerName("customer".into())];
        let sql = normalize(&compile(&reg, &q, "raw_events_v2", "meteroid").unwrap().sql);
        assert!(
            sql.contains("dictGetOrDefault('meteroid.dim_customer_dict', 'name', tuple(ifNull(customer_id, toUUID('00000000-0000-0000-0000-000000000000'))), '(none)') AS _prop_customer"),
            "labels from dim_customer: {sql}"
        );
        let group_by = sql.rsplit("GROUP BY").next().unwrap();
        assert!(
            group_by
                .contains("ifNull(customer_id, toUUID('00000000-0000-0000-0000-000000000000'))"),
            "groups on customer_id: {sql}"
        );
        assert!(sql.contains("AS _dis_customer"), "{sql}");
    }

    #[test]
    fn customer_attr_map_key_is_injection_safe() {
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "net_new_mrr",
            AggregateOp::Sum,
            mrr_fact(),
        ));
        let mut q = base_query("net_new_mrr");
        q.group_by = vec![Dimension::CustomerAttrMap {
            map_key: "x'] = 1 OR dictGet('y".into(),
            alias: "customer_type".into(),
        }];
        assert!(compile(&reg, &q, "raw_events_v2", "meteroid").is_err());
    }

    #[test]
    fn product_attribute_dimension_carries_product_id_through_fx_dedup() {
        // With FX active the read is a dedup sub-query; a product-attr split must still
        // see product_id inside it so the outer dictGet(f.product_id) resolves.
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "billed_revenue",
            AggregateOp::Sum,
            FactSource {
                table: "fact_revenue_item".into(),
                value_column: "amount_subtotal_cents_usd".into(),
                timestamp_column: "billed_at".into(),
                customer_column: "customer_id".into(),
                tenant_column: "tenant_id".into(),
                snapshot: false,
            },
        ));
        let mut q = base_query("billed_revenue");
        q.reporting_currency = Some("EUR".into());
        q.group_by = vec![Dimension::ProductAttr("product".into())];
        let sql = normalize(&compile(&reg, &q, "raw_events_v2", "meteroid").unwrap().sql);
        assert!(
            sql.contains("dictGetOrDefault('meteroid.dim_product_dict', 'name', tuple(ifNull(f.product_id, toUUID('00000000-0000-0000-0000-000000000000'))), '(none)') AS _prop_product"),
            "{sql}"
        );
        assert!(sql.contains("product_id"), "{sql}");
    }

    #[test]
    fn customer_attribute_dimension_carries_customer_id_through_fx_dedup() {
        // With FX conversion active the read is a dedup sub-query; a customer-attr
        // split must still see customer_id inside it even with no customer scope.
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "billed_revenue",
            AggregateOp::Sum,
            FactSource {
                table: "fact_revenue_item".into(),
                value_column: "amount_cents_usd".into(),
                timestamp_column: "billed_at".into(),
                customer_column: "customer_id".into(),
                tenant_column: "tenant_id".into(),
                snapshot: false,
            },
        ));
        let mut q = base_query("billed_revenue");
        q.reporting_currency = Some("EUR".into());
        q.group_by = vec![Dimension::CustomerAttr("country".into())];
        let sql = normalize(&compile(&reg, &q, "raw_events_v2", "meteroid").unwrap().sql);
        assert!(
            sql.contains("dictGet('meteroid.dim_customer_dict', 'country', tuple(f.customer_id))"),
            "{sql}"
        );
        // customer_id is projected into the dedup sub-query so f.customer_id exists.
        assert!(sql.contains("SELECT amount_cents_usd"), "{sql}");
        assert!(sql.contains("customer_id"), "{sql}");
    }

    #[test]
    fn customer_property_dimension_carries_customer_id_through_fx_dedup() {
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "billed_revenue",
            AggregateOp::Sum,
            FactSource {
                table: "fact_revenue_item".into(),
                value_column: "amount_cents_usd".into(),
                timestamp_column: "billed_at".into(),
                customer_column: "customer_id".into(),
                tenant_column: "tenant_id".into(),
                snapshot: false,
            },
        ));
        let mut q = base_query("billed_revenue");
        q.reporting_currency = Some("EUR".into());
        q.group_by = vec![Dimension::CustomerProperty {
            key: "parent_company".into(),
            alias: "cp:parent_company".into(),
        }];
        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        let sql = normalize(&out.sql);
        assert!(
            sql.contains(
                "dictGet('meteroid.dim_customer_dict', 'custom_properties', tuple(f.customer_id))"
            ),
            "{sql}"
        );
        assert_eq!(sql.matches('?').count(), out.binds.len(), "{sql}");
    }

    #[test]
    fn dimension_names_are_injection_safe() {
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "net_new_mrr",
            AggregateOp::Sum,
            mrr_fact(),
        ));
        // An inlined dimension (column / customer-attr) with a quote is rejected,
        // so it can never break out of the SQL.
        let evil = "country','x') = 1 OR dictGet('y";
        let mut q = base_query("net_new_mrr");
        q.group_by = vec![Dimension::CustomerAttr(evil.into())];
        assert!(matches!(
            compile(&reg, &q, "raw_events_v2", "meteroid"),
            Err(MetricError::Invalid(_, _))
        ));
        let mut q2 = base_query("net_new_mrr");
        q2.group_by = vec![Dimension::Column("a; DROP TABLE x".into())];
        assert!(matches!(
            compile(&reg, &q2, "raw_events_v2", "meteroid"),
            Err(MetricError::Invalid(_, _))
        ));
    }

    #[test]
    fn the_acid_test_compound_metric_over_a_cohort() {
        let reg = MetricRegistry::new()
            .with(MetricDef::aggregate(
                "revenue",
                "charge",
                AggregateOp::Sum,
                Some("amount"),
            ))
            .with(MetricDef::aggregate(
                "users",
                "login",
                AggregateOp::CountDistinct,
                Some("user_id"),
            ))
            .with(MetricDef::derived(
                "arppu",
                Expr::Binary(
                    Box::new(Expr::Metric("revenue".into())),
                    BinOp::Div,
                    Box::new(Expr::Metric("users".into())),
                ),
            ))
            .with(MetricDef::fact_aggregate(
                "mrr",
                AggregateOp::Sum,
                mrr_fact(),
            ));

        let mut q = base_query("arppu");
        q.cohort = Some(crate::metric::CohortDef {
            predicates: vec![
                CohortPredicate::MetricThreshold {
                    metric: "mrr".into(),
                    op: ThresholdOp::Gt,
                    threshold: 1000.0,
                },
                CohortPredicate::Dimension {
                    table: "dim_customer".into(),
                    id_column: "id".into(),
                    column: "country".into(),
                    values: vec!["FR".into()],
                },
            ],
        });

        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        assert!(out.sql.contains("m_revenue AS ("));
        assert!(
            out.sql
                .contains("(coalesce(m_revenue.value, 0) / nullIf(m_users.value, 0))")
        );
        assert!(out.sql.contains("FROM fact_mrr_movement FINAL WHERE"));
        assert!(
            out.sql
                .contains("HAVING sum(toFloat64(delta_cents)) > 1000")
        );
        assert!(out.sql.contains("INTERSECT"));
        assert!(
            out.sql
                .contains("FROM dim_customer FINAL WHERE tenant_id = ? AND country IN ?")
        );
        assert_eq!(out.sql.matches("customer_id IN (").count(), 2);
    }

    #[test]
    fn inactive_predicate_anti_joins_the_customer_universe() {
        // "NO usage" — the tenant's dim_customer MINUS the set with any activity on the
        // metric in the window. Pure absence: max_active None defaults the active set to
        // a > 0 aggregate. The active subquery's binds (tenant/code/from/to) sit after the
        // outer dim_customer tenant bind.
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "api_calls",
            "api",
            AggregateOp::Count,
            None,
        ));
        let mut q = base_query("api_calls");
        q.cohort = Some(crate::metric::CohortDef {
            predicates: vec![CohortPredicate::MetricInactive {
                metric: "api_calls".into(),
                max_active: None,
            }],
        });
        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        let sql = normalize(&out.sql);
        assert!(
            sql.contains(
                "SELECT id AS customer_id FROM dim_customer FINAL WHERE tenant_id = ? AND id NOT IN ("
            ),
            "{sql}"
        );
        assert!(
            sql.contains("SELECT customer_id FROM raw_events_v2 WHERE tenant_id = ? AND code = ?"),
            "{sql}"
        );
        assert!(sql.contains("HAVING count(*) > 0"), "{sql}");
        // No string-interpolated identifiers: the metric/code are bound.
        assert!(sql.contains("customer_id IN ("), "{sql}");
    }

    #[test]
    fn inactive_predicate_supports_active_below_threshold() {
        // max_active Some(t) restricts the active set to aggregate > t, so the complement
        // is "inactive or active <= t".
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "mrr",
            AggregateOp::Sum,
            mrr_fact(),
        ));
        let mut q = base_query("mrr");
        q.cohort = Some(crate::metric::CohortDef {
            predicates: vec![CohortPredicate::MetricInactive {
                metric: "mrr".into(),
                max_active: Some(100.0),
            }],
        });
        let sql = normalize(&compile(&reg, &q, "raw_events_v2", "meteroid").unwrap().sql);
        assert!(sql.contains("id NOT IN ("), "{sql}");
        assert!(
            sql.contains("HAVING sum(toFloat64(delta_cents)) > 100"),
            "{sql}"
        );
    }

    #[test]
    fn delta_predicate_compiles_to_conditional_aggregate_window_over_window() {
        // G4 MetricDelta: ONE grouped scan over [prior_from, to] with `sumIf` splitting the
        // current window [from, to] (`cur`) from the prior window [prior_from, from) (`prior`,
        // same op, different time predicate), then a HAVING on the percentage change with a
        // `prior != 0` divide-by-zero guard. "down >=30%" is Lte / -30.
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "bytes",
            "usage",
            AggregateOp::Sum,
            Some("amount"),
        ));
        let cohort = crate::metric::CohortDef {
            predicates: vec![CohortPredicate::MetricDelta {
                metric: "bytes".into(),
                op: ThresholdOp::Lte,
                pct: -30.0,
                prior_window_days: None,
            }],
        };
        let q = base_query("bytes");
        let sql = normalize(
            &compile_cohort_members(&reg, &cohort, &q, "raw_events_v2")
                .unwrap()
                .sql,
        );
        assert!(
            sql.contains(
                "toFloat64(sumIf(toFloat64OrZero(properties[?]), timestamp >= toDateTime(?))) AS cur"
            ),
            "current-window sumIf: {sql}"
        );
        assert!(
            sql.contains(
                "toFloat64(sumIf(toFloat64OrZero(properties[?]), timestamp < toDateTime(?) AND timestamp >= toDateTime(?))) AS prior"
            ),
            "prior-window sumIf: {sql}"
        );
        assert!(
            sql.contains("HAVING prior != 0 AND (cur - prior) / prior * 100 <= -30"),
            "percentage-change HAVING with prior guard: {sql}"
        );
        // A single grouped scan — cur and prior are conditional aggregates over the same
        // GROUP BY, not two separate scans.
        assert_eq!(
            sql.matches("GROUP BY customer_id").count(),
            1,
            "one grouped scan: {sql}"
        );
    }

    #[test]
    fn cohort_threshold_compares_in_converted_units() {
        // A factored usage metric: the threshold is compared against the aggregate divided
        // by the unit-conversion factor, so "> 5" means 5 converted units, not 5 raw units.
        let reg = MetricRegistry::new().with(
            MetricDef::aggregate("bytes", "usage", AggregateOp::Sum, Some("amount"))
                .with_unit_conversion_factor(Some(1024.0)),
        );
        let cohort = crate::metric::CohortDef {
            predicates: vec![CohortPredicate::MetricThreshold {
                metric: "bytes".into(),
                op: ThresholdOp::Gt,
                threshold: 5.0,
            }],
        };
        let sql = normalize(
            &compile_cohort_members(&reg, &cohort, &base_query("bytes"), "raw_events_v2")
                .unwrap()
                .sql,
        );
        assert!(
            sql.contains("HAVING (sum(toFloat64OrZero(properties[?]))) / 1024 > 5"),
            "threshold HAVING divides by factor: {sql}"
        );
    }

    #[test]
    fn cohort_delta_ignores_the_conversion_factor() {
        // A percentage delta is invariant to a constant divisor (it cancels between the
        // current and prior aggregates), so a factored metric compiles byte-identically to
        // the unconverted one — the factor must NOT appear in the delta SQL.
        let base = MetricDef::aggregate("bytes", "usage", AggregateOp::Sum, Some("amount"));
        let cohort = crate::metric::CohortDef {
            predicates: vec![CohortPredicate::MetricDelta {
                metric: "bytes".into(),
                op: ThresholdOp::Lte,
                pct: -30.0,
                prior_window_days: None,
            }],
        };
        let unconverted = normalize(
            &compile_cohort_members(
                &MetricRegistry::new().with(base.clone()),
                &cohort,
                &base_query("bytes"),
                "raw_events_v2",
            )
            .unwrap()
            .sql,
        );
        let converted = normalize(
            &compile_cohort_members(
                &MetricRegistry::new().with(base.with_unit_conversion_factor(Some(1024.0))),
                &cohort,
                &base_query("bytes"),
                "raw_events_v2",
            )
            .unwrap()
            .sql,
        );
        assert_eq!(converted, unconverted, "delta must ignore the factor");
        assert!(
            !converted.contains("1024"),
            "no factor in delta SQL: {converted}"
        );
    }

    #[test]
    fn delta_prior_window_defaults_to_current_length_and_shifts_with_explicit_days() {
        // The prior lower bound is a server-computed bound (never client input). Default:
        // prior length = current window length (base_query is a 1-day window). Explicit
        // `prior_window_days` overrides it, shifting the prior lower bound.
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "bytes",
            "usage",
            AggregateOp::Sum,
            Some("amount"),
        ));
        let q = base_query("bytes");
        let from_ts = q.from.timestamp();
        let to_ts = q.to.unwrap().timestamp();
        let day = 86_400i64;

        let time_binds = |prior_window_days: Option<u32>| -> Vec<i64> {
            let cohort = crate::metric::CohortDef {
                predicates: vec![CohortPredicate::MetricDelta {
                    metric: "bytes".into(),
                    op: ThresholdOp::Lte,
                    pct: -30.0,
                    prior_window_days,
                }],
            };
            compile_cohort_members(&reg, &cohort, &q, "raw_events_v2")
                .unwrap()
                .binds
                .iter()
                .filter_map(|b| match b {
                    BindValue::I64(v) => Some(*v),
                    _ => None,
                })
                .collect()
        };

        let default = time_binds(None);
        assert!(
            default.contains(&from_ts),
            "current lower bound: {default:?}"
        );
        assert!(default.contains(&to_ts), "upper scan bound: {default:?}");
        assert!(
            default.contains(&(from_ts - day)),
            "default prior lower bound = from - (to - from): {default:?}"
        );

        let explicit = time_binds(Some(7));
        assert!(
            explicit.contains(&(from_ts - 7 * day)),
            "explicit prior lower bound = from - 7d: {explicit:?}"
        );
        assert!(
            !explicit.contains(&(from_ts - day)),
            "an explicit prior window shifts the boundary off the default: {explicit:?}"
        );
    }

    #[test]
    fn delta_predicate_over_a_fact_metric_uses_conditional_fact_aggregate() {
        // A FLOW fact metric (net-new MRR): the *If aggregates read the fact value column
        // directly (inlined, no property bind), gated by the fact timestamp column.
        let reg = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "net_new_mrr",
            AggregateOp::Sum,
            mrr_fact(),
        ));
        let cohort = crate::metric::CohortDef {
            predicates: vec![CohortPredicate::MetricDelta {
                metric: "net_new_mrr".into(),
                op: ThresholdOp::Gte,
                pct: 50.0,
                prior_window_days: None,
            }],
        };
        let q = base_query("net_new_mrr");
        let sql = normalize(
            &compile_cohort_members(&reg, &cohort, &q, "raw_events_v2")
                .unwrap()
                .sql,
        );
        assert!(
            sql.contains(
                "toFloat64(sumIf(toFloat64(delta_cents), toDateTime(at) >= toDateTime(?))) AS cur"
            ),
            "fact current-window sumIf: {sql}"
        );
        assert!(
            sql.contains(
                "toFloat64(sumIf(toFloat64(delta_cents), toDateTime(at) < toDateTime(?) AND toDateTime(at) >= toDateTime(?))) AS prior"
            ),
            "fact prior-window sumIf: {sql}"
        );
        assert!(
            sql.contains("HAVING prior != 0 AND (cur - prior) / prior * 100 >= 50"),
            "up >=50% HAVING: {sql}"
        );
        assert!(sql.contains("FROM fact_mrr_movement FINAL WHERE"), "{sql}");
    }

    #[test]
    fn delta_binds_follow_placeholder_order() {
        let delta = |metric: &str| crate::metric::CohortDef {
            predicates: vec![CohortPredicate::MetricDelta {
                metric: metric.into(),
                op: ThresholdOp::Lte,
                pct: -30.0,
                prior_window_days: None,
            }],
        };
        let tenant = TenantId::default().as_uuid();
        let (from, prior_from, to) = (1704067200, 1703980800, 1704153600);

        let raw = MetricRegistry::new().with(MetricDef::aggregate(
            "bytes",
            "usage",
            AggregateOp::Sum,
            Some("amount"),
        ));
        let sql = inline_binds(
            &compile_cohort_members(&raw, &delta("bytes"), &base_query("bytes"), "raw_events_v2")
                .unwrap(),
        );
        assert!(
            sql.contains(&format!(
                "toFloat64(sumIf(toFloat64OrZero(properties['amount']), timestamp >= toDateTime({from}))) AS cur, \
                 toFloat64(sumIf(toFloat64OrZero(properties['amount']), timestamp < toDateTime({from}) AND timestamp >= toDateTime({prior_from}))) AS prior \
                 FROM raw_events_v2 WHERE tenant_id = '{tenant}' AND code = 'usage' \
                 AND timestamp >= toDateTime({prior_from}) AND timestamp <= toDateTime({to})"
            )),
            "{sql}"
        );

        let fact = MetricRegistry::new().with(MetricDef::fact_aggregate(
            "net_new_mrr",
            AggregateOp::Sum,
            mrr_fact(),
        ));
        let sql = inline_binds(
            &compile_cohort_members(
                &fact,
                &delta("net_new_mrr"),
                &base_query("net_new_mrr"),
                "raw_events_v2",
            )
            .unwrap(),
        );
        assert!(
            sql.contains(&format!(
                "sumIf(toFloat64(delta_cents), toDateTime(at) >= toDateTime({from}))) AS cur, \
                 toFloat64(sumIf(toFloat64(delta_cents), toDateTime(at) < toDateTime({from}) AND toDateTime(at) >= toDateTime({prior_from}))) AS prior \
                 FROM fact_mrr_movement FINAL WHERE tenant_id = '{tenant}' \
                 AND toDateTime(at) >= toDateTime({prior_from}) AND toDateTime(at) <= toDateTime({to})"
            )),
            "{sql}"
        );
    }

    #[test]
    fn cohort_boolean_or_not_compiles_to_union_and_anti_join() {
        // A top-level predicate `Or( ActivePlan, Not(MetricThreshold > 0) )`: the OR
        // must UNION its children (each wrapped as a derived table), and the NOT must
        // anti-join the tenant customer universe. The whole tree is still applied as a
        // `customer_id IN (...)` membership filter on the metric read.
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "api_calls",
            "api",
            AggregateOp::Count,
            None,
        ));
        let mut q = base_query("api_calls");
        q.cohort = Some(crate::metric::CohortDef {
            predicates: vec![CohortPredicate::Or {
                children: vec![
                    CohortPredicate::ActivePlan {
                        plan_ids: vec!["pro".into()],
                    },
                    CohortPredicate::Not {
                        child: Box::new(CohortPredicate::MetricThreshold {
                            metric: "api_calls".into(),
                            op: ThresholdOp::Gt,
                            threshold: 0.0,
                        }),
                    },
                ],
            }],
        });
        let sql = normalize(&compile(&reg, &q, "raw_events_v2", "meteroid").unwrap().sql);
        assert!(sql.contains("UNION DISTINCT"), "{sql}");
        assert!(sql.contains("SELECT customer_id FROM ("), "{sql}");
        assert!(
            sql.contains(
                "SELECT id AS customer_id FROM dim_customer FINAL WHERE tenant_id = ? AND id NOT IN ("
            ),
            "{sql}"
        );
        assert!(sql.contains("customer_id IN ("), "{sql}");
    }

    #[test]
    fn compile_cohort_members_emits_distinct_customer_id_over_intersect() {
        // The member-set path returns DISTINCT customer_id of the INTERSECT, not a metric
        // series. An absence leaf AND a plan leaf: the absence anti-join must appear inside.
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "api_calls",
            "api",
            AggregateOp::Count,
            None,
        ));
        let q = base_query("api_calls");
        let cohort = crate::metric::CohortDef {
            predicates: vec![
                CohortPredicate::ActivePlan {
                    plan_ids: vec!["free_plan".into()],
                },
                CohortPredicate::MetricInactive {
                    metric: "api_calls".into(),
                    max_active: None,
                },
            ],
        };
        let out = compile_cohort_members(&reg, &cohort, &q, "raw_events_v2").unwrap();
        let sql = normalize(&out.sql);
        assert!(
            sql.starts_with("SELECT DISTINCT customer_id FROM ("),
            "{sql}"
        );
        assert!(sql.contains("INTERSECT"), "{sql}");
        assert!(
            sql.contains("id NOT IN ("),
            "absence anti-join must be present: {sql}"
        );
    }

    #[test]
    fn compile_cohort_members_rejects_empty_cohort() {
        let reg = MetricRegistry::new();
        let q = base_query("x");
        let cohort = crate::metric::CohortDef { predicates: vec![] };
        assert!(matches!(
            compile_cohort_members(&reg, &cohort, &q, "raw_events_v2").unwrap_err(),
            MetricError::Invalid(_, _)
        ));
    }

    #[test]
    fn cohort_rejects_unsafe_dimension_identifier() {
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "rev",
            "charge",
            AggregateOp::Sum,
            Some("amount"),
        ));
        let mut q = base_query("rev");
        q.cohort = Some(crate::metric::CohortDef {
            predicates: vec![CohortPredicate::Dimension {
                table: "dim_customer; DROP TABLE x".into(),
                id_column: "id".into(),
                column: "country".into(),
                values: vec!["FR".into()],
            }],
        });
        assert!(matches!(
            compile(&reg, &q, "raw_events_v2", "meteroid").unwrap_err(),
            MetricError::Invalid(_, _)
        ));
    }

    #[test]
    fn fact_source_rejects_schema_qualified_table() {
        let mut fs = mrr_fact();
        fs.table = "system.tables".into();
        let reg =
            MetricRegistry::new().with(MetricDef::fact_aggregate("leak", AggregateOp::Sum, fs));
        assert!(matches!(
            compile(&reg, &base_query("leak"), "raw_events_v2", "meteroid").unwrap_err(),
            MetricError::Invalid(_, _)
        ));
    }

    #[test]
    fn cohort_dimension_rejects_schema_qualified_table() {
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "rev",
            "charge",
            AggregateOp::Sum,
            Some("amount"),
        ));
        let mut q = base_query("rev");
        q.cohort = Some(crate::metric::CohortDef {
            predicates: vec![CohortPredicate::Dimension {
                table: "system.tables".into(),
                id_column: "id".into(),
                column: "name".into(),
                values: vec!["x".into()],
            }],
        });
        assert!(matches!(
            compile(&reg, &q, "raw_events_v2", "meteroid").unwrap_err(),
            MetricError::Invalid(_, _)
        ));
    }

    #[test]
    fn time_weighted_rejects_excessive_window_count() {
        let reg = uptime_registry("instance_id");
        let mut q = base_query("uptime");
        q.window = Some(WindowSize::Minute);
        q.from = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        q.to = Some(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap());
        let err = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap_err();
        assert!(matches!(err, MetricError::Invalid(_, _)));
    }

    fn sum_series_sql(keep_last_events: bool) -> String {
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "bytes",
            "usage",
            AggregateOp::Sum,
            Some("amount"),
        ));
        let mut q = base_query("bytes");
        q.keep_last_events = keep_last_events;
        let out = compile(&reg, &q, "raw_events_v2", "meteroid").unwrap();
        assert_eq!(out.sql.matches('?').count(), out.binds.len(), "{}", out.sql);
        normalize(&out.sql)
    }

    const SUM_SERIES_WHERE: &str = "SELECT id, customer_id, timestamp, ingested_at, properties \
        FROM raw_events_v2 \
        WHERE tenant_id = ? AND code = ? AND timestamp >= toDateTime(?) \
        AND timestamp <= toDateTime(?) \
        AND properties[?] != '' AND isNotNull(toFloat64OrNull(properties[?]))";

    #[test]
    fn keep_last_tenant_keeps_the_last_copy_ingested() {
        let sql = sum_series_sql(true);
        assert!(
            sql.contains(&format!(
                "FROM ( {SUM_SERIES_WHERE} ORDER BY ingested_at DESC LIMIT 1 BY id, customer_id )"
            )),
            "{sql}"
        );
    }

    #[test]
    fn default_tenant_reads_without_a_sort() {
        let sql = sum_series_sql(false);
        assert!(
            sql.contains(&format!("FROM ( {SUM_SERIES_WHERE} ) GROUP BY")),
            "{sql}"
        );
        assert!(!sql.contains("LIMIT 1 BY"), "{sql}");
    }

    fn uptime_sql(keep_last_events: bool) -> SafeQuery {
        let mut q = base_query("uptime");
        q.keep_last_events = keep_last_events;
        let out = compile(
            &uptime_registry("instance_id"),
            &q,
            "raw_events_v2",
            "meteroid",
        )
        .unwrap();
        assert_eq!(out.sql.matches('?').count(), out.binds.len(), "{}", out.sql);
        out
    }

    #[test]
    fn time_weighted_default_tenant_never_sorts() {
        let out = uptime_sql(false);
        assert!(!out.sql.contains("LIMIT 1 BY"), "{}", out.sql);
        assert!(
            normalize(&out.sql).contains("FROM raw_events_v2 WHERE code = ? )"),
            "{}",
            out.sql
        );
    }

    #[test]
    fn time_weighted_keep_last_dedups_the_tenant_events_before_the_window_function() {
        let out = uptime_sql(true);
        let sql = normalize(&out.sql);
        assert!(
            sql.contains(
                "FROM ( SELECT id, customer_id, tenant_id, timestamp, properties FROM raw_events_v2 \
                 WHERE tenant_id = ? AND code = ? \
                 ORDER BY ingested_at DESC LIMIT 1 BY id, customer_id ) ), instance_periods AS"
            ),
            "{sql}"
        );
        let tenant = TenantId::default().as_uuid();
        match &out.binds[2..5] {
            [
                BindValue::Uuid(a),
                BindValue::Uuid(b),
                BindValue::String(code),
            ] => {
                assert_eq!((*a, *b, code.as_str()), (tenant, tenant, "compute"));
            }
            other => panic!("params tenant, then the dedup's tenant and code: {other:?}"),
        }
    }

    fn cohort_members_sql(predicate: CohortPredicate, keep_last_events: bool) -> String {
        let reg = MetricRegistry::new().with(MetricDef::aggregate(
            "bytes",
            "usage",
            AggregateOp::Sum,
            Some("amount"),
        ));
        let mut q = base_query("bytes");
        q.keep_last_events = keep_last_events;
        let cohort = crate::metric::CohortDef {
            predicates: vec![predicate],
        };
        let out = compile_cohort_members(&reg, &cohort, &q, "raw_events_v2").unwrap();
        assert_eq!(out.sql.matches('?').count(), out.binds.len(), "{}", out.sql);
        normalize(&out.sql)
    }

    fn threshold_predicate() -> CohortPredicate {
        CohortPredicate::MetricThreshold {
            metric: "bytes".into(),
            op: ThresholdOp::Gt,
            threshold: 5.0,
        }
    }

    fn delta_predicate() -> CohortPredicate {
        CohortPredicate::MetricDelta {
            metric: "bytes".into(),
            op: ThresholdOp::Lte,
            pct: -30.0,
            prior_window_days: None,
        }
    }

    #[rstest]
    #[case::threshold(threshold_predicate())]
    #[case::delta(delta_predicate())]
    fn cohort_raw_reads_dedup_only_for_keep_last_tenants(#[case] predicate: CohortPredicate) {
        let default = cohort_members_sql(predicate.clone(), false);
        assert!(
            default.contains("FROM raw_events_v2 WHERE tenant_id = ?"),
            "{default}"
        );
        assert!(!default.contains("LIMIT 1 BY"), "{default}");

        let keep_last = cohort_members_sql(predicate, true);
        assert!(
            keep_last.contains(
                "FROM ( SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2 \
                 WHERE tenant_id = ? AND code = ?"
            ),
            "{keep_last}"
        );
        assert!(
            keep_last.contains(
                "ORDER BY ingested_at DESC LIMIT 1 BY id, customer_id ) GROUP BY customer_id"
            ),
            "{keep_last}"
        );
    }

    #[rstest]
    #[case(true, EventDedup::LastIngested)]
    #[case(false, EventDedup::Off)]
    fn event_dedup_mode(#[case] keep_last: bool, #[case] mode: EventDedup) {
        assert_eq!(EventDedup::new(keep_last), mode);
    }
}
