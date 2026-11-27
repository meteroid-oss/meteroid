//! ClickHouse rendering of the [`super::ast`] tree. Each `?` is written together with its
//! bind, so the bind list always follows the placeholder order.

use super::ast::{
    DictLookup, EventsRead, Expr, Fill, FrameBound, Ident, Item, JoinKind, Op, Query, Relation,
    Select, SetOp, Setting, Window, With,
};
use super::{BindValue, SafeQuery};

/// Renders `query` with its statement-level `settings`.
pub fn render(query: &Query, settings: &[Setting]) -> SafeQuery {
    let out = Renderer::default().statement(query, settings);
    SafeQuery {
        sql: out.sql,
        binds: out.binds,
    }
}

/// A review form of `query`: one clause or item per line, nested queries indented, each bind
/// shown in place as `?{value}`.
pub fn pretty(query: &Query, settings: &[Setting]) -> String {
    let out = Renderer {
        pretty: true,
        ..Renderer::default()
    };
    out.statement(query, settings).sql
}

fn shown(bind: &BindValue) -> String {
    match bind {
        BindValue::String(v) => format!("{v:?}"),
        BindValue::Strings(v) => format!("{v:?}"),
        BindValue::I64(v) => v.to_string(),
        BindValue::U32(v) => v.to_string(),
        BindValue::Uuid(v) => v.to_string(),
        BindValue::Uuids(v) => format!("{:?}", v.iter().map(|u| u.to_string()).collect::<Vec<_>>()),
    }
}

/// A string literal, backslash-escaped so the text can't close it, with `?` doubled: the
/// ClickHouse client reads every `?` as a placeholder, quoted or not, and `??` as a literal one.
pub fn string_literal(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('?', "??");
    format!("'{escaped}'")
}

#[derive(Default)]
struct Renderer {
    sql: String,
    binds: Vec<BindValue>,
    pretty: bool,
    depth: usize,
}

impl Renderer {
    fn statement(mut self, query: &Query, settings: &[Setting]) -> Renderer {
        self.query(query);
        if !settings.is_empty() {
            self.br();
            self.push("SETTINGS ");
            let settings = settings
                .iter()
                .map(|s| format!("{} = {}", s.name, s.value))
                .collect::<Vec<_>>()
                .join(", ");
            self.push(&settings);
        }
        self
    }

    fn push(&mut self, sql: &str) {
        self.sql.push_str(sql);
    }

    /// A clause break: a space, or a new indented line in the pretty form.
    fn br(&mut self) {
        if self.pretty {
            self.sql.push('\n');
            self.sql.push_str(&"    ".repeat(self.depth));
        } else {
            self.sql.push(' ');
        }
    }

