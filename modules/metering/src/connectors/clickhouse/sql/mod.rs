pub mod init;
pub mod query_raw;

pub const DATABASE: &str = "meteroid"; // TODO config

fn escape_sql_identifier(identifier: &str) -> String {
    identifier.replace('\'', "''")
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
