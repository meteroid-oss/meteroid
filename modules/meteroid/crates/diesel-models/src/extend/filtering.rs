//! Dynamic, typed list-filter predicates.
//!
//! The store resolves API filters (validating the field/operator against custom-property
//! definitions and coercing values) into plain-data enums here; this module renders each into a
//! boxed diesel predicate that folds into a boxed list query. Custom properties are matched on the
//! entity's `custom_properties` JSONB column: equality/membership use `@>` containment
//! (GIN-indexed), comparisons use `->>` casts. All untrusted *values* are bound; only definition
//! `key`s — already constrained to `[A-Za-z0-9_]` at creation — are interpolated, and re-checked
//! here defensively.
//!
//! Both renderers are generic over the query source `QS` (a table or a join), since their
//! predicates are raw SQL fragments that appear on any source. Native-column predicates
//! ([`NativeFilter`]) name their column with a trusted, static SQL expression chosen by the store's
//! per-entity field registry; values are always bound (as text, cast to the column type).

use diesel::dsl::sql;
use diesel::pg::Pg;
use diesel::sql_types::{Array, Bool, Double, Jsonb, Text};
use diesel::{BoolExpressionMethods, BoxableExpression};

/// A boxed boolean predicate over query source `QS`.
pub type BoxedExpr<QS> = Box<dyn BoxableExpression<QS, Pg, SqlType = Bool>>;

/// Comparison operator for scalar (`->>`) custom-property predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
}

impl CmpOp {
    fn as_sql(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "<>",
            CmpOp::Gt => ">",
            CmpOp::Gte => ">=",
            CmpOp::Lt => "<",
            CmpOp::Lte => "<=",
        }
    }
}

/// A resolved predicate over one custom property. `Contains*`/`NotContains` carry the full
/// `{key: value}` JSONB object (so the renderer needs no key for those); the `->>`-based variants
/// carry the bare key in [`CpFilter::key`].
#[derive(Debug, Clone)]
pub enum CpKind {
    Contains(serde_json::Value),
    ContainsAny(Vec<serde_json::Value>),
    NotContains(serde_json::Value),
    NumCmp(CmpOp, f64),
    NumIn(Vec<f64>),
    NumBetween(f64, f64),
    LexCmp(CmpOp, String),
    LexBetween(String, String),
    /// Case-insensitive substring; the pattern is already `%`-wrapped.
    Ilike(String),
    KeyExists,
    KeyEmpty,
}

#[derive(Debug, Clone)]
pub struct CpFilter {
    pub key: String,
    pub kind: CpKind,
}

/// SQL type a native column is compared as. Values are bound as text and cast to it, so one
/// renderer serves every column and query source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCast {
    Text,
    Date,
    Timestamp,
    Timestamptz,
    Uuid,
}

impl NativeCast {
    fn as_sql(self) -> &'static str {
        match self {
            NativeCast::Text => "text",
            NativeCast::Date => "date",
            NativeCast::Timestamp => "timestamp",
            NativeCast::Timestamptz => "timestamptz",
            NativeCast::Uuid => "uuid",
        }
    }
}