    /// `items` one level deeper, each after a break, separated by `sep`.
    fn block<T>(&mut self, items: &[T], sep: &str, mut each: impl FnMut(&mut Self, &T)) {
        self.depth += 1;
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                self.push(sep);
            }
            self.br();
            each(self, item);
        }
        self.depth -= 1;
    }

    /// `WHERE`/`HAVING` conjuncts: `a AND b`, or one per line in the pretty form.
    fn conjuncts(&mut self, keyword: &str, items: &[Expr]) {
        self.br();
        self.push(keyword);
        self.depth += 1;
        for (i, e) in items.iter().enumerate() {
            self.br();
            if i > 0 {
                self.push("AND ");
            }
            self.operand(e, AND);
        }
        self.depth -= 1;
    }

    fn list<T>(&mut self, items: &[T], sep: &str, mut each: impl FnMut(&mut Self, &T)) {
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                self.push(sep);
            }
            each(self, item);
        }
    }

    fn query(&mut self, query: &Query) {
        match query {
            Query::Select(select) => self.select(select),
            Query::With(items, body) => {
                self.push("WITH");
                self.block(items, ",", |r, item| match item {
                    With::Cte(name, q) => {
                        r.ident(name);
                        r.push(" AS ");
                        r.nested(q);
                    }
                    With::Scalar(expr, name) => {
                        r.expr(expr);
                        r.push(" AS ");
                        r.ident(name);
                    }
                });
                self.br();
                self.query(body);
            }
            Query::SetOp(op, parts) => {
                let keyword = match op {
                    SetOp::Intersect => "INTERSECT",
                    SetOp::UnionDistinct => "UNION DISTINCT",
                    SetOp::UnionAll => "UNION ALL",
                };
                for (i, q) in parts.iter().enumerate() {
                    if i > 0 {
                        self.br();
                        self.push(keyword);
                        self.br();
                    }
                    self.query(q);
                }
            }
            Query::Nested(q) => self.nested(q),
            Query::Insert {
                table,
                columns,
                body,
            } => {
                self.push("INSERT INTO ");
                self.table_name(table);
                self.push(" (");
                self.list(columns, ", ", |r, c| r.ident(c));
                self.push(")");
                self.br();
                self.query(body);
            }
        }
    }

    fn nested(&mut self, query: &Query) {
        self.push("(");
        self.depth += 1;
        self.br();
        self.query(query);
        self.depth -= 1;
        self.br();
        self.push(")");
    }

    fn select(&mut self, s: &Select) {
        self.push(if s.distinct {
            "SELECT DISTINCT"
        } else {
            "SELECT"
        });
        self.block(&s.items, ",", |r, item| r.item(item));
        if let Some(from) = &s.from {
            self.br();
            self.push("FROM ");
            self.relation(from);
        }
        if !s.filter.is_empty() {
            self.conjuncts("WHERE", &s.filter);
        }
        if !s.group_by.is_empty() {
            self.br();
            self.push("GROUP BY");
            self.block(&s.group_by, ",", |r, e| r.expr(e));
        }
        if !s.having.is_empty() {
            self.conjuncts("HAVING", &s.having);
        }
        if !s.windows.is_empty() {
            self.br();
            self.push("WINDOW");
            self.block(&s.windows, ",", |r, (name, w)| {
                r.ident(name);
                r.push(" AS (");
                r.window(w);
                r.push(")");
            });
        }
        if !s.order_by.is_empty() {
            self.br();
            self.push("ORDER BY");
            self.block(&s.order_by, ",", |r, o| {
                r.expr(&o.expr);
                match o.descending {
                    Some(true) => r.push(" DESC"),
                    Some(false) => r.push(" ASC"),
                    None => {}
                }
            });
        }
        if let Some(fill) = &s.fill {
            self.fill(fill);
        }
        if let Some((n, by)) = &s.limit_by {
            self.br();
            self.push(&format!("LIMIT {n} BY "));
            self.list(by, ", ", |r, e| r.expr(e));
        }
        if let Some(limit) = &s.limit {
            self.br();
            self.push("LIMIT ");
            self.expr(limit);
        }
        if let Some(offset) = &s.offset {
            self.push(" OFFSET ");
            self.expr(offset);
        }
    }

    fn fill(&mut self, fill: &Fill) {
        self.push(" WITH FILL FROM ");
        self.operand(&fill.from, ATOM);
        self.push(" TO ");
        self.operand(&fill.to, ATOM);
        self.push(" STEP ");
        self.operand(&fill.step, ATOM);
        if !fill.interpolate.is_empty() {
            self.push(" INTERPOLATE (");
            self.list(&fill.interpolate, ", ", |r, (col, e)| {
                r.ident(col);
                r.push(" AS ");
                r.expr(e);
            });
            self.push(")");
        }
    }

    fn item(&mut self, item: &Item) {
        match &item.alias {
            // An alias closes the whole item: a logical one keeps its parentheses for that.
            Some(_) => self.operand(&item.expr, COMPARISON),
            None => self.expr(&item.expr),
        }
        if let Some(alias) = &item.alias {
            self.push(" AS ");
            self.ident(alias);
        }
    }

    fn table_name(&mut self, table: &super::ast::Table) {
        if let Some(database) = &table.database {
            self.ident(database);
            self.push(".");
        }
        self.ident(&table.name);
    }

    fn ident(&mut self, ident: &Ident) {
        self.sql.push_str(ident.as_str());
    }

    fn relation(&mut self, relation: &Relation) {
        match relation {
            Relation::Table {
                table,
                alias,
                final_,
            } => {
                self.table_name(table);
                if let Some(alias) = alias {
                    self.push(" AS ");
                    self.ident(alias);
                }
                if *final_ {
                    self.push(" FINAL");
                }
            }
            Relation::Named(name, alias) => {
                self.ident(name);
                if let Some(alias) = alias {
                    self.push(" AS ");
                    self.ident(alias);
                }
            }
            Relation::Subquery(q, alias) => {
                self.nested(q);
                if let Some(alias) = alias {
                    self.push(" AS ");
                    self.ident(alias);
                }
            }
            Relation::Join {
                left,
                kind,
                right,
                on,
                using,
            } => {
                self.relation(left);
                self.br();
                self.push(match kind {
                    JoinKind::Inner => "INNER JOIN ",
                    JoinKind::Left => "LEFT JOIN ",
                    JoinKind::FullOuter => "FULL OUTER JOIN ",
                    JoinKind::Cross => "CROSS JOIN ",
                    JoinKind::AsofLeft => "ASOF LEFT JOIN ",
                });
                self.relation(right);
                if let Some(on) = on {
                    self.push(" ON ");
                    self.expr(on);
                }
                if !using.is_empty() {
                    self.push(" USING (");
                    self.list(using, ", ", |r, c| r.ident(c));
                    self.push(")");
                }
            }
            Relation::ArrayJoin { from, columns } => {
                self.relation(from);
                self.br();
                self.push("ARRAY JOIN ");
                self.list(columns, ", ", |r, (e, alias)| {
                    r.expr(e);
                    r.push(" AS ");
                    r.ident(alias);
                });
            }
            Relation::Function(call) => self.expr(call),
            Relation::Events(read) => self.events(read),
        }
    }

    /// `( SELECT <columns> FROM <events> WHERE (<filter>) AND <live>[ <dedup>] )`, wrapped in a
    /// `WHERE <kept>` over the kept versions when it has kept conjuncts.
    fn events(&mut self, read: &EventsRead) {
        if read.flat {
            self.push(&read.table);
            self.br();
            self.events_filter(read);
            return;
        }
        self.push("(");
        self.depth += 1;
        self.br();
        self.push("SELECT ");
        self.list(&read.columns, ", ", |r, c| r.ident(c));
        self.br();
        self.push("FROM ");
        if read.kept.is_empty() {
            self.push(&read.table);
            self.br();
            self.events_filter(read);
            self.push(read.dedup.limit_by_suffix());
        } else {
            self.push("(");
            self.depth += 1;
            self.br();
            self.push("SELECT ");
            let mut columns: Vec<&Ident> = read.columns.iter().collect();
            let mut kept_reads = Vec::new();
            read.kept
                .iter()
                .for_each(|e| e.own_columns(&mut kept_reads));
            for column in kept_reads {
                if !columns.contains(&column) {
                    columns.push(column);
                }
            }
            self.list(&columns, ", ", |r, c| r.ident(c));
            self.br();
            self.push("FROM ");
            self.push(&read.table);
            self.br();
            self.events_filter(read);
            self.push(read.dedup.limit_by_suffix());
            self.depth -= 1;
            self.br();
            self.push(")");
            self.br();
            self.push("WHERE ");
            self.list(&read.kept, " AND ", |r, e| r.operand(e, AND));
        }
        self.depth -= 1;
        self.br();
        self.push(")");
    }

    /// `WHERE (<filter>) AND <live>`.
    fn events_filter(&mut self, read: &EventsRead) {
        self.push("WHERE ");
        if !read.filter.is_empty() {
            self.push("(");
            self.list(&read.filter, " AND ", |r, e| r.operand(e, AND));
            self.push(") AND ");
        }
        let live = super::live_filter_sql(&read.database, read.tenant_id, &read.scope);
        self.push(&live);
    }

    fn expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Column(qualifier, name) => {
                if let Some(q) = qualifier {
                    self.ident(q);
                    self.push(".");
                }
                self.ident(name);
            }
            Expr::Bind(value) => {
                if self.pretty {
                    self.push(&format!("?{{{}}}", shown(value)));
                } else {
                    self.push("?");
                }
                self.binds.push(value.clone());
            }
            Expr::Uuid(id) => self.push(&format!("toUUID('{id}')")),
            Expr::Date(d) => self.push(&format!("toDate('{d}')")),
            Expr::Interval(n, unit) => self.push(&format!("INTERVAL {n} {unit}")),
            Expr::Int(v) => self.push(&v.to_string()),
            Expr::Literal(text) => self.push(text),
            Expr::Float(v) => self.push(&v.to_string()),
            Expr::Str(s) => self.push(&string_literal(s)),
            Expr::Null => self.push("NULL"),
            Expr::Star => self.push("*"),
            Expr::Call(name, args) => {
                self.push(name);
                self.push("(");
                self.list(args, ", ", |r, e| r.expr(e));
                self.push(")");
            }
            Expr::Parametric(name, params, args) => {
                self.push(name);
                self.push("(");
                self.list(params, ", ", |r, e| r.expr(e));
                self.push(")(");
                self.list(args, ", ", |r, e| r.expr(e));
                self.push(")");
            }
            Expr::Binary(l, op, r) => {
                let level = op_level(*op);
                self.operand(l, level);
                self.push(" ");
                self.push(op.as_sql());
                self.push(" ");
                // Left-associative: an equal-level right operand keeps its grouping.
                self.operand(r, level + 1);
            }
            Expr::And(items) => self.list(items, " AND ", |r, e| r.operand(e, AND)),
            // An AND inside an OR keeps its parentheses, for the reader.
            Expr::Or(items) => self.list(items, " OR ", |r, e| r.operand(e, NOT)),
            Expr::Not(e) => {
                self.push("NOT ");
                self.operand(e, NOT);
            }
            Expr::Neg(e) => {
                self.push("-");
                self.operand(e, ATOM);
            }
            Expr::Between { expr, low, high } => {
                self.operand(expr, ADDITIVE);
                self.push(" BETWEEN ");
                self.operand(low, ADDITIVE);
                self.push(" AND ");
                self.operand(high, ADDITIVE);
            }
            Expr::IsNull(e) => {
                self.operand(e, ADDITIVE);
                self.push(" IS NULL");
            }
            Expr::IsNotNull(e) => {
                self.operand(e, ADDITIVE);
                self.push(" IS NOT NULL");
            }
            Expr::OverNamed(call, name) => {
                self.expr(call);
                self.push(" OVER ");
                self.ident(name);
            }
            Expr::In { expr, set, negated } => {
                self.operand(expr, ADDITIVE);
                self.push(if *negated { " NOT IN " } else { " IN " });
                self.operand(set, ADDITIVE);
            }
            Expr::Subquery(q) => self.nested(q),
            Expr::Tuple(items) => {
                self.push("(");
                self.list(items, ", ", |r, e| r.expr(e));
                self.push(")");
            }
            Expr::Index(base, index) => {
                self.expr(base);
                self.push("[");
                self.expr(index);
                self.push("]");
            }
            Expr::Lambda(param, body) => {
                self.ident(param);
                self.push(" -> ");
                self.expr(body);
            }
            Expr::Over {
                call,
                ignore_nulls,
                window,
            } => {
                self.expr(call);
                if *ignore_nulls {
                    self.push(" IGNORE NULLS");
                }
                self.push(" OVER (");
                self.window(window);
                self.push(")");
            }
            Expr::Dict(lookup) => self.dict(lookup),
        }
    }

    /// `expr`, in parentheses when it binds more loosely than `min`.
    fn operand(&mut self, expr: &Expr, min: u8) {
        if level(expr) < min {
            self.push("(");
            self.expr(expr);
            self.push(")");
        } else {
            self.expr(expr);
        }
    }

    fn window(&mut self, w: &Window) {
        let mut clauses = 0;
        let mut clause = |r: &mut Self| {
            if clauses > 0 {
                r.push(" ");
            }
            clauses += 1;
        };
        if !w.partition_by.is_empty() {
            clause(self);
            self.push("PARTITION BY ");
            self.list(&w.partition_by, ", ", |r, e| r.expr(e));
        }
        if !w.order_by.is_empty() {
            clause(self);
            self.push("ORDER BY ");
            self.list(&w.order_by, ", ", |r, e| r.expr(e));
        }
        if let Some(frame) = w.frame {
            clause(self);
            self.push("ROWS BETWEEN ");
            self.bound(frame.start);
            self.push(" AND ");
            self.bound(frame.end);
        }
    }

    fn bound(&mut self, bound: FrameBound) {
        match bound {
            FrameBound::UnboundedPreceding => self.push("UNBOUNDED PRECEDING"),
            FrameBound::Preceding(n) => self.push(&format!("{n} PRECEDING")),
            FrameBound::CurrentRow => self.push("CURRENT ROW"),
            FrameBound::UnboundedFollowing => self.push("UNBOUNDED FOLLOWING"),
        }
    }

    fn dict(&mut self, d: &DictLookup) {
        self.push(if d.default.is_some() {
            "dictGetOrDefault("
        } else {
            "dictGet("
        });
        self.push(&string_literal(&format!("{}.{}", d.database, d.dictionary)));
        self.push(", ");
        self.push(&string_literal(&d.attribute));
        self.push(", tuple(");
        self.expr(&d.key);
        self.push(")");
        if let Some(default) = &d.default {
            self.push(", ");
            self.expr(default);
        }
        self.push(")");
    }
}

