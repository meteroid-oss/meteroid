use crate::domain::enums::{CustomPropertyEntityType, CustomPropertyType};
use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
use common_domain::ids::{CustomPropertyDefinitionId, TenantId};
use diesel_models::custom_property_definitions::CustomPropertyDefinitionRow;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A single choice for a `SINGLE_SELECT` / `MULTI_SELECT` property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Type-specific configuration for a property definition. All fields are optional; only the ones
/// relevant to the property's `property_type` are interpreted.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PropertyConfig {
    /// Allowed choices for `SINGLE_SELECT` / `MULTI_SELECT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<SelectOption>>,
    /// Inclusive numeric bounds for `NUMBER`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Maximum length for `TEXT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
}
json_value_serde!(PropertyConfig);

impl PropertyConfig {
    fn option_values(&self) -> Vec<&str> {
        self.options
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|o| o.value.as_str())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CustomPropertyDefinition {
    pub id: CustomPropertyDefinitionId,
    pub tenant_id: TenantId,
    pub entity_type: CustomPropertyEntityType,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub property_type: CustomPropertyType,
    pub config: PropertyConfig,
    pub required: bool,
    pub default_value: Option<serde_json::Value>,
    pub display_order: i32,
    pub archived_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone)]
pub struct CustomPropertyDefinitionNew {
    pub tenant_id: TenantId,
    pub entity_type: CustomPropertyEntityType,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub property_type: CustomPropertyType,
    pub config: PropertyConfig,
    pub required: bool,
    pub default_value: Option<serde_json::Value>,
    pub display_order: i32,
}

/// Sparse update. `key`, `entity_type` and `property_type` are immutable once values may exist,
/// so they are absent here. `Option<Option<_>>` distinguishes "unchanged" from "set to null".
#[derive(Debug, Clone)]
pub struct CustomPropertyDefinitionPatch {
    pub id: CustomPropertyDefinitionId,
    pub tenant_id: TenantId,
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub config: Option<PropertyConfig>,
    pub required: Option<bool>,
    pub default_value: Option<Option<serde_json::Value>>,
    pub display_order: Option<i32>,
    /// Set to archive/unarchive. Outer `None` leaves it unchanged.
    pub archived_at: Option<Option<NaiveDateTime>>,
}

impl TryFrom<CustomPropertyDefinitionRow> for CustomPropertyDefinition {
    type Error = StoreErrorReport;

