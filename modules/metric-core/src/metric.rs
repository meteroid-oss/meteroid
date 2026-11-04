//! Engine-agnostic semantic metric model.

use crate::domain::WindowSize;
use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateOp {
    Sum,
    Avg,
    Min,
    Max,
    Count,
    CountDistinct,
    Latest,
}

/// How a cohort *threshold* metric reduces its fact rows over the evaluation window.
///
/// A **flow** sums over `[from, to]` (net-new MRR, revenue booked in the period). A
/// **stock** is a level as-of `to`: the compiler drops the `>= from` lower bound so a
/// `Sum` becomes the running total of deltas through `to` (the MRR level) and a
/// `Latest`/`argMax` picks the newest daily snapshot on-or-before `to` (the AR level).
/// Only read by the cohort predicate compiler; inert for windowed series queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CohortReduction {
    /// Sum/aggregate over `[from, to]` — a flow.
    #[default]
    Windowed,
    /// Reduce to the level as-of `to` — a stock (drops the lower time bound).
    AsOf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregateSpec {
    pub op: AggregateOp,
    /// `None` is only valid for `Count`.
    pub value_property: Option<String>,
    /// Stock-vs-flow reduction for the cohort-threshold predicate path; see
    /// [`CohortReduction`]. Inert for windowed series queries.
    #[serde(default)]
    pub cohort_reduction: CohortReduction,
    /// Billable-metric unit-conversion divisor: the per-window/segment aggregate is
    /// divided by this factor, mirroring the billing path (invoice line computation).
    /// `None` (or ≤ 1) leaves the aggregate untouched. Only honoured on the windowed
    /// series path (raw-event custom metrics); the cohort/delta paths ignore it.
    #[serde(default)]
    pub unit_conversion_factor: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeWeight {
    Seconds,
    ValueSeconds(String),
}

/// Time-weighted "liveness" integral: per-entity events define intervals, summed per window as weighted overlap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeWeightedSpec {
    pub identity_property: String,
    /// Lifecycle event codes; empty means all events for this metric's `event_code`.
    pub lifecycle_codes: Vec<String>,
    pub weight: TimeWeight,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GaugeOp {
    Latest,
    Max,
    TimeWeighted(TimeWeightedSpec),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GaugeSpec {
    pub op: GaugeOp,
    /// Ignored for `TimeWeighted`.
    pub value_property: Option<String>,
    /// When `true`, a window with no event inherits the prior window's gauge level (a persisting
    /// balance) instead of being null/zero. Ignored for `TimeWeighted`. Default keeps old behaviour.
    #[serde(default)]
    pub carry_over: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnaryOp {
    Neg,
    Abs,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expr {
    Metric(String),
    Const(f64),
    Binary(Box<Expr>, BinOp, Box<Expr>),
    Unary(UnaryOp, Box<Expr>),
    Clamp {
        expr: Box<Expr>,
        min: Option<Box<Expr>>,
        max: Option<Box<Expr>>,
    },
    Coalesce(Vec<Expr>),
    Greatest(Vec<Expr>),
    Least(Vec<Expr>),
    /// A leaf operand that references a metric whose **sub-series** carries its own
    /// per-operand [`CrossPeriod`] transform, applied before the derived join.
    ///
    /// `Expr` stays a pure per-window scalar algebra (DESIGN §3.1): the transform is
    /// not a scalar op — it is a directive to the compiler to build *this operand's*
    /// `m_<key>` CTE as a windowed sub-series (e.g. a lagged running total = *opening*
    /// balance) rather than the default within-window value. This is the mechanism that
    /// unlocks server-side retention ratios (flow ÷ opening MRR; DESIGN §6): one operand
    /// composes as a lagged cumulative, another as a within-window sum, then divide.
    MetricTransformed {
        metric: String,
        transform: CrossPeriod,
    },
}

impl Expr {
    pub fn referenced_metrics(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_refs(&mut out);
        out
    }

    fn collect_refs(&self, out: &mut Vec<String>) {
        match self {
            Expr::Metric(k) => out.push(k.clone()),
            Expr::MetricTransformed { metric, .. } => out.push(metric.clone()),
            Expr::Const(_) => {}
            Expr::Binary(l, _, r) => {
                l.collect_refs(out);
                r.collect_refs(out);
            }
            Expr::Unary(_, e) => e.collect_refs(out),
            Expr::Clamp { expr, min, max } => {
                expr.collect_refs(out);
                if let Some(min) = min {
                    min.collect_refs(out);
                }
                if let Some(max) = max {
                    max.collect_refs(out);
                }
            }
            Expr::Coalesce(items) | Expr::Greatest(items) | Expr::Least(items) => {
                for item in items {
                    item.collect_refs(out);
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    Aggregate(AggregateSpec),
    Gauge(GaugeSpec),
    Derived(Expr),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MetricSource {
    #[default]
    RawEvents,
    Fact(FactSource),
}

/// Column/table names are SQL identifiers (inlined, then allowlist-validated).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactSource {
    pub table: String,
    pub value_column: String,
    #[serde(default = "default_ts_col")]
    pub timestamp_column: String,
    #[serde(default = "default_customer_col")]
    pub customer_column: String,
    #[serde(default = "default_tenant_col")]
    pub tenant_column: String,
    /// A point-in-time daily **snapshot** (a *stock*): every `(entity, day)` row is a
    /// standing balance, so a bucket wider than one day must reduce to its LATEST day's
    /// value, not sum every day (which inflates the level ~N× — the AR-aging bug). Flow
    /// facts (movements, revenue, payments) leave this `false` and sum over the window.
    /// The windowed-series compiler honours it ([`crate::sql::compiler`]); the cohort
    /// path reduces stocks separately via [`CohortReduction::AsOf`].
    #[serde(default)]
    pub snapshot: bool,
}

fn default_ts_col() -> String {
    "at".to_string()
}
fn default_customer_col() -> String {
    "customer_id".to_string()
}
fn default_tenant_col() -> String {
    "tenant_id".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricDef {
    pub key: String,
    /// `raw_events.code` this metric reads. Unused for derived and fact sources.
    pub event_code: String,
    pub kind: MetricKind,
    #[serde(default)]
    pub source: MetricSource,
    /// Per-metric dimension filters, ANDed with the request-level filters into *this metric's*
    /// own read (a sumIf-style isolation). The retention enabler from DESIGN §6: it lets one
    /// derived query isolate several movement components (churn, contraction, …) as separate
    /// sub-series while leaving the opening-balance operand unfiltered. Ignored by derived
    /// metrics (they have no direct source). Empty = no per-metric filter.
    #[serde(default)]
    pub filters: Vec<DimensionFilter>,
}

impl MetricDef {
    pub fn aggregate(
        key: &str,
        event_code: &str,
        op: AggregateOp,
        value_property: Option<&str>,
    ) -> Self {
        MetricDef {
            key: key.to_string(),
            event_code: event_code.to_string(),
            kind: MetricKind::Aggregate(AggregateSpec {
                op,
                value_property: value_property.map(str::to_string),
                cohort_reduction: CohortReduction::Windowed,
                unit_conversion_factor: None,
            }),
            source: MetricSource::RawEvents,
            filters: Vec::new(),
        }
    }

    pub fn fact_aggregate(key: &str, op: AggregateOp, source: FactSource) -> Self {
        MetricDef {
            key: key.to_string(),
            event_code: String::new(),
            kind: MetricKind::Aggregate(AggregateSpec {
                op,
                value_property: Some(source.value_column.clone()),
                cohort_reduction: CohortReduction::Windowed,
                unit_conversion_factor: None,
            }),
            source: MetricSource::Fact(source),
            filters: Vec::new(),
        }
    }

    /// Builder: attach a per-metric [`filters`](Self::filters) `In` dimension filter.
    pub fn with_filter(mut self, property: &str, values: &[&str]) -> Self {
        self.filters.push(DimensionFilter {
            property: property.to_string(),
            values: values.iter().map(|v| v.to_string()).collect(),
            op: FilterOp::In,
        });
        self
    }

    /// Builder: divide the aggregate by a unit-conversion factor (see
    /// [`AggregateSpec::unit_conversion_factor`]). Only a finite factor > 1 is applied;
    /// anything else (`None`, ≤ 1, non-finite) leaves the aggregate untouched, so the
    /// emitted SQL stays byte-identical to the unconverted metric. A no-op on
    /// non-aggregate metrics.
    pub fn with_unit_conversion_factor(mut self, factor: Option<f64>) -> Self {
        if let MetricKind::Aggregate(spec) = &mut self.kind {
            spec.unit_conversion_factor = factor.filter(|f| f.is_finite() && *f > 1.0);
        }
        self
    }

    /// Builder: mark this metric as a **stock** for cohort thresholds — its aggregate
    /// reduces to the level as-of `to` (running total / latest snapshot) rather than a
    /// windowed sum (see [`CohortReduction`]). A no-op on non-aggregate metrics.
    pub fn as_of(mut self) -> Self {
        if let MetricKind::Aggregate(spec) = &mut self.kind {
            spec.cohort_reduction = CohortReduction::AsOf;
        }
        self
    }

    pub fn derived(key: &str, expr: Expr) -> Self {
        MetricDef {
            key: key.to_string(),
            event_code: String::new(),
            kind: MetricKind::Derived(expr),
            source: MetricSource::RawEvents,
            filters: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MetricRegistry {
    metrics: HashMap<String, MetricDef>,
}

impl MetricRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, def: MetricDef) {
        self.metrics.insert(def.key.clone(), def);
    }

    pub fn with(mut self, def: MetricDef) -> Self {
        self.insert(def);
        self
    }

    pub fn get(&self, key: &str) -> Option<&MetricDef> {
        self.metrics.get(key)
    }

    /// Every referenced metric must exist and the derived dependency graph must be acyclic.
    pub fn validate(&self) -> Result<(), MetricError> {
        for def in self.metrics.values() {
            if let MetricKind::Derived(expr) = &def.kind {
                for r in expr.referenced_metrics() {
                    if !self.metrics.contains_key(&r) {
                        return Err(MetricError::UnknownMetric(r));
                    }
                }
                let mut stack = vec![def.key.clone()];
                self.check_acyclic(&def.key, &mut stack)?;
            }
        }
        Ok(())
    }

    fn check_acyclic(&self, current: &str, stack: &mut Vec<String>) -> Result<(), MetricError> {
        let Some(def) = self.metrics.get(current) else {
            return Ok(());
        };
        if let MetricKind::Derived(expr) = &def.kind {
            for r in expr.referenced_metrics() {
                if stack.contains(&r) {
                    return Err(MetricError::Cycle(r));
                }
                stack.push(r.clone());
                self.check_acyclic(&r, stack)?;
                stack.pop();
            }
        }
        Ok(())
    }
}

/// Match mode of a [`DimensionFilter`]. `In` keeps rows whose property is one of
/// `values`; `NotIn` keeps rows whose property is **present** and is none of `values`
/// (an absent property is excluded — presence-required negation, so an event that never
/// carries the property can't leak into a "not in" result). UI `equal`/`not_equal`
/// collapse to `In`/`NotIn` over a single value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    #[default]
    In,
    NotIn,
}

/// OR within a filter, AND across filters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DimensionFilter {
    pub property: String,
    pub values: Vec<String>,
    /// `#[serde(default)]` → legacy filters without the field deserialize as [`FilterOp::In`],
    /// preserving the historical include-only semantics byte-for-byte.
    #[serde(default)]
    pub op: FilterOp,
}

/// A dimension to slice a metric series by. It resolves to one of three SQL shapes
/// so a metric can be grouped by *any* property — like a general data explorer —
/// while staying safe: every open-ended name is bound as a value or allowlist-
/// validated by the compiler, never interpolated as a raw SQL identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dimension {
    /// Source default: a native column on a fact source, or an event property on
    /// raw events. Preserves the historical bare-string behaviour.
    Auto(String),
    /// An explicit native column on the source table.
    Column(String),
    /// A dynamic event property read from the `properties` map (usage metrics only).
    Property(String),
    /// A customer attribute (country, plan tier, custom metadata) resolved by
    /// customer id through the `dim_customer` dictionary at query time.
    CustomerAttr(String),
    /// A plan attribute (name) resolved by plan id through the `dim_plan`
    /// dictionary at query time — turns the raw `plan_id` split into a readable label.
    PlanAttr(String),
    /// A key inside the customer `attributes` map, resolved by customer id through the
    /// `dim_customer` dictionary, which projects the map as a JSON string:
    /// `JSONExtractString(dictGet(dict,'attributes',id), map_key)`. Unlike [`CustomerAttr`]
    /// (a top-level dict column), this reads the map — the home of the derived
    /// `lifecycle_state` and synced CRM properties. `map_key` is the attribute read from
    /// the map (allowlist-validated); `alias` is the output dimension name the caller
    /// looks results up by, which can differ from the stored key (e.g. read
    /// `lifecycle_state`, expose it as the `customer_type` breakdown).
    CustomerAttrMap { map_key: String, alias: String },
    /// A product attribute (name) resolved by product id through the `dim_product`
    /// dictionary at query time — turns the raw `product_id` split into a readable
    /// label. Only valid on fact sources that carry `product_id` (the revenue items).
    ProductAttr(String),
    /// A per-customer split: the series is one line per individual customer, grouped
    /// by `customer_id` and labelled with the customer name via the `dim_customer`
    /// dictionary. Like [`PlanAttr`]/[`ProductAttr`] it keys on the id (distinct
    /// same-named customers stay distinct) with the name as a display label.
    CustomerName(String),
    /// A tenant-defined customer custom property, read as text from the `dim_customer`
    /// dictionary's `custom_properties` JSON by customer id. `key` is the property key
    /// (bound, never inlined); `alias` is the output dimension name.
    CustomerProperty { key: String, alias: String },
}

impl Dimension {
    /// The human dimension key — what the API echoes back as the breakdown name and
    /// what the result column maps to.
    pub fn name(&self) -> &str {
        match self {
            Dimension::Auto(s)
            | Dimension::Column(s)
            | Dimension::Property(s)
            | Dimension::CustomerAttr(s)
            | Dimension::PlanAttr(s)
            | Dimension::ProductAttr(s)
            | Dimension::CustomerName(s) => s,
            Dimension::CustomerAttrMap { alias, .. }
            | Dimension::CustomerProperty { alias, .. } => alias,
        }
    }

    /// An "entity" dimension groups on an id (plan/product/customer) with a
    /// dictionary-resolved name as the display label, so two entities sharing a name
    /// stay distinct series. The compiler groups these by id and emits a
    /// disambiguator column; non-entity dimensions key on their own value.
    pub fn is_entity(&self) -> bool {
        matches!(
            self,
            Dimension::PlanAttr(_) | Dimension::ProductAttr(_) | Dimension::CustomerName(_)
        )
    }
}

impl From<String> for Dimension {
    fn from(s: String) -> Self {
        Dimension::Auto(s)
    }
}

impl From<&str> for Dimension {
    fn from(s: &str) -> Self {
        Dimension::Auto(s.to_string())
    }
}

// Wire form: a bare string is the `Auto` default (back-compat with the historical
// `dimensions: ["plan"]` shape); a single-key object names an explicit kind, e.g.
// `{"customer_attr": "country"}` or `{"property": "region"}`.
impl Serialize for Dimension {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        // The map variant carries two fields, so it serializes to a nested object
        // `{"customer_attr_map": {"map_key": …, "alias": …}}` rather than the
        // single-string shape the other variants share.
        if let Dimension::CustomerAttrMap { map_key, alias } = self {
            let mut m = s.serialize_map(Some(1))?;
            m.serialize_entry(
                "customer_attr_map",
                &serde_json::json!({ "map_key": map_key, "alias": alias }),
            )?;
            return m.end();
        }
        if let Dimension::CustomerProperty { key, alias } = self {
            let mut m = s.serialize_map(Some(1))?;
            m.serialize_entry(
                "customer_property",
                &serde_json::json!({ "key": key, "alias": alias }),
            )?;
            return m.end();
        }
        let (key, name) = match self {
            Dimension::Auto(n) => return s.serialize_str(n),
            Dimension::Column(n) => ("column", n),
            Dimension::Property(n) => ("property", n),
            Dimension::CustomerAttr(n) => ("customer_attr", n),
            Dimension::PlanAttr(n) => ("plan_attr", n),
            Dimension::ProductAttr(n) => ("product_attr", n),
            Dimension::CustomerName(n) => ("customer_name", n),
            Dimension::CustomerAttrMap { .. } | Dimension::CustomerProperty { .. } => {
                unreachable!("handled above")
            }
        };
        let mut m = s.serialize_map(Some(1))?;
        m.serialize_entry(key, name)?;
        m.end()
    }
}

impl<'de> Deserialize<'de> for Dimension {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Tagged {
            Auto(String),
            Column(String),
            Property(String),
            CustomerAttr(String),
            PlanAttr(String),
            CustomerAttrMap { map_key: String, alias: String },
            ProductAttr(String),
            CustomerName(String),
            CustomerProperty { key: String, alias: String },
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Bare(String),
            Tagged(Tagged),
        }
        Ok(match Wire::deserialize(d)? {
            Wire::Bare(s) => Dimension::Auto(s),
            Wire::Tagged(Tagged::Auto(n)) => Dimension::Auto(n),
            Wire::Tagged(Tagged::Column(n)) => Dimension::Column(n),
            Wire::Tagged(Tagged::Property(n)) => Dimension::Property(n),
            Wire::Tagged(Tagged::CustomerAttr(n)) => Dimension::CustomerAttr(n),
            Wire::Tagged(Tagged::PlanAttr(n)) => Dimension::PlanAttr(n),
            Wire::Tagged(Tagged::CustomerAttrMap { map_key, alias }) => {
                Dimension::CustomerAttrMap { map_key, alias }
            }
            Wire::Tagged(Tagged::ProductAttr(n)) => Dimension::ProductAttr(n),
            Wire::Tagged(Tagged::CustomerName(n)) => Dimension::CustomerName(n),
            Wire::Tagged(Tagged::CustomerProperty { key, alias }) => {
                Dimension::CustomerProperty { key, alias }
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossPeriod {
    RunningTotal,
    /// Running total **through the previous window** — a lagged cumulative, i.e. the
    /// *opening* balance of each window: `sum(value) OVER (… ROWS BETWEEN UNBOUNDED
    /// PRECEDING AND 1 PRECEDING)`. Like [`RunningTotal`](Self::RunningTotal) it is
    /// primed from history before `from`, so the first displayed window has a valid
    /// opening balance. Used as a per-operand transform (see [`Expr::MetricTransformed`])
    /// to express retention ratios (flow ÷ opening MRR; DESIGN §6).
    RunningTotalLagged,
    PeriodOverPeriodDelta,
    PercentChange,
    MovingAverage {
        periods: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdOp {
    Gt,
    Gte,
    Lt,
    Lte,
}

impl ThresholdOp {
    pub fn as_sql(self) -> &'static str {
        match self {
            ThresholdOp::Gt => ">",
            ThresholdOp::Gte => ">=",
            ThresholdOp::Lt => "<",
            ThresholdOp::Lte => "<=",
        }
    }
}

/// The value family of a custom property, selecting the typed `JSONExtract*` accessor the
/// compiler emits (and, at the wire boundary, the operators that apply). The richer Postgres
/// custom-property types collapse here to the four comparison families ClickHouse needs:
/// TEXT/URL/EMAIL/SELECTs → `Text`, NUMBER → `Number`, BOOLEAN → `Boolean`, DATE/DATETIME →
/// `Date` (ISO strings, lexically ordered).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyValueType {
    Text,
    Number,
    Boolean,
    Date,
}

/// A comparison operator over a typed custom-property value. Which operators are valid for a
/// given [`PropertyValueType`] is enforced at the wire boundary (the analytics catalog), so the
/// compiler only sees type-appropriate combinations.
///
/// The serde names deliberately match the list-filter `FilterOperator` (`equals`, `not_equals`,
/// …) so the same web `FilterOp` type and operator-matrix drive both a list filter and a cohort
/// leaf — one property, one mental model, two surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyOp {
    Equals,
    NotEquals,
    In,
    Contains,
    Gt,
    Gte,
    Lt,
    Lte,
    Before,
    After,
    Between,
    IsSet,
    IsEmpty,
    IsTrue,
    IsFalse,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CohortPredicate {
    /// Metric must be non-derived.
    MetricThreshold {
        metric: String,
        op: ThresholdOp,
        threshold: f64,
    },
    /// Table/columns are inlined identifiers (allowlist-validated); `values` are bound.
    Dimension {
        table: String,
        id_column: String,
        column: String,
        values: Vec<String>,
    },
    /// Semi-join on a `Map(String,String)` column by key: `map_column[key] IN values`.
    /// `table`/`id_column`/`map_column` are inlined (validated); `key` + `values` are
    /// bound — so an arbitrary customer property is matched without inlining anything.
    DimensionMap {
        table: String,
        id_column: String,
        map_column: String,
        key: String,
        values: Vec<String>,
    },
    /// A *typed* predicate over a customer's native-JSON `custom_properties` column. Unlike the
    /// stringly-typed [`Self::DimensionMap`] leaf, `value_type` selects the `JSONExtract*` accessor,
    /// so numeric `>`/date-range comparisons are correct rather than lexical-on-stringified.
    /// `table`/`id_column`/`json_column` are inlined (allowlist-validated); `key` + `values` are bound.
    CustomProperty {
        table: String,
        id_column: String,
        json_column: String,
        key: String,
        op: PropertyOp,
        value_type: PropertyValueType,
        values: Vec<String>,
    },
    /// Customers with a currently-active subscription on one of `plan_ids`: the
    /// latest state-changing subscription event is an activation. Derived from
    /// `fact_subscription_event`; `plan_ids` are bound.
    ActivePlan { plan_ids: Vec<String> },
    /// Customers that are NOT active on `metric` over the query window: the tenant
    /// customer universe (`dim_customer`) ANTI JOIN the set with qualifying activity.
    /// The one shape the positive predicates can't express — a metric a customer never
    /// emits produces no group row, so `MetricThreshold` can't see who is missing.
    /// `metric` must be non-derived. With `max_active = None` it is pure absence ("NO
    /// usage"); with `Some(t)` the active-set is restricted to customers whose aggregate
    /// exceeds `t`, so the complement is "inactive or active ≤ t".
    MetricInactive {
        metric: String,
        #[serde(default)]
        max_active: Option<f64>,
    },
    /// Relative change of `metric`'s aggregate in the current window `[from, to]`
    /// versus the customer's OWN prior window `[from - W, from)` — the churn leading
    /// indicator the single-window absolute-threshold predicates can't express.
    /// `W = prior_window_days` days (default the current window length `to - from`, a
    /// symmetric prior). Keeps customers with `prior != 0` (a zero baseline is no
    /// signal, mirroring `stats.rs::change_pct`) AND `(cur - prior) / prior * 100`
    /// satisfying `op pct` — so "usage down ≥30%" is `op = Lte, pct = -30`. `metric`
    /// must be a non-derived aggregate (matched exactly as `MetricThreshold`/`MetricInactive`).
    MetricDelta {
        metric: String,
        op: ThresholdOp,
        pct: f64,
        #[serde(default)]
        prior_window_days: Option<u32>,
    },
    /// All children must match — set INTERSECT of their customer-id sets. An
    /// explicit nesting form; the root `CohortDef.predicates` list is already an
    /// implicit AND, so this is only needed inside an `Or`/`Not`.
    And { children: Vec<CohortPredicate> },
    /// Any child matches — set UNION of their customer-id sets.
    Or { children: Vec<CohortPredicate> },
    /// Customers NOT matching the child, within the tenant customer universe
    /// (`dim_customer` EXCEPT the child set), mirroring `MetricInactive`'s anti-join.
    Not { child: Box<CohortPredicate> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CohortDef {
    pub predicates: Vec<CohortPredicate>,
}

#[derive(Debug, Clone)]
pub struct MetricQuery {
    pub tenant_id: TenantId,
    pub metric: String,
    pub from: DateTime<Utc>,
    pub to: Option<DateTime<Utc>>,
    pub window: Option<WindowSize>,
    pub window_time_zone: Option<chrono_tz::Tz>,
    pub group_by: Vec<Dimension>,
    pub customer_ids: Vec<CustomerId>,
    pub filters: Vec<DimensionFilter>,
    pub transform: Option<CrossPeriod>,
    pub cohort: Option<CohortDef>,
    /// ISO code; when set and not USD, `*_cents_usd` columns are converted per-row at the fact date's historical rate via ASOF join.
    pub reporting_currency: Option<String>,
    /// The tenant's `keep_last_events`: raw-event reads keep the last copy ingested of an id.
    pub keep_last_events: bool,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MetricError {
    #[error("unknown metric: {0}")]
    UnknownMetric(String),
    #[error("cyclic derived metric reference: {0}")]
    Cycle(String),
    #[error("invalid metric {0}: {1}")]
    Invalid(String, String),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn referenced_metrics_walks_expression() {
        let expr = Expr::Binary(
            Box::new(Expr::Metric("revenue".into())),
            BinOp::Div,
            Box::new(Expr::Binary(
                Box::new(Expr::Metric("users".into())),
                BinOp::Add,
                Box::new(Expr::Const(1.0)),
            )),
        );
        assert_eq!(expr.referenced_metrics(), vec!["revenue", "users"]);
    }

    #[test]
    fn referenced_metrics_walks_new_variants() {
        // Unary, Clamp (with both bounds), Coalesce, Greatest and Least must all be traversed.
        let expr = Expr::Coalesce(vec![
            Expr::Unary(UnaryOp::Abs, Box::new(Expr::Metric("a".into()))),
            Expr::Clamp {
                expr: Box::new(Expr::Metric("b".into())),
                min: Some(Box::new(Expr::Metric("lo".into()))),
                max: Some(Box::new(Expr::Metric("hi".into()))),
            },
            Expr::Greatest(vec![
                Expr::Metric("c".into()),
                Expr::Least(vec![Expr::Metric("d".into()), Expr::Const(0.0)]),
            ]),
        ]);
        assert_eq!(
            expr.referenced_metrics(),
            vec!["a", "b", "lo", "hi", "c", "d"]
        );
    }

    #[test]
    fn validate_detects_unknown_reference_through_new_variants() {
        let reg = MetricRegistry::new()
            .with(MetricDef::aggregate(
                "a",
                "evt",
                AggregateOp::Sum,
                Some("v"),
            ))
            .with(MetricDef::derived(
                "clamped",
                Expr::Clamp {
                    expr: Box::new(Expr::Metric("a".into())),
                    min: Some(Box::new(Expr::Metric("missing".into()))),
                    max: None,
                },
            ));
        assert_eq!(
            reg.validate(),
            Err(MetricError::UnknownMetric("missing".into()))
        );
    }

    #[test]
    fn validate_detects_cycle_through_new_variants() {
        let reg = MetricRegistry::new()
            .with(MetricDef::derived(
                "a",
                Expr::Unary(UnaryOp::Neg, Box::new(Expr::Metric("b".into()))),
            ))
            .with(MetricDef::derived(
                "b",
                Expr::Greatest(vec![Expr::Metric("a".into()), Expr::Const(1.0)]),
            ));
        assert!(matches!(reg.validate(), Err(MetricError::Cycle(_))));
    }

    #[test]
    fn gauge_spec_carry_over_defaults_false() {
        // `carry_over` is `#[serde(default)]`, so legacy JSON without the field still deserializes.
        let spec: GaugeSpec =
            serde_json::from_str(r#"{"op":"latest","value_property":"balance"}"#).unwrap();
        assert!(!spec.carry_over);
    }

    #[test]
    fn validate_detects_unknown_reference() {
        let reg = MetricRegistry::new()
            .with(MetricDef::aggregate(
                "a",
                "evt",
                AggregateOp::Sum,
                Some("v"),
            ))
            .with(MetricDef::derived(
                "ratio",
                Expr::Binary(
                    Box::new(Expr::Metric("a".into())),
                    BinOp::Div,
                    Box::new(Expr::Metric("missing".into())),
                ),
            ));
        assert_eq!(
            reg.validate(),
            Err(MetricError::UnknownMetric("missing".into()))
        );
    }

    #[test]
    fn validate_detects_cycle() {
        let reg = MetricRegistry::new()
            .with(MetricDef::derived("a", Expr::Metric("b".into())))
            .with(MetricDef::derived("b", Expr::Metric("a".into())));
        assert!(matches!(reg.validate(), Err(MetricError::Cycle(_))));
    }

    #[test]
    fn validate_accepts_acyclic_graph() {
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
        assert_eq!(reg.validate(), Ok(()));
    }
}
