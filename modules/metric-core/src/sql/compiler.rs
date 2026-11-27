//! Compiler from the engine-agnostic metric model to ClickHouse SQL, through the typed
//! [`super::ast`] tree: the compiler builds the tree, [`super::scope`] checks its tenant
//! isolation, [`super::render`] writes the SQL with its binds.

use crate::domain::WindowSize;
use crate::metric::{
    AggregateOp, AggregateSpec, BinOp, CohortDef, CohortPredicate, CohortReduction, CrossPeriod,
    Dimension, DimensionFilter, Expr as MetricExpr, FactSource, FilterOp, GaugeOp, GaugeSpec,
    MetricDef, MetricError, MetricKind, MetricQuery, MetricRegistry, MetricSource, PropertyOp,
    PropertyValueType, ThresholdOp, TimeWeight, TimeWeightedSpec, UnaryOp,
};
use crate::sql::ast::{
    DictLookup, Expr, Fill, Frame, FrameBound, Ident, Item, JoinKind, Op, Query, Relation, Select,
    SetOp, Setting, Table, TableScope, Window, literal_key,
};
use crate::sql::{
    BindValue, EventDedup, EventsSource, EventsTable, SafeQuery, VoidScope,
    dimension_disambiguator_alias, latest_event_order,
};
use chrono::Utc;
use common_domain::ids::BaseId;

/// `customer_id` → customer attributes (country, plan tier, custom metadata), backed by
/// `dim_customer` and held in memory: a `dictGet` during `GROUP BY` is a lookup, not a join.
const CUSTOMER_DICT: &str = "dim_customer_dict";

/// `plan_id` → plan name, backed by `dim_plan`.
const PLAN_DICT: &str = "dim_plan_dict";

/// `product_id` → product name, backed by `dim_product`.
const PRODUCT_DICT: &str = "dim_product_dict";

/// The breakdown label for rows whose plan/product id is NULL (e.g. one-time or usage revenue
/// lines with no plan/product lineage).
const NONE_LABEL: &str = "(none)";

/// `dictGet` rejects a Nullable key, so a nullable id is coalesced to this UUID, absent from
/// every dictionary.
fn nil_uuid() -> Expr {
    Expr::call(
        "toUUID",
        [Expr::str("00000000-0000-0000-0000-000000000000")],
    )
}

fn bind_str(value: &str) -> Expr {
    Expr::Bind(BindValue::String(value.to_string()))
}

/// `properties[?]`: the property name is bound, never inlined.
fn property(name: &str) -> Expr {
    Expr::Index(Box::new(Expr::col("properties")), Box::new(bind_str(name)))
}

/// `properties['name']`, for a name validated by [`literal_key`].
fn property_literal(name: &str) -> Expr {
    Expr::Index(Box::new(Expr::col("properties")), Box::new(Expr::str(name)))
}

fn column(qualifier: &Option<Ident>, name: &Ident) -> Expr {
    Expr::Column(qualifier.clone(), name.clone())
}

fn alias(name: String) -> Ident {
    Ident::from_alias(name)
}

fn customer_uuids(query: &MetricQuery) -> Expr {
    Expr::Bind(BindValue::Uuids(
        query.customer_ids.iter().map(|id| id.as_uuid()).collect(),
    ))
}

fn tenant_bind(query: &MetricQuery) -> Expr {
    Expr::Bind(BindValue::Uuid(query.tenant_id.as_uuid()))
}

/// The per-source context a [`Dimension`] resolves against.
struct DimCtx<'a> {
    database: &'a str,
    /// `f` when the fact read is joined to FX rates, else unqualified.
    qualifier: Option<Ident>,
    customer_column: Ident,
    /// Only valid on fact sources that carry it.
    plan_column: Ident,
    /// Only valid on fact sources that carry it (the revenue-item fact).
    product_column: Ident,
    is_fact: bool,
}

impl DimCtx<'_> {
    fn lookup(&self, dictionary: &'static str, attribute: &str, key: Expr) -> Expr {
        Expr::Dict(DictLookup {
            database: self.database.to_string(),
            dictionary,
            attribute: attribute.to_string(),
            key: Box::new(key),
            default: None,
        })
    }

    /// A name lookup on a nullable id, NULL ids landing in the `(none)` bucket.
    fn name_lookup(&self, dictionary: &'static str, id_column: &Ident) -> Expr {
        Expr::Dict(DictLookup {
            database: self.database.to_string(),
            dictionary,
            attribute: "name".to_string(),
            key: Box::new(self.null_guarded(id_column)),
            default: Some(Box::new(Expr::str(NONE_LABEL))),
        })
    }

    fn null_guarded(&self, id_column: &Ident) -> Expr {
        Expr::call("ifNull", [column(&self.qualifier, id_column), nil_uuid()])
    }

    fn customer(&self) -> Expr {
        column(&self.qualifier, &self.customer_column)
    }
}

/// The value of a [`Dimension`]. Open-ended names are bound (`properties[?]`) or validated
/// before inlining, so a user-chosen dimension can never inject SQL.
fn dim_value(dim: &Dimension, ctx: &DimCtx) -> Result<Expr, MetricError> {
    match dim {
        Dimension::Auto(n) if ctx.is_fact => Ok(column(&ctx.qualifier, &Ident::column(n)?)),
        Dimension::Auto(n) | Dimension::Property(n) => {
            if matches!(dim, Dimension::Property(_)) && ctx.is_fact {
                return Err(MetricError::Invalid(
                    n.clone(),
                    "property dimensions need a usage (raw-event) metric".to_string(),
                ));
            }
            Ok(property(n))
        }
        Dimension::Column(n) => Ok(column(&ctx.qualifier, &Ident::column(n)?)),
        Dimension::CustomerAttr(n) => {
            literal_key(n)?;
            Ok(ctx.lookup(CUSTOMER_DICT, n, ctx.customer()))
        }
        // The plan dictionary exposes only `name`; `n` is the output key.
        Dimension::PlanAttr(n) => {
            literal_key(n)?;
            Ok(ctx.name_lookup(PLAN_DICT, &ctx.plan_column))
        }
        // ClickHouse rejects Map as a dictionary attribute, so `attributes` is projected as JSON.
        Dimension::CustomerAttrMap { map_key, .. } => {
            literal_key(map_key)?;
            let json = ctx.lookup(CUSTOMER_DICT, "attributes", ctx.customer());
            Ok(Expr::call("JSONExtractString", [json, Expr::str(map_key)]))
        }
        // Text whatever the stored JSON type: strings unquoted, other scalars raw, null in (none).
        Dimension::CustomerProperty { key, .. } => {
            let json = ctx.lookup(CUSTOMER_DICT, "custom_properties", ctx.customer());
            let typed = |json: &Expr| Expr::call("JSONType", [json.clone(), bind_str(key)]);
            Ok(Expr::call(
                "multiIf",
                [
                    typed(&json).eq(Expr::str("String")),
                    Expr::call("JSONExtractString", [json.clone(), bind_str(key)]),
                    typed(&json).eq(Expr::str("Null")),
                    Expr::str(NONE_LABEL),
                    Expr::call("JSONExtractRaw", [json, bind_str(key)]),
                ],
            ))
        }
        Dimension::ProductAttr(n) => {
            literal_key(n)?;
            Ok(ctx.name_lookup(PRODUCT_DICT, &ctx.product_column))
        }
        // Groups on the customer id (see `entity_id`), so same-named customers stay distinct.
        Dimension::CustomerName(n) => {
            literal_key(n)?;
            Ok(ctx.name_lookup(CUSTOMER_DICT, &ctx.customer_column))
        }
    }
}

/// The id an [entity dimension](Dimension::is_entity) groups on, so two entities sharing a
/// display name remain distinct series. `None` for dimensions keyed on their own value.
fn entity_id(dim: &Dimension, ctx: &DimCtx) -> Option<Expr> {
    let col = match dim {
        Dimension::PlanAttr(_) => &ctx.plan_column,
        Dimension::ProductAttr(_) => &ctx.product_column,
        Dimension::CustomerName(_) => &ctx.customer_column,
        _ => return None,
    };
    Some(ctx.null_guarded(col))
}

fn dim_group(dim: &Dimension, ctx: &DimCtx) -> Result<Expr, MetricError> {
    match entity_id(dim, ctx) {
        Some(id) => Ok(id),
        None => dim_value(dim, ctx),
    }
}

/// The dimension's value under its `dimension_alias`, plus for entity dimensions the grouping
/// id under its disambiguator alias.
fn dim_items(dim: &Dimension, ctx: &DimCtx) -> Result<Vec<Item>, MetricError> {
    let mut items = vec![Item::aliased(dim_value(dim, ctx)?, dimension_alias(dim))];
    if let Some(id) = entity_id(dim, ctx) {
        items.push(Item::aliased(
            Expr::call("toString", [id]),
            alias(dimension_disambiguator_alias(dim)),
        ));
    }
    Ok(items)
}

fn dimension_alias(dim: &Dimension) -> Ident {
    alias(crate::sql::dimension_alias(dim))
}

/// DoS cap: the time-weighted gauge compiles to a cross join, so an unbounded window count melts the cluster.
const MAX_GAUGE_WINDOWS: i64 = 100_000;

const MAX_WINDOWS: i64 = 100_000;

struct Series {
    query: Query,
    dims: Vec<Ident>,
    /// Statement-level, so only the outermost query carries them.
    settings: Vec<Setting>,
}

impl Series {
    fn select(select: Select, dims: Vec<Ident>) -> Series {
        Series {
            query: select.query(),
            dims,
            settings: vec![],
        }
    }
}

pub fn compile(
    registry: &MetricRegistry,
    query: &MetricQuery,
    events: impl Into<EventsTable>,
    database: &str,
) -> Result<SafeQuery, MetricError> {
    let (tree, settings) = compile_tree(registry, query, events, database)?;
    finish(&tree, &settings, query)
}

/// The tree [`compile`] renders, with its statement settings.
pub fn compile_tree(
    registry: &MetricRegistry,
    query: &MetricQuery,
    events: impl Into<EventsTable>,
    database: &str,
) -> Result<(Query, Vec<Setting>), MetricError> {
    registry.validate()?;
    let table: EventsTable = events.into();
    let events = table.source(
        database,
        query.tenant_id.as_uuid(),
        EventDedup::new(query.keep_last_events),
    );
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
        .map(|c| compile_cohort(registry, c, query, events))
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
        Some(t) => Some(history_from(t, query.from, w)),
        None if carry_over_gauge => Some(epoch()),
        None => None,
    });

    let mut series = if let Some(history_from) = history_from {
        let mut history_query = query.clone();
        history_query.from = history_from;
        build_series(
            registry,
            def,
            &history_query,
            events,
            database,
            cohort.as_ref(),
        )?
    } else {
        build_series(registry, def, query, events, database, cohort.as_ref())?
    };

    // Forward-fill the gauge level across empty windows; the floor back to `>= from` happens here (a transform, if any, re-windows the already-filled series).
    if carry_over_gauge && let Some(w) = &query.window {
        series = carry_over_gauge_series(series, query, w);
    }

    let settings = std::mem::take(&mut series.settings);
    let final_query = match &query.transform {
        None => series.query,
        Some(transform) => {
            let floor_ts = history_from.map(|_| query.from.timestamp());
            // A running total is a continuous balance: gap-fill the visible range so the line carries the cumulative value across empty windows.
            let fill = match (transform, &query.window) {
                (CrossPeriod::RunningTotal, Some(w)) => Some(GapFill {
                    from_ts: query.from.timestamp(),
                    to_ts: query.to.unwrap_or_else(Utc::now).timestamp(),
                    window: w.clone(),
                    tz: tz_name(query),
                }),
                _ => None,
            };
            apply_cross_period(series, transform, floor_ts, fill)
        }
    };
    Ok((final_query, settings))
}

fn finish(
    tree: &Query,
    settings: &[Setting],
    query: &MetricQuery,
) -> Result<SafeQuery, MetricError> {
    crate::sql::render_scoped(tree, settings, query.tenant_id.as_uuid())
        .map_err(|e| MetricError::Invalid("tenant_scope".to_string(), e))
}