    fn try_from(row: CustomPropertyDefinitionRow) -> Result<Self, Self::Error> {
        Ok(CustomPropertyDefinition {
            id: row.id,
            tenant_id: row.tenant_id,
            entity_type: row.entity_type.into(),
            key: row.key,
            name: row.name,
            description: row.description,
            property_type: row.property_type.into(),
            config: row.config.try_into()?,
            required: row.required,
            default_value: row.default_value,
            display_order: row.display_order,
            archived_at: row.archived_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// Write semantics for a custom-property bag.
///
/// - `Replace` (entity create / full update): the provided map is authoritative — `required`
///   properties must be present (or have a default), missing defaults are filled, and any prior
///   values are discarded.
/// - `Merge` (entity patch): the provided map is a delta applied on top of the existing values —
///   only the provided keys change, a `null` value deletes a key (Stripe-style), and `required`
///   is not re-checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomPropertyMode {
    Replace,
    Merge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomPropertyValidationError {
    UnknownKey(String),
    MissingRequired(String),
    WrongType {
        key: String,
        expected: CustomPropertyType,
    },
    NotAnOption {
        key: String,
        value: String,
    },
    OutOfRange(String),
    TooLong(String),
    InvalidFormat {
        key: String,
        expected: CustomPropertyType,
    },
}

impl std::fmt::Display for CustomPropertyValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKey(k) => write!(f, "unknown custom property '{k}'"),
            Self::MissingRequired(k) => write!(f, "missing required custom property '{k}'"),
            Self::WrongType { key, expected } => {
                write!(f, "custom property '{key}' must be of type {expected}")
            }
            Self::NotAnOption { key, value } => {
                write!(
                    f,
                    "'{value}' is not an allowed option for custom property '{key}'"
                )
            }
            Self::OutOfRange(k) => write!(f, "custom property '{k}' is out of range"),
            Self::TooLong(k) => write!(f, "custom property '{k}' exceeds its maximum length"),
            Self::InvalidFormat { key, expected } => {
                write!(f, "custom property '{key}' is not a valid {expected}")
            }
        }
    }
}

fn as_object(
    value: &serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>, CustomPropertyValidationError> {
    match value {
        serde_json::Value::Null => Ok(serde_json::Map::new()),
        serde_json::Value::Object(map) => Ok(map.clone()),
        // A non-object payload can't be a valid property bag.
        _ => Err(CustomPropertyValidationError::UnknownKey(
            "<root>".to_string(),
        )),
    }
}

/// Validates and normalizes a bag of custom-property values against a set of definitions.
///
/// - unknown keys are always rejected (strict, HubSpot-style typed system);
/// - each provided value is type-checked against its definition;
/// - `SINGLE_SELECT`/`MULTI_SELECT` values must be among the configured options;
/// - see [`CustomPropertyMode`] for `Replace` vs `Merge` semantics.
///
/// `existing` is the entity's current property bag; it is only consulted in `Merge` mode.
/// Returns the full normalized map to persist on the entity.
pub fn validate_and_normalize(
    definitions: &[CustomPropertyDefinition],
    provided: &serde_json::Value,
    existing: &serde_json::Value,
    mode: CustomPropertyMode,
) -> Result<serde_json::Value, CustomPropertyValidationError> {
    let provided = as_object(provided)?;

    let by_key: BTreeMap<&str, &CustomPropertyDefinition> =
        definitions.iter().map(|d| (d.key.as_str(), d)).collect();

    // Reject anything not backed by a definition, regardless of mode.
    for key in provided.keys() {
        if !by_key.contains_key(key.as_str()) {
            return Err(CustomPropertyValidationError::UnknownKey(key.clone()));
        }
    }

    match mode {
        CustomPropertyMode::Replace => {
            let mut out = serde_json::Map::new();
            for def in definitions {
                match provided.get(&def.key) {
                    Some(serde_json::Value::Null) | None => {
                        if let Some(default) = &def.default_value {
                            out.insert(def.key.clone(), default.clone());
                        } else if def.required {
                            return Err(CustomPropertyValidationError::MissingRequired(
                                def.key.clone(),
                            ));
                        }
                    }
                    Some(value) => {
                        out.insert(def.key.clone(), validate_value(def, value)?);
                    }
                }
            }
            Ok(serde_json::Value::Object(out))
        }
        CustomPropertyMode::Merge => {
            let mut out = as_object(existing)?;
            for (key, value) in &provided {
                // key is guaranteed to be a known definition (checked above).
                let def = by_key[key.as_str()];
                if value.is_null() {
                    out.remove(key);
                } else {
                    out.insert(key.clone(), validate_value(def, value)?);
                }
            }
            Ok(serde_json::Value::Object(out))
        }
    }
}

fn validate_value(
    def: &CustomPropertyDefinition,
    value: &serde_json::Value,
) -> Result<serde_json::Value, CustomPropertyValidationError> {
    let wrong = || CustomPropertyValidationError::WrongType {
        key: def.key.clone(),
        expected: def.property_type,
    };

    let invalid_format = || CustomPropertyValidationError::InvalidFormat {
        key: def.key.clone(),
        expected: def.property_type,
    };

    match def.property_type {
        CustomPropertyType::Text => {
            let s = value.as_str().ok_or_else(wrong)?;
            if def
                .config
                .max_length
                .is_some_and(|max| s.chars().count() > max as usize)
            {
                return Err(CustomPropertyValidationError::TooLong(def.key.clone()));
            }
            Ok(value.clone())
        }
        CustomPropertyType::Url => {
            let s = value.as_str().ok_or_else(wrong)?;
            match url::Url::parse(s) {
                Ok(u) if matches!(u.scheme(), "http" | "https") && u.has_host() => {
                    Ok(value.clone())
                }
                _ => Err(invalid_format()),
            }
        }
        CustomPropertyType::Email => {
            let s = value.as_str().ok_or_else(wrong)?;
            if validator::ValidateEmail::validate_email(&s) {
                Ok(value.clone())
            } else {
                Err(invalid_format())
            }
        }
        CustomPropertyType::Number => {
            let n = value.as_f64().ok_or_else(wrong)?;
            if def.config.min.is_some_and(|min| n < min)
                || def.config.max.is_some_and(|max| n > max)
            {
                return Err(CustomPropertyValidationError::OutOfRange(def.key.clone()));
            }
            Ok(value.clone())
        }
        CustomPropertyType::Boolean => {
            value.as_bool().ok_or_else(wrong)?;
            Ok(value.clone())
        }
        // Parse and normalize to a canonical shape so the public API exposes one stable format:
        // DATE -> `YYYY-MM-DD`, DATETIME -> RFC3339 UTC (e.g. `2026-07-17T14:30:00Z`).
        CustomPropertyType::Date => {
            let s = value.as_str().ok_or_else(wrong)?;
            let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| invalid_format())?;
            Ok(serde_json::Value::String(
                date.format("%Y-%m-%d").to_string(),
            ))
        }
        CustomPropertyType::Datetime => {
            let s = value.as_str().ok_or_else(wrong)?;
            let dt = parse_datetime(s).ok_or_else(invalid_format)?;
            Ok(serde_json::Value::String(
                dt.to_rfc3339_opts(SecondsFormat::Secs, true),
            ))
        }
        CustomPropertyType::SingleSelect => {
            let s = value.as_str().ok_or_else(wrong)?;
            let allowed = def.config.option_values();
            if !allowed.is_empty() && !allowed.contains(&s) {
                return Err(CustomPropertyValidationError::NotAnOption {
                    key: def.key.clone(),
                    value: s.to_string(),
                });
            }
            Ok(value.clone())
        }
        CustomPropertyType::MultiSelect => {
            let arr = value.as_array().ok_or_else(wrong)?;
            let allowed = def.config.option_values();
            for item in arr {
                let s = item.as_str().ok_or_else(wrong)?;
                if !allowed.is_empty() && !allowed.contains(&s) {
                    return Err(CustomPropertyValidationError::NotAnOption {
                        key: def.key.clone(),
                        value: s.to_string(),
                    });
                }
            }
            Ok(value.clone())
        }
        // JSON accepts any structured value as-is.
        CustomPropertyType::Json => Ok(value.clone()),
    }
}

/// Coerces a raw filter-value string into the canonical JSON shape used for *stored* values, so a
/// filter compares against exactly what's persisted (dates/datetimes normalized identically to
/// [`validate_value`]). Unlike writes, this does NOT enforce SELECT option-membership, so
/// legacy/removed option values remain filterable. Returns the scalar value; array wrapping for
/// `MULTI_SELECT` containment is the query layer's concern.
pub fn coerce_filter_value(
    def: &CustomPropertyDefinition,
    raw: &str,
) -> Result<serde_json::Value, CustomPropertyValidationError> {
    let invalid = || CustomPropertyValidationError::InvalidFormat {
        key: def.key.clone(),
        expected: def.property_type,
    };
    match def.property_type {
        CustomPropertyType::Text
        | CustomPropertyType::Url
        | CustomPropertyType::Email
        | CustomPropertyType::SingleSelect
        | CustomPropertyType::MultiSelect
        | CustomPropertyType::Json => Ok(serde_json::Value::String(raw.to_string())),
        CustomPropertyType::Number => {
            let n: f64 = raw.parse().map_err(|_| invalid())?;
            Ok(serde_json::json!(n))
        }
        CustomPropertyType::Boolean => {
            let b: bool = raw.parse().map_err(|_| invalid())?;
            Ok(serde_json::Value::Bool(b))
        }
        CustomPropertyType::Date => {
            let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| invalid())?;
            Ok(serde_json::Value::String(
                date.format("%Y-%m-%d").to_string(),
            ))
        }
        CustomPropertyType::Datetime => {
            let dt = parse_datetime(raw).ok_or_else(invalid)?;
            Ok(serde_json::Value::String(
                dt.to_rfc3339_opts(SecondsFormat::Secs, true),
            ))
        }
    }
}

