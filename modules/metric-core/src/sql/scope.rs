//! Tenant isolation as a property of the tree: every tenant-scoped table and every raw-events
//! read must be filtered on the expected tenant, by a conjunct of the `WHERE` reading it (or of
//! its join's `ON`), on its own tenant column.

use super::BindValue;
use super::ast::{EventsRead, Expr, Ident, Op, Query, Relation, Select, TableScope, With};

/// The only table function a query may read: it generates rows, it reads none.
const TABLE_FUNCTIONS: &[&str] = &["numbers"];

pub fn verify_tenant_scope(query: &Query, tenant: uuid::Uuid) -> Result<(), String> {
    Verifier {
        tenant,
        ctes: Vec::new(),
    }
    .query(query)
}

struct Verifier {
    tenant: uuid::Uuid,
    /// The CTE names visible where the walk stands.
    ctes: Vec<Ident>,
}

/// A table read, with the conjuncts that may scope it.
struct Read<'a> {
    filter: Vec<&'a Expr>,
    /// Several tables in one `FROM`: an unqualified tenant column would be ambiguous.
    joined: bool,
}

impl Verifier {
    fn query(&mut self, query: &Query) -> Result<(), String> {
        match query {
            Query::Select(select) => self.select(select),
            Query::With(items, body) => {
                let depth = self.ctes.len();
                for item in items {
                    match item {
                        With::Cte(name, q) => {
                            self.query(q)?;
                            self.ctes.push(name.clone());
                        }
                        With::Scalar(expr, _) => self.expr(expr)?,
                    }
                }
                let result = self.query(body);
                self.ctes.truncate(depth);
                result
            }
            Query::SetOp(_, parts) => parts.iter().try_for_each(|q| self.query(q)),
            Query::Nested(q) => self.query(q),
            Query::Insert { body, .. } => self.query(body),
        }
    }

    fn select(&mut self, select: &Select) -> Result<(), String> {
        match &select.from {
            Some(Relation::Events(events)) if events.flat => {
                if !select.filter.is_empty() {
                    return Err(
                        "a flat raw-events read carries its select's whole WHERE".to_string()
                    );
                }
                self.events(events)?;
            }
            Some(from) => {
                let read = Read {
                    filter: select.filter.iter().collect(),
                    joined: matches!(from, Relation::Join { .. }),
                };
                self.relation(from, &read, select)?;
            }
            None => {}
        }
        let fill = select.fill.iter().flat_map(|f| {
            [&f.from, &f.to, &f.step]
                .into_iter()
                .chain(f.interpolate.iter().map(|(_, e)| e))
        });
        let exprs = select
            .items
            .iter()
            .map(|i| &i.expr)
            .chain(&select.filter)
            .chain(&select.group_by)
            .chain(&select.having)
            .chain(select.order_by.iter().map(|o| &o.expr))
            .chain(fill);
        exprs.into_iter().try_for_each(|e| self.expr(e))
    }

    fn relation(
        &mut self,
        relation: &Relation,
        read: &Read,
        select: &Select,
    ) -> Result<(), String> {
        match relation {
            Relation::Table { table, alias, .. } => match &table.scope {
                TableScope::Global => Ok(()),
                TableScope::Tenant(column) => {
                    let name = alias.as_ref().unwrap_or(&table.name);
                    shadows(select, column, table.name.as_str())?;
                    self.require(read, column, Some(name), table.name.as_str())
                }
            },
            Relation::Named(name, _) => {
                if self.ctes.contains(name) {
                    Ok(())
                } else {
                    Err(format!("{} is not a CTE in scope", name.as_str()))
                }
            }
            Relation::Function(call) => match call {
                Expr::Call(name, args) if TABLE_FUNCTIONS.contains(name) => {
                    args.iter().try_for_each(|e| self.expr(e))
                }
                _ => Err("table functions other than numbers() are not allowed".to_string()),
            },
            Relation::Subquery(q, _) => self.query(q),
            Relation::Join {
                left, right, on, ..
            } => {
                let mut scoped = Read {
                    filter: read.filter.clone(),
                    joined: true,
                };
                if let Some(on) = on {
                    scoped.filter.extend(conjuncts(on));
                    self.expr(on)?;
                }
                self.relation(left, &scoped, select)?;
                self.relation(right, &scoped, select)
            }
            Relation::ArrayJoin { from, columns } => {
                self.relation(from, read, select)?;
                columns.iter().try_for_each(|(e, _)| self.expr(e))
            }
            Relation::Events(events) if events.flat => {
                Err("a flat raw-events read must be its select's whole FROM".to_string())
            }
            Relation::Events(events) => self.events(events),
        }
    }

    fn events(&mut self, events: &EventsRead) -> Result<(), String> {
        if events.tenant_id != self.tenant {
            return Err(format!(
                "raw events read for tenant {}, expected {}",
                events.tenant_id, self.tenant
            ));
        }
        let own = Read {
            filter: events.filter.iter().collect(),
            joined: false,
        };
        self.require(&own, &Ident::trusted("tenant_id"), None, &events.table)?;
        events
            .filter
            .iter()
            .chain(&events.kept)
            .try_for_each(|e| self.expr(e))
    }

