pub mod query_raw;

#[derive(Debug)]
pub enum BindValue {
    String(String),
    Strings(Vec<String>),
    I64(i64),
    U32(u32),
    Uuid(uuid::Uuid),
    Uuids(Vec<uuid::Uuid>),
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

pub struct PropertyColumn<'a>(pub &'a str);

impl<'a> PropertyColumn<'a> {
    const ALIAS_PREFIX: &'static str = "_prop_";

    pub fn raw_name(&self) -> &'a str {
        self.0
    }

    pub fn as_alias(&self) -> String {
        let sanitized: String = self
            .0
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("{}{}", Self::ALIAS_PREFIX, sanitized)
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