fn build_series(
    registry: &MetricRegistry,
    def: &MetricDef,
    query: &MetricQuery,
    events: EventsSource<'_>,
    database: &str,
    cohort: Option<&Query>,
) -> Result<Series, MetricError> {
    // Per-metric filters AND into *this* metric's read only: clone the query with the
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
                events,
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
                    events,
                    database,
                    cohort,
                ),
                MetricSource::Fact(_) => Err(MetricError::Unsupported(
                    "gauge metrics require a raw-events source".to_string(),
                )),
            },
            GaugeOp::TimeWeighted(tw) => {
                time_weighted_series(&def.event_code, tw, query, events, cohort)
            }
        },
        MetricKind::Derived(expr) => {
            derived_series(registry, expr, query, events, database, cohort)
        }
    }
}

enum ValueAgg {
    Aggregate(AggregateSpec),
    Gauge(GaugeSpec),
}

fn interval(window: &WindowSize) -> Expr {
    intervals(window, Expr::Int(1))
}

fn epoch() -> chrono::DateTime<Utc> {
    chrono::DateTime::from_timestamp(0, 0).expect("epoch is valid")
}

/// Where a transform starts reading, so its first visible window sees the windows it compares
/// against: from epoch for cumulative totals, else as many windows back as it looks.
fn history_from(
    transform: &CrossPeriod,
    from: chrono::DateTime<Utc>,
    window: &WindowSize,
) -> chrono::DateTime<Utc> {
    match transform {
        CrossPeriod::RunningTotal | CrossPeriod::RunningTotalLagged => epoch(),
        CrossPeriod::PeriodOverPeriodDelta | CrossPeriod::PercentChange => {
            from - window_duration(window)
        }
        CrossPeriod::MovingAverage { periods } => {
            from - window_duration(window) * periods.saturating_sub(1) as i32
        }
    }
}

/// `toInterval<Unit>(n)`: `n` windows.
fn intervals(window: &WindowSize, n: Expr) -> Expr {
    let name = match window {
        WindowSize::Minute => "toIntervalMinute",
        WindowSize::Hour => "toIntervalHour",
        WindowSize::Day => "toIntervalDay",
        WindowSize::Week => "toIntervalWeek",
    };
    Expr::call(name, [n])
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

/// `tumbleStart(toDateTime(<ts>), <interval>, <tz>)`, or `tumbleEnd`.
fn tumble(function: &'static str, ts: Expr, window: &WindowSize, tz: &str) -> Expr {
    Expr::call(
        function,
        [
            Expr::call("toDateTime", [ts]),
            interval(window),
            bind_str(tz),
        ],
    )
}

/// `tumbleStart`/`tumbleEnd` return a `Date` for week intervals, which the JSON read-back can't
/// parse as a timestamp: `toDateTime(_, tz)` makes every grain a `DateTime`.
fn as_datetime(value: Expr, tz: &str) -> Expr {
    Expr::call("toDateTime", [value, bind_str(tz)])
}

/// The `window_start`/`window_end` items: tumbling windows, or one bucket over the whole range
/// with constant bounds so several metrics align.
fn window_items(query: &MetricQuery, ts: &Expr) -> Vec<Item> {
    match &query.window {
        Some(window) => {
            let tz = tz_name(query);
            vec![
                Item::as_(
                    as_datetime(tumble("tumbleStart", ts.clone(), window, &tz), &tz),
                    "window_start",
                ),
                Item::as_(
                    as_datetime(tumble("tumbleEnd", ts.clone(), window, &tz), &tz),
                    "window_end",
                ),
            ]
        }
        None => vec![
            Item::as_(Expr::datetime(query.from.timestamp()), "window_start"),
            Item::as_(
                Expr::datetime(query.to.unwrap_or_else(Utc::now).timestamp()),
                "window_end",
            ),
        ],
    }
}

fn dim_columns(query: &MetricQuery) -> Vec<Ident> {
    // A `customer_ids` scope is a WHERE filter only, never an output dimension —
    // otherwise a non-broken-down metric returns one row per (window × customer).
    query.group_by.iter().map(dimension_alias).collect()
}

/// The raw-events grouping context (no qualifier; properties live in the event map).
fn event_dim_ctx(database: &str) -> DimCtx<'_> {
    DimCtx {
        database,
        qualifier: None,
        customer_column: Ident::trusted("customer_id"),
        plan_column: Ident::trusted("plan_id"),
        product_column: Ident::trusted("product_id"),
        is_fact: false,
    }
}

fn meter_event_columns() -> Vec<Ident> {
    [
        "id",
        "customer_id",
        "timestamp",
        "ingested_at",
        "properties",
    ]
    .into_iter()
    .map(Ident::trusted)
    .collect()
}

/// `tenant_id = ? AND code = ? AND timestamp >= toDateTime(?) [AND timestamp <= toDateTime(?)]`.
fn raw_event_bounds(
    query: &MetricQuery,
    event_code: &str,
    from_ts: i64,
    to_ts: Option<i64>,
) -> Vec<Expr> {
    let mut filter = vec![
        Expr::col("tenant_id").eq(tenant_bind(query)),
        Expr::col("code").eq(bind_str(event_code)),
        Expr::col("timestamp").ge(Expr::datetime(from_ts)),
    ];
    if let Some(to) = to_ts {
        filter.push(Expr::col("timestamp").le(Expr::datetime(to)));
    }
    filter
}

/// The void set narrowed to a raw read's code and time range.
fn void_scope(event_code: &str, from_ts: i64, to_ts: Option<i64>) -> VoidScope {
    let scope = VoidScope::default()
        .code(event_code)
        .since(format!("toDateTime({from_ts})"));
    match to_ts {
        Some(to) => scope.until(format!("toDateTime({to})")),
        None => scope,
    }
}

fn simple_series(
    event_code: &str,
    agg: ValueAgg,
    query: &MetricQuery,
    events: EventsSource<'_>,
    database: &str,
    cohort: Option<&Query>,
) -> Result<Series, MetricError> {
    let mut items = window_items(query, &Expr::col("timestamp"));
    let value = match &agg {
        ValueAgg::Aggregate(spec) => aggregate_value(spec)?,
        ValueAgg::Gauge(spec) => gauge_value(spec)?,
    };
    items.push(Item::as_(value, "value"));

    let dim_ctx = event_dim_ctx(database);
    for d in &query.group_by {
        items.extend(dim_items(d, &dim_ctx)?);
    }

    let from_ts = query.from.timestamp();
    let to_ts = query.to.map(|t| t.timestamp());
    let mut filter = raw_event_bounds(query, event_code, from_ts, to_ts);
    if !query.customer_ids.is_empty() {
        filter.push(Expr::col("customer_id").in_set(customer_uuids(query)));
    }
    filter.extend(dimension_filters(&query.filters));
    if let Some(value_prop) = value_property_of(&agg) {
        filter.push(property(value_prop).ne(Expr::str("")));
        // CountDistinct counts raw values, often non-numeric (a user id): presence is enough.
        // Every other aggregation coerces through toFloat64, so it also needs a parseable value.
        if !matches!(&agg, ValueAgg::Aggregate(s) if matches!(s.op, AggregateOp::CountDistinct)) {
            filter.push(Expr::call(
                "isNotNull",
                [Expr::call("toFloat64OrNull", [property(value_prop)])],
            ));
        }
    }
    if let Some(c) = cohort {
        filter.push(Expr::col("customer_id").in_set(Expr::subquery(c.clone())));
    }

    let mut group_by = Vec::new();
    if let Some(window) = &query.window {
        let tz = tz_name(query);
        group_by.push(tumble("tumbleStart", Expr::col("timestamp"), window, &tz));
        group_by.push(tumble("tumbleEnd", Expr::col("timestamp"), window, &tz));
    }
    for d in &query.group_by {
        group_by.push(dim_group(d, &dim_ctx)?);
    }

    let scope = void_scope(event_code, from_ts, to_ts)
        .customers(query.customer_ids.iter().map(|id| id.as_uuid()));
    let source = events.read(meter_event_columns(), filter, scope);
    let select = Select::new(items, source)
        .group_by(group_by)
        .order_by_window_start();
    Ok(Series::select(select, dim_columns(query)))
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

/// The aggregate call for an [`AggregateOp`] other than `Count`/`CountDistinct`, whose
/// renderings differ per call site: `value` is the per-row numeric expression, `latest` what
/// `Latest` ranks by.
fn aggregate(op: AggregateOp, value: Expr, latest: Expr) -> Expr {
    match op {
        AggregateOp::Sum => Expr::call("sum", [value]),
        AggregateOp::Avg => Expr::call("avg", [value]),
        AggregateOp::Min => Expr::call("min", [value]),
        AggregateOp::Max => Expr::call("max", [value]),
        AggregateOp::Latest => Expr::call("argMax", [value, latest]),
        AggregateOp::Count | AggregateOp::CountDistinct => {
            unreachable!("count aggregates are rendered by their call site")
        }
    }
}

/// The `*If` form of an aggregate, reducing only the rows where `predicate` holds:
/// `countIf(p)`, `uniqIf(distinct, p)`, `sumIf(value, p)`, `argMaxIf(value, latest, p)`, ….
fn aggregate_if(
    op: AggregateOp,
    value: Expr,
    latest: Expr,
    distinct: Expr,
    predicate: Expr,
) -> Expr {
    match op {
        AggregateOp::Count => Expr::call("countIf", [predicate]),
        AggregateOp::Sum => Expr::call("sumIf", [value, predicate]),
        AggregateOp::Avg => Expr::call("avgIf", [value, predicate]),
        AggregateOp::Min => Expr::call("minIf", [value, predicate]),
        AggregateOp::Max => Expr::call("maxIf", [value, predicate]),
        AggregateOp::Latest => Expr::call("argMaxIf", [value, latest, predicate]),
        AggregateOp::CountDistinct => Expr::call("uniqIf", [distinct, predicate]),
    }
}

fn required_value_property<'a>(
    spec: &'a AggregateSpec,
    name: String,
    message: &str,
) -> Result<&'a str, MetricError> {
    spec.value_property
        .as_deref()
        .ok_or_else(|| MetricError::Invalid(name, message.to_string()))
}

fn aggregate_value(spec: &AggregateSpec) -> Result<Expr, MetricError> {
    let expr = match spec.op {
        AggregateOp::Count => Expr::call("toFloat64", [Expr::call("count", [Expr::Star])]),
        AggregateOp::CountDistinct => {
            let prop = required_value_property(
                spec,
                "CountDistinct".to_string(),
                "value_property is required",
            )?;
            Expr::call("toFloat64", [Expr::call("uniq", [property(prop)])])
        }
        op => {
            let prop =
                required_value_property(spec, format!("{op:?}"), "value_property is required")?;
            let value = Expr::call("toFloat64OrZero", [property(prop)]);
            aggregate(op, value, latest_event_order())
        }
    };
    Ok(apply_unit_conversion(spec, expr))
}

/// Divide an aggregate by the metric's unit-conversion factor, matching the billing path
/// (invoice line computation). `None` / ≤ 1 / non-finite leaves it unchanged.
fn apply_unit_conversion(spec: &AggregateSpec, expr: Expr) -> Expr {
    match spec.unit_conversion_factor {
        Some(factor) if factor.is_finite() && factor > 1.0 => expr.op(Op::Div, Expr::Float(factor)),
        _ => expr,
    }
}

fn gauge_value(spec: &GaugeSpec) -> Result<Expr, MetricError> {
    let prop = spec.value_property.as_deref().ok_or_else(|| {
        MetricError::Invalid(
            "gauge".to_string(),
            "value_property is required".to_string(),
        )
    })?;
    let value = Expr::call("toFloat64OrZero", [property(prop)]);
    match spec.op {
        GaugeOp::Latest => Ok(Expr::call("argMax", [value, latest_event_order()])),
        GaugeOp::Max => Ok(Expr::call("max", [value])),
        GaugeOp::TimeWeighted(_) => Err(MetricError::Unsupported(
            "time-weighted gauge is compiled by time_weighted_series".to_string(),
        )),
    }
}

