//! The webhook event catalog shown in the app: every event type with its description,
//! an example payload and the payload's fields, in each payload shape an endpoint receives
//! (ADR-0013 §15). Each is read once from the same OpenAPI document its public API reference is
//! generated from, so the two never drift apart: API v1's payloads from v1's document, API v2's
//! from v2's (its `webhooks`, envelopes whose `data` is the resource's schema or the
//! notification's).

use std::collections::HashMap;
use std::sync::LazyLock;

use serde_json::{Map, Value, json};
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
static CATALOG_V2: LazyLock<Vec<WebhookEventDoc>> = LazyLock::new(build_v2);

/// The catalog built from this crate's OpenAPI document.
pub struct RestWebhookCatalog;

impl WebhookCatalog for RestWebhookCatalog {
    fn events(&self) -> &'static [WebhookEventDoc] {
        &CATALOG
    }

    fn events_v2(&self) -> &'static [WebhookEventDoc] {
        &CATALOG_V2
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

/// API v2's catalog: every event type its payloads carry (the retired ones left out), from the
/// v2 document with every event type's webhook attached (`v2::document::attach_webhooks`), so a
/// module's event types are described as in the edition emitting them. Each example is the
/// envelope at the latest version: its `data` the document's own example of the resource where
/// a GET shows one (`v2::examples`), else built from the schema.
fn build_v2() -> Vec<WebhookEventDoc> {
    use crate::v2::document::{RETIRED_EVENT_TYPES, attach_webhooks};
    use meteroid_rest_common::spec::EditionSpec;

    let carried: Vec<WebhookOutEventTypeEnum> = WebhookOutEventTypeEnum::iter()
        .filter(|event| !RETIRED_EVENT_TYPES.contains(&event.to_string().as_str()))
        .collect();
    let mut doc = crate::v2::spec_json(&crate::v2::spec(&EditionSpec::core()));
    let every: Map<String, Value> = carried
        .iter()
        .map(|event| (event.to_string(), json!({})))
        .collect();
    if let Some(root) = doc.as_object_mut() {
        let mut v1 = Map::new();
        v1.insert("webhooks".to_owned(), Value::Object(every));
        attach_webhooks(root, &v1);
    }
    let schemas = &doc["components"]["schemas"];
    let shown = shown_examples(&doc);
    let latest = meteroid_rest_common::api_versions::latest();

    carried
        .into_iter()
        .map(|event| {
            let name = event.to_string();
            let schema = &doc["webhooks"][&name]["post"]["requestBody"]["content"]
                ["application/json"]["schema"];

            let mut fields = Vec::new();
            flatten(schemas, schema, "", &mut fields, 0);
            present_envelope_v2(&mut fields, &name);

            let data = example_of(
                schemas,
                &shown,
                &json!({ "$ref": format!("{SCHEMA_REF}{}", crate::v2::webhook_payloads::data_schema(event)) }),
                &mut Vec::new(),
            );
            let example = json!({
                "id": schemas["EventId"]["examples"][0],
                "type": name,
                "created_at": "2026-12-15T09:30:00.000Z",
                "api_version": latest,
                "data": data,
            });

            WebhookEventDoc {
                group: event.group().to_string(),
                beta: event.group().is_beta(),
                needs_module: event.needs_module(),
                description: event.description(),
                example,
                fields,
                name,
            }
        })
        .collect()
}

const SCHEMA_REF: &str = "#/components/schemas/";

/// The examples the document shows of a resource: each GET's answer, by the schema it answers.
fn shown_examples(doc: &Value) -> HashMap<String, Value> {
    let mut shown = HashMap::new();
    for item in doc["paths"].as_object().into_iter().flat_map(Map::values) {
        let content = &item["get"]["responses"]["200"]["content"]["application/json"];
        if let (Some(name), Some(example)) = (ref_name(&content["schema"]), content.get("example"))
        {
            shown
                .entry(name.to_owned())
                .or_insert_with(|| example.clone());
        }
    }
    shown
}

