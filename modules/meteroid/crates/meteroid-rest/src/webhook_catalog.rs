//! The webhook event catalog shown in the app: every event type with its description,
//! an example payload and the payload's fields. It is read once from the same OpenAPI
//! document the public API reference is generated from, so the two never drift apart.

use std::sync::LazyLock;

use serde_json::Value;
use strum::IntoEnumIterator;
use utoipa::OpenApi;

use crate::openapi::ApiDoc;
use crate::webhooks::out_model::WebhookOutEventTypeEnum;

/// How deep nested objects are expanded into dotted field paths.
const MAX_DEPTH: usize = 4;

pub use meteroid_rest_common::webhook_catalog::{
    WebhookCatalog, WebhookEventDoc, WebhookEventField,
};

static CATALOG: LazyLock<Vec<WebhookEventDoc>> = LazyLock::new(build);

/// The catalog built from this crate's OpenAPI document.
pub struct RestWebhookCatalog;

impl WebhookCatalog for RestWebhookCatalog {
    fn events(&self) -> &'static [WebhookEventDoc] {
        &CATALOG
    }
}

pub fn catalog() -> &'static [WebhookEventDoc] {
    RestWebhookCatalog.events()
}

fn build() -> Vec<WebhookEventDoc> {
    let doc = serde_json::to_value(ApiDoc::openapi()).unwrap_or_else(|e| {
        log::error!("Cannot build the webhook event catalog: {e}");
        Value::Null
    });
    let schemas = &doc["components"]["schemas"];
    // utoipa flattens extensions, so the `webhooks` extension sits at the top level.
    let webhooks = &doc["webhooks"];

    WebhookOutEventTypeEnum::iter()
        .map(|event| {
            let name = event.to_string();
            let content = &webhooks[&name]["post"]["requestBody"]["content"]["application/json"];

            let mut fields = Vec::new();
            flatten(schemas, &content["schema"], "", &mut fields, 0);
            present_envelope(&mut fields, &name);

            WebhookEventDoc {
                group: event.group().to_string(),
                beta: event.group().is_beta(),
                needs_module: event.needs_module(),
                description: event.description(),
                example: content["example"].clone(),
                fields,
                name,
            }
        })
        .collect()
}

const ENVELOPE: [&str; 3] = ["id", "type", "timestamp"];

/// Lists the envelope first, and documents `type` as the one value it has for this event
/// rather than as the enum of every event type.
fn present_envelope(fields: &mut [WebhookEventField], event_name: &str) {
    fields.sort_by_key(|f| {
        ENVELOPE
            .iter()
            .position(|e| *e == f.path)
            .unwrap_or(ENVELOPE.len())
    });

    for field in fields.iter_mut() {
        match field.path.as_str() {
            "id" => {
                field.description = Some(
                    "Unique event id, also sent as the webhook-id header. Use it to deduplicate."
                        .to_string(),
                )
            }
            "type" => {
                field.type_name = format!("\"{event_name}\"");
                field.description = Some("The event type.".to_string());
            }
            "timestamp" => field.description = Some("When the event happened.".to_string()),
            _ => {}
        }
    }
}

fn ref_name(schema: &Value) -> Option<&str> {
    schema["$ref"]
        .as_str()
        .and_then(|r| r.strip_prefix("#/components/schemas/"))
}

/// Follows `$ref`s and unwraps the `oneOf: [null, X]` utoipa emits for an `Option<X>`.
fn resolve<'a>(schemas: &'a Value, schema: &'a Value) -> (&'a Value, bool) {
    let mut current = schema;
    let mut nullable = false;

    for _ in 0..8 {
        if let Some(name) = ref_name(current) {
            current = &schemas[name];
            continue;
        }
        let variants = current["oneOf"].as_array().or(current["anyOf"].as_array());
        if let Some(variants) = variants {
            let non_null: Vec<&Value> = variants
                .iter()
                .filter(|v| v["type"].as_str() != Some("null"))
                .collect();
            nullable |= non_null.len() < variants.len();
            if let [only] = non_null[..] {
                current = only;
                continue;
            }
        }
        if let Some([only]) = current["allOf"].as_array().map(Vec::as_slice) {
            current = only;
            continue;
        }
        break;
    }

    (current, nullable)
}

/// The JSON type of a schema, ignoring a `"null"` alongside it.
fn base_type(schema: &Value) -> Option<&str> {
    match &schema["type"] {
        Value::String(t) => Some(t.as_str()),
        Value::Array(types) => types
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null"),
        _ => None,
    }
}

