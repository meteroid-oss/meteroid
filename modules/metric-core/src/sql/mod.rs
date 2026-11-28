pub mod ast;
pub mod compiler;
#[cfg(test)]
mod golden;
pub mod render;
pub mod scope;

#[derive(Debug, Clone, PartialEq)]
pub enum BindValue {
    String(String),
    Strings(Vec<String>),
    I64(i64),
    U32(u32),
    Uuid(uuid::Uuid),
    Uuids(Vec<uuid::Uuid>),
}

/// What `Latest` ranks raw events by: distinct events sharing a timestamp would tie in `argMax`,
/// which then picks either value non-deterministically.
pub const LATEST_EVENT_ORDER: &str = "(timestamp, ingested_at, id)";

/// [`LATEST_EVENT_ORDER`] as a tree.
pub fn latest_event_order() -> ast::Expr {
    ast::Expr::Tuple(vec![
        ast::Expr::col("timestamp"),
        ast::Expr::col("ingested_at"),
        ast::Expr::col("id"),
    ])
}

/// How a raw-events read collapses re-sent event ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventDedup {
    /// Keep-last tenant: the last copy ingested wins.
    LastIngested,
    /// The consumer already dropped re-sent ids: no read-time sort.
    Off,
}

impl EventDedup {
    pub fn new(keep_last_events: bool) -> Self {
        if keep_last_events {
            EventDedup::LastIngested
        } else {
            EventDedup::Off
        }
    }

    /// Appended to a filtered raw-events `SELECT`.
    pub fn limit_by_suffix(self) -> &'static str {
        match self {
            EventDedup::LastIngested => " ORDER BY ingested_at DESC LIMIT 1 BY id, customer_id",
            EventDedup::Off => "",
        }
    }
}

/// Fed by `event_voids_mv` from tombstones: the stored copies an amend or a void cancelled.
pub const EVENT_VOIDS_TABLE: &str = "event_voids_local";

/// The raw events table. Readers reach it only through [`EventsTable::source`]
/// (`tests/events_source_only.rs` fails on any other use of its name, ADR-0012 §6).
#[derive(Debug, Clone)]
pub struct EventsTable(String);

impl EventsTable {
    pub fn new(name: impl Into<String>) -> Self {
        EventsTable(name.into())
    }

    /// Without the void filter: first-ingestion reads and the consumer only.
    pub fn unfiltered(&self) -> &str {
        &self.0
    }

    /// `database` holds `event_voids_local`.
    pub fn source<'a>(
        &'a self,
        database: &'a str,
        tenant_id: uuid::Uuid,
        dedup: EventDedup,
    ) -> EventsSource<'a> {
        EventsSource {
            table: &self.0,
            database,
            tenant_id,
            dedup,
        }
    }
}

impl EventsTable {
    /// The live events of a view shared by tenants, as a `FROM` item aliased `alias`: the tenant
    /// is `tenant` (trusted SQL, e.g. a setting), `timestamp` in [`since`, `until`), voids and
    /// tombstones out, and the last ingested copy of each event kept, whatever the tenant's dedup.
    pub fn shared_view_source(
        &self,
        database: &str,
        alias: &str,
        tenant: &str,
        since: &str,
        until: &str,
    ) -> String {
        format!(
            "(SELECT * FROM {database}.{table} AS e \
             WHERE e.tenant_id = {tenant} AND e.timestamp >= {since} AND e.timestamp < {until} \
               AND e.is_tombstone = 0 \
               AND (e.customer_id, e.id, e.ingested_at) NOT IN (\
                 SELECT v.customer_id, v.id, v.ingested_at FROM {database}.{EVENT_VOIDS_TABLE} AS v \
                 WHERE v.tenant_id = {tenant} AND v.timestamp >= {since} AND v.timestamp <= {until}) \
             {dedup}) AS {alias}",
            table = self.0,
            dedup = EventDedup::LastIngested
                .limit_by_suffix()
                .replace("ingested_at", "e.ingested_at")
                .replace("BY id, customer_id", "BY e.id, e.customer_id"),
        )
    }
}

impl From<&str> for EventsTable {
    fn from(name: &str) -> Self {
        EventsTable::new(name)
    }
}

/// A tenant's live events: no tombstone, no voided copy, then the keep-last dedup.
#[derive(Debug, Clone, Copy)]
pub struct EventsSource<'a> {
    table: &'a str,
    database: &'a str,
    tenant_id: uuid::Uuid,
    dedup: EventDedup,
}