/// An example of `schema`: the document's own where it has one (an `example`, the first of its
/// `examples`, a GET's answer of that schema), else built from its fields, every one present
/// (the first shape of a choice, the first value of an enum). `seen` holds the schemas being
/// built, a schema naming itself stopping there.
fn example_of(
    schemas: &Value,
    shown: &HashMap<String, Value>,
    schema: &Value,
    seen: &mut Vec<String>,
) -> Value {
    if let Some(name) = ref_name(schema) {
        if let Some(example) = shown.get(name) {
            return example.clone();
        }
        if seen.iter().any(|building| building == name) {
            return Value::Null;
        }
        seen.push(name.to_owned());
        let example = example_of(schemas, shown, &schemas[name], seen);
        seen.pop();
        return example;
    }
    if let Some(example) = schema.get("example") {
        return example.clone();
    }
    if let Some(example) = schema["examples"].as_array().and_then(|all| all.first()) {
        return example.clone();
    }
    if let Some(value) = schema.get("const") {
        return value.clone();
    }
    if let Some(value) = schema["enum"]
        .as_array()
        .and_then(|values| values.iter().find(|value| !value.is_null()))
    {
        return value.clone();
    }
    for choice in ["oneOf", "anyOf"] {
        if let Some(variants) = schema[choice].as_array() {
            return variants
                .iter()
                .find(|variant| variant["type"].as_str() != Some("null"))
                .map_or(Value::Null, |variant| {
                    example_of(schemas, shown, variant, seen)
                });
        }
    }
    if let Some(parts) = schema["allOf"].as_array() {
        let mut merged = Map::new();
        for part in parts {
            if let Value::Object(fields) = example_of(schemas, shown, part, seen) {
                merged.extend(fields);
            }
        }
        if let Value::Object(fields) = object_example(schemas, shown, schema, seen) {
            merged.extend(fields);
        }
        return Value::Object(merged);
    }
    match base_type(schema) {
        Some("array") => match example_of(schemas, shown, &schema["items"], seen) {
            Value::Null => json!([]),
            item => json!([item]),
        },
        Some("string") => json!(match schema["format"].as_str() {
            Some("date-time") => "2026-12-15T09:30:00.000Z",
            Some("date") => "2026-12-15",
            Some("decimal") => "1",
            Some("uri") => "https://example.com",
            Some("email") => "billing@example.com",
            Some("uuid") => "0193b1f2-6d3a-7c1e-9f4b-2a5d8c7e6f10",
            _ => "string",
        }),
        Some("integer") | Some("number") => json!(0),
        Some("boolean") => json!(false),
        Some("null") => Value::Null,
        _ => object_example(schemas, shown, schema, seen),
    }
}

/// A string's example where its name says what it holds.
fn named_string(key: &str) -> Option<&'static str> {
    match key {
        "currency" => Some("EUR"),
        "country" => Some("DE"),
        "email" => Some("billing@example.com"),
        "locale" => Some("en"),
        "url" => Some("https://example.com"),
        _ => None,
    }
}

/// An object's example, its properties each by [`example_of`]: the required ones, and the
/// optional ones holding a single value (an optional object or list is left out, as a payload
/// leaves out what it does not have).
fn object_example(
    schemas: &Value,
    shown: &HashMap<String, Value>,
    schema: &Value,
    seen: &mut Vec<String>,
) -> Value {
    let Some(properties) = schema["properties"].as_object() else {
        return json!({});
    };
    let required: Vec<&str> = schema["required"]
        .as_array()
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut fields = Map::new();
    for (key, property) in properties {
        let mut example = example_of(schemas, shown, property, seen);
        if example == json!("string")
            && let Some(named) = named_string(key)
        {
            example = json!(named);
        }
        let single = !(example.is_null() || example.is_object() || example.is_array());
        if required.contains(&key.as_str()) || single {
            fields.insert(key.clone(), example);
        }
    }
    Value::Object(fields)
}

const ENVELOPE_V2: [&str; 5] = ["id", "type", "created_at", "api_version", "data"];

