use std::borrow::Cow;
use std::fmt;
use std::ops::Deref;

use serde::Deserialize;
use serde::de::{Deserializer, Visitor};

/// Normalizes CSV bytes to UTF-8.
///
/// CSV files from Excel are typically UTF-8 (with/without BOM), UTF-16 LE/BE,
/// or Windows-1252. We check for these in order and default to Windows-1252
/// for any non-UTF-8 input, which is the standard legacy encoding for
/// Western European Excel exports.
///
/// Returns `Cow::Borrowed` if the input is already valid UTF-8 (zero-copy fast path).
pub fn normalize_csv_encoding(data: &[u8]) -> Cow<'_, [u8]> {
    // 1. UTF-8 without BOM — zero-copy fast path
    if !data.starts_with(&[0xEF, 0xBB, 0xBF]) && std::str::from_utf8(data).is_ok() {
        return Cow::Borrowed(data);
    }

    // 2. Pick encoding based on BOM, default to Windows-1252
    let encoding = if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        encoding_rs::UTF_8 // strips the BOM
    } else if data.starts_with(&[0xFF, 0xFE]) {
        encoding_rs::UTF_16LE
    } else if data.starts_with(&[0xFE, 0xFF]) {
        encoding_rs::UTF_16BE
    } else {
        encoding_rs::WINDOWS_1252
    };

    let (cow, actual_encoding, had_errors) = encoding.decode(data);
    if had_errors {
        tracing::warn!("Encoding conversion had replacement characters");
    }

    if actual_encoding != encoding_rs::UTF_8 {
        tracing::info!(
            encoding = %actual_encoding.name(),
            "CSV encoding detected, converted to UTF-8"
        );
    }

    Cow::Owned(cow.as_bytes().to_vec())
}

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
                v.trim()
                    .parse::<Decimal>()
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

pub mod optional_country_code {
    use common_domain::country::CountryCode;
    use serde::Deserializer;
    use serde::de::{self, Visitor};
    use std::fmt;

    struct OptionalCountryCodeVisitor;

    impl<'de> Visitor<'de> for OptionalCountryCodeVisitor {
        type Value = Option<CountryCode>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an ISO 3166-1 alpha-2 country code (e.g., 'US', 'FR', 'DE')")
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
            let trimmed = v.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                CountryCode::parse_as_opt(trimmed).map(Some).ok_or_else(|| {
                    de::Error::custom(format!(
                        "invalid country '{}' — expected ISO 3166-1 alpha-2 code (e.g., 'US', 'FR', 'DE')",
                        trimmed
                    ))
                })
            }
        }

        fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&v)
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<CountryCode>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_option(OptionalCountryCodeVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_utf8_passthrough() {
        let data = b"name,currency\nCaf\xc3\xa9,EUR\n"; // "Café" in UTF-8
        let result = normalize_csv_encoding(data);
        // Should be borrowed (zero-copy) since it's already valid UTF-8
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result.as_ref(), data.as_slice());
    }

    #[test]
    fn test_normalize_utf8_bom() {
        let mut data = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
        data.extend_from_slice(b"name,currency\n");
        let result = normalize_csv_encoding(&data);
        assert_eq!(result.as_ref(), b"name,currency\n");
    }

    #[test]
    fn test_normalize_utf16le_bom() {
        // "name\n" encoded as UTF-16 LE with BOM
        let data: Vec<u8> = vec![
            0xFF, 0xFE, // UTF-16 LE BOM
            b'n', 0x00, b'a', 0x00, b'm', 0x00, b'e', 0x00, b'\n', 0x00,
        ];
        let result = normalize_csv_encoding(&data);
        assert_eq!(result.as_ref(), b"name\n");
    }

    #[test]
    fn test_normalize_windows1252() {
        // "Café" in Windows-1252: 0xE9 is é
        let data = b"name,currency\nCaf\xe9,EUR\n";
        let result = normalize_csv_encoding(data);
        let result_str = std::str::from_utf8(result.as_ref()).unwrap();
        assert!(result_str.contains("Café"));
    }
}
