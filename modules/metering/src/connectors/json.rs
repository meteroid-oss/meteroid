use chrono::{DateTime, NaiveDateTime, Utc};
use serde::de::DeserializeOwned;
use uuid::Uuid;

pub trait JsonFieldExtractor {
    fn get<T: DeserializeOwned>(&self, field: &str) -> Option<T>;

    fn get_string(&self, field: &str) -> Option<String> {
        self.get::<String>(field)
    }

    fn get_f64(&self, field: &str) -> Option<f64> {
        self.get::<f64>(field)
    }

    fn get_uuid(&self, field: &str) -> Option<Uuid> {
        self.get_string(field)
            .and_then(|s| Uuid::parse_str(&s).ok())
    }

    fn get_id<ID: From<Uuid>>(&self, field: &str) -> Option<ID> {
        self.get_uuid(field).map(ID::from)
    }

    fn get_timestamp_utc(&self, field: &str) -> Option<DateTime<Utc>> {
        self.get_string(field)
            .and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S%.f").ok())
            .map(|dt| dt.and_utc())
    }
}

impl JsonFieldExtractor for serde_json::Value {
    fn get<T: DeserializeOwned>(&self, field: &str) -> Option<T> {
        self.get(field).and_then(|v| T::deserialize(v).ok())
    }
}
