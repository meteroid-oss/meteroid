use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString, IntoEnumIterator};

/// The API areas a secret key's permissions are granted on.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    EnumIter,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ApiResource {
    Customers,
    Subscriptions,
    Invoices,
    /// Payments, top-ups, credit grants, wallets, budgets and credit notes: what moves money.
    Payments,
    Catalog,
    /// Event ingestion and usage reads.
    Usage,
    Entitlements,
    Webhooks,
    BatchJobs,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    Display,
    EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AccessLevel {
    #[default]
    None,
    Read,
    Write,
}

/// A restricted key's grants; a resource left out is not accessible.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApiKeyPermissions(BTreeMap<ApiResource, AccessLevel>);

impl ApiKeyPermissions {
    pub fn new(grants: impl IntoIterator<Item = (ApiResource, AccessLevel)>) -> Self {
        Self(
            grants
                .into_iter()
                .filter(|(_, level)| *level != AccessLevel::None)
                .collect(),
        )
    }

    pub fn level(&self, resource: ApiResource) -> AccessLevel {
        self.0.get(&resource).copied().unwrap_or_default()
    }

    pub fn allows(&self, resource: ApiResource, write: bool) -> bool {
        let needed = if write {
            AccessLevel::Write
        } else {
            AccessLevel::Read
        };
        self.level(resource) >= needed
    }

    /// Every resource with its level, `None` included.
    pub fn grants(&self) -> impl Iterator<Item = (ApiResource, AccessLevel)> + '_ {
        ApiResource::iter().map(|resource| (resource, self.level(resource)))
    }

    pub fn to_db(&self) -> serde_json::Value {
        serde_json::to_value(&self.0).unwrap_or_default()
    }

    /// Unknown resources or levels grant nothing, so a value written by a newer version fails
    /// closed rather than open.
    pub fn from_db(value: serde_json::Value) -> Self {
        let raw: BTreeMap<String, String> = serde_json::from_value(value).unwrap_or_else(|err| {
            tracing::warn!("ignoring unreadable api key permissions: {err}");
            BTreeMap::new()
        });
        Self::new(
            raw.into_iter().filter_map(|(resource, level)| {
                Some((resource.parse().ok()?, level.parse().ok()?))
            }),
        )
    }
}

/// What a secret key may do. `None` permissions: the whole API.
pub fn key_allows(
    permissions: Option<&ApiKeyPermissions>,
    resource: ApiResource,
    write: bool,
) -> bool {
    permissions.is_none_or(|p| p.allows(resource, write))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_implies_read_and_missing_means_none() {
        let p = ApiKeyPermissions::new([
            (ApiResource::Customers, AccessLevel::Write),
            (ApiResource::Invoices, AccessLevel::Read),
        ]);
        assert!(p.allows(ApiResource::Customers, true));
        assert!(p.allows(ApiResource::Customers, false));
        assert!(p.allows(ApiResource::Invoices, false));
        assert!(!p.allows(ApiResource::Invoices, true));
        assert!(!p.allows(ApiResource::Payments, false));
    }

    #[test]
    fn unrestricted_key_allows_everything() {
        assert!(key_allows(None, ApiResource::Payments, true));
    }

    #[test]
    fn db_round_trip_and_unknown_values_fail_closed() {
        let p = ApiKeyPermissions::new([(ApiResource::Usage, AccessLevel::Write)]);
        assert_eq!(ApiKeyPermissions::from_db(p.to_db()), p);

        let stored = serde_json::json!({"usage": "admin", "teleport": "write", "catalog": "read"});
        let p = ApiKeyPermissions::from_db(stored);
        assert_eq!(p.level(ApiResource::Usage), AccessLevel::None);
        assert_eq!(p.level(ApiResource::Catalog), AccessLevel::Read);

        let p = ApiKeyPermissions::from_db(serde_json::json!(["nonsense"]));
        assert!(p.grants().all(|(_, level)| level == AccessLevel::None));
    }
}