/// v2's envelope first, `type` as the one value it has for this event.
fn present_envelope_v2(fields: &mut [WebhookEventField], event_name: &str) {
    fields.sort_by_key(|f| {
        ENVELOPE_V2
            .iter()
            .position(|e| *e == f.path)
            .unwrap_or(ENVELOPE_V2.len())
    });
    for field in fields.iter_mut() {
        if field.path == "type" {
            field.type_name = format!("\"{event_name}\"");
            field.description = Some("The event type.".to_string());
        }
    }
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
    fn v2s_catalog_is_its_payloads() {
        let catalog = RestWebhookCatalog.events_v2();
        assert_eq!(
            catalog.len(),
            WebhookOutEventTypeEnum::iter().count() - 1,
            "every event type but the retired one"
        );
        for event in catalog {
            let kind = WebhookOutEventTypeEnum::from_str(&event.name).expect("a known type");
            assert_eq!(event.needs_module, kind.needs_module(), "{}", event.name);
            let envelope: Vec<&str> = event.fields[..5].iter().map(|f| f.path.as_str()).collect();
            assert_eq!(
                envelope, ENVELOPE_V2,
                "{} lists the envelope first",
                event.name
            );
            assert_eq!(event.fields[1].type_name, format!("\"{}\"", event.name));
            assert_eq!(event.example["type"], event.name.as_str());
            assert_eq!(
                event.example["api_version"],
                meteroid_rest_common::api_versions::latest()
            );
            assert!(
                event.example["data"].is_object(),
                "{}: {:#}",
                event.name,
                event.example
            );
            assert!(
                event.fields.iter().any(|f| f.path.starts_with("data.")),
                "{} documents its data",
                event.name
            );
        }
        let find = |name: &str| catalog.iter().find(|doc| doc.name == name).unwrap();
        // A resource a GET shows by example carries that example.
        assert_eq!(
            find("customer.created").example["data"],
            crate::v2::examples::response("get_customer")
        );
        let funded = &find("wallet.funded").example["data"];
        for resource in ["wallet", "ledger_entry", "credit_grant"] {
            assert!(funded[resource].is_object(), "{funded:#}");
        }
        assert_eq!(funded["wallet"]["currency"], "EUR");
        assert!(
            funded["ledger_entry"]["balance_after"].is_i64(),
            "{funded:#}"
        );
        assert!(
            find("checkout.payment_kept_as_credit").example["data"]["checkout_session"]["status"]
                .is_string()
        );
        // A retention notice carries its subscription as the document shows one.
        for name in ["retention.session_completed", "retention.renewal_upcoming"] {
            let data = &find(name).example["data"];
            assert!(data["subscription"]["plan"].is_object(), "{name}: {data:#}");
        }
        let renewal = &find("retention.renewal_upcoming").example["data"];
        assert!(renewal["mrr"]["amount"].is_i64(), "{renewal:#}");
        assert_eq!(renewal["mode"], "renewing");
        assert_eq!(
            find("retention.session_completed").example["data"]["outcome"],
            "saved"
        );
    }

    #[test]
    fn a_v2_example_carries_the_given_id_time_and_version() {
        let id = common_domain::ids::EventId::default();
        let example = RestWebhookCatalog
            .example_payload_v2(
                "invoice.paid",
                id,
                meteroid_rest_common::datetime::ApiDateTime::default(),
                "2026-12-15",
            )
            .expect("known event type");
        assert_eq!(example["id"], id.to_string());
        assert_eq!(example["type"], "invoice.paid");
        assert_eq!(example["api_version"], "2026-12-15");
        assert_eq!(example["created_at"], "1970-01-01T00:00:00.000Z");
        assert!(example["data"]["id"].as_str().unwrap().starts_with("inv_"));
        assert!(
            RestWebhookCatalog
                .example_payload_v2(
                    "entitlement.too_many_groups",
                    id,
                    meteroid_rest_common::datetime::ApiDateTime::default(),
                    "2026-12-15"
                )
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