/// Parses a datetime accepting both RFC3339 (with offset/`Z`) and the offset-less shape emitted by
/// the dashboard's `datetime-local` input (`YYYY-MM-DDTHH:MM[:SS]`), which is interpreted as UTC.
fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(DateTime::from_naive_utc_and_offset(naive, Utc));
        }
    }
    None
}

impl From<CustomPropertyValidationError> for StoreErrorReport {
    fn from(e: CustomPropertyValidationError) -> Self {
        StoreError::InvalidArgument(e.to_string()).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;
    use serde_json::json;

    fn def(key: &str, property_type: CustomPropertyType) -> CustomPropertyDefinition {
        CustomPropertyDefinition {
            id: CustomPropertyDefinitionId::new(),
            tenant_id: TenantId::new(),
            entity_type: CustomPropertyEntityType::Customer,
            key: key.to_string(),
            name: key.to_string(),
            description: None,
            property_type,
            config: PropertyConfig::default(),
            required: false,
            default_value: None,
            display_order: 0,
            archived_at: None,
            created_at: chrono::DateTime::UNIX_EPOCH.naive_utc(),
            updated_at: None,
        }
    }

    #[test]
    fn rejects_unknown_keys() {
        let defs = vec![def("known", CustomPropertyType::Text)];
        let err = validate_and_normalize(
            &defs,
            &json!({ "surprise": "x" }),
            &json!({}),
            CustomPropertyMode::Replace,
        )
        .unwrap_err();
        assert_eq!(
            err,
            CustomPropertyValidationError::UnknownKey("surprise".into())
        );
    }

    #[test]
    fn type_checks_values() {
        let defs = vec![def("count", CustomPropertyType::Number)];
        assert!(matches!(
            validate_and_normalize(
                &defs,
                &json!({ "count": "nope" }),
                &json!({}),
                CustomPropertyMode::Replace
            ),
            Err(CustomPropertyValidationError::WrongType { .. })
        ));
        let ok = validate_and_normalize(
            &defs,
            &json!({ "count": 42 }),
            &json!({}),
            CustomPropertyMode::Replace,
        )
        .unwrap();
        assert_eq!(ok, json!({ "count": 42 }));
    }

    #[test]
    fn enforces_required_and_fills_defaults_on_replace() {
        let mut required = def("plan", CustomPropertyType::Text);
        required.required = true;
        let mut with_default = def("tier", CustomPropertyType::Text);
        with_default.default_value = Some(json!("standard"));
        let defs = vec![required, with_default];

        // missing required -> error
        assert!(matches!(
            validate_and_normalize(&defs, &json!({}), &json!({}), CustomPropertyMode::Replace),
            Err(CustomPropertyValidationError::MissingRequired(_))
        ));

        // required present -> default filled for the other
        let ok = validate_and_normalize(
            &defs,
            &json!({ "plan": "enterprise" }),
            &json!({}),
            CustomPropertyMode::Replace,
        )
        .unwrap();
        assert_eq!(ok, json!({ "plan": "enterprise", "tier": "standard" }));
    }

    #[test]
    fn validates_select_options() {
        let mut d = def("status", CustomPropertyType::SingleSelect);
        d.config.options = Some(vec![
            SelectOption {
                value: "rejected".into(),
                label: None,
            },
            SelectOption {
                value: "approved".into(),
                label: None,
            },
        ]);
        let defs = vec![d];
        assert!(
            validate_and_normalize(
                &defs,
                &json!({ "status": "approved" }),
                &json!({}),
                CustomPropertyMode::Replace
            )
            .is_ok()
        );
        assert!(matches!(
            validate_and_normalize(
                &defs,
                &json!({ "status": "maybe" }),
                &json!({}),
                CustomPropertyMode::Replace
            ),
            Err(CustomPropertyValidationError::NotAnOption { .. })
        ));
    }

    #[test]
    fn merge_updates_and_deletes_keys() {
        let defs = vec![
            def("a", CustomPropertyType::Number),
            def("b", CustomPropertyType::Text),
        ];
        let existing = json!({ "a": 1, "b": "keep" });

        // update a, delete b via null, leave nothing else
        let merged = validate_and_normalize(
            &defs,
            &json!({ "a": 2, "b": null }),
            &existing,
            CustomPropertyMode::Merge,
        )
        .unwrap();
        assert_eq!(merged, json!({ "a": 2 }));

        // merge does not enforce required and keeps untouched keys
        let merged2 = validate_and_normalize(
            &defs,
            &json!({ "a": 5 }),
            &existing,
            CustomPropertyMode::Merge,
        )
        .unwrap();
        assert_eq!(merged2, json!({ "a": 5, "b": "keep" }));
    }

    #[test]
    fn multi_select_must_be_array_of_options() {
        let mut d = def("tags", CustomPropertyType::MultiSelect);
        d.config.options = Some(vec![
            SelectOption {
                value: "vip".into(),
                label: None,
            },
            SelectOption {
                value: "eu".into(),
                label: None,
            },
        ]);
        let defs = vec![d];
        assert!(
            validate_and_normalize(
                &defs,
                &json!({ "tags": ["vip", "eu"] }),
                &json!({}),
                CustomPropertyMode::Replace
            )
            .is_ok()
        );
        assert!(matches!(
            validate_and_normalize(
                &defs,
                &json!({ "tags": ["vip", "us"] }),
                &json!({}),
                CustomPropertyMode::Replace
            ),
            Err(CustomPropertyValidationError::NotAnOption { .. })
        ));
        assert!(matches!(
            validate_and_normalize(
                &defs,
                &json!({ "tags": "vip" }),
                &json!({}),
                CustomPropertyMode::Replace
            ),
            Err(CustomPropertyValidationError::WrongType { .. })
        ));
    }

    fn check(property_type: CustomPropertyType, value: serde_json::Value) -> serde_json::Value {
        let defs = vec![def("v", property_type)];
        validate_and_normalize(
            &defs,
            &json!({ "v": value }),
            &json!({}),
            CustomPropertyMode::Replace,
        )
        .unwrap()
    }

    fn check_err(property_type: CustomPropertyType, value: serde_json::Value) {
        let defs = vec![def("v", property_type)];
        assert!(matches!(
            validate_and_normalize(
                &defs,
                &json!({ "v": value }),
                &json!({}),
                CustomPropertyMode::Replace
            ),
            Err(CustomPropertyValidationError::InvalidFormat { .. })
        ));
    }

    #[test]
    fn validates_url() {
        assert_eq!(
            check(CustomPropertyType::Url, json!("https://a.com/x")),
            json!({ "v": "https://a.com/x" })
        );
        check_err(CustomPropertyType::Url, json!("not a url"));
        check_err(CustomPropertyType::Url, json!("ftp://a.com"));
        check_err(CustomPropertyType::Url, json!("mailto:a@b.com"));
    }

    #[test]
    fn validates_email() {
        assert_eq!(
            check(CustomPropertyType::Email, json!("a@b.com")),
            json!({ "v": "a@b.com" })
        );
        check_err(CustomPropertyType::Email, json!("nope"));
        check_err(CustomPropertyType::Email, json!("a@"));
    }

    #[test]
    fn validates_and_normalizes_date() {
        assert_eq!(
            check(CustomPropertyType::Date, json!("2026-07-17")),
            json!({ "v": "2026-07-17" })
        );
        check_err(CustomPropertyType::Date, json!("17/07/2026"));
        check_err(CustomPropertyType::Date, json!("banana"));
    }

    #[test]
    fn validates_and_normalizes_datetime_to_rfc3339_utc() {
        // datetime-local shape (no offset) -> treated as UTC
        assert_eq!(
            check(CustomPropertyType::Datetime, json!("2026-07-17T14:30")),
            json!({ "v": "2026-07-17T14:30:00Z" })
        );
        // offset is converted to UTC
        assert_eq!(
            check(
                CustomPropertyType::Datetime,
                json!("2026-07-17T16:30:00+02:00")
            ),
            json!({ "v": "2026-07-17T14:30:00Z" })
        );
        check_err(CustomPropertyType::Datetime, json!("2026-07-17"));
        check_err(CustomPropertyType::Datetime, json!("banana"));
    }
}
