use crate::api_rest::invoices::model::InvoiceStatus;
use crate::api_rest::metrics::model::BillingMetricAggregateEnum;
use crate::api_rest::model::BillingPeriodEnum;
use crate::api_rest::subscriptions::model::SubscriptionStatusEnum;
use crate::api_rest::webhooks::out_model::{
    CreditNoteStatus, WebhookOutCreditNoteEventData, WebhookOutCustomerEventData, WebhookOutEvent,
    WebhookOutEventData, WebhookOutEventGroupEnum, WebhookOutEventTypeEnum,
    WebhookOutInvoiceEventData, WebhookOutMetricEventData, WebhookOutQuoteEventData,
    WebhookOutSubscriptionEventData,
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

    std::fs::write(path, open_api.clone().to_pretty_json().unwrap())
        .expect("Unable to write openapi.json file");
}

#[derive(OpenApi)]
#[openapi(
    modifiers(&SecurityAddon, &WebhooksAddon),
    components(
      schemas(
        WebhookOutEvent,
      )
    ),
    tags((name = "meteroid", description = "Meteroid API"))
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
            let group_name = group.to_string();

            let example_payload = match group {
                WebhookOutEventGroupEnum::Customer => {
                    WebhookOutEventData::Customer(WebhookOutCustomerEventData {
                        id: CustomerId::default(),
                        name: "Acme Corporation".to_string(),
                        alias: Some("ACME".to_string()),
                        billing_email: Some("billing@acme.example".to_string()),
                        invoicing_emails: vec!["invoices@acme.example".to_string()],
                        phone: Some("+1-555-0123".to_string()),
                        currency: "USD".to_string(),
                    })
                }
                WebhookOutEventGroupEnum::Subscription => {
                    WebhookOutEventData::Subscription(WebhookOutSubscriptionEventData {
                        id: SubscriptionId::default(),
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
                    })
                }
                WebhookOutEventGroupEnum::Invoice => {
                    WebhookOutEventData::Invoice(WebhookOutInvoiceEventData {
                        id: InvoiceId::default(),
                        customer_id: Default::default(),
                        status: InvoiceStatus::Draft,
                        currency: "EUR".to_string(),
                        total: 10000,
                        tax_amount: 2000,
                        created_at: Default::default(),
                    })
                }
                WebhookOutEventGroupEnum::BillableMetric => {
                    WebhookOutEventData::Metric(WebhookOutMetricEventData {
                        id: BillableMetricId::default(),
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
                    })
                }
                WebhookOutEventGroupEnum::Quote => {
                    WebhookOutEventData::Quote(WebhookOutQuoteEventData {
                        id: QuoteId::default(),
                        customer_id: Default::default(),
                        subscription_id: None,
                    })
                }
                WebhookOutEventGroupEnum::CreditNote => {
                    WebhookOutEventData::CreditNote(WebhookOutCreditNoteEventData {
                        id: CreditNoteId::default(),
                        customer_id: Default::default(),
                        invoice_id: Default::default(),
                        status: CreditNoteStatus::Draft,
                        currency: "EUR".to_string(),
                        total: 5000,
                        tax_amount: 1000,
                        refunded_amount_cents: 0,
                        credited_amount_cents: 5000,
                        created_at: Default::default(),
                    })
                }
            };

            let example_event = WebhookOutEvent {
                id: EventId::default(),
                event_type: event,
                data: example_payload,
                timestamp: Default::default(),
            };

            let example =
                serde_json::to_value(&example_event).expect("Failed to serialize webhook example");

            let webhook_def = json!({
                "post": {
                    "summary": format!("{} webhook", event_name),
                    "description": description,
                    "operationId": format!("webhook_{}", event_name.replace('.', "_")),
                    "tags": [group_name],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {
                                    "$ref": "#/components/schemas/WebhookOutEvent"
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
