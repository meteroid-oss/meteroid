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
