use serde::Deserialize;
use serde::de::{Deserializer, Visitor};
use std::fmt;
use std::ops::Deref;

/// A string-like type that accepts numbers or strings during deserialization.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CsvString(pub String);

impl Deref for CsvString {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<&str> for CsvString {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<CsvString> for String {
    fn from(f: CsvString) -> Self {
        f.0
    }
}

impl fmt::Display for CsvString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'de> Deserialize<'de> for CsvString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CsvStringVisitor;

        impl<'de> Visitor<'de> for CsvStringVisitor {
            type Value = CsvString;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a string or a primitive convertible to string")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
                Ok(CsvString(v.to_owned()))
            }
            fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
                Ok(CsvString(v))
            }

            // Numbers -> string
            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
                Ok(CsvString(v.to_string()))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
                Ok(CsvString(v.to_string()))
            }
            fn visit_i128<E>(self, v: i128) -> Result<Self::Value, E> {
                Ok(CsvString(v.to_string()))
            }
            fn visit_u128<E>(self, v: u128) -> Result<Self::Value, E> {
                Ok(CsvString(v.to_string()))
            }
            fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E> {
                Ok(CsvString(v.to_string()))
            }

            // Bool -> "true"/"false"
            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
                Ok(CsvString(v.to_string()))
            }
        }

        deserializer.deserialize_any(CsvStringVisitor)
    }
}

pub mod optional_csv_string {
    use super::CsvString;
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<CsvString>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let csv_string: CsvString = CsvString::deserialize(deserializer)?;
        if csv_string.0.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(csv_string))
        }
    }
}

pub mod optional_bool {
    use serde::Deserializer;
    use serde::de::{self, Visitor};
    use std::fmt;

    struct OptionalBoolVisitor;

    impl<'de> Visitor<'de> for OptionalBoolVisitor {
        type Value = Option<bool>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a boolean or a string representing a boolean")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_any(self)
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if v.trim().is_empty() {
                Ok(None)
            } else {
                v.parse::<bool>().map(Some).map_err(de::Error::custom)
            }
        }

        fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&v)
        }

        fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(v))
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_option(OptionalBoolVisitor)
    }
}

pub mod optional_u16 {
    use serde::Deserializer;
    use serde::de::{self, Visitor};
    use std::fmt;

    struct OptionalU16Visitor;

    impl<'de> Visitor<'de> for OptionalU16Visitor {
        type Value = Option<u16>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an integer or a string representing a u16")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_any(self)
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if v.trim().is_empty() {
                Ok(None)
            } else {
                v.trim().parse::<u16>().map(Some).map_err(de::Error::custom)
            }
        }

        fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&v)
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u16::try_from(v).map(Some).map_err(de::Error::custom)
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u16::try_from(v).map(Some).map_err(de::Error::custom)
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<u16>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_option(OptionalU16Visitor)
    }
}

pub mod optional_u32 {
    use serde::Deserializer;
    use serde::de::{self, Visitor};
    use std::fmt;

    struct OptionalU32Visitor;

    impl<'de> Visitor<'de> for OptionalU32Visitor {
        type Value = Option<u32>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an integer or a string representing a u32")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_any(self)
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if v.trim().is_empty() {
                Ok(None)
            } else {
                v.trim().parse::<u32>().map(Some).map_err(de::Error::custom)
            }
        }

        fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&v)
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u32::try_from(v).map(Some).map_err(de::Error::custom)
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u32::try_from(v).map(Some).map_err(de::Error::custom)
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_option(OptionalU32Visitor)
    }
}

pub mod optional_naive_date {
    use chrono::NaiveDate;
    use serde::Deserializer;
    use serde::de::{self, Visitor};
    use std::fmt;

    struct OptionalNaiveDateVisitor;

    impl<'de> Visitor<'de> for OptionalNaiveDateVisitor {
        type Value = Option<NaiveDate>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a date string in YYYY-MM-DD format or empty")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_any(self)
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if v.trim().is_empty() {
                Ok(None)
            } else {
                NaiveDate::parse_from_str(v.trim(), "%Y-%m-%d")
                    .map(Some)
                    .map_err(de::Error::custom)
            }
        }

        fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&v)
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<NaiveDate>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_option(OptionalNaiveDateVisitor)
    }
}

pub mod optional_decimal {
    use rust_decimal::Decimal;
    use serde::Deserializer;
    use serde::de::{self, Visitor};
    use std::fmt;

    struct OptionalDecimalVisitor;

    impl<'de> Visitor<'de> for OptionalDecimalVisitor {
        type Value = Option<Decimal>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a decimal number or a string representing a decimal")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_any(self)
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if v.trim().is_empty() {
                Ok(None)
            } else {
                v.parse::<f64>()
                    .map_err(de::Error::custom)
                    .and_then(|f| Decimal::try_from(f).map_err(de::Error::custom))
                    .map(Some)
            }
        }

        fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&v)
        }

        fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Decimal::try_from(v).map(Some).map_err(de::Error::custom)
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(Decimal::from(v)))
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(Decimal::from(v)))
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Decimal>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_option(OptionalDecimalVisitor)
    }
}
