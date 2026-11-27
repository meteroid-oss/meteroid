//! The typed SQL tree the metric compiler builds. Binds live in the tree, identifiers are
//! validated [`Ident`]s, and every table read declares its tenant scope ([`Table`]); ClickHouse
//! constructs (`FINAL`, `ASOF`, `WITH FILL`, dictionaries, window frames) are nodes.

use super::{BindValue, EventDedup, VoidScope};
use crate::metric::MetricError;

/// A SQL identifier inlined into the query text. Built only from a trusted constant, a
/// validated name or a sanitized one, so it can never carry SQL.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ident(String);

impl Ident {
    /// A compile-time constant of the compiler itself.
    pub fn trusted(name: &'static str) -> Ident {
        debug_assert!(
            name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "{name}"
        );
        Ident(name.to_string())
    }

    /// A user-chosen column name, inlined bare: `[A-Za-z_][A-Za-z0-9_]*`.
    pub fn column(name: &str) -> Result<Ident, MetricError> {
        let mut chars = name.chars();
        let bare = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        literal_key(name)?;
        if bare {
            Ok(Ident(name.to_string()))
        } else {
            Err(MetricError::Invalid(
                name.to_string(),
                "property name contains characters not allowed in an inlined identifier"
                    .to_string(),
            ))
        }
    }

    /// An unqualified table name: a client-supplied source can never reach `system.tables`.
    pub fn table(name: &str) -> Result<Ident, MetricError> {
        let mut chars = name.chars();
        let valid = name.len() <= 128
            && matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if valid {
            Ok(Ident(name.to_string()))
        } else {
            Err(MetricError::Invalid(
                name.to_string(),
                "table name must be an unqualified identifier (no schema, no '.')".to_string(),
            ))
        }
    }

    /// An alias derived from any name by [`super::sanitize_identifier`].
    pub fn sanitized(name: &str) -> Ident {
        Ident(super::sanitize_identifier(name))
    }