/// Raw-event property filters. `NotIn` requires the property to be present: an absent key reads
/// as `''` and would otherwise slip through every `!=`.
fn dimension_filters(filters: &[DimensionFilter]) -> Vec<Expr> {
    filters
        .iter()
        .filter(|f| !f.values.is_empty())
        .map(|f| match f.op {
            FilterOp::In => Expr::Or(
                f.values
                    .iter()
                    .map(|v| property(&f.property).eq(bind_str(v)))
                    .collect(),
            ),
            FilterOp::NotIn => {
                let mut parts = vec![Expr::call(
                    "mapContains",
                    [Expr::col("properties"), bind_str(&f.property)],
                )];
                parts.extend(
                    f.values
                        .iter()
                        .map(|v| property(&f.property).ne(bind_str(v))),
                );
                Expr::And(parts)
            }
        })
        .collect()
}

fn time_weighted_series(
    event_code: &str,
    tw: &TimeWeightedSpec,
    query: &MetricQuery,
    events: EventsSource<'_>,
    cohort: Option<&Query>,
) -> Result<Series, MetricError> {
    let to = query.to.unwrap_or_else(Utc::now);

    // The gauge reads the raw-event property map, so only property dimensions apply.
    let identity = literal_key(&tw.identity_property)?;
    let mut dims: Vec<(&str, Ident)> = Vec::new();
    for d in &query.group_by {
        match d {
            Dimension::Auto(n) | Dimension::Property(n) => {
                dims.push((literal_key(n)?, dimension_alias(d)))
            }
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
    let weight = match &tw.weight {
        TimeWeight::Seconds => None,
        TimeWeight::ValueSeconds(prop) => Some(literal_key(prop)?),
    };

    let interval_secs = match &query.window {
        Some(w) => window_seconds(w),
        None => (to.timestamp() - query.from.timestamp()).max(1),
    };

    // Bound the cross-join blast radius (see MAX_GAUGE_WINDOWS).
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

    let el = |name: &Ident| Expr::Column(Some(Ident::trusted("el")), name.clone());
    let ip = |name: &Ident| Expr::Column(Some(Ident::trusted("ip")), name.clone());
    let identity_access = property_literal(identity);

    let params = Select {
        items: vec![
            Item::as_(Expr::datetime(query.from.timestamp()), "window_start"),
            Item::as_(Expr::datetime(to.timestamp()), "window_end"),
            Item::as_(
                Expr::call("toUUID", [tenant_bind(query)]),
                "param_tenant_id",
            ),
        ],
        ..Select::default()
    };

    let (code_filter, void_scope) = if tw.lifecycle_codes.is_empty() {
        (
            Expr::col("code").eq(bind_str(event_code)),
            VoidScope::default().code(event_code),
        )
    } else {
        (
            Expr::col("code").in_set(Expr::Bind(BindValue::Strings(tw.lifecycle_codes.clone()))),
            VoidScope::default().codes(&tw.lifecycle_codes),
        )
    };
    // Filtered (and keep-last deduped) before the window function, scoped to the tenant.
    let lifecycle_source = events.read(
        ["id", "customer_id", "tenant_id", "timestamp", "properties"]
            .into_iter()
            .map(Ident::trusted)
            .collect(),
        [Expr::col("tenant_id").eq(tenant_bind(query)), code_filter]
            .into_iter()
            .chain(dimension_filters(&query.filters))
            .collect(),
        void_scope,
    );
    let mut lifecycle_items = vec![
        Item::new(Expr::col("customer_id")),
        Item::new(Expr::col("tenant_id")),
        Item::as_(identity_access.clone(), "identity"),
        Item::as_(Expr::col("timestamp"), "start_time"),
        Item::as_(
            Expr::call(
                "leadInFrame",
                [Expr::call("toNullable", [Expr::col("timestamp")])],
            )
            .over(Window {
                partition_by: vec![
                    Expr::col("customer_id"),
                    Expr::col("tenant_id"),
                    identity_access,
                ],
                order_by: vec![Expr::col("timestamp")],
                frame: Some(Frame {
                    start: FrameBound::CurrentRow,
                    end: FrameBound::UnboundedFollowing,
                }),
            }),
            "end_time",
        ),
    ];
    for (name, dim_alias) in &dims {
        lifecycle_items.push(Item::aliased(property_literal(name), dim_alias.clone()));
    }
    if let Some(prop) = &weight {
        lifecycle_items.push(Item::as_(property_literal(prop), "weight_value"));
    }
    let event_lifecycle = Select::new(lifecycle_items, lifecycle_source);

    let window_start = Expr::qcol("params", "window_start");
    let window_end = Expr::qcol("params", "window_end");
    let el_end = Expr::qcol("el", "end_time");
    let weight_expr = match &weight {
        None => Expr::Int(1),
        Some(_) => Expr::call("toFloat64OrZero", [Expr::qcol("el", "weight_value")]),
    };
    let mut period_items = vec![
        Item::new(Expr::qcol("el", "customer_id")),
        Item::as_(
            Expr::call(
                "greatest",
                [Expr::qcol("el", "start_time"), window_start.clone()],
            ),
            "period_start",
        ),
        Item::as_(
            Expr::call(
                "least",
                [
                    Expr::call("COALESCE", [el_end.clone(), window_end.clone()]),
                    window_end.clone(),
                ],
            ),
            "period_end",
        ),
        Item::as_(weight_expr, "w"),
    ];
    period_items.extend(dims.iter().map(|(_, a)| Item::new(el(a))));
    let mut period_filter =
        vec![Expr::qcol("el", "tenant_id").eq(Expr::qcol("params", "param_tenant_id"))];
    if !query.customer_ids.is_empty() {
        period_filter.push(Expr::qcol("el", "customer_id").in_set(customer_uuids(query)));
    }
    if let Some(c) = cohort {
        period_filter.push(Expr::qcol("el", "customer_id").in_set(Expr::subquery(c.clone())));
    }
    period_filter.push(Expr::qcol("el", "start_time").lt(window_end.clone()));
    period_filter.push(Expr::Or(vec![
        el_end.clone().gt(window_start.clone()),
        Expr::IsNull(Box::new(el_end)),
    ]));
    let instance_periods = Select::new(
        period_items,
        Relation::Named(
            Ident::trusted("event_lifecycle"),
            Some(Ident::trusted("el")),
        )
        .join(
            JoinKind::Cross,
            Relation::Named(Ident::trusted("params"), None),
            None,
        ),
    )
    .filter(period_filter);

    // Qualified: the grid's own `window_start`/`window_end` aliases would otherwise shadow the
    // params columns inside the ARRAY JOIN, a cycle the analyzer reports as an unknown `number`.
    // The grid starts at the window holding `from`, in the query's time zone, and steps by
    // calendar intervals (a DST day is not 86400 s); without a window it is one bucket.
    let tz = tz_name(query);
    let origin = match &query.window {
        Some(w) => as_datetime(tumble("tumbleStart", window_start.clone(), w, &tz), &tz),
        None => Expr::call("toDateTime", [window_start.clone()]),
    };
    let grid_bound = |n: Expr| match &query.window {
        Some(w) => origin.clone().op(Op::Add, intervals(w, n)),
        None => Expr::call(
            "toDateTime",
            [origin
                .clone()
                .op(Op::Add, n.op(Op::Mul, Expr::Int(interval_secs)))],
        ),
    };
    let time_series = Select::new(
        vec![
            Item::as_(grid_bound(Expr::col("number")), "window_start"),
            Item::as_(
                grid_bound(Expr::col("number").op(Op::Add, Expr::Int(1))),
                "window_end",
            ),
        ],
        Relation::ArrayJoin {
            from: Box::new(Relation::Named(Ident::trusted("params"), None)),
            columns: vec![(
                Expr::call(
                    "range",
                    [Expr::call(
                        "toUInt32",
                        [Expr::call("toUnixTimestamp", [window_end.clone()])
                            .op(Op::Sub, Expr::call("toUnixTimestamp", [origin.clone()]))
                            .op(Op::Div, Expr::Int(interval_secs))],
                    )
                    .op(Op::Add, Expr::Int(1))],
                ),
                Ident::trusted("number"),
            )],
        },
    );

    let ts_start = Expr::qcol("ts", "window_start");
    let ts_end = Expr::qcol("ts", "window_end");
    let overlaps = || {
        vec![
            Expr::qcol("ip", "period_start").lt(ts_end.clone()),
            Expr::qcol("ip", "period_end").gt(ts_start.clone()),
        ]
    };
    let overlap_seconds = Expr::call(
        "dateDiff",
        [
            Expr::str("second"),
            Expr::call(
                "greatest",
                [ts_start.clone(), Expr::qcol("ip", "period_start")],
            ),
            Expr::call("least", [ts_end.clone(), Expr::qcol("ip", "period_end")]),
        ],
    )
    .op(Op::Mul, Expr::qcol("ip", "w"));
    let mut items = vec![
        Item::as_(ts_start.clone(), "window_start"),
        Item::as_(ts_end.clone(), "window_end"),
        Item::as_(
            Expr::call(
                "sum",
                [Expr::call(
                    "if",
                    [Expr::And(overlaps()), overlap_seconds, Expr::Int(0)],
                )],
            ),
            "value",
        ),
        Item::as_(Expr::qcol("ip", "customer_id"), "customer_id"),
    ];
    items.extend(dims.iter().map(|(_, a)| Item::aliased(ip(a), a.clone())));
    let mut group_by = vec![
        ts_start.clone(),
        ts_end.clone(),
        Expr::qcol("ip", "customer_id"),
    ];
    group_by.extend(dims.iter().map(|(_, a)| ip(a)));
    let body = Select::new(
        items,
        Relation::Named(Ident::trusted("time_series"), Some(Ident::trusted("ts"))).join(
            JoinKind::Cross,
            Relation::Named(
                Ident::trusted("instance_periods"),
                Some(Ident::trusted("ip")),
            ),
            None,
        ),
    )
    .filter(overlaps())
    .group_by(group_by)
    .order_by_window_start();

    let query_tree = Query::with_ctes(
        vec![
            (Ident::trusted("params"), params.query()),
            (Ident::trusted("event_lifecycle"), event_lifecycle.query()),
            (Ident::trusted("instance_periods"), instance_periods.query()),
            (Ident::trusted("time_series"), time_series.query()),
        ],
        body.query(),
    );

    let mut out_dims = vec![Ident::trusted("customer_id")];
    out_dims.extend(dims.into_iter().map(|(_, a)| a));
    Ok(Series {
        query: query_tree,
        dims: out_dims,
        settings: vec![],
    })
}

/// Fact-column filters, one parenthesised condition each. Shared by the read's `WHERE` and by
/// the per-operand `*If` predicates of a fused read.
fn fact_filter_conditions(filters: &[DimensionFilter]) -> Result<Vec<Expr>, MetricError> {
    let mut out = Vec::new();
    for f in filters.iter().filter(|f| !f.values.is_empty()) {
        let col = Expr::ident(&Ident::column(&f.property)?);
        // Fact columns are always present, so negation is a plain per-value !=.
        let (op, cmp) = match f.op {
            FilterOp::In => (Expr::Or as fn(Vec<Expr>) -> Expr, Op::Eq),
            FilterOp::NotIn => (Expr::And as fn(Vec<Expr>) -> Expr, Op::Ne),
        };
        let parts = f
            .values
            .iter()
            .map(|v| col.clone().op(cmp, bind_str(v)))
            .collect();
        out.push(op(parts));
    }
    Ok(out)
}

/// Whether filters actually restrict anything. A value-less filter is skipped when rendered, so
/// it must not count towards a member being "filtered" either.
fn has_effective_filters(filters: &[DimensionFilter]) -> bool {
    filters.iter().any(|f| !f.values.is_empty())
}

/// One operand of a fused fact read: the column it lands in and the per-metric filters that
/// isolate it. A lone member with no filters is the ordinary single-metric read.
struct FusedMember<'a> {
    column: Ident,
    filters: &'a [DimensionFilter],
}

fn fact_series(
    fs: &FactSource,
    spec: &AggregateSpec,
    query: &MetricQuery,
    database: &str,
    cohort: Option<&Query>,
) -> Result<Series, MetricError> {
    let lone = [FusedMember {
        column: Ident::trusted("value"),
        filters: &[],
    }];
    fact_series_fused(fs, spec, query, database, cohort, &lone)
}

const FX_TABLE: &str = "dim_fx_rate";

/// The fact read behind one or more operands. Operands sharing a fact source, aggregate and
/// request scope differ only by their per-metric filters, so one grouped scan can emit all of
/// them as `*If` aggregates — replacing N scans, N FX joins and N-1 outer joins with one.
fn fact_series_fused(
    fs: &FactSource,
    spec: &AggregateSpec,
    query: &MetricQuery,
    database: &str,
    cohort: Option<&Query>,
    members: &[FusedMember<'_>],
) -> Result<Series, MetricError> {
    let table = Ident::table(&fs.table)?;
    let valcol = Ident::column(&fs.value_column)?;
    let ts = Ident::column(&fs.timestamp_column)?;
    let cust = Ident::column(&fs.customer_column)?;
    let tenant_col = Ident::column(&fs.tenant_column)?;
    for f in &query.filters {
        Ident::column(&f.property)?;
    }

    // FX: convert each USD money row at its own date's historical rate BEFORE aggregating, so a running total accumulates already-converted deltas.
    let fx_active = matches!(
        spec.op,
        AggregateOp::Sum
            | AggregateOp::Avg
            | AggregateOp::Min
            | AggregateOp::Max
            | AggregateOp::Latest
    ) && valcol.as_str().ends_with("_usd")
        && query
            .reporting_currency
            .as_deref()
            .is_some_and(|c| !c.is_empty() && c != "USD");
    let q = fx_active.then(|| Ident::trusted("f"));
    // Native-currency column paired with the USD one: a row already in the reporting currency is summed natively to avoid sub-cent FX drift.
    let native_col = Ident::column(
        valcol
            .as_str()
            .strip_suffix("_usd")
            .unwrap_or(valcol.as_str()),
    )?;
    // A daily-snapshot (stock) fact: a bucket reports its latest day's level, not the sum of
    // every day in it (two-level reduction below).
    let snapshot = fs.snapshot;
    let ts_expr = column(&q, &ts);
    let ts_datetime = Expr::call("toDateTime", [ts_expr.clone()]);

    let mut items = window_items(query, &ts_expr);

    let value_expr = if fx_active {
        Expr::call(
            "if",
            [
                Expr::qcol("f", "currency").eq(Expr::qcol("f", "fx_currency")),
                Expr::call("toFloat64", [column(&q, &native_col)]),
                Expr::call("toFloat64", [column(&q, &valcol)])
                    .op(Op::Mul, Expr::qcol("fx", "rate_from_usd")),
            ],
        )
    } else {
        Expr::call("toFloat64", [Expr::ident(&valcol)])
    };
    // A filtered member reduces only its own rows via `*If`: the isolation the unfused path gets
    // from a per-metric `WHERE`.
    for m in members {
        let conds = fact_filter_conditions(m.filters)?;
        let predicate = (!conds.is_empty()).then_some(Expr::And(conds));
        let value = match (predicate, spec.op) {
            (None, AggregateOp::Count) => {
                Expr::call("toFloat64", [Expr::call("count", [Expr::Star])])
            }
            (None, AggregateOp::CountDistinct) => {
                Expr::call("toFloat64", [Expr::call("uniq", [Expr::ident(&valcol)])])
            }
            (None, op) => aggregate(op, value_expr.clone(), ts_datetime.clone()),
            (Some(p), AggregateOp::Count) => Expr::call("toFloat64", [Expr::call("countIf", [p])]),
            (Some(p), AggregateOp::CountDistinct) => Expr::call(
                "toFloat64",
                [Expr::call("uniqIf", [Expr::ident(&valcol), p])],
            ),
            (Some(p), op) => aggregate_if(
                op,
                value_expr.clone(),
                ts_datetime.clone(),
                Expr::ident(&valcol),
                p,
            ),
        };
        items.push(Item::aliased(value, m.column.clone()));
    }

    let fact_ctx = DimCtx {
        database,
        qualifier: q.clone(),
        customer_column: cust.clone(),
        plan_column: Ident::trusted("plan_id"),
        product_column: Ident::trusted("product_id"),
        is_fact: true,
    };
    for d in &query.group_by {
        items.extend(dim_items(d, &fact_ctx)?);
    }
    // Carry each row's snapshot day so the outer reduction can keep the bucket's latest.
    if snapshot {
        items.push(Item::as_(
            Expr::call("toDate", [ts_expr.clone()]),
            "_snap_day",
        ));
    }

    let unqualified_ts = Expr::call("toDateTime", [Expr::ident(&ts)]);
    let mut filter = vec![
        Expr::ident(&tenant_col).eq(tenant_bind(query)),
        unqualified_ts
            .clone()
            .ge(Expr::datetime(query.from.timestamp())),
    ];
    if let Some(to) = query.to {
        filter.push(unqualified_ts.le(Expr::datetime(to.timestamp())));
    }
    if !query.customer_ids.is_empty() {
        filter.push(Expr::ident(&cust).in_set(customer_uuids(query)));
    }
    filter.extend(fact_filter_conditions(&query.filters)?);
    // Narrow the read to the union of the members' rows; the `*If` predicates above split that
    // union back out per operand. Skipped when any member is unfiltered: its union is everything.
    if !members.is_empty() && members.iter().all(|m| has_effective_filters(m.filters)) {
        let mut ored = Vec::new();
        for m in members {
            let mut conds = fact_filter_conditions(m.filters)?;
            ored.push(if conds.len() > 1 {
                Expr::And(conds)
            } else {
                conds.remove(0)
            });
        }
        filter.push(if ored.len() > 1 {
            Expr::Or(ored)
        } else {
            ored.remove(0)
        });
    }
    if let Some(c) = cohort {
        filter.push(Expr::ident(&cust).in_set(Expr::subquery(c.clone())));
    }

    let mut group_by = Vec::new();
    if let Some(window) = &query.window {
        let tz = tz_name(query);
        group_by.push(as_datetime(
            tumble("tumbleStart", ts_expr.clone(), window, &tz),
            &tz,
        ));
        group_by.push(as_datetime(
            tumble("tumbleEnd", ts_expr.clone(), window, &tz),
            &tz,
        ));
    }
    for d in &query.group_by {
        group_by.push(dim_group(d, &fact_ctx)?);
    }
    // Snapshot: total per (bucket, day) first, so the outer wrap can pick the latest day.
    if snapshot {
        group_by.push(Expr::call("toDate", [ts_expr.clone()]));
    }

    // FINAL: fact sources are ReplacingMergeTree(version); the read must dedup to the latest version or aggregates double-count.
    let fact_table = Relation::final_table(Table::tenant(table, tenant_col));
    let inner = if fx_active {
        let fx_from = fx_join(
            query,
            members,
            &valcol,
            &native_col,
            &ts,
            &cust,
            fact_table,
            filter,
        )?;
        Select::new(items, fx_from).group_by(group_by)
    } else {
        Select::new(items, fact_table)
            .filter(filter)
            .group_by(group_by)
    };

    // A snapshot (daily level) must not sum a multi-day bucket's rows, which multiplies the
    // balance by the number of days: each bucket keeps only its latest day's total.
    let select = if snapshot {
        let mut carried: Vec<Expr> = Vec::new();
        for d in &query.group_by {
            carried.push(Expr::ident(&dimension_alias(d)));
            if entity_id(d, &fact_ctx).is_some() {
                carried.push(Expr::ident(&alias(dimension_disambiguator_alias(d))));
            }
        }
        let mut outer_items = vec![
            Item::new(Expr::col("window_start")),
            Item::new(Expr::col("window_end")),
            Item::as_(
                Expr::call("argMax", [Expr::col("value"), Expr::col("_snap_day")]),
                "value",
            ),
        ];
        outer_items.extend(carried.iter().cloned().map(Item::new));
        let mut outer_group = vec![Expr::col("window_start"), Expr::col("window_end")];
        outer_group.extend(carried);
        Select::new(outer_items, Relation::subquery(inner.query())).group_by(outer_group)
    } else {
        inner
    };

    Ok(Series::select(
        select.order_by_window_start(),
        dim_columns(query),
    ))
}

/// The fact rows joined, as of their own date, to the reporting currency's USD rate:
/// `( SELECT … FROM <fact> FINAL WHERE … ) AS f ASOF LEFT JOIN ( <rates> ) AS fx`.
#[allow(clippy::too_many_arguments)]
fn fx_join(
    query: &MetricQuery,
    members: &[FusedMember<'_>],
    valcol: &Ident,
    native_col: &Ident,
    ts: &Ident,
    cust: &Ident,
    fact_table: Relation,
    filter: Vec<Expr>,
) -> Result<Relation, MetricError> {
    let mut inner_cols: Vec<Ident> = vec![
        valcol.clone(),
        native_col.clone(),
        Ident::trusted("currency"),
        ts.clone(),
    ];
    // A customer-attribute split resolves the customer column in the outer query.
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
        inner_cols.push(cust.clone());
    }
    for d in &query.group_by {
        match d {
            Dimension::CustomerAttr(_)
            | Dimension::CustomerAttrMap { .. }
            | Dimension::CustomerProperty { .. }
            | Dimension::CustomerName(_) => {}
            Dimension::PlanAttr(_) => inner_cols.push(Ident::trusted("plan_id")),
            Dimension::ProductAttr(_) => inner_cols.push(Ident::trusted("product_id")),
            _ => inner_cols.push(Ident::column(d.name())?),
        }
    }
    // A fused member's `*If` predicate is evaluated above this projection.
    for m in members {
        for f in m.filters.iter().filter(|f| !f.values.is_empty()) {
            inner_cols.push(Ident::column(&f.property)?);
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut inner_items: Vec<Item> = inner_cols
        .into_iter()
        .filter(|c| seen.insert(c.clone()))
        .map(|c| Item::new(Expr::ident(&c)))
        .collect();
    let currency = bind_str(query.reporting_currency.as_deref().unwrap_or_default());
    inner_items.push(Item::as_(
        Expr::call("toLowCardinality", [currency.clone()]),
        "fx_currency",
    ));
    let facts = Select::new(inner_items, fact_table).filter(filter);

    // Restrict the ASOF build side to the one currency, through `to`: no lower bound, since
    // history-primed operands read from epoch and rates are sparse before ~2024. FINAL because
    // duplicate (currency, date) keys with conflicting rates could match a stale one.
    let mut fx_filter = vec![Expr::col("currency").eq(currency)];
    if let Some(to) = query.to {
        fx_filter
            .push(Expr::col("date").le(Expr::call("toDate", [Expr::datetime(to.timestamp())])));
    }
    let rates = Select::new(
        vec![
            Item::new(Expr::col("currency")),
            Item::new(Expr::col("date")),
            Item::new(Expr::col("rate_from_usd")),
        ],
        Relation::final_table(Table {
            database: None,
            name: Ident::trusted(FX_TABLE),
            scope: TableScope::Global,
        }),
    )
    .filter(fx_filter);

    Ok(Relation::aliased_subquery(facts.query(), "f").join(
        JoinKind::AsofLeft,
        Relation::aliased_subquery(rates.query(), "fx"),
        Some(Expr::And(vec![
            Expr::qcol("f", "fx_currency").eq(Expr::qcol("fx", "currency")),
            Expr::qcol("fx", "date").le(Expr::call(
                "toDate",
                [Expr::Column(Some(Ident::trusted("f")), ts.clone())],
            )),
        ])),
    ))
}

/// A threshold inlined as a literal: an integer when it is one, else the float. NaN/inf are
/// rejected: their comparisons are always false, a silently empty cohort.
fn threshold_literal(v: f64) -> Result<Expr, MetricError> {
    if !v.is_finite() {
        return Err(MetricError::Invalid(
            "threshold".to_string(),
            format!("threshold must be a finite number, got {v}"),
        ));
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Ok(Expr::Int(v as i64))
    } else {
        Ok(Expr::Float(v))
    }
}

fn threshold_op(op: ThresholdOp) -> Op {
    match op {
        ThresholdOp::Gt => Op::Gt,
        ThresholdOp::Gte => Op::Ge,
        ThresholdOp::Lt => Op::Lt,
        ThresholdOp::Lte => Op::Le,
    }
}

fn cohort_needs_predicates(cohort: &CohortDef) -> Result<(), MetricError> {
    if cohort.predicates.is_empty() {
        return Err(MetricError::Invalid(
            "cohort".to_string(),
            "a cohort needs at least one predicate".to_string(),
        ));
    }
    Ok(())
}

/// `SELECT customer_id FROM ( <set> ) AS <alias>`, the single-column shape set operations
/// combine.
fn customer_ids_of(set: Query, alias: &'static str) -> Query {
    Select::new(
        vec![Item::new(Expr::col("customer_id"))],
        Relation::aliased_subquery(set, alias),
    )
    .query()
}

/// A cohort's member set: `SELECT DISTINCT customer_id` over the INTERSECT of its predicates.
/// Only the tenant and time range of `query` apply (`analytics::segment::resolve_segment_members`).
pub fn compile_cohort_members(
    registry: &MetricRegistry,
    cohort: &CohortDef,
    query: &MetricQuery,
    events: impl Into<EventsTable>,
    database: &str,
) -> Result<SafeQuery, MetricError> {
    let tree = compile_cohort_members_tree(registry, cohort, query, events, database)?;
    finish(&tree, &[], query)
}

/// The tree [`compile_cohort_members`] renders.
pub fn compile_cohort_members_tree(
    registry: &MetricRegistry,
    cohort: &CohortDef,
    query: &MetricQuery,
    events: impl Into<EventsTable>,
    database: &str,
) -> Result<Query, MetricError> {
    registry.validate()?;
    let table: EventsTable = events.into();
    let events = table.source(
        database,
        query.tenant_id.as_uuid(),
        EventDedup::new(query.keep_last_events),
    );
    cohort_needs_predicates(cohort)?;
    // Each predicate projects one id column under varying names (`id`, `customer_id`): every
    // branch is re-aliased to `customer_id` so the INTERSECT is name-stable.
    let mut parts = Vec::new();
    for p in &cohort.predicates {
        let predicate = compile_cohort_predicate(registry, p, query, events)?;
        parts.push(customer_ids_of(predicate, "p").nested());
    }
    let members = Select {
        distinct: true,
        ..Select::new(
            vec![Item::new(Expr::col("customer_id"))],
            Relation::aliased_subquery(Query::SetOp(SetOp::Intersect, parts), "members"),
        )
    };
    Ok(members.query())
}

fn compile_cohort(
    registry: &MetricRegistry,
    cohort: &CohortDef,
    query: &MetricQuery,
    events: EventsSource<'_>,
) -> Result<Query, MetricError> {
    cohort_needs_predicates(cohort)?;
    let mut parts = Vec::new();
    for p in &cohort.predicates {
        parts.push(compile_cohort_predicate(registry, p, query, events)?.nested());
    }
    Ok(Query::SetOp(SetOp::Intersect, parts))
}

/// `SELECT <id> AS customer_id FROM <table> FINAL WHERE tenant_id = ? AND <condition>`.
fn dimension_members(
    query: &MetricQuery,
    table: &str,
    id_column: &str,
    condition: impl FnOnce() -> Result<Expr, MetricError>,
    checked: &[&str],
) -> Result<Query, MetricError> {
    let table = Ident::table(table)?;
    let id = Ident::column(id_column)?;
    for ident in checked {
        Ident::column(ident)?;
    }
    let tenant = Ident::trusted("tenant_id");
    let filter = vec![Expr::ident(&tenant).eq(tenant_bind(query)), condition()?];
    Ok(Select::new(
        vec![Item::as_(Expr::ident(&id), "customer_id")],
        Relation::final_table(Table::tenant(table, tenant)),
    )
    .filter(filter)
    .query())
}

/// The tenant's customers outside `excluded` (an anti-join on `dim_customer`).
fn customers_not_in(query: &MetricQuery, excluded: Query) -> Query {
    let tenant = Ident::trusted("tenant_id");
    Select::new(
        vec![Item::as_(Expr::col("id"), "customer_id")],
        Relation::final_table(Table::tenant(
            Ident::trusted("dim_customer"),
            tenant.clone(),
        )),
    )
    .filter(vec![
        Expr::ident(&tenant).eq(tenant_bind(query)),
        Expr::col("id").not_in_set(Expr::subquery(excluded)),
    ])
    .query()
}

fn compile_cohort_predicate(
    registry: &MetricRegistry,
    predicate: &CohortPredicate,
    query: &MetricQuery,
    events: EventsSource<'_>,
) -> Result<Query, MetricError> {
    match predicate {
        CohortPredicate::Dimension {
            table,
            id_column,
            column,
            values,
        } => dimension_members(
            query,
            table,
            id_column,
            || {
                Ok(Expr::ident(&Ident::column(column)?)
                    .in_set(Expr::Bind(BindValue::Strings(values.clone()))))
            },
            &[column],
        ),
        // `key` is bound into the map subscript: an arbitrary property is matched, never inlined.
        CohortPredicate::DimensionMap {
            table,
            id_column,
            map_column,
            key,
            values,
        } => dimension_members(
            query,
            table,
            id_column,
            || {
                let map = Expr::ident(&Ident::column(map_column)?);
                Ok(Expr::Index(Box::new(map), Box::new(bind_str(key)))
                    .in_set(Expr::Bind(BindValue::Strings(values.clone()))))
            },
            &[map_column],
        ),
        CohortPredicate::CustomProperty {
            table,
            id_column,
            json_column,
            key,
            op,
            value_type,
            values,
        } => dimension_members(
            query,
            table,
            id_column,
            || {
                let json = Ident::column(json_column)?;
                custom_property_condition(&json, key, *op, *value_type, values)
            },
            &[json_column],
        ),
        CohortPredicate::ActivePlan { plan_ids } => {
            // Per subscription: the latest plan, and the lifecycle delta of its latest
            // state-changing event (+1 active / -1 left). Active on plan = delta is +1.
            let tenant = Ident::trusted("tenant_id");
            let at = Expr::col("at");
            let latest = Select::new(
                vec![
                    Item::new(Expr::col("subscription_id")),
                    Item::as_(Expr::call("any", [Expr::col("customer_id")]), "customer_id"),
                    Item::as_(
                        Expr::call("argMax", [Expr::col("plan_id"), at.clone()]),
                        "_plan",
                    ),
                    Item::as_(
                        Expr::call(
                            "argMaxIf",
                            [
                                Expr::col("lifecycle_delta"),
                                at,
                                Expr::col("lifecycle_delta").ne(Expr::Int(0)),
                            ],
                        ),
                        "_state",
                    ),
                ],
                Relation::final_table(Table::tenant(
                    Ident::trusted("fact_subscription_event"),
                    tenant.clone(),
                )),
            )
            .filter(vec![Expr::ident(&tenant).eq(tenant_bind(query))])
            .group_by(vec![Expr::col("subscription_id")]);
            Ok(Select::new(
                vec![Item::new(Expr::col("customer_id"))],
                Relation::subquery(latest.query()),
            )
            .filter(vec![
                Expr::col("_state").eq(Expr::Int(1)),
                Expr::call("toString", [Expr::col("_plan")])
                    .in_set(Expr::Bind(BindValue::Strings(plan_ids.clone()))),
            ])
            .query())
        }
        CohortPredicate::MetricThreshold {
            metric,
            op,
            threshold,
        } => metric_active_set(registry, metric, *op, *threshold, query, events),
        CohortPredicate::MetricInactive { metric, max_active } => {
            // A metric a customer never emits leaves no group row, so the active set can't see
            // who's missing: `dim_customer` minus the active set is the only way.
            let active = metric_active_set(
                registry,
                metric,
                ThresholdOp::Gt,
                max_active.unwrap_or(0.0),
                query,
                events,
            )?;
            Ok(customers_not_in(query, active))
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
            events,
        ),
        CohortPredicate::And { children } => {
            compile_cohort_set_op(registry, children, query, events, SetOp::Intersect)
        }
        CohortPredicate::Or { children } => {
            compile_cohort_set_op(registry, children, query, events, SetOp::UnionDistinct)
        }
        CohortPredicate::Not { child } => {
            let excluded = compile_cohort_predicate(registry, child, query, events)?;
            Ok(customers_not_in(query, excluded))
        }
    }
}

/// The condition of a [`CohortPredicate::CustomProperty`] leaf: `JSONExtract*` over
/// `toString(col)` (a bound key), typed by `value_type`. Comparisons a missing key would
/// satisfy are guarded with `JSONHas`.
fn custom_property_condition(
    json_column: &Ident,
    key: &str,
    op: PropertyOp,
    value_type: PropertyValueType,
    values: &[String],
) -> Result<Expr, MetricError> {
    let js = Expr::call("toString", [Expr::ident(json_column)]);
    let has = Expr::call("JSONHas", [js.clone(), bind_str(key)]);
    let accessor = Expr::call(
        match value_type {
            PropertyValueType::Number => "JSONExtractFloat",
            PropertyValueType::Boolean => "JSONExtractBool",
            PropertyValueType::Text | PropertyValueType::Date => "JSONExtractString",
        },
        [js, bind_str(key)],
    );
    let nth = |i: usize| -> Result<&String, MetricError> {
        values.get(i).ok_or_else(|| {
            MetricError::Invalid(
                "custom property".to_string(),
                format!("operator {op:?} needs at least {} value(s)", i + 1),
            )
        })
    };
    // Numeric literals are bound as strings and cast in SQL.
    let operand = |raw: &String| match value_type {
        PropertyValueType::Number => Expr::call("toFloat64", [bind_str(raw)]),
        _ => bind_str(raw),
    };
    let guarded = |cmp: Op, raw: &String| {
        Expr::And(vec![has.clone(), accessor.clone().op(cmp, operand(raw))])
    };

    let cond = match op {
        PropertyOp::IsSet => has,
        PropertyOp::IsEmpty => Expr::Not(Box::new(has)),
        PropertyOp::IsTrue => accessor.eq(Expr::Int(1)),
        PropertyOp::IsFalse => Expr::And(vec![has, accessor.eq(Expr::Int(0))]),
        PropertyOp::Equals => guarded(Op::Eq, nth(0)?),
        PropertyOp::NotEquals => guarded(Op::Ne, nth(0)?),
        PropertyOp::Gt | PropertyOp::After => guarded(Op::Gt, nth(0)?),
        PropertyOp::Gte => guarded(Op::Ge, nth(0)?),
        PropertyOp::Lt | PropertyOp::Before => guarded(Op::Lt, nth(0)?),
        PropertyOp::Lte => guarded(Op::Le, nth(0)?),
        PropertyOp::Between => {
            let (lo, hi) = (nth(0)?, nth(1)?);
            Expr::And(vec![
                has,
                accessor.clone().ge(operand(lo)),
                accessor.le(operand(hi)),
            ])
        }
        PropertyOp::Contains => {
            Expr::call("positionCaseInsensitive", [accessor, bind_str(nth(0)?)]).gt(Expr::Int(0))
        }
        PropertyOp::In => {
            if values.is_empty() {
                return Err(MetricError::Invalid(
                    "custom property".to_string(),
                    "IN needs at least one value".to_string(),
                ));
            }
            let set = Expr::Bind(BindValue::Strings(values.to_vec()));
            match value_type {
                PropertyValueType::Number => {
                    let v = Ident::trusted("v");
                    let as_floats = Expr::call(
                        "arrayMap",
                        [
                            Expr::Lambda(
                                v.clone(),
                                Box::new(Expr::call("toFloat64", [Expr::ident(&v)])),
                            ),
                            set,
                        ],
                    );
                    Expr::And(vec![has, Expr::call("has", [as_floats, accessor])])
                }
                _ => accessor.in_set(set),
            }
        }
    };
    Ok(cond)
}

/// Combine child predicates' customer-id sets with a set operator. Each child sits in a derived
/// table so its own set operations keep their precedence.
fn compile_cohort_set_op(
    registry: &MetricRegistry,
    children: &[CohortPredicate],
    query: &MetricQuery,
    events: EventsSource<'_>,
    op: SetOp,
) -> Result<Query, MetricError> {
    if children.is_empty() {
        return Err(MetricError::Invalid(
            "cohort".to_string(),
            "a boolean group needs at least one child".to_string(),
        ));
    }
    let mut parts = Vec::new();
    for child in children {
        let child = compile_cohort_predicate(registry, child, query, events)?;
        parts.push(customer_ids_of(child, "c"));
    }
    Ok(Query::SetOp(op, parts))
}

fn aggregate_spec<'a>(def: &'a MetricDef, what: &str) -> Result<&'a AggregateSpec, MetricError> {
    match &def.kind {
        MetricKind::Aggregate(s) => Ok(s),
        _ => Err(MetricError::Unsupported(format!(
            "cohort {what} require an aggregate metric"
        ))),
    }
}

/// The identifiers of a fact source, validated: table, value, timestamp, customer, tenant.
struct FactIdents {
    table: Ident,
    value: Ident,
    ts: Ident,
    customer: Ident,
    tenant: Ident,
}

impl FactIdents {
    fn new(fs: &FactSource) -> Result<FactIdents, MetricError> {
        Ok(FactIdents {
            table: Ident::table(&fs.table)?,
            value: Ident::column(&fs.value_column)?,
            ts: Ident::column(&fs.timestamp_column)?,
            customer: Ident::column(&fs.customer_column)?,
            tenant: Ident::column(&fs.tenant_column)?,
        })
    }

    fn source(&self) -> Relation {
        Relation::final_table(Table::tenant(self.table.clone(), self.tenant.clone()))
    }

    fn ts(&self) -> Expr {
        Expr::call("toDateTime", [Expr::ident(&self.ts)])
    }
}

/// The customers whose aggregate of `metric` (a non-derived aggregate) over the query window
/// satisfies `<op> <threshold>`. Also the active set `MetricInactive` anti-joins.
fn metric_active_set(
    registry: &MetricRegistry,
    metric: &str,
    op: ThresholdOp,
    threshold: f64,
    query: &MetricQuery,
    events: EventsSource<'_>,
) -> Result<Query, MetricError> {
    let def = registry
        .get(metric)
        .ok_or_else(|| MetricError::UnknownMetric(metric.to_string()))?;
    let spec = aggregate_spec(def, "thresholds")?;
    let lit = threshold_literal(threshold)?;
    let cmp = threshold_op(op);

    match &def.source {
        MetricSource::RawEvents => {
            let from_ts = query.from.timestamp();
            let to_ts = query.to.map(|t| t.timestamp());
            let filter = raw_event_bounds(query, &def.event_code, from_ts, to_ts);
            // Divided by the unit-conversion factor: the threshold is in the billed unit.
            let having = apply_unit_conversion(spec, raw_having_expr(spec)?);
            let source = events.read(
                meter_event_columns(),
                filter,
                void_scope(&def.event_code, from_ts, to_ts),
            );
            Ok(
                Select::new(vec![Item::new(Expr::col("customer_id"))], source)
                    .group_by(vec![Expr::col("customer_id")])
                    .having(vec![having.op(cmp, lit)])
                    .query(),
            )
        }
        MetricSource::Fact(fs) => {
            let f = FactIdents::new(fs)?;
            let mut filter = vec![Expr::ident(&f.tenant).eq(tenant_bind(query))];
            // A stock (AsOf) is the level as-of `to`: no lower bound, so a `Sum` accumulates
            // every delta through `to` and a `Latest` picks the newest snapshot on or before it.
            if !matches!(spec.cohort_reduction, CohortReduction::AsOf) {
                filter.push(f.ts().ge(Expr::datetime(query.from.timestamp())));
            }
            if let Some(to) = query.to {
                filter.push(f.ts().le(Expr::datetime(to.timestamp())));
            }
            let having = match spec.op {
                AggregateOp::Count => Expr::call("count", [Expr::Star]),
                AggregateOp::CountDistinct => Expr::call("uniq", [Expr::ident(&f.value)]),
                op => aggregate(op, Expr::call("toFloat64", [Expr::ident(&f.value)]), f.ts()),
            };
            Ok(Select::new(
                vec![Item::as_(Expr::ident(&f.customer), "customer_id")],
                f.source(),
            )
            .filter(filter)
            .group_by(vec![Expr::ident(&f.customer)])
            .having(vec![having.op(cmp, lit)])
            .query())
        }
    }
}

/// `prior != 0 AND (cur - prior) / prior * 100 <op> <pct>`.
fn delta_having(op: ThresholdOp, pct: Expr) -> Vec<Expr> {
    let (cur, prior) = (Expr::col("cur"), Expr::col("prior"));
    vec![
        prior.clone().ne(Expr::Int(0)),
        cur.op(Op::Sub, prior.clone())
            .op(Op::Div, prior)
            .op(Op::Mul, Expr::Int(100))
            .op(threshold_op(op), pct),
    ]
}

/// The customers whose aggregate changed by `<op> pct` percent between `[from, to]` and the
/// `prior_window_days` before it (default: as long), in one scan split by `*If` aggregates.
/// No unit conversion: a constant divisor cancels in the ratio.
fn metric_delta_set(
    registry: &MetricRegistry,
    metric: &str,
    op: ThresholdOp,
    pct: f64,
    prior_window_days: Option<u32>,
    query: &MetricQuery,
    events: EventsSource<'_>,
) -> Result<Query, MetricError> {
    let def = registry
        .get(metric)
        .ok_or_else(|| MetricError::UnknownMetric(metric.to_string()))?;
    let spec = aggregate_spec(def, "deltas")?;

    let to = query.to.unwrap_or(query.from);
    let window_days = (to - query.from).num_days();
    let prior_days = prior_window_days.map(i64::from).unwrap_or(window_days);
    let prior_from = query.from - chrono::Duration::days(prior_days);
    let from_ts = query.from.timestamp();
    let prior_from_ts = prior_from.timestamp();
    let to_ts = to.timestamp();

    let pct = threshold_literal(pct)?;
    let windows = |ts: Expr| {
        let cur = ts.clone().ge(Expr::datetime(from_ts));
        let prior = Expr::And(vec![
            ts.clone().lt(Expr::datetime(from_ts)),
            ts.ge(Expr::datetime(prior_from_ts)),
        ]);
        (cur, prior)
    };

    let inner = match &def.source {
        MetricSource::RawEvents => {
            let (cur_pred, prior_pred) = windows(Expr::col("timestamp"));
            let cur = raw_delta_agg(spec, cur_pred)?;
            let prior = raw_delta_agg(spec, prior_pred)?;
            let filter = raw_event_bounds(query, &def.event_code, prior_from_ts, Some(to_ts));
            let source = events.read(
                meter_event_columns(),
                filter,
                void_scope(&def.event_code, prior_from_ts, Some(to_ts)),
            );
            Select::new(
                vec![
                    Item::new(Expr::col("customer_id")),
                    Item::as_(Expr::call("toFloat64", [cur]), "cur"),
                    Item::as_(Expr::call("toFloat64", [prior]), "prior"),
                ],
                source,
            )
            .group_by(vec![Expr::col("customer_id")])
            .having(delta_having(op, pct))
        }
        MetricSource::Fact(fs) => {
            let f = FactIdents::new(fs)?;
            let (cur_pred, prior_pred) = windows(f.ts());
            let agg = |predicate: Expr| {
                aggregate_if(
                    spec.op,
                    Expr::call("toFloat64", [Expr::ident(&f.value)]),
                    f.ts(),
                    Expr::ident(&f.value),
                    predicate,
                )
            };
            let filter = vec![
                Expr::ident(&f.tenant).eq(tenant_bind(query)),
                f.ts().ge(Expr::datetime(prior_from_ts)),
                f.ts().le(Expr::datetime(to_ts)),
            ];
            Select::new(
                vec![
                    Item::as_(Expr::ident(&f.customer), "customer_id"),
                    Item::as_(Expr::call("toFloat64", [agg(cur_pred)]), "cur"),
                    Item::as_(Expr::call("toFloat64", [agg(prior_pred)]), "prior"),
                ],
                f.source(),
            )
            .filter(filter)
            .group_by(vec![Expr::ident(&f.customer)])
            .having(delta_having(op, pct))
        }
    };
    Ok(customer_ids_of(inner.query(), "d"))
}

/// One conditional aggregate (`cur` or `prior`) of a raw-events delta.
fn raw_delta_agg(spec: &AggregateSpec, predicate: Expr) -> Result<Expr, MetricError> {
    if matches!(spec.op, AggregateOp::Count) {
        return Ok(Expr::call("countIf", [predicate]));
    }
    let prop = required_value_property(spec, format!("{:?}", spec.op), "value_property required")?;
    Ok(aggregate_if(
        spec.op,
        Expr::call("toFloat64OrZero", [property(prop)]),
        latest_event_order(),
        property(prop),
        predicate,
    ))
}

fn raw_having_expr(spec: &AggregateSpec) -> Result<Expr, MetricError> {
    match spec.op {
        AggregateOp::Count => Ok(Expr::call("count", [Expr::Star])),
        op => {
            let prop = required_value_property(spec, format!("{op:?}"), "value_property required")?;
            Ok(match op {
                AggregateOp::CountDistinct => Expr::call("uniq", [property(prop)]),
                _ => aggregate(
                    op,
                    Expr::call("toFloat64OrZero", [property(prop)]),
                    latest_event_order(),
                ),
            })
        }
    }
}

/// A (metric key, optional transform) pair, as `collect_operands` yields it.
type OperandKey = (String, Option<CrossPeriod>);

/// Where an operand's value lands. Fusion puts several operands in one CTE, so the column is no
/// longer always `value`.
struct OperandRef {
    alias: Ident,
    column: Ident,
}

/// What makes two operands fusable: the same fact read under the same aggregate. Everything else
/// about them is already shared (tenant, range, window, dimensions, cohort).
#[derive(PartialEq)]
struct FusionKey {
    fs: FactSource,
    spec: AggregateSpec,
}

/// Operands fuse when one grouped scan, split by their own filters, produces all of them. A
/// transform (history-primed range), a snapshot (second reduction) or a unit conversion
/// (rescaled series) keeps its own CTE.
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
    expr: &MetricExpr,
    query: &MetricQuery,
    events: EventsSource<'_>,
    database: &str,
    cohort: Option<&Query>,
) -> Result<Series, MetricError> {
    // Each operand is a (metric key, optional per-operand transform) pair, deduped while keeping
    // expression order. The same metric may appear both plain and transformed — they become two
    // distinct CTEs (see `operand_alias`), which is how a retention ratio composes a
    // lagged-cumulative *opening balance* against a within-window flow.
    let operands = dedup_operands(collect_operands(expr));
    if operands.is_empty() {
        return Err(MetricError::Invalid(
            "derived".to_string(),
            "expression references no metrics".to_string(),
        ));
    }

    let dims = dim_columns(query);
    let mut ctes: Vec<(Ident, Query)> = Vec::new();
    // A history-primed operand (a cumulative built from epoch) makes the join span pre-`from`
    // windows, so the result is floored.
    let mut history_primed = false;

    // Partition operands into fusion groups in first-appearance order. Operands that can't fuse
    // (or are alone) keep their own CTE.
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
    let mut aliases: Vec<Ident> = Vec::new();
    // A fused CTE's synthetic alias can't collide with a metric literally keyed `fused_0`, whose
    // own CTE would be `m_fused_0`.
    let mut used: std::collections::HashSet<String> = operands
        .iter()
        .map(|(k, t)| operand_alias(k, t.as_ref()).as_str().to_string())
        .collect();
    for (fkey, members) in &groups {
        if let (Some(FusionKey { fs, spec }), true) = (fkey.as_ref(), members.len() > 1) {
            let fused_alias = (0..)
                .map(|n| format!("m_fused_{n}"))
                .find(|a| used.insert(a.clone()))
                .map(alias)
                .expect("an unused alias always exists");
            let defs: Vec<&MetricDef> = members
                .iter()
                .map(|&i| registry.get(&operands[i].0).expect("resolved above"))
                .collect();
            // Each column is named after its metric; a suffix breaks the rare sanitize tie.
            let mut taken = std::collections::HashSet::new();
            let columns: Vec<Ident> = members
                .iter()
                .map(|&i| {
                    let base = format!("value_{}", crate::sql::sanitize_identifier(&operands[i].0));
                    (0..)
                        .map(|n| {
                            if n == 0 {
                                base.clone()
                            } else {
                                format!("{base}_{n}")
                            }
                        })
                        .find(|c| taken.insert(c.clone()))
                        .map(alias)
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
            ctes.push((fused_alias.clone(), sub.query));
            for (&i, column) in members.iter().zip(&columns) {
                refs.push((
                    operands[i].clone(),
                    OperandRef {
                        alias: fused_alias.clone(),
                        column: column.clone(),
                    },
                ));
            }
            aliases.push(fused_alias);
            continue;
        }

        let i = members[0];
        let (key, transform) = &operands[i];
        let def = registry.get(key).expect("resolved above");
        let operand = operand_alias(key, transform.as_ref());
        let sub = match transform {
            None => build_series(registry, def, query, events, database, cohort)?,
            // A transformed operand is built over a history-primed range, then windowed.
            // Running/lagged cumulatives prime from epoch so the first *displayed* window already
            // carries a correct opening balance.
            Some(t) => {
                let primed_from = match (t, &query.window) {
                    (CrossPeriod::RunningTotal | CrossPeriod::RunningTotalLagged, _) => {
                        Some(epoch())
                    }
                    (_, Some(w)) => Some(history_from(t, query.from, w)),
                    (_, None) => None,
                };
                let mut sub_query = query.clone();
                if let Some(from) = primed_from {
                    history_primed = true;
                    sub_query.from = from;
                }
                let base = build_series(registry, def, &sub_query, events, database, cohort)?;
                transformed_sub_series(base, t)
            }
        };
        ctes.push((operand.clone(), sub.query));
        refs.push((
            operands[i].clone(),
            OperandRef {
                alias: operand.clone(),
                column: Ident::trusted("value"),
            },
        ));
        aliases.push(operand);
    }

    // FULL OUTER JOIN chain over the CTEs, one per fusion group. Explicit ON conditions (bare
    // USING-key resolution is analyzer-dependent), run with `join_use_nulls = 1`.
    let mut key_cols = vec![Ident::trusted("window_start"), Ident::trusted("window_end")];
    key_cols.extend(dims.iter().cloned());

    let coalesce_key = |aliases: &[Ident], col: &Ident| -> Expr {
        let qualified = |a: &Ident| Expr::Column(Some(a.clone()), col.clone());
        if aliases.len() == 1 {
            qualified(&aliases[0])
        } else {
            Expr::call(
                "coalesce",
                aliases.iter().map(qualified).collect::<Vec<_>>(),
            )
        }
    };

    let mut from = Relation::Named(aliases[0].clone(), None);
    for (i, a) in aliases.iter().enumerate().skip(1) {
        let on = key_cols
            .iter()
            .map(|col| {
                coalesce_key(&aliases[..i], col).eq(Expr::Column(Some(a.clone()), col.clone()))
            })
            .collect();
        from = from.join(
            JoinKind::FullOuter,
            Relation::Named(a.clone(), None),
            Some(Expr::And(on)),
        );
    }

    let mut items = vec![
        Item::as_(
            coalesce_key(&aliases, &Ident::trusted("window_start")),
            "window_start",
        ),
        Item::as_(
            coalesce_key(&aliases, &Ident::trusted("window_end")),
            "window_end",
        ),
        Item::as_(compile_expr(expr, &refs), "value"),
    ];
    for d in &dims {
        items.push(Item::aliased(coalesce_key(&aliases, d), d.clone()));
    }
    let joined = Query::with_ctes(
        ctes,
        Select::new(items, from).order_by_window_start().query(),
    );

    // Floor the *result* back to `>= from` in an outer select, never inside an operand's WHERE,
    // which would erase the history the window function needs.
    let query_tree = if history_primed {
        series_columns(&dims, Relation::subquery(joined))
            .filter(vec![
                Expr::col("window_start").ge(Expr::datetime(query.from.timestamp())),
            ])
            .order_by_window_start()
            .query()
    } else {
        joined
    };

    // Force NULL (not type defaults) for missing-side columns so the key coalesce is correct under every server config.
    let settings = if aliases.len() > 1 {
        vec![Setting {
            name: "join_use_nulls",
            value: 1,
        }]
    } else {
        vec![]
    };

    Ok(Series {
        query: query_tree,
        dims,
        settings,
    })
}

/// `SELECT window_start, window_end, value, <dims> FROM <from>`.
fn series_columns(dims: &[Ident], from: Relation) -> Select {
    let mut items = vec![
        Item::new(Expr::col("window_start")),
        Item::new(Expr::col("window_end")),
        Item::new(Expr::col("value")),
    ];
    items.extend(dims.iter().map(|d| Item::new(Expr::ident(d))));
    Select::new(items, from)
}

/// Collect `(metric key, optional per-operand transform)` operands in expression order.
fn collect_operands(expr: &MetricExpr) -> Vec<(String, Option<CrossPeriod>)> {
    let mut out = Vec::new();
    fn walk(e: &MetricExpr, out: &mut Vec<(String, Option<CrossPeriod>)>) {
        match e {
            MetricExpr::Metric(k) => out.push((k.clone(), None)),
            MetricExpr::MetricTransformed { metric, transform } => {
                out.push((metric.clone(), Some(transform.clone())))
            }
            MetricExpr::Const(_) => {}
            MetricExpr::Binary(l, _, r) => {
                walk(l, out);
                walk(r, out);
            }
            MetricExpr::Unary(_, e) => walk(e, out),
            MetricExpr::Clamp { expr, min, max } => {
                walk(expr, out);
                if let Some(min) = min {
                    walk(min, out);
                }
                if let Some(max) = max {
                    walk(max, out);
                }
            }
            MetricExpr::Coalesce(items)
            | MetricExpr::Greatest(items)
            | MetricExpr::Least(items) => {
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

/// The window-function `value` for a [`CrossPeriod`] transform over a series partitioned by
/// `dims` and ordered by `window_start`.
fn cross_period_value(transform: &CrossPeriod, dims: &[Ident]) -> Expr {
    let window = |(start, end): (FrameBound, FrameBound)| Window {
        partition_by: dims.iter().map(Expr::ident).collect(),
        order_by: vec![Expr::col("window_start")],
        frame: Some(Frame { start, end }),
    };
    let lag = || {
        Expr::call(
            "lagInFrame",
            [Expr::col("value"), Expr::Int(1), Expr::Int(0)],
        )
        .over(window((FrameBound::Preceding(1), FrameBound::CurrentRow)))
    };
    let value = Expr::col("value");
    match transform {
        CrossPeriod::RunningTotal => Expr::call("sum", [value]).over(window((
            FrameBound::UnboundedPreceding,
            FrameBound::CurrentRow,
        ))),
        // Opening balance: the cumulative total *through the previous window*.
        CrossPeriod::RunningTotalLagged => Expr::call("sum", [value]).over(window((
            FrameBound::UnboundedPreceding,
            FrameBound::Preceding(1),
        ))),
        CrossPeriod::PeriodOverPeriodDelta => value.op(Op::Sub, lag()),
        CrossPeriod::PercentChange => value
            .op(Op::Sub, lag())
            .op(Op::Div, Expr::call("nullIf", [lag(), Expr::Int(0)])),
        CrossPeriod::MovingAverage { periods } => Expr::call("avg", [value]).over(window((
            FrameBound::Preceding(periods.saturating_sub(1)),
            FrameBound::CurrentRow,
        ))),
    }
}

/// `SELECT window_start, window_end, <transform> AS value, <dims> FROM ( <series> )`.
fn window_series(series: Series, transform: &CrossPeriod) -> (Select, Vec<Ident>, Vec<Setting>) {
    let mut items = vec![
        Item::new(Expr::col("window_start")),
        Item::new(Expr::col("window_end")),
        Item::as_(cross_period_value(transform, &series.dims), "value"),
    ];
    items.extend(series.dims.iter().map(|d| Item::new(Expr::ident(d))));
    (
        Select::new(items, Relation::subquery(series.query)),
        series.dims,
        series.settings,
    )
}

/// A transformed operand of a derived metric: no floor, the derived join floors once.
fn transformed_sub_series(base: Series, transform: &CrossPeriod) -> Series {
    let (select, dims, settings) = window_series(base, transform);
    Series {
        query: select.query(),
        dims,
        settings,
    }
}

fn operand_value(
    refs: &[(OperandKey, OperandRef)],
    key: &str,
    transform: Option<&CrossPeriod>,
) -> Expr {
    refs.iter()
        .find(|((k, t), _)| k == key && t.as_ref() == transform)
        .map(|(_, r)| Expr::Column(Some(r.alias.clone()), r.column.clone()))
        .unwrap_or_else(|| {
            Expr::Column(Some(operand_alias(key, transform)), Ident::trusted("value"))
        })
}

fn compile_expr(expr: &MetricExpr, refs: &[(OperandKey, OperandRef)]) -> Expr {
    match expr {
        // Zero-fill: under `join_use_nulls = 1` a window missing from one sub-series yields NULL.
        MetricExpr::Metric(k) => {
            Expr::call("coalesce", [operand_value(refs, k, None), Expr::Int(0)])
        }
        // An *opening balance* of NULL propagates to a NULL ratio, not a guarded 0 that would
        // read as "0% retention".
        MetricExpr::MetricTransformed { metric, transform } => {
            operand_value(refs, metric, Some(transform))
        }
        MetricExpr::Const(c) => Expr::Float(*c),
        MetricExpr::Binary(l, op, r) => {
            let l = compile_expr(l, refs);
            match op {
                BinOp::Add => l.op(Op::Add, compile_expr(r, refs)),
                BinOp::Sub => l.op(Op::Sub, compile_expr(r, refs)),
                BinOp::Mul => l.op(Op::Mul, compile_expr(r, refs)),
                BinOp::Div => {
                    let denom = match r.as_ref() {
                        MetricExpr::Metric(k) => operand_value(refs, k, None),
                        MetricExpr::MetricTransformed { metric, transform } => {
                            operand_value(refs, metric, Some(transform))
                        }
                        _ => compile_expr(r, refs),
                    };
                    l.op(Op::Div, Expr::call("nullIf", [denom, Expr::Int(0)]))
                }
            }
        }
        MetricExpr::Unary(op, e) => {
            let e = compile_expr(e, refs);
            match op {
                UnaryOp::Neg => Expr::Neg(Box::new(e)),
                UnaryOp::Abs => Expr::call("abs", [e]),
            }
        }
        MetricExpr::Clamp { expr, min, max } => {
            let mut out = compile_expr(expr, refs);
            // Lower bound first, so the upper bound also holds when both are present.
            if let Some(min) = min {
                out = Expr::call("greatest", [compile_expr(min, refs), out]);
            }
            if let Some(max) = max {
                out = Expr::call("least", [compile_expr(max, refs), out]);
            }
            out
        }
        MetricExpr::Coalesce(items) => Expr::call("coalesce", compile_exprs(items, refs)),
        MetricExpr::Greatest(items) => Expr::call("greatest", compile_exprs(items, refs)),
        MetricExpr::Least(items) => Expr::call("least", compile_exprs(items, refs)),
    }
}

fn compile_exprs(items: &[MetricExpr], refs: &[(OperandKey, OperandRef)]) -> Vec<Expr> {
    items.iter().map(|i| compile_expr(i, refs)).collect()
}

/// The CTE of a (possibly transformed) operand: `m_<key>`, or `m_<key>__<transform>` so one
/// metric can be both a flow and a lagged cumulative in one expression (retention ratios).
fn operand_alias(key: &str, transform: Option<&CrossPeriod>) -> Ident {
    let key = crate::sql::sanitize_identifier(key);
    alias(match transform {
        None => format!("m_{key}"),
        Some(t) => format!("m_{key}__{}", transform_suffix(t)),
    })
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

struct GapFill {
    from_ts: i64,
    to_ts: i64,
    window: WindowSize,
    /// The fill start floors to the same bucket boundary as the window function, else a
    /// non-aligned `from` (any weekday for a week grain) puts fill rows off the real buckets.
    tz: String,
}

fn apply_cross_period(
    series: Series,
    transform: &CrossPeriod,
    floor_ts: Option<i64>,
    fill: Option<GapFill>,
) -> Query {
    let (mut windowed, dims, _) = window_series(series, transform);

    // Densify a dimensionless cumulative series, carrying `value` into empty windows.
    let filled = fill.filter(|_| dims.is_empty());
    if let Some(g) = &filled {
        windowed.order_by = vec![Expr::col("window_start").into()];
        windowed.fill = Some(Fill {
            from: as_datetime(
                tumble(
                    "tumbleStart",
                    Expr::Bind(BindValue::I64(g.from_ts)),
                    &g.window,
                    &g.tz,
                ),
                &g.tz,
            ),
            to: Expr::datetime(g.to_ts),
            step: interval(&g.window),
            interpolate: vec![(Ident::trusted("value"), Expr::col("value"))],
        });
    }

    match floor_ts {
        None => windowed.order_by_window_start().query(),
        // The window function must see pre-`from` rows for the opening balance, so floor in an outer select, never in its WHERE.
        Some(ts) => {
            let window_end = match &filled {
                Some(g) => Item::as_(
                    Expr::col("window_start").op(Op::Add, interval(&g.window)),
                    "window_end",
                ),
                None => Item::new(Expr::col("window_end")),
            };
            let mut select = series_columns(&dims, Relation::subquery(windowed.query()));
            select.items[1] = window_end;
            select
                .filter(vec![Expr::col("window_start").ge(Expr::datetime(ts))])
                .order_by_window_start()
                .query()
        }
    }
}

/// Forward-fill a Latest/Max gauge: empty windows inherit the prior level. The epoch-primed
/// `series` is densified into a window grid, the last level carried forward, then floored to `from`.
fn carry_over_gauge_series(series: Series, query: &MetricQuery, window: &WindowSize) -> Series {
    let step = Expr::Int(window_seconds(window));
    let tz = tz_name(query);
    let from_ts = query.from.timestamp();
    let to_ts = query.to.unwrap_or_else(Utc::now).timestamp();
    let dims = series.dims.clone();
    let window_bounds = || {
        vec![
            Item::new(Expr::col("window_start")),
            Item::new(Expr::col("window_end")),
        ]
    };

    // Only the ungrouped (single balance) series gets a dense grid: a scaffold can't know which
    // dimension tuples to densify. Grouped gauges forward-fill over the rows they produced.
    let inner = if dims.is_empty() {
        let grid_start = |offset: Expr| {
            Expr::call(
                "toDateTime",
                [tumble(
                    "tumbleStart",
                    Expr::Bind(BindValue::I64(from_ts)),
                    window,
                    &tz,
                )
                .op(Op::Add, offset.op(Op::Mul, step.clone()))],
            )
        };
        let unix = |secs: i64| Expr::call("toUnixTimestamp", [Expr::datetime(secs)]);
        let count = Expr::call(
            "toUInt64",
            [unix(to_ts)
                .op(Op::Sub, unix(from_ts))
                .op(Op::Div, step.clone())],
        )
        .op(Op::Add, Expr::Int(1));
        let grid = Select {
            items: vec![
                Item::as_(grid_start(Expr::col("number")), "window_start"),
                Item::as_(
                    grid_start(Expr::col("number").op(Op::Add, Expr::Int(1))),
                    "window_end",
                ),
            ],
            from: Some(Relation::Function(Expr::call("numbers", [count]))),
            ..Select::default()
        };
        // `max` collapses the (real, scaffold-NULL) pair per window to the real level.
        let mut real = window_bounds();
        real.push(Item::new(Expr::col("value")));
        let mut scaffold = window_bounds();
        scaffold.push(Item::as_(Expr::Null, "value"));
        let both = Query::SetOp(
            SetOp::UnionAll,
            vec![
                Select::new(real, Relation::subquery(series.query)).query(),
                Select::new(scaffold, Relation::subquery(grid.query())).query(),
            ],
        );
        let mut items = window_bounds();
        items.push(Item::as_(Expr::call("max", [Expr::col("value")]), "value"));
        Select::new(items, Relation::subquery(both))
            .group_by(vec![Expr::col("window_start"), Expr::col("window_end")])
    } else {
        series_columns(&dims, Relation::subquery(series.query))
    };

    let carried = Expr::Over {
        call: Box::new(Expr::call("last_value", [Expr::col("value")])),
        ignore_nulls: true,
        window: Window {
            partition_by: dims.iter().map(Expr::ident).collect(),
            order_by: vec![Expr::col("window_start")],
            frame: Some(Frame {
                start: FrameBound::UnboundedPreceding,
                end: FrameBound::CurrentRow,
            }),
        },
    };
    let mut items = window_bounds();
    items.push(Item::as_(
        Expr::call("coalesce", [carried, Expr::Int(0)]),
        "value",
    ));
    items.extend(dims.iter().map(|d| Item::new(Expr::ident(d))));
    let select = Select::new(items, Relation::subquery(inner.query()))
        .filter(vec![Expr::col("window_start").ge(Expr::datetime(from_ts))])
        .order_by_window_start();

    Series {
        query: select.query(),
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

    /// The void filter `EventsSource` appends for `base_query`'s tenant, narrowed by `scope`.
    fn live(scope: &str) -> String {
        let voids = format!("{}{scope}", voids_of_tenant());
        format!(
            ") AND is_tombstone = 0 AND ((SELECT count() {voids}) = 0 OR (customer_id, id, \
             ingested_at) NOT IN (SELECT customer_id, id, ingested_at {voids}))"
        )
    }

    fn voids_of_tenant() -> String {
        format!(
            "FROM meteroid.event_voids_local WHERE tenant_id = toUUID('{}')",
            TenantId::default().as_uuid()
        )
    }

    /// `base_query`'s code and window.
    const USAGE_DAY: &str = " AND code IN ('usage') AND timestamp >= toDateTime(1704067200) \
        AND timestamp <= toDateTime(1704153600)";

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
        let expected = format!(
            r#"
            SELECT
                toDateTime(tumbleStart(toDateTime(timestamp), toIntervalDay(1), ?), ?) AS window_start,
                toDateTime(tumbleEnd(toDateTime(timestamp), toIntervalDay(1), ?), ?) AS window_end,
                sum(toFloat64OrZero(properties[?])) AS value
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties
                FROM raw_events_v2
                WHERE (tenant_id = ? AND code = ? AND timestamp >= toDateTime(?)
                    AND timestamp <= toDateTime(?)
                    AND properties[?] != '' AND isNotNull(toFloat64OrNull(properties[?])){}
            )
            GROUP BY
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), ?),
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), ?)
            ORDER BY window_start
        "#,
            live(USAGE_DAY)
        );
        assert_eq!(normalize(&out.sql), normalize(&expected));
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
            normalize(&out.sql).contains("sum(toFloat64OrZero(properties[?])) / 1024 AS value"),
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
            sql.contains("mapContains(properties, ?) AND properties[?] != ?"),
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
            EventsTable::new("raw_events_v2").source(
                "meteroid",
                q.tenant_id.as_uuid(),
                EventDedup::Off,
            ),
            "meteroid",
            None,
        )
        .unwrap();

        let sql = normalize(&crate::sql::render::render(&series.query, &[]).sql);
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
            !series.dims.iter().any(|d| d.as_str() == "customer_id"),
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
        assert!(out.sql.contains("+ toIntervalHour(number)"));
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
                .contains("coalesce(m_revenue.value, 0) / nullIf(m_users.value, 0) AS value")
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
        assert!(out.sql.contains("-coalesce(m_revenue.value, 0) AS value"));

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
                .contains("/ nullIf(m_delta__running_total_lagged.value, 0) AS value"),
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
                "sumIf(toFloat64(delta_cents), movement_type = 'NewBusiness') AS value_gained"
            ) && sql
                .contains("sumIf(toFloat64(delta_cents), movement_type = 'Churn') AS value_lost"),
            "each operand becomes its own conditional aggregate, named for its metric: {sql}"
        );
        // The read is narrowed to the union, so unrelated movement types never enter the group by.
        assert!(
            sql.contains("AND (movement_type = 'NewBusiness' OR movement_type = 'Churn')"),
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
                .contains("coalesce(m_revenue.value, 0) / nullIf(m_users.value, 0)")
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
            sql.contains(
                "SELECT customer_id FROM ( SELECT id, customer_id, timestamp, ingested_at, properties \
                 FROM raw_events_v2 WHERE (tenant_id = ? AND code = ?"
            ),
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
            &compile_cohort_members(&reg, &cohort, &q, "raw_events_v2", "meteroid")
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
            &compile_cohort_members(
                &reg,
                &cohort,
                &base_query("bytes"),
                "raw_events_v2",
                "meteroid",
            )
            .unwrap()
            .sql,
        );
        assert!(
            sql.contains("HAVING sum(toFloat64OrZero(properties[?])) / 1024 > 5"),
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
                "meteroid",
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
                "meteroid",
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
            compile_cohort_members(&reg, &cohort, &q, "raw_events_v2", "meteroid")
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
            &compile_cohort_members(&reg, &cohort, &q, "raw_events_v2", "meteroid")
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
        let out = compile_cohort_members(&reg, &cohort, &q, "raw_events_v2", "meteroid").unwrap();
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
            compile_cohort_members(&reg, &cohort, &q, "raw_events_v2", "meteroid").unwrap_err(),
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

    fn sum_series_where() -> String {
        format!(
            "SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2 \
             WHERE (tenant_id = ? AND code = ? AND timestamp >= toDateTime(?) \
             AND timestamp <= toDateTime(?) \
             AND properties[?] != '' AND isNotNull(toFloat64OrNull(properties[?])){}",
            live(USAGE_DAY)
        )
    }

    #[test]
    fn keep_last_tenant_keeps_the_last_copy_ingested() {
        let sql = sum_series_sql(true);
        assert!(
            sql.contains(&format!(
                "FROM ( {} ORDER BY ingested_at DESC LIMIT 1 BY id, customer_id )",
                sum_series_where()
            )),
            "{sql}"
        );
    }

    #[test]
    fn default_tenant_reads_without_a_sort() {
        let sql = sum_series_sql(false);
        assert!(
            sql.contains(&format!("FROM ( {} ) GROUP BY", sum_series_where())),
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
            normalize(&out.sql).contains(&format!(
                "FROM ( SELECT id, customer_id, tenant_id, timestamp, properties FROM raw_events_v2 \
                 WHERE (tenant_id = ? AND code = ?{} ) ), instance_periods AS",
                live(" AND code IN ('compute')")
            )),
            "{}",
            out.sql
        );
    }

    #[test]
    fn time_weighted_keep_last_dedups_the_tenant_events_before_the_window_function() {
        let out = uptime_sql(true);
        let sql = normalize(&out.sql);
        assert!(
            sql.contains(&format!(
                "FROM ( SELECT id, customer_id, tenant_id, timestamp, properties FROM raw_events_v2 \
                 WHERE (tenant_id = ? AND code = ?{} \
                 ORDER BY ingested_at DESC LIMIT 1 BY id, customer_id ) ), instance_periods AS",
                live(" AND code IN ('compute')")
            )),
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
        let out = compile_cohort_members(&reg, &cohort, &q, "raw_events_v2", "meteroid").unwrap();
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
            default.contains(
                "FROM ( SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2 \
                 WHERE (tenant_id = ? AND code = ?"
            ),
            "{default}"
        );
        let narrowed = format!(
            "(SELECT count() {} AND code IN ('usage') AND",
            voids_of_tenant()
        );
        assert!(default.contains(&narrowed), "{default}");
        assert!(!default.contains("LIMIT 1 BY"), "{default}");

        let keep_last = cohort_members_sql(predicate, true);
        assert!(
            keep_last.contains(
                "FROM ( SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2 \
                 WHERE (tenant_id = ? AND code = ?"
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
