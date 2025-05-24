use chrono::{DateTime, NaiveDateTime, Utc};

pub trait JsonFieldExtractor {
    fn get_string(&self, field: &str) -> Option<String>;

    fn get_f64(&self, field: &str) -> Option<f64>;

    fn get_timestamp_utc(&self, field: &str) -> Option<DateTime<Utc>> {
        self.get_string(field)
            .and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S%.f").ok())
            .map(|dt| dt.and_utc())
    }
}

impl JsonFieldExtractor for serde_json::Value {
    fn get_string(&self, field: &str) -> Option<String> {
        self.get(field)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    fn get_f64(&self, field: &str) -> Option<f64> {
        self.get(field).and_then(|v| v.as_f64())
    }
}
