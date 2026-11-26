//! Resource types and resources (ADR 0011 §3.4): one level below the customer in v1.
use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, ResourceId, ResourceTypeId, TenantId};
use std::collections::BTreeMap;

/// `{"<type code>.<attribute>": "<value>"}`, what conditions match against.
pub type Attributes = BTreeMap<String, String>;

/// The path of the customer itself.
pub const CUSTOMER_PATH: &str = "";

#[derive(Clone, Debug, PartialEq)]
pub struct ResourceType {
    pub id: ResourceTypeId,
    pub tenant_id: TenantId,
    pub code: String,
    pub name: String,
    pub plural_name: String,
    /// Canonical first, then aliases.
    pub keys: Vec<String>,
    /// Each attribute with its closed list of values.
    pub attributes: BTreeMap<String, Vec<String>>,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    pub id: ResourceId,
    pub customer_id: CustomerId,
    pub resource_type_id: ResourceTypeId,
    pub key_value: String,
    pub name: Option<String>,
    /// `{"<attribute>": "<value>"}`, without the type code.
    pub attributes: BTreeMap<String, String>,
    pub archived_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceRef {
    Stored(ResourceId),
    AdHoc {
        type_id: ResourceTypeId,
        key_value: String,
    },
}

impl ResourceRef {
    pub fn stored_id(&self) -> Option<ResourceId> {
        match self {
            ResourceRef::Stored(id) => Some(*id),
            ResourceRef::AdHoc { .. } => None,
        }
    }
}

/// A check's `resources` (`{"site_id": "s1"}`, any key or alias) and asserted `attributes`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceInput {
    pub resources: BTreeMap<String, String>,
    pub attributes: Attributes,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedResource {
    pub type_id: ResourceTypeId,
    pub type_code: String,
    /// The value of whichever key the call used, which counters key by canonically.
    pub key_value: String,
    pub path: String,
    pub reference: ResourceRef,
    /// Stored attributes, then the asserted ones the stored resource lacks.
    pub attributes: Attributes,
    /// Asserted attributes the stored ones overrode.
    pub attributes_ignored: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResourceInputError {
    #[error("a check names at most one resource")]
    TooManyResources,
    #[error("`{0}` is not a key of any resource type")]
    UnknownKey(String),
    #[error("`{0}` needs a non-empty value")]
    EmptyKeyValue(String),
    #[error("`{0}` is not an attribute of the resource the check names")]
    UnknownAttribute(String),
    #[error("`{value}` is not a value of `{attribute}`")]
    UnknownAttributeValue { attribute: String, value: String },
}

impl From<ResourceInputError> for crate::errors::StoreErrorReport {
    fn from(error: ResourceInputError) -> Self {
        crate::errors::StoreError::InvalidArgument(error.to_string()).into()
    }
}

impl ResourceType {
    /// Attributes without the type code: an undeclared attribute or value is refused.
    pub fn validate_attributes(
        &self,
        attributes: &BTreeMap<String, String>,
    ) -> Result<(), ResourceInputError> {
        attributes.iter().try_for_each(|(attribute, value)| {
            let qualified = || format!("{}.{attribute}", self.code);
            match self.attributes.get(attribute) {
                None => Err(ResourceInputError::UnknownAttribute(qualified())),
                Some(allowed) if !allowed.contains(value) => {
                    Err(ResourceInputError::UnknownAttributeValue {
                        attribute: qualified(),
                        value: value.clone(),
                    })
                }
                Some(_) => Ok(()),
            }
        })
    }
}

/// `site=s1`; `%`, `/` and `=` in the value are percent-encoded so a path splits unambiguously.
pub fn canonical_path(type_code: &str, key_value: &str) -> String {
    let mut path = format!("{type_code}=");
    for c in key_value.chars() {
        match c {
            '%' => path.push_str("%25"),
            '/' => path.push_str("%2F"),
            '=' => path.push_str("%3D"),
            c => path.push(c),
        }
    }
    path
}

/// The canonical key of the type that declares `key`; an undeclared key stays as named.
pub fn canonical_key<'a>(types: &'a [ResourceType], key: &'a str) -> &'a str {
    types
        .iter()
        .find(|t| t.keys.iter().any(|k| k == key))
        .and_then(|t| t.keys.first())
        .map_or(key, String::as_str)
}

/// The type code and key value of a path; `None` for the customer's.
pub fn split_path(path: &str) -> Option<(&str, String)> {
    let (code, encoded) = path.split_once('=')?;
    let value = encoded
        .replace("%2F", "/")
        .replace("%3D", "=")
        .replace("%25", "%");
    Some((code, value))
}

/// Resolves a check's resource against the tenant's types and the customer's stored resources for its key
/// value. `None` when the check names the customer only.
pub fn resolve_input(
    types: &[ResourceType],
    stored: &[Resource],
    input: &ResourceInput,
) -> Result<Option<ResolvedResource>, ResourceInputError> {
    let mut named = input.resources.iter();
    let (key, key_value) = match (named.next(), named.next()) {
        (_, Some(_)) => return Err(ResourceInputError::TooManyResources),
        (Some(named), None) => named,
        (None, None) => {
            return match input.attributes.keys().next() {
                Some(attribute) => Err(ResourceInputError::UnknownAttribute(attribute.clone())),
                None => Ok(None),
            };
        }
    };
    let resource_type = types
        .iter()
        .find(|t| t.keys.contains(key))
        .ok_or_else(|| ResourceInputError::UnknownKey(key.clone()))?;
    if key_value.is_empty() {
        return Err(ResourceInputError::EmptyKeyValue(key.clone()));
    }

    let prefix = format!("{}.", resource_type.code);
    let asserted = input
        .attributes
        .iter()
        .map(|(qualified, value)| match qualified.strip_prefix(&prefix) {
            Some(attribute) => Ok((attribute.to_owned(), value.clone())),
            None => Err(ResourceInputError::UnknownAttribute(qualified.clone())),
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    resource_type.validate_attributes(&asserted)?;

    let stored = stored
        .iter()
        .find(|r| r.resource_type_id == resource_type.id && r.key_value == *key_value);
    let stored_attributes = stored.map(|r| &r.attributes);
    let attributes_ignored = asserted
        .iter()
        .filter(|(a, v)| {
            stored_attributes
                .and_then(|s| s.get(*a))
                .is_some_and(|s| s != *v)
        })
        .map(|(a, _)| format!("{prefix}{a}"))
        .collect();
    let mut merged = asserted;
    merged.extend(
        stored_attributes
            .into_iter()
            .flatten()
            .map(|(a, v)| (a.clone(), v.clone())),
    );

    Ok(Some(ResolvedResource {
        type_id: resource_type.id,
        type_code: resource_type.code.clone(),
        key_value: key_value.clone(),
        path: canonical_path(&resource_type.code, key_value),
        reference: stored.map_or_else(
            || ResourceRef::AdHoc {
                type_id: resource_type.id,
                key_value: key_value.clone(),
            },
            |r| ResourceRef::Stored(r.id),
        ),
        attributes: merged
            .into_iter()
            .map(|(a, v)| (format!("{prefix}{a}"), v))
            .collect(),
        attributes_ignored,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;

    fn site() -> ResourceType {
        ResourceType {
            id: ResourceTypeId::new(),
            tenant_id: TenantId::new(),
            code: "site".into(),
            name: "Site".into(),
            plural_name: "Sites".into(),
            keys: vec!["site_id".into(), "website_id".into()],
            attributes: BTreeMap::from([(
                "environment".to_owned(),
                vec!["production".to_owned(), "staging".to_owned()],
            )]),
            archived_at: None,
            created_at: common_clock::now(),
        }
    }

    fn stored(t: &ResourceType, key_value: &str, environment: &str) -> Resource {
        Resource {
            id: ResourceId::new(),
            customer_id: CustomerId::new(),
            resource_type_id: t.id,
            key_value: key_value.into(),
            name: None,
            attributes: BTreeMap::from([("environment".to_owned(), environment.to_owned())]),
            archived_at: None,
        }
    }

    fn input(resources: &[(&str, &str)], attributes: &[(&str, &str)]) -> ResourceInput {
        let map = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        ResourceInput {
            resources: map(resources),
            attributes: map(attributes),
        }
    }

    #[test]
    fn paths_are_canonical_and_round_trip() {
        assert_eq!(canonical_path("site", "s1"), "site=s1");
        assert_eq!(canonical_path("site", "a/b=c%d"), "site=a%2Fb%3Dc%25d");
        for value in ["s1", "a/b=c%d", "%2F", "x==y//", "100%"] {
            let path = canonical_path("site", value);
            assert_eq!(
                split_path(&path),
                Some(("site", value.to_owned())),
                "{value}"
            );
        }
        assert_ne!(canonical_path("site", "s1"), canonical_path("site", "s10"));
        assert_eq!(split_path(CUSTOMER_PATH), None);
    }

    #[test]
    fn an_alias_resolves_to_the_canonical_path() {
        let t = site();
        let types = [t.clone()];
        let by_key = resolve_input(&types, &[], &input(&[("site_id", "s1")], &[])).unwrap();
        let by_alias = resolve_input(&types, &[], &input(&[("website_id", "s1")], &[])).unwrap();
        assert_eq!(by_key, by_alias);
        let resolved = by_key.unwrap();
        assert_eq!(resolved.path, "site=s1");
        assert_eq!(
            resolved.reference,
            ResourceRef::AdHoc {
                type_id: t.id,
                key_value: "s1".into()
            }
        );
    }

    #[test]
    fn the_customer_only() {
        assert_eq!(
            resolve_input(&[site()], &[], &ResourceInput::default()),
            Ok(None)
        );
    }

    #[test]
    fn refusals() {
        let types = [site()];
        let cases = [
            (
                input(&[("site_id", "s1"), ("website_id", "s2")], &[]),
                ResourceInputError::TooManyResources,
            ),
            (
                input(&[("user_id", "u1")], &[]),
                ResourceInputError::UnknownKey("user_id".into()),
            ),
            (
                input(&[("site_id", "")], &[]),
                ResourceInputError::EmptyKeyValue("site_id".into()),
            ),
            (
                input(&[("site_id", "s1")], &[("site.region", "eu")]),
                ResourceInputError::UnknownAttribute("site.region".into()),
            ),
            (
                input(&[("site_id", "s1")], &[("tenant.environment", "staging")]),
                ResourceInputError::UnknownAttribute("tenant.environment".into()),
            ),
            (
                input(&[("site_id", "s1")], &[("site.environment", "qa")]),
                ResourceInputError::UnknownAttributeValue {
                    attribute: "site.environment".into(),
                    value: "qa".into(),
                },
            ),
            (
                input(&[], &[("site.environment", "staging")]),
                ResourceInputError::UnknownAttribute("site.environment".into()),
            ),
        ];
        for (given, expected) in cases {
            assert_eq!(
                resolve_input(&types, &[], &given),
                Err(expected),
                "{given:?}"
            );
        }
    }

    #[test]
    fn stored_attributes_win_over_asserted_ones() {
        let t = site();
        let s1 = stored(&t, "s1", "production");
        let other_type = Resource {
            resource_type_id: ResourceTypeId::new(),
            ..stored(&t, "s2", "staging")
        };
        let resolved = resolve_input(
            &[t],
            &[other_type, s1.clone()],
            &input(&[("website_id", "s1")], &[("site.environment", "staging")]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(resolved.reference, ResourceRef::Stored(s1.id));
        assert_eq!(resolved.attributes["site.environment"], "production");
        assert_eq!(
            resolved.attributes_ignored,
            vec!["site.environment".to_owned()]
        );
    }

    #[test]
    fn an_assertion_matching_or_missing_from_the_stored_resource_is_not_ignored() {
        let t = site();
        let mut bare = stored(&t, "s1", "production");
        bare.attributes.clear();
        let agrees = stored(&t, "s2", "staging");
        for (resource, key_value) in [(bare, "s1"), (agrees, "s2")] {
            let resolved = resolve_input(
                std::slice::from_ref(&t),
                &[resource],
                &input(
                    &[("site_id", key_value)],
                    &[("site.environment", "staging")],
                ),
            )
            .unwrap()
            .unwrap();
            assert_eq!(resolved.attributes["site.environment"], "staging");
            assert!(resolved.attributes_ignored.is_empty());
        }
    }

    #[test]
    fn an_ad_hoc_resource_keeps_its_asserted_attributes() {
        let resolved = resolve_input(
            &[site()],
            &[],
            &input(&[("site_id", "s9")], &[("site.environment", "staging")]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            resolved.attributes,
            Attributes::from([("site.environment".into(), "staging".into())])
        );
        assert!(resolved.reference.stored_id().is_none());
    }
}