/// A resolved predicate over one native column.
#[derive(Debug, Clone)]
pub enum NativeKind {
    /// `col = ANY(values)`, or `col <> ALL(values)` when negated (NULLs never match).
    In {
        values: Vec<String>,
        negated: bool,
    },
    Cmp(CmpOp, String),
    Between(String, String),
    /// Non-null (and non-empty, for text).
    IsSet,
    /// Null (or empty, for text).
    IsEmpty,
    /// A minor-unit amount column compared against a major-unit decimal value, scaled per row by
    /// `scale_sql` — a trusted SQL expression yielding the row currency's precision (e.g. a
    /// `CASE currency WHEN 'JPY' THEN 0 … ELSE 2 END` built from the static ISO table).
    MoneyCmp {
        op: CmpOp,
        value: String,
        scale_sql: String,
    },
    MoneyBetween {
        lo: String,
        hi: String,
        scale_sql: String,
    },
    /// `col IN (<subquery_sql> ANY($values))`, e.g. plan versions of the given plans.
    /// `subquery_sql` is a trusted static prefix ending right before the `ANY(...)`.
    InSubquery {
        subquery_sql: &'static str,
        values: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct NativeFilter {
    /// Trusted SQL expression for the column (e.g. `"invoice.due_at"`), from a static registry.
    pub column: &'static str,
    pub cast: NativeCast,
    pub kind: NativeKind,
}

/// A resolved list filter: a custom property or a native column.
#[derive(Debug, Clone)]
pub enum ListFilter {
    CustomProperty(CpFilter),
    Native(NativeFilter),
}

fn key_is_safe(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A predicate that matches nothing — used when a filter resolves to an empty set (e.g. `IN []`)
/// or an unexpectedly unsafe key.
fn never<QS: 'static>() -> BoxedExpr<QS> {
    Box::new(sql::<Bool>("false"))
}

fn contains<QS: 'static>(col: &str, obj: serde_json::Value) -> BoxedExpr<QS> {
    Box::new(sql::<Bool>(&format!("{col} @> ")).bind::<Jsonb, _>(obj))
}

/// Render a resolved custom-property filter against the `custom_properties` JSONB column named by
/// `col` (e.g. `"customer.custom_properties"`), boxed for query source `QS`.
pub fn render_cp<QS: 'static>(filter: CpFilter, col: &str) -> BoxedExpr<QS> {
    let CpFilter { key, kind } = filter;

    // Containment variants embed the key inside the bound JSONB, so they don't interpolate it.
    // The `->>` variants do — guard the key even though definitions already constrain it.
    let needs_key = !matches!(
        kind,
        CpKind::Contains(_) | CpKind::ContainsAny(_) | CpKind::NotContains(_)
    );
    if needs_key && !key_is_safe(&key) {
        return never();
    }

    match kind {
        CpKind::Contains(obj) => contains(col, obj),
        CpKind::ContainsAny(objs) => {
            let mut it = objs.into_iter();
            match it.next() {
                None => never(),
                Some(first) => it.fold(contains(col, first), |acc, obj| {
                    Box::new(acc.or(contains(col, obj)))
                }),
            }
        }
        CpKind::NotContains(obj) => Box::new(
            sql::<Bool>(&format!("NOT ({col} @> "))
                .bind::<Jsonb, _>(obj)
                .sql(")"),
        ),
        CpKind::NumCmp(op, n) => Box::new(
            sql::<Bool>(&format!(
                "({col} ->> '{key}')::double precision {} ",
                op.as_sql()
            ))
            .bind::<Double, _>(n),
        ),
        CpKind::NumIn(ns) => {
            let mut it = ns.into_iter();
            let col = col.to_string();
            let one = move |n: f64| -> BoxedExpr<QS> {
                Box::new(
                    sql::<Bool>(&format!("({col} ->> '{key}')::double precision = "))
                        .bind::<Double, _>(n),
                )
            };
            match it.next() {
                None => never(),
                Some(first) => it.fold(one(first), |acc, n| Box::new(acc.or(one(n)))),
            }
        }
        CpKind::NumBetween(lo, hi) => Box::new(
            sql::<Bool>(&format!("({col} ->> '{key}')::double precision >= "))
                .bind::<Double, _>(lo)
                .sql(&format!(" AND ({col} ->> '{key}')::double precision <= "))
                .bind::<Double, _>(hi),
        ),
        CpKind::LexCmp(op, s) => Box::new(
            sql::<Bool>(&format!("({col} ->> '{key}') {} ", op.as_sql())).bind::<Text, _>(s),
        ),
        CpKind::LexBetween(lo, hi) => Box::new(
            sql::<Bool>(&format!("({col} ->> '{key}') >= "))
                .bind::<Text, _>(lo)
                .sql(&format!(" AND ({col} ->> '{key}') <= "))
                .bind::<Text, _>(hi),
        ),
        CpKind::Ilike(pattern) => {
            Box::new(sql::<Bool>(&format!("({col} ->> '{key}') ILIKE ")).bind::<Text, _>(pattern))
        }
        CpKind::KeyExists => Box::new(sql::<Bool>(&format!("jsonb_exists({col}, '{key}')"))),
        CpKind::KeyEmpty => Box::new(sql::<Bool>(&format!(
            "(NOT jsonb_exists({col}, '{key}') \
             OR {col} -> '{key}' = 'null'::jsonb \
             OR {col} ->> '{key}' = '')"
        ))),
    }
}

