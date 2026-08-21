//! HTTP client for the GoCardless Pro API. Each resource module exposes a trait
//! implemented on `GoCardlessClient`. Only the
//! `meteroid-store::adapters::payment::gocardless` adapter should use this crate.

pub mod billing_requests;
pub mod client;
pub mod customer_bank_accounts;
pub mod customers;
pub mod error;
pub mod mandates;
pub mod payments;
pub mod refunds;
pub mod request;
pub mod webhook;

/// Deserialize a value that may be JSON `null` into `T::default()`.
///
/// GoCardless returns `"metadata": null` for resources created without
/// metadata; `#[serde(default)]` alone only covers an absent key, not an
/// explicit null, so a bare `HashMap` field would fail to parse.
pub(crate) fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Ok(<Option<T> as serde::Deserialize>::deserialize(deserializer)?.unwrap_or_default())
}

/// GoCardless caps metadata at 3 keys, key names at 50 chars and values at 500
/// chars. Enforce it client-side so the create fails fast with a clear error
/// instead of a round-trip 422.
pub(crate) fn validate_metadata(
    metadata: Option<&std::collections::HashMap<String, String>>,
) -> Result<(), crate::error::GoCardlessError> {
    let Some(m) = metadata else {
        return Ok(());
    };
    if m.len() > 3 {
        return Err(crate::error::client_validation(format!(
            "gocardless metadata allows at most 3 keys, got {}",
            m.len()
        )));
    }
    for (k, v) in m {
        if k.chars().count() > 50 {
            return Err(crate::error::client_validation(format!(
                "gocardless metadata key {k:?} exceeds 50 characters"
            )));
        }
        if v.chars().count() > 500 {
            return Err(crate::error::client_validation(format!(
                "gocardless metadata value for key {k:?} exceeds 500 characters"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod metadata_tests {
    use super::validate_metadata;
    use std::collections::HashMap;

    #[test]
    fn none_and_within_limits_ok() {
        assert!(validate_metadata(None).is_ok());
        let ok = HashMap::from([
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "2".to_string()),
            ("c".to_string(), "3".to_string()),
        ]);
        assert!(validate_metadata(Some(&ok)).is_ok());
    }

    #[test]
    fn too_many_keys_rejected() {
        let m = HashMap::from([
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "2".to_string()),
            ("c".to_string(), "3".to_string()),
            ("d".to_string(), "4".to_string()),
        ]);
        assert!(validate_metadata(Some(&m)).is_err());
    }

    #[test]
    fn oversized_value_and_key_rejected() {
        let long_val = HashMap::from([("k".to_string(), "x".repeat(501))]);
        assert!(validate_metadata(Some(&long_val)).is_err());

        let long_key = HashMap::from([("k".repeat(51), "v".to_string())]);
        assert!(validate_metadata(Some(&long_key)).is_err());
    }
}
