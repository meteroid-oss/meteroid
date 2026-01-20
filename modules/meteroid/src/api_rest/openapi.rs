use crate::api_rest::invoices::model::InvoiceStatus;
use crate::api_rest::metrics::model::BillingMetricAggregateEnum;
use crate::api_rest::model::BillingPeriodEnum;
use crate::api_rest::subscriptions::model::SubscriptionStatusEnum;
use crate::api_rest::webhooks::out_model::{
    CreditNoteStatus, WebhookOutCreditNoteEvent, WebhookOutCreditNoteEventData,
    WebhookOutCustomerEvent, WebhookOutCustomerEventData, WebhookOutEventGroupEnum,
    WebhookOutEventTypeEnum, WebhookOutInvoiceEvent, WebhookOutInvoiceEventData,
    WebhookOutMetricEvent, WebhookOutMetricEventData, WebhookOutQuoteEvent,
    WebhookOutQuoteEventData, WebhookOutSubscriptionEvent, WebhookOutSubscriptionEventData,
};
use crate::api_rest::{AppState, api_routes};
use common_domain::ids::{
    BillableMetricId, CreditNoteId, CustomerId, EventId, InvoiceId, QuoteId, SubscriptionId,
};
use strum::IntoEnumIterator;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;

pub fn generate_spec() {
    let path = "spec/api/v1/openapi.json";

    println!("Generating OpenAPI spec {path:?}");

    let (_router, open_api) = OpenApiRouter::<AppState>::with_openapi(ApiDoc::openapi())
        .merge(api_routes())
        .split_for_parts();

    // Convert to JSON and move webhooks from extensions to top-level
    let mut json_value: serde_json::Value =
        serde_json::from_str(&open_api.to_pretty_json().unwrap())
            .expect("Failed to parse OpenAPI JSON");

    // Extract webhooks from extensions
    let webhooks = json_value
        .get_mut("extensions")
        .and_then(|ext| ext.as_object_mut())
        .and_then(|ext| ext.remove("webhooks"));

    // Rebuild JSON with proper key ordering: openapi, info, paths, webhooks, components
    let root = json_value.as_object_mut().unwrap();
    root.remove("extensions"); // Remove empty extensions

    let mut ordered = serde_json::Map::new();
    let key_order = ["openapi", "info", "paths", "webhooks", "components", "tags"];

    for key in key_order {
        if key == "webhooks" {
            if let Some(wh) = &webhooks {
                ordered.insert(key.to_string(), wh.clone());
            }
        } else if let Some(value) = root.remove(key) {
            ordered.insert(key.to_string(), value);
        }
    }

    // Add any remaining keys not in our order list
    for (key, value) in root.iter() {
        ordered.insert(key.clone(), value.clone());
    }

    let output = serde_json::to_string_pretty(&serde_json::Value::Object(ordered))
        .expect("Failed to serialize OpenAPI");

    std::fs::write(path, output).expect("Unable to write openapi.json file");
}

#[derive(OpenApi)]
#[openapi(
    modifiers(&SecurityAddon, &WebhooksAddon),
    components(
      schemas(
        WebhookOutCustomerEvent,
        WebhookOutInvoiceEvent,
        WebhookOutSubscriptionEvent,
        WebhookOutMetricEvent,
        WebhookOutQuoteEvent,
        WebhookOutCreditNoteEvent,
      )
    ),
    tags((name = "Meteroid", description = "Meteroid API"))
)]
pub struct ApiDoc;

struct SecurityAddon;

struct WebhooksAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
            );
        }
    }
}

impl Modify for WebhooksAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use serde_json::json;
        use std::collections::BTreeMap;

        // Build webhook definitions
        let mut webhooks = BTreeMap::new();

        for event in WebhookOutEventTypeEnum::iter() {
            let event_name = event.to_string();
            let description = event.description();
            let group = event.group();

            let example: serde_json::Value = match group {
                WebhookOutEventGroupEnum::Customer => {
                    let event = WebhookOutCustomerEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutCustomerEventData {
                            customer_id: CustomerId::default(),
                            name: "Acme Corporation".to_string(),
                            alias: Some("ACME".to_string()),
                            billing_email: Some("billing@acme.example".to_string()),
                            invoicing_emails: vec!["invoices@acme.example".to_string()],
                            phone: Some("+1-555-0123".to_string()),
                            currency: "USD".to_string(),
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::Subscription => {
                    let event = WebhookOutSubscriptionEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutSubscriptionEventData {
                            subscription_id: SubscriptionId::default(),
                            customer_id: Default::default(),
                            customer_alias: Some("ACME".to_string()),
                            customer_name: "Acme Corporation".to_string(),
                            billing_day_anchor: 1,
                            currency: "EUR".to_string(),
                            trial_duration: None,
                            start_date: Default::default(),
                            end_date: None,
                            billing_start_date: None,
                            plan_name: "default".to_string(),
                            version: 0,
                            created_at: Default::default(),
                            net_terms: 0,
                            invoice_memo: None,
                            invoice_threshold: None,
                            activated_at: None,
                            mrr_cents: 0,
                            period: BillingPeriodEnum::Monthly,
                            status: SubscriptionStatusEnum::PendingActivation,
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::Invoice => {
                    let event = WebhookOutInvoiceEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutInvoiceEventData {
                            invoice_id: InvoiceId::default(),
                            customer_id: Default::default(),
                            status: InvoiceStatus::Draft,
                            currency: "EUR".to_string(),
                            total: 10000,
                            tax_amount: 2000,
                            created_at: Default::default(),
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::BillableMetric => {
                    let event = WebhookOutMetricEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutMetricEventData {
                            metric_id: BillableMetricId::default(),
                            name: "Api Calls".to_string(),
                            description: None,
                            code: "api_calls".to_string(),
                            aggregation_type: BillingMetricAggregateEnum::Count,
                            aggregation_key: None,
                            unit_conversion_factor: None,
                            unit_conversion_rounding: None,
                            segmentation_matrix: None,
                            usage_group_key: None,
                            created_at: Default::default(),
                            product_family_id: Default::default(),
                            product_id: None,
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::Quote => {
                    let event = WebhookOutQuoteEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutQuoteEventData {
                            quote_id: QuoteId::default(),
                            customer_id: Default::default(),
                            subscription_id: None,
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::CreditNote => {
                    let event = WebhookOutCreditNoteEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutCreditNoteEventData {
                            credit_note_id: CreditNoteId::default(),
                            customer_id: Default::default(),
                            invoice_id: Default::default(),
                            status: CreditNoteStatus::Draft,
                            currency: "EUR".to_string(),
                            total: 5000,
                            tax_amount: 1000,
                            refunded_amount_cents: 0,
                            credited_amount_cents: 5000,
                            created_at: Default::default(),
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
            };

            let webhook_def = json!({
                "post": {
                    "summary": format!("{} webhook", event_name),
                    "description": description,
                    "operationId": format!("webhook_{}", event_name.replace('.', "_")),
                    "tags": ["webhooks"],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {
                                    "$ref": format!("#/components/schemas/{}", group.schema_name())
                                },
                                "example": example
                            }
                        }
                    },
                    "responses": {
                        "200": {
                            "description": "Webhook received successfully"
                        }
                    }
                }
            });

            webhooks.insert(event_name, webhook_def);
        }

        // Add webhooks extension
        if openapi.extensions.is_none() {
            openapi.extensions = Some(Default::default());
        }

        if let Some(extensions) = openapi.extensions.as_mut() {
            extensions.insert("webhooks".to_string(), json!(webhooks));
        }
    }
}