fn type_name(schemas: &Value, schema: &Value) -> String {
    let (resolved, nullable) = resolve(schemas, schema);
    let nullable = nullable
        || resolved["type"]
            .as_array()
            .is_some_and(|types| types.iter().any(|t| t == "null"));

    let name = if let Some(values) = resolved["enum"].as_array() {
        values
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" | ")
    } else {
        match base_type(resolved) {
            Some("array") => format!("array of {}", type_name(schemas, &resolved["items"])),
            Some("string") => match resolved["format"].as_str() {
                Some("MeteroidId") => "string (id)".to_string(),
                Some(format) => format!("string ({format})"),
                None => "string".to_string(),
            },
            Some(other) => other.to_string(),
            None if resolved["allOf"].is_array() => "object".to_string(),
            None if resolved["oneOf"].is_array() => "one of several shapes".to_string(),
            None => "object".to_string(),
        }
    };

    if nullable {
        format!("{name}, nullable")
    } else {
        name
    }
}

fn flatten(
    schemas: &Value,
    schema: &Value,
    prefix: &str,
    out: &mut Vec<WebhookEventField>,
    depth: usize,
) {
    let (resolved, _) = resolve(schemas, schema);

    // An event is `allOf: [its fields, the envelope]`: both parts are top-level fields.
    if let Some(parts) = resolved["allOf"].as_array() {
        for part in parts {
            flatten(schemas, part, prefix, out, depth);
        }
        return;
    }

    let Some(properties) = resolved["properties"].as_object() else {
        return;
    };
    let required: Vec<&str> = resolved["required"]
        .as_array()
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    for (key, property) in properties {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        let (property_resolved, _) = resolve(schemas, property);

        out.push(WebhookEventField {
            type_name: type_name(schemas, property),
            description: property["description"]
                .as_str()
                .or(property_resolved["description"].as_str())
                .map(str::to_string),
            required: required.contains(&key.as_str()),
            path: path.clone(),
        });

        if depth + 1 < MAX_DEPTH {
            if property_resolved["properties"].is_object() || property_resolved["allOf"].is_array()
            {
                flatten(schemas, property, &path, out, depth + 1);
            } else if base_type(property_resolved) == Some("array") {
                flatten(
                    schemas,
                    &property_resolved["items"],
                    &format!("{path}[]"),
                    out,
                    depth + 1,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn every_event_type_is_documented() {
        let catalog = catalog();
        assert_eq!(catalog.len(), WebhookOutEventTypeEnum::iter().count());

        for event in catalog {
            assert!(!event.description.is_empty(), "{}", event.name);
            let kind = WebhookOutEventTypeEnum::from_str(&event.name).expect("a known type");
            assert_eq!(event.beta, kind.group().is_beta(), "{}", event.name);
            assert!(event.example.is_object(), "{} has an example", event.name);
            let envelope: Vec<&str> = event.fields[..3].iter().map(|f| f.path.as_str()).collect();
            assert_eq!(
                envelope, ENVELOPE,
                "{} lists the envelope first",
                event.name
            );
            assert_eq!(event.fields[1].type_name, format!("\"{}\"", event.name));
            assert!(
                event
                    .fields
                    .iter()
                    .any(|f| !["id", "type", "timestamp"].contains(&f.path.as_str())),
                "{} documents its own fields",
                event.name
            );
        }
    }

    #[test]
    fn an_example_carries_the_given_id_and_time() {
        let example = RestWebhookCatalog
            .example_payload("invoice.paid", "evt_1", "2026-10-08T00:00:00.000Z")
            .expect("known event type");
        assert_eq!(example["id"], "evt_1");
        assert_eq!(example["type"], "invoice.paid");
        assert_eq!(example["timestamp"], "2026-10-08T00:00:00.000Z");
        assert!(
            RestWebhookCatalog
                .example_payload("nope", "evt_1", "t")
                .is_none()
        );
    }

    #[test]
    fn nullable_refs_and_enums_are_named() {
        let schemas = serde_json::json!({
            "Status": { "type": "string", "enum": ["DRAFT", "PAID"] },
        });
        assert_eq!(
            type_name(
                &schemas,
                &serde_json::json!({ "$ref": "#/components/schemas/Status" })
            ),
            "DRAFT | PAID"
        );
        assert_eq!(
            type_name(
                &schemas,
                &serde_json::json!({ "oneOf": [{ "type": "null" }, { "$ref": "#/components/schemas/Status" }] })
            ),
            "DRAFT | PAID, nullable"
        );
        assert_eq!(
            type_name(
                &schemas,
                &serde_json::json!({ "type": ["string", "null"], "format": "date" })
            ),
            "string (date), nullable"
        );
    }
}
