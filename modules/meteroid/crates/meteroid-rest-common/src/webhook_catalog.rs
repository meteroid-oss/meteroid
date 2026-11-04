//! The webhook event catalog's shape. The REST crate builds it from its OpenAPI document; the
//! gRPC API receives it as a [`WebhookCatalog`] so it does not depend on the REST crate.

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
    /// Every webhook event type.
    fn events(&self) -> &'static [WebhookEventDoc];

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
}
