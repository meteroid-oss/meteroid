pub mod create_meter;
pub mod init;
pub mod query_meter;
pub mod query_raw;

pub const DATABASE: &str = "meteroid"; // TODO config

const METER_TABLE_PREFIX: &str = "METER";

fn escape_sql_identifier(identifier: &str) -> String {
    identifier.replace('\'', "''")
}

fn encode_identifier(identifier: &str) -> String {
    identifier
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect()
}

pub fn get_meter_view_name(namespace: &str, meter_slug: &str) -> String {
    format!(
        "{}.{}_NS{}_M{}",
        DATABASE,
        METER_TABLE_PREFIX,
        encode_identifier(namespace),
        encode_identifier(meter_slug)
    )
}

struct Column {
    name: String,
    col_type: String,
}

pub struct PropertyColumn<'a>(&'a str);

impl<'a> PropertyColumn<'a> {
    const ALIAS_PREFIX: &'static str = "_prop_";

    pub fn raw_name(&self) -> &'a str {
        self.0
    }

    pub fn as_alias(&self) -> String {
        format!("{}{}", Self::ALIAS_PREFIX, self.raw_name())
    }

    pub fn path(&self) -> String {
        let escaped = escape_sql_identifier(self.0);
        format!("properties['{escaped}']")
    }

    pub fn as_select(&self) -> String {
        let path = self.path();
        let alias = self.as_alias();
        format!("{path} AS {alias}")
    }

    pub fn from_str_ref(s: &'a str) -> Self {
        PropertyColumn(s)
    }
}