fn money_expr(value_bind_sql: &str, scale_sql: &str) -> String {
    format!("(CAST({value_bind_sql} AS numeric) * power(10::numeric, {scale_sql}))")
}

/// Render a resolved native-column filter, boxed for query source `QS`.
pub fn render_native<QS: 'static>(filter: NativeFilter) -> BoxedExpr<QS> {
    let NativeFilter { column, cast, kind } = filter;
    let ty = cast.as_sql();
    match kind {
        NativeKind::In { values, negated } => {
            if values.is_empty() {
                return never();
            }
            let op = if negated { "<> ALL" } else { "= ANY" };
            Box::new(
                sql::<Bool>(&format!("{column} {op}(CAST("))
                    .bind::<Array<Text>, _>(values)
                    .sql(&format!(" AS {ty}[]))")),
            )
        }
        NativeKind::Cmp(op, v) => Box::new(
            sql::<Bool>(&format!("{column} {} CAST(", op.as_sql()))
                .bind::<Text, _>(v)
                .sql(&format!(" AS {ty})")),
        ),
        NativeKind::Between(lo, hi) => Box::new(
            sql::<Bool>(&format!("{column} >= CAST("))
                .bind::<Text, _>(lo)
                .sql(&format!(" AS {ty}) AND {column} <= CAST("))
                .bind::<Text, _>(hi)
                .sql(&format!(" AS {ty})")),
        ),
        NativeKind::IsSet => Box::new(sql::<Bool>(&if cast == NativeCast::Text {
            format!("({column} IS NOT NULL AND {column} <> '')")
        } else {
            format!("{column} IS NOT NULL")
        })),
        NativeKind::IsEmpty => Box::new(sql::<Bool>(&if cast == NativeCast::Text {
            format!("({column} IS NULL OR {column} = '')")
        } else {
            format!("{column} IS NULL")
        })),
        NativeKind::MoneyCmp {
            op,
            value,
            scale_sql,
        } => {
            let (head, tail) = split_bind(&money_expr("$", &scale_sql));
            Box::new(
                sql::<Bool>(&format!("{column} {} {head}", op.as_sql()))
                    .bind::<Text, _>(value)
                    .sql(&tail),
            )
        }
        NativeKind::MoneyBetween { lo, hi, scale_sql } => {
            let (head, tail) = split_bind(&money_expr("$", &scale_sql));
            Box::new(
                sql::<Bool>(&format!("{column} >= {head}"))
                    .bind::<Text, _>(lo)
                    .sql(&format!("{tail} AND {column} <= {head}"))
                    .bind::<Text, _>(hi)
                    .sql(&tail),
            )
        }
        NativeKind::InSubquery {
            subquery_sql,
            values,
        } => {
            if values.is_empty() {
                return never();
            }
            Box::new(
                sql::<Bool>(&format!("{column} IN ({subquery_sql} ANY(CAST("))
                    .bind::<Array<Text>, _>(values)
                    .sql(" AS uuid[])))"),
            )
        }
    }
}

/// Split a SQL template around its single `$` placeholder (where a value gets bound).
fn split_bind(template: &str) -> (String, String) {
    let (head, tail) = template
        .split_once('$')
        .expect("template has a bind placeholder");
    (head.to_string(), tail.to_string())
}

/// Render one resolved list filter; custom properties are read from the JSONB column `cp_col`
/// (e.g. `"invoice.custom_properties"`).
pub fn render_list_filter<QS: 'static>(filter: ListFilter, cp_col: &str) -> BoxedExpr<QS> {
    match filter {
        ListFilter::CustomProperty(f) => render_cp(f, cp_col),
        ListFilter::Native(f) => render_native(f),
    }
}