    /// An alias built by [`super::sanitize_identifier`], carried as text.
    pub(crate) fn from_alias(alias: String) -> Ident {
        debug_assert!(
            alias.chars().all(|c| c.is_alphanumeric() || c == '_'),
            "{alias}"
        );
        Ident(alias)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A user-chosen name inlined inside a string literal (a map key, a dictionary attribute): a
/// conservative charset that can't break out of it.
pub fn literal_key(name: &str) -> Result<&str, MetricError> {
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
        Ok(name)
    } else {
        Err(MetricError::Invalid(
            name.to_string(),
            "property name contains characters not allowed in an inlined identifier".to_string(),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Like,
    ILike,
}

impl Op {
    pub fn as_sql(self) -> &'static str {
        match self {
            Op::Eq => "=",
            Op::Ne => "!=",
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::Like => "LIKE",
            Op::ILike => "ILIKE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameBound {
    UnboundedPreceding,
    Preceding(u32),
    CurrentRow,
    UnboundedFollowing,
}

/// `OVER ([PARTITION BY …] ORDER BY … ROWS BETWEEN start AND end)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub partition_by: Vec<Expr>,
    pub order_by: Vec<Expr>,
    pub frame: Option<Frame>,
}

/// `ROWS BETWEEN start AND end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub start: FrameBound,
    pub end: FrameBound,
}

/// A ClickHouse dictionary keyed by a single (complex) key, read with `dictGet`.
#[derive(Debug, Clone, PartialEq)]
pub struct DictLookup {
    /// Unqualified dictionary names resolve against the session database, which a Distributed
    /// read resets to `default`: the reference is always database-qualified.
    pub database: String,
    pub dictionary: &'static str,
    pub attribute: String,
    pub key: Box<Expr>,
    /// `dictGetOrDefault`'s fallback.
    pub default: Option<Box<Expr>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Column(Option<Ident>, Ident),
    Bind(BindValue),
    /// `toUUID('…')`, a typed id inlined as a literal.
    Uuid(uuid::Uuid),
    /// `toDate('…')`.
    Date(chrono::NaiveDate),
    /// `INTERVAL n UNIT`.
    Interval(i64, &'static str),
    Int(i64),
    Float(f64),
    /// A numeric literal written as is (`1e18`); compile-time text only.
    Literal(&'static str),
    Str(String),
    Null,
    /// The `*` of `count(*)`.
    Star,
    Call(&'static str, Vec<Expr>),
    /// A parametric aggregate: `name(params)(args)`, as `quantile(0.95)(x)`.
    Parametric(&'static str, Vec<Expr>, Vec<Expr>),
    Binary(Box<Expr>, Op, Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    IsNull(Box<Expr>),
    IsNotNull(Box<Expr>),
    Between {
        expr: Box<Expr>,
        low: Box<Expr>,
        high: Box<Expr>,
    },
    In {
        expr: Box<Expr>,
        set: Box<Expr>,
        negated: bool,
    },
    Subquery(Box<Query>),
    Tuple(Vec<Expr>),
    Index(Box<Expr>, Box<Expr>),
    Lambda(Ident, Box<Expr>),
    Over {
        call: Box<Expr>,
        ignore_nulls: bool,
        window: Window,
    },
    /// `call OVER name`, a window of the select's `WINDOW` clause.
    OverNamed(Box<Expr>, Ident),
    Dict(DictLookup),
}

impl Expr {
    fn visit_tables(&mut self, f: &mut impl FnMut(&mut Table)) {
        match self {
            Expr::Subquery(q) => q.visit_tables(f),
            Expr::Call(_, args) | Expr::And(args) | Expr::Or(args) | Expr::Tuple(args) => {
                args.iter_mut().for_each(|e| e.visit_tables(f))
            }
            Expr::Parametric(_, params, args) => params
                .iter_mut()
                .chain(args)
                .for_each(|e| e.visit_tables(f)),
            Expr::Binary(l, _, r) | Expr::Index(l, r) => {
                l.visit_tables(f);
                r.visit_tables(f);
            }
            Expr::In { expr, set, .. } => {
                expr.visit_tables(f);
                set.visit_tables(f);
            }
            Expr::Between { expr, low, high } => {
                expr.visit_tables(f);
                low.visit_tables(f);
                high.visit_tables(f);
            }
            Expr::Not(e)
            | Expr::Neg(e)
            | Expr::IsNull(e)
            | Expr::IsNotNull(e)
            | Expr::Lambda(_, e)
            | Expr::OverNamed(e, _) => e.visit_tables(f),
            Expr::Over { call, window, .. } => {
                call.visit_tables(f);
                window
                    .partition_by
                    .iter_mut()
                    .chain(&mut window.order_by)
                    .for_each(|e| e.visit_tables(f));
            }
            Expr::Dict(d) => {
                d.key.visit_tables(f);
                d.default.iter_mut().for_each(|e| e.visit_tables(f));
            }
            Expr::Column(..)
            | Expr::Bind(_)
            | Expr::Uuid(_)
            | Expr::Date(_)
            | Expr::Interval(..)
            | Expr::Int(_)
            | Expr::Float(_)
            | Expr::Literal(_)
            | Expr::Str(_)
            | Expr::Null
            | Expr::Star => {}
        }
    }

    pub fn col(name: &'static str) -> Expr {
        Expr::Column(None, Ident::trusted(name))
    }

    pub fn qcol(qualifier: &'static str, name: &'static str) -> Expr {
        Expr::Column(Some(Ident::trusted(qualifier)), Ident::trusted(name))
    }

    pub fn ident(name: &Ident) -> Expr {
        Expr::Column(None, name.clone())
    }

    pub fn call(name: &'static str, args: impl Into<Vec<Expr>>) -> Expr {
        Expr::Call(name, args.into())
    }

    pub fn str(value: &str) -> Expr {
        Expr::Str(value.to_string())
    }

    pub fn op(self, op: Op, rhs: Expr) -> Expr {
        Expr::Binary(Box::new(self), op, Box::new(rhs))
    }

    pub fn eq(self, rhs: Expr) -> Expr {
        self.op(Op::Eq, rhs)
    }

    pub fn ne(self, rhs: Expr) -> Expr {
        self.op(Op::Ne, rhs)
    }

    pub fn lt(self, rhs: Expr) -> Expr {
        self.op(Op::Lt, rhs)
    }

    pub fn le(self, rhs: Expr) -> Expr {
        self.op(Op::Le, rhs)
    }

    pub fn gt(self, rhs: Expr) -> Expr {
        self.op(Op::Gt, rhs)
    }

    pub fn ge(self, rhs: Expr) -> Expr {
        self.op(Op::Ge, rhs)
    }

    pub fn like(self, pattern: &str) -> Expr {
        self.op(Op::Like, Expr::str(pattern))
    }

    pub fn ilike(self, pattern: Expr) -> Expr {
        self.op(Op::ILike, pattern)
    }

    pub fn in_set(self, set: Expr) -> Expr {
        Expr::In {
            expr: Box::new(self),
            set: Box::new(set),
            negated: false,
        }
    }

    pub fn not_in_set(self, set: Expr) -> Expr {
        Expr::In {
            expr: Box::new(self),
            set: Box::new(set),
            negated: true,
        }
    }

    pub fn subquery(query: Query) -> Expr {
        Expr::Subquery(Box::new(query))
    }

    pub fn over(self, window: Window) -> Expr {
        Expr::Over {
            call: Box::new(self),
            ignore_nulls: false,
            window,
        }
    }

    /// `toDateTime(?)` over epoch seconds.
    pub fn datetime(secs: i64) -> Expr {
        Expr::call("toDateTime", [Expr::Bind(BindValue::I64(secs))])
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub expr: Expr,
    pub alias: Option<Ident>,
}

impl Item {
    pub fn new(expr: Expr) -> Item {
        Item { expr, alias: None }
    }

    pub fn aliased(expr: Expr, alias: Ident) -> Item {
        Item {
            expr,
            alias: Some(alias),
        }
    }

    pub fn as_(expr: Expr, alias: &'static str) -> Item {
        Item::aliased(expr, Ident::trusted(alias))
    }
}

/// Who may read a table: one tenant's rows (filtered on its tenant column) or everyone's.
#[derive(Debug, Clone, PartialEq)]
pub enum TableScope {
    Tenant(Ident),
    /// Shared reference data (FX rates).
    Global,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub database: Option<Ident>,
    pub name: Ident,
    pub scope: TableScope,
}

impl Table {
    pub fn tenant(name: Ident, tenant_column: Ident) -> Table {
        Table {
            database: None,
            name,
            scope: TableScope::Tenant(tenant_column),
        }
    }

    pub fn in_database(mut self, database: Ident) -> Table {
        self.database = Some(database);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    Inner,
    Left,
    FullOuter,
    Cross,
    AsofLeft,
}

/// A tenant's live raw events (no tombstone, no voided copy, keep-last dedup), only built by
/// [`super::EventsSource::read`].
#[derive(Debug, Clone, PartialEq)]
pub struct EventsRead {
    pub(super) table: String,
    pub(super) database: String,
    pub(super) tenant_id: uuid::Uuid,
    pub(super) dedup: EventDedup,
    pub(super) columns: Vec<Ident>,
    pub(super) filter: Vec<Expr>,
    pub(super) scope: VoidScope,
    /// Read as `<table> WHERE …` by the enclosing select instead of a subquery, for a read that
    /// sorts and limits the table itself; never with a dedup.
    pub(crate) flat: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Relation {
    /// `name [AS alias] [FINAL]`. `FINAL` collapses a ReplacingMergeTree to its latest versions.
    Table {
        table: Table,
        alias: Option<Ident>,
        final_: bool,
    },
    /// A CTE, read as `name [AS alias]`.
    Named(Ident, Option<Ident>),
    Subquery(Box<Query>, Option<Ident>),
    Join {
        left: Box<Relation>,
        kind: JoinKind,
        right: Box<Relation>,
        on: Option<Expr>,
        /// `USING (columns)`, when not empty.
        using: Vec<Ident>,
    },
    /// `from ARRAY JOIN expr AS alias[, …]`.
    ArrayJoin {
        from: Box<Relation>,
        columns: Vec<(Expr, Ident)>,
    },
    /// A table function such as `numbers(n)`.
    Function(Expr),
    Events(EventsRead),
}

impl Relation {
    fn visit_tables(&mut self, f: &mut impl FnMut(&mut Table)) {
        match self {
            Relation::Table { table, .. } => f(table),
            Relation::Named(..) => {}
            Relation::Subquery(q, _) => q.visit_tables(f),
            Relation::Join {
                left, right, on, ..
            } => {
                left.visit_tables(f);
                right.visit_tables(f);
                on.iter_mut().for_each(|e| e.visit_tables(f));
            }
            Relation::ArrayJoin { from, columns } => {
                from.visit_tables(f);
                columns.iter_mut().for_each(|(e, _)| e.visit_tables(f));
            }
            Relation::Function(e) => e.visit_tables(f),
            Relation::Events(read) => read.filter.iter_mut().for_each(|e| e.visit_tables(f)),
        }
    }

    pub fn table(table: Table) -> Relation {
        Relation::Table {
            table,
            alias: None,
            final_: false,
        }
    }

    pub fn final_table(table: Table) -> Relation {
        Relation::Table {
            table,
            alias: None,
            final_: true,
        }
    }

    /// The alias of a table read; a no-op on other relations.
    pub fn alias(mut self, name: &'static str) -> Relation {
        if let Relation::Table { alias, .. } = &mut self {
            *alias = Some(Ident::trusted(name));
        }
        self
    }

    pub fn subquery(query: Query) -> Relation {
        Relation::Subquery(Box::new(query), None)
    }

    pub fn aliased_subquery(query: Query, alias: &'static str) -> Relation {
        Relation::Subquery(Box::new(query), Some(Ident::trusted(alias)))
    }

    pub fn join(self, kind: JoinKind, right: Relation, on: Option<Expr>) -> Relation {
        Relation::Join {
            left: Box::new(self),
            kind,
            right: Box::new(right),
            on,
            using: vec![],
        }
    }

    pub fn join_using(self, kind: JoinKind, right: Relation, using: Vec<Ident>) -> Relation {
        Relation::Join {
            left: Box::new(self),
            kind,
            right: Box::new(right),
            on: None,
            using,
        }
    }
}

/// `WITH FILL FROM … TO … STEP … INTERPOLATE (…)` on the `ORDER BY`.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub from: Expr,
    pub to: Expr,
    pub step: Expr,
    pub interpolate: Vec<(Ident, Expr)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Select {
    pub distinct: bool,
    pub items: Vec<Item>,
    pub from: Option<Relation>,
    /// `WHERE` conjuncts.
    pub filter: Vec<Expr>,
    pub group_by: Vec<Expr>,
    /// `HAVING` conjuncts.
    pub having: Vec<Expr>,
    /// `WINDOW name AS (…)` definitions, read by [`Expr::OverNamed`].
    pub windows: Vec<(Ident, Window)>,
    pub order_by: Vec<Order>,
    pub fill: Option<Fill>,
    /// `LIMIT n BY columns`.
    pub limit_by: Option<(u64, Vec<Expr>)>,
    pub limit: Option<Expr>,
    pub offset: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub expr: Expr,
    /// `None` leaves the direction implicit (ascending).
    pub descending: Option<bool>,
}

impl From<Expr> for Order {
    fn from(expr: Expr) -> Order {
        Order {
            expr,
            descending: None,
        }
    }
}

impl Order {
    pub fn asc(expr: Expr) -> Order {
        Order {
            expr,
            descending: Some(false),
        }
    }

    pub fn desc(expr: Expr) -> Order {
        Order {
            expr,
            descending: Some(true),
        }
    }
}

impl Select {
    fn visit_tables(&mut self, f: &mut impl FnMut(&mut Table)) {
        if let Some(from) = &mut self.from {
            from.visit_tables(f);
        }
        let fill = self.fill.iter_mut().flat_map(|fill| {
            [&mut fill.from, &mut fill.to, &mut fill.step]
                .into_iter()
                .chain(fill.interpolate.iter_mut().map(|(_, e)| e))
        });
        self.items
            .iter_mut()
            .map(|i| &mut i.expr)
            .chain(&mut self.filter)
            .chain(&mut self.group_by)
            .chain(&mut self.having)
            .chain(self.order_by.iter_mut().map(|o| &mut o.expr))
            .chain(fill)
            .for_each(|e| e.visit_tables(f));
    }

    pub fn new(items: Vec<Item>, from: Relation) -> Select {
        Select {
            items,
            from: Some(from),
            ..Select::default()
        }
    }

    pub fn filter(mut self, filter: Vec<Expr>) -> Select {
        self.filter = filter;
        self
    }

    pub fn group_by(mut self, group_by: Vec<Expr>) -> Select {
        self.group_by = group_by;
        self
    }

    pub fn window(mut self, name: &'static str, window: Window) -> Select {
        self.windows.push((Ident::trusted(name), window));
        self
    }

    pub fn having(mut self, having: Vec<Expr>) -> Select {
        self.having = having;
        self
    }

    pub fn order_by_window_start(mut self) -> Select {
        self.order_by = vec![Expr::col("window_start").into()];
        self
    }

    pub fn order_by(mut self, order_by: Vec<Order>) -> Select {
        self.order_by = order_by;
        self
    }

    pub fn limit(mut self, limit: u64) -> Select {
        self.limit = Some(Expr::Int(limit as i64));
        self
    }

    /// `LIMIT <limit> OFFSET <offset>`, as bound values.
    pub fn page(mut self, limit: Expr, offset: Expr) -> Select {
        self.limit = Some(limit);
        self.offset = Some(offset);
        self
    }

    pub fn limit_by(mut self, n: u64, by: Vec<Expr>) -> Select {
        self.limit_by = Some((n, by));
        self
    }

    pub fn query(self) -> Query {
        Query::Select(Box::new(self))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOp {
    Intersect,
    UnionDistinct,
    UnionAll,
}

/// One item of a `WITH`: a CTE (`name AS ( query )`) or a scalar (`expr AS name`).
#[derive(Debug, Clone, PartialEq)]
pub enum With {
    Cte(Ident, Query),
    Scalar(Expr, Ident),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Query {
    Select(Box<Select>),
    With(Vec<With>, Box<Query>),
    SetOp(SetOp, Vec<Query>),
    /// `( query )`, where a set operation needs its operand grouped.
    Nested(Box<Query>),
    /// `INSERT INTO table (columns) query`.
    Insert {
        table: Table,
        columns: Vec<Ident>,
        body: Box<Query>,
    },
}

impl Query {
    /// Reads `to` wherever the tree reads the table `from`.
    pub fn rename_table(&mut self, from: &str, to: &Ident) {
        self.visit_tables(&mut |table| {
            if table.name.as_str() == from {
                table.name = to.clone();
            }
        });
    }

    fn visit_tables(&mut self, f: &mut impl FnMut(&mut Table)) {
        match self {
            Query::Select(select) => select.visit_tables(f),
            Query::With(items, body) => {
                for item in items {
                    match item {
                        With::Cte(_, q) => q.visit_tables(f),
                        With::Scalar(e, _) => e.visit_tables(f),
                    }
                }
                body.visit_tables(f);
            }
            Query::SetOp(_, parts) => parts.iter_mut().for_each(|q| q.visit_tables(f)),
            Query::Nested(q) => q.visit_tables(f),
            Query::Insert { table, body, .. } => {
                f(table);
                body.visit_tables(f);
            }
        }
    }

    pub fn with_ctes(ctes: Vec<(Ident, Query)>, body: Query) -> Query {
        let ctes = ctes.into_iter().map(|(n, q)| With::Cte(n, q)).collect();
        Query::With(ctes, Box::new(body))
    }

    pub fn insert(table: Table, columns: &[&'static str], body: Query) -> Query {
        Query::Insert {
            table,
            columns: columns.iter().map(|c| Ident::trusted(c)).collect(),
            body: Box::new(body),
        }
    }

    pub fn nested(self) -> Query {
        Query::Nested(Box::new(self))
    }
}

/// A per-statement `SETTINGS name = value`.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    pub name: &'static str,
    pub value: i64,
}
