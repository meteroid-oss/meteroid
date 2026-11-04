use csv::StringRecord;
use meteroid_store::domain::custom_property_definitions::CustomPropertyDefinition;
use meteroid_store::domain::enums::CustomPropertyType;
use serde_json::{Map, Number, Value};

/// Header prefix for custom-property columns: `custom_properties.<key>`.
pub const CUSTOM_PROPERTY_PREFIX: &str = "custom_properties.";

/// The `custom_properties.<key>` columns of a CSV, resolved once per file against the tenant's
/// active definitions so each row only pays the string -> JSON conversion.
#[derive(Debug)]
pub struct CustomPropertyColumns {
    columns: Vec<(usize, CustomPropertyDefinition)>,
}

impl CustomPropertyColumns {
    /// Resolves the custom-property headers against `definitions`. Unknown keys are rejected here
    /// so a mistyped column fails the import upfront instead of silently dropping every value.
    pub fn from_headers(
        headers: &StringRecord,
        definitions: &[CustomPropertyDefinition],
    ) -> Result<Self, String> {
        let mut columns = Vec::new();
        let mut unknown = Vec::new();

        for (idx, header) in headers.iter().enumerate() {
            let header = header.trim();
            if !header
                .to_ascii_lowercase()
                .starts_with(CUSTOM_PROPERTY_PREFIX)
            {
                continue;
            }
            let key = header[CUSTOM_PROPERTY_PREFIX.len()..].trim();
            match definitions.iter().find(|d| d.key == key) {
                Some(def) => columns.push((idx, def.clone())),
                None => unknown.push(key.to_string()),
            }
        }

        if !unknown.is_empty() {
            let known: Vec<&str> = definitions.iter().map(|d| d.key.as_str()).collect();
            return Err(format!(
                "Unknown custom properties: {}. Defined properties: {}",
                unknown.join(", "),
                if known.is_empty() {
                    "none".to_string()
                } else {
                    known.join(", ")
                }
            ));
        }

        Ok(Self { columns })
    }

    /// Builds the property bag for one row. A file that declares `custom_properties.*` columns is
    /// the source of truth for those properties: the returned bag replaces the entity's, so an
    /// empty cell clears the value. A file that declares none yields `Null` (leave as-is).
    pub fn parse_row(&self, record: &StringRecord) -> Result<Value, String> {
        // A file without a single `custom_properties.*` column declares nothing about
        // properties: `Null` tells the store to leave the entity's bag alone.
        if self.columns.is_empty() {
            return Ok(Value::Null);
        }

        let mut out = Map::new();

        for (idx, def) in &self.columns {
            let raw = record.get(*idx).unwrap_or_default().trim();
            if raw.is_empty() {
                continue;
            }
            out.insert(def.key.clone(), coerce(def, raw)?);
        }

        Ok(Value::Object(out))
    }
}

/// Converts a raw CSV cell to the JSON shape expected by the property's type. Only the type
/// conversion happens here — options, ranges and date formats are validated by the store.
fn coerce(def: &CustomPropertyDefinition, raw: &str) -> Result<Value, String> {
    let invalid = |expected: &str| {
        format!(
            "custom property '{}': '{raw}' is not a valid {expected}",
            def.key
        )
    };

    let value = match def.property_type {
        CustomPropertyType::Text
        | CustomPropertyType::Url
        | CustomPropertyType::Email
        | CustomPropertyType::SingleSelect
        | CustomPropertyType::Date
        | CustomPropertyType::Datetime => Value::String(raw.to_string()),
        // Integers stay integral so `42` doesn't come back as `42.0`.
        CustomPropertyType::Number => raw
            .parse::<i64>()
            .map(Number::from)
            .ok()
            .or_else(|| raw.parse::<f64>().ok().and_then(Number::from_f64))
            .map(Value::Number)
            .ok_or_else(|| invalid("number"))?,
        CustomPropertyType::Boolean => match raw.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "y" => Value::Bool(true),
            "false" | "0" | "no" | "n" => Value::Bool(false),
            _ => return Err(invalid("boolean (true/false)")),
        },
        CustomPropertyType::MultiSelect => Value::Array(
            raw.split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| Value::String(v.to_string()))
                .collect(),
        ),
        CustomPropertyType::Json => serde_json::from_str(raw)
            .map_err(|e| format!("custom property '{}': invalid JSON ({e})", def.key))?,
    };

    Ok(value)
}

