//! The one timestamp type of the REST API and its webhook payloads.

use std::fmt;

use chrono::{DateTime, FixedOffset, NaiveDateTime, SecondsFormat, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// A point in time, written as RFC 3339 in UTC with millisecond precision, e.g.
/// `2026-10-30T09:12:33.123Z`, which is what the `date-time` format of the spec promises.
///
/// Inputs may use any offset, or none for UTC. The OpenAPI schema comes from the `ApiDateTime`
/// alias in `build.rs`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ApiDateTime(DateTime<Utc>);

impl ApiDateTime {
    pub fn now() -> Self {
        Self(Utc::now())
    }

    pub fn to_utc(self) -> DateTime<Utc> {
        self.0
    }

    pub fn naive_utc(self) -> NaiveDateTime {
        self.0.naive_utc()
    }
}

impl From<DateTime<Utc>> for ApiDateTime {
    fn from(value: DateTime<Utc>) -> Self {
        Self(value)
    }
}

/// Timestamps stored without a time zone are UTC.
impl From<NaiveDateTime> for ApiDateTime {
    fn from(value: NaiveDateTime) -> Self {
        Self(value.and_utc())
    }
}

impl From<ApiDateTime> for DateTime<Utc> {
    fn from(value: ApiDateTime) -> Self {
        value.0
    }
}

impl From<ApiDateTime> for NaiveDateTime {
    fn from(value: ApiDateTime) -> Self {
        value.0.naive_utc()
    }
}

impl fmt::Display for ApiDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.to_rfc3339_opts(SecondsFormat::Millis, true))
    }
}

impl std::str::FromStr for ApiDateTime {
    type Err = chrono::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.parse::<DateTime<FixedOffset>>() {
            Ok(at) => Ok(Self(at.to_utc())),
            Err(error) => s
                .parse::<NaiveDateTime>()
                .map(Self::from)
                .map_err(|_| error),
        }
    }
}

impl Serialize for ApiDateTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ApiDateTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_utc_with_milliseconds() {
        let at: ApiDateTime = "2026-10-30T09:12:33.123456".parse().unwrap();
        assert_eq!(
            serde_json::to_value(at).unwrap(),
            "2026-10-30T09:12:33.123Z"
        );
        let whole: ApiDateTime = "2026-10-30T11:12:33+02:00".parse().unwrap();
        assert_eq!(whole.to_string(), "2026-10-30T09:12:33.000Z");
    }

    #[test]
    fn rejects_what_is_not_a_timestamp() {
        assert!("2026-10-30".parse::<ApiDateTime>().is_err());
        assert!(serde_json::from_value::<ApiDateTime>("yesterday".into()).is_err());
    }
}