/// The read's own predicates, repeated on the void set. A tombstone is an exact copy of the row it
/// cancels, so the void of any row the read keeps matches them too.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VoidScope {
    codes: Vec<String>,
    customers: Vec<uuid::Uuid>,
    since: Option<String>,
    until: Option<String>,
}

impl VoidScope {
    pub fn code(self, code: &str) -> Self {
        self.codes(&[code.to_string()])
    }

    pub fn codes(mut self, codes: &[String]) -> Self {
        self.codes = codes.to_vec();
        self
    }

    pub fn customers(mut self, ids: impl IntoIterator<Item = uuid::Uuid>) -> Self {
        self.customers = ids.into_iter().collect();
        self
    }

    /// Inclusive lower bound on `timestamp`, as trusted SQL (`toDateTime(<secs>)`, `now() - …`).
    pub fn since(mut self, sql: impl Into<String>) -> Self {
        self.since = Some(sql.into());
        self
    }

    /// Upper bound on `timestamp`, kept inclusive: a superset of a strict read's voids.
    pub fn until(mut self, sql: impl Into<String>) -> Self {
        self.until = Some(sql.into());
        self
    }

    fn sql(&self, tenant_id: uuid::Uuid) -> String {
        let list = |items: Vec<String>| items.join(", ");
        let mut terms = vec![format!("tenant_id = toUUID('{tenant_id}')")];
        if !self.codes.is_empty() {
            let codes = self
                .codes
                .iter()
                .map(|c| render::string_literal(c))
                .collect();
            terms.push(format!("code IN ({})", list(codes)));
        }
        if !self.customers.is_empty() {
            let ids = self.customers.iter().map(|id| format!("toUUID('{id}')"));
            terms.push(format!("customer_id IN ({})", list(ids.collect())));
        }
        terms.extend(self.since.iter().map(|s| format!("timestamp >= {s}")));
        terms.extend(self.until.iter().map(|u| format!("timestamp <= {u}")));
        terms.join(" AND ")
    }
}

impl EventsSource<'_> {
    pub fn dedup(&self) -> EventDedup {
        self.dedup
    }

    /// The live rows of `columns` matching every `filter` conjunct, as a typed relation. Under a
    /// dedup, conjuncts on `properties` judge the version it keeps, so a re-send that changes a
    /// filtered property moves the event instead of leaving its older version behind.
    pub fn read(
        &self,
        columns: Vec<ast::Ident>,
        filter: Vec<ast::Expr>,
        scope: VoidScope,
    ) -> ast::Relation {
        let (kept, filter) = match self.dedup {
            EventDedup::Off => (vec![], filter),
            EventDedup::LastIngested => filter
                .into_iter()
                .flat_map(ast::Expr::into_conjuncts)
                .partition(|conjunct| conjunct.reads_column("properties")),
        };
        ast::Relation::Events(ast::EventsRead {
            table: self.table.to_string(),
            database: self.database.to_string(),
            tenant_id: self.tenant_id,
            dedup: self.dedup,
            columns,
            filter,
            kept,
            scope,
            flat: false,
        })
    }

    /// [`Self::read`] as `<table> WHERE …`, for a select that sorts and limits the table
    /// itself. `None` when the tenant dedups, which needs the subquery.
    pub fn read_flat(&self, filter: Vec<ast::Expr>, scope: VoidScope) -> Option<ast::Relation> {
        (self.dedup == EventDedup::Off).then(|| {
            ast::Relation::Events(ast::EventsRead {
                table: self.table.to_string(),
                database: self.database.to_string(),
                tenant_id: self.tenant_id,
                dedup: self.dedup,
                columns: vec![],
                filter,
                kept: vec![],
                scope,
                flat: true,
            })
        })
    }
}

/// No tombstone, and no copy cancelled by a void in `scope`; an empty void set skips the `NOT IN`.
fn live_filter_sql(database: &str, tenant_id: uuid::Uuid, scope: &VoidScope) -> String {
    let voids = format!(
        "FROM {database}.{EVENT_VOIDS_TABLE} WHERE {}",
        scope.sql(tenant_id)
    );
    format!(
        "is_tombstone = 0 AND ((SELECT count() {voids}) = 0 OR (customer_id, id, ingested_at) \
         NOT IN (SELECT customer_id, id, ingested_at {voids}))"
    )
}

/// Checks the tenant isolation of `query`, then renders it.
pub fn render_scoped(
    query: &ast::Query,
    settings: &[ast::Setting],
    tenant: uuid::Uuid,
) -> Result<SafeQuery, String> {
    scope::verify_tenant_scope(query, tenant)?;
    Ok(render::render(query, settings))
}

