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

/// How a raw-events read collapses re-sent event ids (ADR-0011 §4).
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