    /// A conjunct `<table>.<column> = <tenant>`; unqualified only when the table reads alone.
    fn require(
        &self,
        read: &Read,
        column: &Ident,
        table: Option<&Ident>,
        name: &str,
    ) -> Result<(), String> {
        let on_this_table = |q: &Option<Ident>| match q {
            None => !read.joined,
            Some(q) => table == Some(q),
        };
        let scoped = read.filter.iter().any(|conjunct| match conjunct {
            Expr::Binary(lhs, Op::Eq, rhs) => {
                matches!(lhs.as_ref(), Expr::Column(q, c) if c == column && on_this_table(q))
                    && self.is_tenant(rhs)
            }
            _ => false,
        });
        if scoped {
            Ok(())
        } else {
            Err(format!(
                "read of {name} is not filtered on {} = <tenant>",
                column.as_str()
            ))
        }
    }

    fn is_tenant(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Bind(BindValue::Uuid(t)) | Expr::Uuid(t) => *t == self.tenant,
            _ => false,
        }
    }

    /// Subqueries nested in expressions (`IN ( … )`, scalar subqueries).
    fn expr(&mut self, expr: &Expr) -> Result<(), String> {
        match expr {
            Expr::Subquery(q) => self.query(q),
            Expr::Call(_, args) | Expr::And(args) | Expr::Or(args) | Expr::Tuple(args) => {
                args.iter().try_for_each(|e| self.expr(e))
            }
            Expr::Parametric(_, params, args) => {
                params.iter().chain(args).try_for_each(|e| self.expr(e))
            }
            Expr::Binary(l, _, r) | Expr::Index(l, r) => {
                self.expr(l)?;
                self.expr(r)
            }
            Expr::In { expr, set, .. } => {
                self.expr(expr)?;
                self.expr(set)
            }
            Expr::Between { expr, low, high } => {
                self.expr(expr)?;
                self.expr(low)?;
                self.expr(high)
            }
            Expr::Not(e)
            | Expr::Neg(e)
            | Expr::IsNull(e)
            | Expr::IsNotNull(e)
            | Expr::Lambda(_, e)
            | Expr::OverNamed(e, _) => self.expr(e),
            Expr::Over { call, window, .. } => {
                self.expr(call)?;
                window
                    .partition_by
                    .iter()
                    .chain(&window.order_by)
                    .try_for_each(|e| self.expr(e))
            }
            Expr::Dict(d) => {
                self.expr(&d.key)?;
                d.default.iter().try_for_each(|e| self.expr(e))
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
            | Expr::Star => Ok(()),
        }
    }
}

/// ClickHouse resolves a `SELECT` alias before a column in `WHERE`: an item aliased as the tenant
/// column would turn the tenant filter into a comparison with itself.
fn shadows(select: &Select, column: &Ident, table: &str) -> Result<(), String> {
    match select
        .items
        .iter()
        .find(|i| i.alias.as_ref() == Some(column))
    {
        Some(_) => Err(format!(
            "a read of {table} aliases an item as its tenant column {}",
            column.as_str()
        )),
        None => Ok(()),
    }
}