#[derive(Debug)]
pub struct SafeQuery {
    pub sql: String,
    pub binds: Vec<BindValue>,
}

impl SafeQuery {
    /// The `?` the ClickHouse client binds: `??` is a literal one.
    pub fn placeholders(&self) -> usize {
        self.sql.replace("??", "").matches('?').count()
    }

    pub fn into_query(self, client: &clickhouse::Client) -> clickhouse::query::Query {
        let mut q = client.query(&self.sql);
        for b in self.binds {
            q = match b {
                BindValue::String(s) => q.bind(s),
                BindValue::Strings(v) => q.bind(v),
                BindValue::I64(v) => q.bind(v),
                BindValue::U32(v) => q.bind(v),
                BindValue::Uuid(v) => q.bind(v),
                BindValue::Uuids(v) => q.bind(v),
            };
        }
        q
    }
}

pub struct PropertyColumn<'a>(pub &'a str);

/// Replace every char that isn't alphanumeric or `_` with `_`, so an arbitrary metric/property
/// name becomes a safe SQL identifier fragment. Shared by `PropertyColumn::as_alias` and the
/// compiler's operand-alias `sanitize`.
pub fn sanitize_identifier(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl<'a> PropertyColumn<'a> {
    const ALIAS_PREFIX: &'static str = "_prop_";

    pub fn raw_name(&self) -> &'a str {
        self.0
    }

    pub fn as_alias(&self) -> String {
        format!("{}{}", Self::ALIAS_PREFIX, sanitize_identifier(self.0))
    }

    pub fn path_sql(&self, binds: &mut Vec<BindValue>) -> String {
        binds.push(BindValue::String(self.0.to_string()));
        "properties[?]".to_string()
    }

    pub fn select_sql(&self, binds: &mut Vec<BindValue>) -> String {
        binds.push(BindValue::String(self.0.to_string()));
        format!("properties[?] AS {}", self.as_alias())
    }

    pub fn from_str_ref(s: &'a str) -> Self {
        PropertyColumn(s)
    }
}

/// The stable result-column alias for a grouped [`Dimension`], shared by the
/// compiler (when it emits `… AS <alias>`) and the backend (when it reads the
/// column back), so a result column always maps to its dimension name. Reuses the
/// property-alias scheme so existing `Auto`/property dimensions keep their column.
pub fn dimension_alias(dim: &crate::metric::Dimension) -> String {
    PropertyColumn(dim.name()).as_alias()
}