// Binding strength, loosest first: a child binding more loosely than its parent is parenthesised.
const LAMBDA: u8 = 0;
const OR: u8 = 1;
const AND: u8 = 2;
const NOT: u8 = 3;
const COMPARISON: u8 = 4;
const ADDITIVE: u8 = 5;
const MULTIPLICATIVE: u8 = 6;
const UNARY: u8 = 7;
const ATOM: u8 = 8;

fn op_level(op: Op) -> u8 {
    match op {
        Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge | Op::Like | Op::ILike => COMPARISON,
        Op::Add | Op::Sub => ADDITIVE,
        Op::Mul | Op::Div => MULTIPLICATIVE,
    }
}

fn level(expr: &Expr) -> u8 {
    match expr {
        Expr::Lambda(..) => LAMBDA,
        Expr::Or(items) if items.len() > 1 => OR,
        Expr::And(items) if items.len() > 1 => AND,
        Expr::Or(items) | Expr::And(items) => items.first().map_or(ATOM, level),
        Expr::Not(_) => NOT,
        Expr::In { .. } | Expr::Between { .. } | Expr::IsNull(_) | Expr::IsNotNull(_) => COMPARISON,
        Expr::Binary(_, op, _) => op_level(*op),
        // A negative literal after a `-` would start a `--` comment.
        Expr::Neg(_) => UNARY,
        Expr::Int(v) if *v < 0 => UNARY,
        Expr::Float(v) if v.is_sign_negative() => UNARY,
        _ => ATOM,
    }
}