fn conjuncts(expr: &Expr) -> Vec<&Expr> {
    match expr {
        Expr::And(items) => items.iter().flat_map(conjuncts).collect(),
        other => vec![other],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql::VoidScope;
    use crate::sql::ast::{Item, JoinKind, Table};

    fn tenant() -> uuid::Uuid {
        uuid::Uuid::from_bytes([1; 16])
    }

    fn customers() -> Relation {
        let table = Table::tenant(Ident::trusted("dim_customer"), Ident::trusted("tenant_id"));
        Relation::final_table(table)
    }

    fn read(filter: Vec<Expr>) -> Query {
        Select::new(vec![Item::new(Expr::col("id"))], customers())
            .filter(filter)
            .query()
    }

    fn tenant_eq(qualifier: Option<&'static str>, t: uuid::Uuid) -> Expr {
        let column = Expr::Column(qualifier.map(Ident::trusted), Ident::trusted("tenant_id"));
        column.eq(Expr::Bind(BindValue::Uuid(t)))
    }

    fn check(q: &Query) -> Result<(), String> {
        verify_tenant_scope(q, tenant())
    }

    #[test]
    fn a_tenant_table_needs_its_tenant_filter() {
        let other = uuid::Uuid::from_bytes([2; 16]);
        assert!(check(&read(vec![tenant_eq(None, tenant())])).is_ok());
        assert!(check(&read(vec![Expr::col("tenant_id").eq(Expr::Uuid(tenant()))])).is_ok());
        assert!(check(&read(vec![])).is_err());
        assert!(check(&read(vec![tenant_eq(None, other)])).is_err());
        let nested_or = Expr::Or(vec![
            tenant_eq(None, tenant()),
            Expr::Int(1).eq(Expr::Int(1)),
        ]);
        assert!(check(&read(vec![nested_or])).is_err());
    }

    #[test]
    fn a_subquery_in_an_expression_is_checked() {
        let outer = Query::with_ctes(
            vec![(Ident::trusted("cte"), read(vec![tenant_eq(None, tenant())]))],
            Select::new(
                vec![Item::new(Expr::col("id"))],
                Relation::Named(Ident::trusted("cte"), None),
            )
            .filter(vec![Expr::col("id").in_set(Expr::subquery(read(vec![])))])
            .query(),
        );
        assert!(check(&outer).is_err());
    }

    #[test]
    fn a_joined_table_is_scoped_by_its_own_qualified_column() {
        let joined = |filter: Vec<Expr>, on: Expr| {
            let left = Relation::Subquery(
                Box::new(read(vec![tenant_eq(None, tenant())])),
                Some(Ident::trusted("r")),
            );
            Select::new(
                vec![Item::new(Expr::qcol("c", "id"))],
                left.join(JoinKind::Left, customers().alias("c"), Some(on)),
            )
            .filter(filter)
            .query()
        };
        let on_id = Expr::qcol("c", "id").eq(Expr::qcol("r", "id"));
        assert!(check(&joined(vec![tenant_eq(Some("c"), tenant())], on_id.clone())).is_ok());
        let in_on = Expr::And(vec![on_id.clone(), tenant_eq(Some("c"), tenant())]);
        assert!(check(&joined(vec![], in_on)).is_ok());
        assert!(check(&joined(vec![tenant_eq(None, tenant())], on_id.clone())).is_err());
        assert!(check(&joined(vec![tenant_eq(Some("r"), tenant())], on_id)).is_err());
    }

    #[test]
    fn an_alias_shadowing_the_tenant_column_is_refused() {
        let shadowed = Select::new(
            vec![Item::as_(Expr::Uuid(tenant()), "tenant_id")],
            customers(),
        )
        .filter(vec![tenant_eq(None, tenant())])
        .query();
        assert!(check(&shadowed).is_err());
    }

    #[test]
    fn rename_table_reaches_every_read() {
        let revenue = |name: &'static str| {
            Relation::final_table(Table::tenant(
                Ident::trusted(name),
                Ident::trusted("tenant_id"),
            ))
        };
        let inner = Select::new(vec![Item::new(Expr::col("customer_id"))], revenue("rev_v2"))
            .filter(vec![tenant_eq(None, tenant())])
            .query();
        let body = Select::new(
            vec![Item::new(Expr::qcol("c", "id"))],
            Relation::Named(Ident::trusted("cte"), Some(Ident::trusted("c"))).join(
                JoinKind::Left,
                Relation::aliased_subquery(inner.clone(), "r"),
                None,
            ),
        )
        .filter(vec![Expr::col("id").in_set(Expr::subquery(inner.clone()))]);
        let mut query = Query::with_ctes(vec![(Ident::trusted("cte"), inner)], body.query());
        query.rename_table("rev_v2", &Ident::trusted("rev_v1"));
        let sql = crate::sql::render::render(&query, &[]).sql;
        assert_eq!(sql.matches("rev_v1 FINAL").count(), 3, "{sql}");
        assert!(!sql.contains("rev_v2"), "{sql}");
    }

    #[test]
    fn a_flat_events_read_is_only_a_whole_from() {
        let table = crate::sql::EventsTable::new("events");
        let source = table.source("db", tenant(), crate::sql::EventDedup::Off);
        let flat = || {
            source
                .read_flat(vec![tenant_eq(None, tenant())], VoidScope::default())
                .unwrap()
        };
        let select = |from: Relation| Select::new(vec![Item::new(Expr::col("id"))], from);
        assert!(check(&select(flat()).query()).is_ok());
        assert!(
            check(
                &select(flat())
                    .filter(vec![Expr::col("id").eq(Expr::Int(1))])
                    .query()
            )
            .is_err()
        );
        let array_join = Relation::ArrayJoin {
            from: Box::new(flat()),
            columns: vec![(Expr::col("ks"), Ident::trusted("k"))],
        };
        assert!(check(&select(array_join).query()).is_err());
        let joined = flat().join(JoinKind::Inner, customers(), None);
        assert!(check(&select(joined).query()).is_err());
    }

    #[test]
    fn unknown_ctes_and_table_functions_are_refused() {
        let named = |name: &'static str| {
            Select::new(
                vec![Item::new(Expr::col("id"))],
                Relation::Named(Ident::trusted(name), None),
            )
            .query()
        };
        assert!(check(&named("raw_events_v2")).is_err());
        let function = |name: &'static str| {
            Select::new(
                vec![Item::new(Expr::col("number"))],
                Relation::Function(Expr::call(name, [Expr::Int(3)])),
            )
            .query()
        };
        assert!(check(&function("numbers")).is_ok());
        assert!(check(&function("remote")).is_err());
    }
}