/// The result-column alias carrying an [entity dimension](crate::metric::Dimension::is_entity)'s
/// grouping id, alongside its `dimension_alias` name column. The backend reads it to
/// disambiguate distinct entities that share a display name (two "Acme" customers).
pub fn dimension_disambiguator_alias(dim: &crate::metric::Dimension) -> String {
    format!("_dis_{}", sanitize_identifier(dim.name()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn latest_event_order_renders_the_constant() {
        let select = super::ast::Select {
            items: vec![super::ast::Item::new(super::latest_event_order())],
            ..Default::default()
        };
        let sql = super::render::render(&select.query(), &[]).sql;
        assert_eq!(sql, format!("SELECT {}", super::LATEST_EVENT_ORDER));
    }

    use super::*;

    /// `FROM <the read>` of `id, properties` filtered on `tenant_id = ? AND code = ?`.
    fn source_sql(dedup: EventDedup, scope: &VoidScope) -> String {
        let read = EventsTable::new("raw_events_v2")
            .source("meteroid", uuid::Uuid::from_u128(7), dedup)
            .read(
                vec![ast::Ident::trusted("id"), ast::Ident::trusted("properties")],
                vec![
                    ast::Expr::col("tenant_id")
                        .eq(ast::Expr::Bind(BindValue::Uuid(uuid::Uuid::from_u128(7)))),
                    ast::Expr::col("code").eq(ast::Expr::Bind(BindValue::String("c".into()))),
                ],
                scope.clone(),
            );
        let select = ast::Select::new(vec![ast::Item::new(ast::Expr::Star)], read);
        let sql = render::render(&select.query(), &[]).sql;
        sql.strip_prefix("SELECT * ").unwrap().to_string()
    }

    const VOIDS: &str = "FROM meteroid.event_voids_local WHERE tenant_id = toUUID('00000000-0000-0000-0000-000000000007')";

    #[test]
    fn events_source_filters_tombstones_and_the_tenant_voids() {
        assert_eq!(
            source_sql(EventDedup::Off, &VoidScope::default()),
            format!(
                "FROM ( SELECT id, properties FROM raw_events_v2 WHERE (tenant_id = ? AND code = ?) \
                 AND is_tombstone = 0 AND ((SELECT count() {VOIDS}) = 0 OR (customer_id, id, \
                 ingested_at) NOT IN (SELECT customer_id, id, ingested_at {VOIDS})) )"
            )
        );
    }

    #[test]
    fn the_void_set_is_narrowed_by_the_read_predicates() {
        let scope = VoidScope::default()
            .code("it's")
            .customers([uuid::Uuid::from_u128(1)])
            .since("toDateTime(10)")
            .until("toDateTime(20)");
        let narrowed = format!(
            "{VOIDS} AND code IN ('it\\'s') AND customer_id IN \
             (toUUID('00000000-0000-0000-0000-000000000001')) AND timestamp >= toDateTime(10) \
             AND timestamp <= toDateTime(20)"
        );
        let sql = source_sql(EventDedup::Off, &scope);
        assert_eq!(sql.matches(&narrowed).count(), 2, "{sql}");
    }

    #[test]
    fn keep_last_filters_properties_on_the_kept_version() {
        let tenant = uuid::Uuid::from_u128(7);
        let cohort = ast::Select::new(
            vec![ast::Item::new(ast::Expr::col("customer_id"))],
            ast::Relation::Named(ast::Ident::trusted("cohort"), None),
        )
        .filter(vec![ast::Expr::col("properties").eq(ast::Expr::str("x"))])
        .query();
        let sql = |dedup| {
            let read = EventsTable::new("raw_events_v2")
                .source("meteroid", tenant, dedup)
                .read(
                    vec![ast::Ident::trusted("id")],
                    vec![
                        ast::Expr::col("tenant_id").eq(ast::Expr::Uuid(tenant)),
                        ast::Expr::col("customer_id").in_set(ast::Expr::subquery(cohort.clone())),
                        ast::Expr::Index(
                            Box::new(ast::Expr::col("properties")),
                            Box::new(ast::Expr::str("region")),
                        )
                        .eq(ast::Expr::str("eu")),
                    ],
                    VoidScope::default(),
                );
            let select = ast::Select::new(vec![ast::Item::new(ast::Expr::Star)], read);
            render::render(&select.query(), &[]).sql
        };
        let kept = sql(EventDedup::LastIngested);
        assert!(
            kept.contains(
                "FROM ( SELECT id FROM ( SELECT id, properties FROM raw_events_v2 WHERE \
                 (tenant_id = toUUID('00000000-0000-0000-0000-000000000007') AND customer_id IN \
                 ( SELECT customer_id FROM cohort WHERE properties = 'x' )) AND is_tombstone = 0"
            ),
            "{kept}"
        );
        assert!(
            kept.ends_with(
                " ORDER BY ingested_at DESC LIMIT 1 BY id, customer_id ) WHERE properties['region'] = 'eu' )"
            ),
            "{kept}"
        );
        let off = sql(EventDedup::Off);
        assert!(
            off.contains("AND properties['region'] = 'eu') AND is_tombstone = 0"),
            "{off}"
        );
    }

    #[test]
    fn a_mixed_conjunct_keeps_its_bounds_before_the_dedup() {
        let read = EventsTable::new("raw_events_v2")
            .source(
                "meteroid",
                uuid::Uuid::from_u128(7),
                EventDedup::LastIngested,
            )
            .read(
                vec![ast::Ident::trusted("properties")],
                vec![ast::Expr::And(vec![
                    ast::Expr::col("timestamp").ge(ast::Expr::Int(1)),
                    ast::Expr::col("properties").ne(ast::Expr::str("")),
                ])],
                VoidScope::default(),
            );
        let ast::Relation::Events(read) = read else {
            unreachable!()
        };
        assert_eq!(
            read.filter,
            vec![ast::Expr::col("timestamp").ge(ast::Expr::Int(1))]
        );
        assert_eq!(
            read.kept,
            vec![ast::Expr::col("properties").ne(ast::Expr::str(""))]
        );
    }

    #[test]
    fn keep_last_dedups_the_live_events() {
        assert!(
            source_sql(EventDedup::LastIngested, &VoidScope::default())
                .ends_with(" ORDER BY ingested_at DESC LIMIT 1 BY id, customer_id )")
        );
    }
}
