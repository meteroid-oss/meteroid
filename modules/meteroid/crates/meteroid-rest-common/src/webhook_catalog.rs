//! The webhook event catalog's shape, one per payload shape: API v1's and API v2's (ADR-0013
//! §15). The REST crate builds each from its OpenAPI document; the gRPC API receives them as a
//! [`WebhookCatalog`] so it does not depend on the REST crate.

use crate::api_versions::webhook_envelope;
use crate::datetime::ApiDateTime;
use common_domain::ids::EventId;
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct WebhookEventDoc {
    pub name: String,
    pub group: String,
    /// Sent only to endpoints that list it.
    pub beta: bool,
    /// Emitted only when a module of the edition emits it (`WebhookOutEventTypeEnum::needs_module`).
    pub needs_module: bool,
    pub description: String,
    pub example: Value,
    pub fields: Vec<WebhookEventField>,
}

/// One payload field, e.g. `data.invoice_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookEventField {
    pub path: String,
    pub type_name: String,
    pub description: Option<String>,
    pub required: bool,
}

pub trait WebhookCatalog: Send + Sync {
    /// Every webhook event type, in API v1's payloads.
    fn events(&self) -> &'static [WebhookEventDoc];

    /// Every event type API v2's payloads carry, each example the envelope `{id, type,
    /// created_at, api_version, data}` at the latest version.
    fn events_v2(&self) -> &'static [WebhookEventDoc];

    fn is_known(&self, event_type: &str) -> bool {
        self.events().iter().any(|doc| doc.name == event_type)
    }

    /// An event type's example payload, stamped with a real event id and the current time,
    /// for "Send example". `None` for an unknown event type.
    fn example_payload(&self, event_type: &str, id: &str, timestamp: &str) -> Option<Value> {
        let mut example = self
            .events()
            .iter()
            .find(|doc| doc.name == event_type)?
            .example
            .clone();

        if let Some(fields) = example.as_object_mut() {
            fields.insert("id".to_string(), Value::String(id.to_string()));
            fields.insert("type".to_string(), Value::String(event_type.to_string()));
            fields.insert(
                "timestamp".to_string(),
                Value::String(timestamp.to_string()),
            );
        }

        Some(example)
    }

    /// An event type's example in API v2's payloads, as an endpoint holding `api_version` receives
    /// it: its envelope stamped with a real event id and `created_at`. `None` for an event type
    /// v2's payloads do not carry.
    fn example_payload_v2(
        &self,
        event_type: &str,
        id: EventId,
        created_at: ApiDateTime,
        api_version: &str,
    ) -> Option<Value> {
        let doc = self.events_v2().iter().find(|doc| doc.name == event_type)?;
        Some(webhook_envelope(
            id,
            event_type,
            created_at,
            api_version,
            &doc.example["data"],
        ))
    }
}
