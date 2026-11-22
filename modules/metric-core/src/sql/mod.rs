pub mod compiler;

#[derive(Debug, Clone)]
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
#[derive(Debug, Clone, Default)]
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
            let codes = self.codes.iter().map(|c| string_literal(c)).collect();
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

fn string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

impl EventsSource<'_> {
    /// `FROM ( SELECT <columns> FROM <events> WHERE (<conditions>) AND <live> )`. The void set is
    /// read from the local table (one shard), narrowed by `scope`; an empty one skips the `NOT IN`.
    pub fn from_sql(&self, columns: &str, conditions: &str, scope: &VoidScope) -> String {
        format!(
            "FROM ( SELECT {columns} FROM {} WHERE {}{} )",
            self.table,
            self.live_filter(conditions, scope),
            self.dedup.limit_by_suffix()
        )
    }

    /// `FROM <events> WHERE (<conditions>) AND <live>`, without a subquery or the dedup: for a
    /// read that sorts and limits the table itself.
    pub fn flat_from_sql(&self, conditions: &str, scope: &VoidScope) -> String {
        format!(
            "FROM {} WHERE {}",
            self.table,
            self.live_filter(conditions, scope)
        )
    }

    pub fn dedup(&self) -> EventDedup {
        self.dedup
    }

    fn live_filter(&self, conditions: &str, scope: &VoidScope) -> String {
        let voids = format!(
            "FROM {}.{EVENT_VOIDS_TABLE} WHERE {}",
            self.database,
            scope.sql(self.tenant_id)
        );
        let live = format!(
            "is_tombstone = 0 AND ((SELECT count() {voids}) = 0 OR (customer_id, id, ingested_at) \
             NOT IN (SELECT customer_id, id, ingested_at {voids}))"
        );
        if conditions.is_empty() {
            live
        } else {
            format!("({conditions}) AND {live}")
        }
    }
}

#[derive(Debug)]
pub struct SafeQuery {
    pub sql: String,
    pub binds: Vec<BindValue>,
}

impl SafeQuery {
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

/// A piece of SQL together with the positional binds it consumes, kept in lock-step.
///
/// Threading separate `Vec<BindValue>` next to a `String` makes the "Nth `?` ⇄ Nth bind"
/// invariant the caller's problem (see the carry-over gauge / `simple_series` reassembly
/// comments). `Fragment` instead owns both: every method that emits a `?` pushes its bind at
/// the same moment, so order and count can't drift.
#[derive(Debug, Default)]
pub struct Fragment {
    pub sql: String,
    pub binds: Vec<BindValue>,
}

impl Fragment {
    pub fn new() -> Self {
        Fragment::default()
    }

    /// Append raw SQL that carries no bind.
    pub fn push_sql(&mut self, sql: &str) {
        self.sql.push_str(sql);
    }

    /// Append a single `?` placeholder and the value it binds, atomically.
    pub fn push_placeholder(&mut self, value: BindValue) {
        self.sql.push('?');
        self.binds.push(value);
    }

    /// Append `prefix`, then a `?` placeholder bound to `value`, then `suffix` — the common
    /// "wrap a single bound value in some surrounding SQL" shape, kept atomic.
    pub fn push_bound(&mut self, prefix: &str, value: BindValue, suffix: &str) {
        self.sql.push_str(prefix);
        self.push_placeholder(value);
        self.sql.push_str(suffix);
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
    use super::*;

    fn source_sql(dedup: EventDedup, scope: &VoidScope) -> String {
        EventsTable::new("raw_events_v2")
            .source("meteroid", uuid::Uuid::from_u128(7), dedup)
            .from_sql("id, properties", "tenant_id = ? AND code = ?", scope)
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
    fn keep_last_dedups_the_live_events() {
        assert!(
            source_sql(EventDedup::LastIngested, &VoidScope::default())
                .ends_with(" ORDER BY ingested_at DESC LIMIT 1 BY id, customer_id )")
        );
    }
}