#[doc(hidden)]
pub fn test_definition(key: &str, property_type: CustomPropertyType) -> CustomPropertyDefinition {
    use common_domain::ids::{BaseId, CustomPropertyDefinitionId, TenantId};
    use meteroid_store::domain::custom_property_definitions::PropertyConfig;
    use meteroid_store::domain::enums::CustomPropertyEntityType;

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use super::test_definition as def;

    fn parse(headers: &[&str], row: &[&str], definitions: &[CustomPropertyDefinition]) -> Value {
        let headers = StringRecord::from(headers.to_vec());
        CustomPropertyColumns::from_headers(&headers, definitions)
            .unwrap()
            .parse_row(&StringRecord::from(row.to_vec()))
            .unwrap()
    }

    #[test]
    fn ignores_files_without_custom_property_columns() {
        let defs = vec![def("tier", CustomPropertyType::Text)];
        assert_eq!(
            parse(&["name", "currency"], &["Acme", "EUR"], &defs),
            json!(null)
        );
    }

    #[test]
    fn rejects_unknown_keys_at_header_level() {
        let defs = vec![def("tier", CustomPropertyType::Text)];
        let headers = StringRecord::from(vec!["name", "custom_properties.nope"]);
        let err = CustomPropertyColumns::from_headers(&headers, &defs).unwrap_err();
        assert!(err.contains("nope"), "{err}");
        assert!(err.contains("tier"), "{err}");
    }

    #[test]
    fn coerces_each_type() {
        let defs = vec![
            def("tier", CustomPropertyType::Text),
            def("seats", CustomPropertyType::Number),
            def("vip", CustomPropertyType::Boolean),
            def("tags", CustomPropertyType::MultiSelect),
            def("meta", CustomPropertyType::Json),
            def("signed_on", CustomPropertyType::Date),
        ];
        let parsed = parse(
            &[
                "custom_properties.tier",
                "custom_properties.seats",
                "custom_properties.vip",
                "custom_properties.tags",
                "custom_properties.meta",
                "custom_properties.signed_on",
            ],
            &[
                "enterprise",
                "42",
                "yes",
                "vip, eu",
                r#"{"crm":"hubspot"}"#,
                "2026-07-17",
            ],
            &defs,
        );
        assert_eq!(
            parsed,
            json!({
                "tier": "enterprise",
                "seats": 42,
                "vip": true,
                "tags": ["vip", "eu"],
                "meta": {"crm": "hubspot"},
                "signed_on": "2026-07-17",
            })
        );
    }

    /// An empty cell leaves the key out of the bag, which under replace semantics is what
    /// clears the stored value.
    #[test]
    fn empty_cells_are_absent_from_the_bag() {
        let defs = vec![
            def("tier", CustomPropertyType::Text),
            def("seats", CustomPropertyType::Number),
        ];
        let parsed = parse(
            &["custom_properties.tier", "custom_properties.seats"],
            &["  ", "7"],
            &defs,
        );
        assert_eq!(parsed, json!({ "seats": 7 }));

        let empty = parse(
            &["custom_properties.tier", "custom_properties.seats"],
            &["", ""],
            &defs,
        );
        assert_eq!(empty, json!({}));
    }

    #[test]
    fn reports_bad_values_per_row() {
        let defs = vec![def("seats", CustomPropertyType::Number)];
        let headers = StringRecord::from(vec!["custom_properties.seats"]);
        let err = CustomPropertyColumns::from_headers(&headers, &defs)
            .unwrap()
            .parse_row(&StringRecord::from(vec!["many"]))
            .unwrap_err();
        assert!(err.contains("seats"), "{err}");
        assert!(err.contains("number"), "{err}");
    }
}
