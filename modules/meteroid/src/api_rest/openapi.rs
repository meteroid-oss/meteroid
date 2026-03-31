use crate::api_rest::invoices::model::InvoiceStatus;
use crate::api_rest::metrics::model::BillingMetricAggregateEnum;
use crate::api_rest::model::BillingPeriodEnum;
use crate::api_rest::products::model::ProductFeeTypeEnum;
use crate::api_rest::subscriptions::model::SubscriptionStatusEnum;
use crate::api_rest::webhooks::out_model::{
    CreditNoteStatus, WebhookOutAddOnEvent, WebhookOutAddOnEventData, WebhookOutCouponEvent,
    WebhookOutCouponEventData, WebhookOutCreditNoteEvent, WebhookOutCreditNoteEventData,
    WebhookOutCustomerEvent, WebhookOutCustomerEventData, WebhookOutEventGroupEnum,
    WebhookOutEventTypeEnum, WebhookOutInvoiceEvent, WebhookOutInvoiceEventData,
    WebhookOutMetricEvent, WebhookOutMetricEventData, WebhookOutPlanEvent, WebhookOutPlanEventData,
    WebhookOutProductEvent, WebhookOutProductEventData, WebhookOutQuoteEvent,
    WebhookOutQuoteEventData, WebhookOutSubscriptionEvent, WebhookOutSubscriptionEventData,
};
use crate::api_rest::{AppState, api_routes};
use common_domain::ids::{
    AddOnId, BillableMetricId, CouponId, CreditNoteId, CustomerId, EventId, InvoiceId, PlanId,
    ProductFamilyId, ProductId, QuoteId, SubscriptionId,
};
use strum::IntoEnumIterator;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;

pub fn generate_spec() {
    let path = "spec/api/v1/openapi.json";

    println!("Generating OpenAPI spec {path:?}");

    let (_router, mut open_api) = OpenApiRouter::<AppState>::with_openapi(ApiDoc::openapi())
        .merge(api_routes())
        .split_for_parts();

    add_rate_limit_responses(&mut open_api);

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
        WebhookOutPlanEvent,
        WebhookOutProductEvent,
        WebhookOutCouponEvent,
        WebhookOutAddOnEvent,
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

/// Adds a `429 Too Many Requests` response to every operation whose path starts
/// with `/api/`. Call this **after** routes have been merged into the `OpenApi`
/// object (i.e. after `split_for_parts()`), because utoipa modifiers run before
/// routes are incorporated and would see an empty path list.
pub fn add_rate_limit_responses(openapi: &mut utoipa::openapi::OpenApi) {
    use utoipa::openapi::content::ContentBuilder;
    use utoipa::openapi::response::ResponseBuilder;
    use utoipa::openapi::{Ref, RefOr};

    let rate_limited_response = RefOr::T(
        ResponseBuilder::new()
            .description("Too many requests")
            .content(
                "application/json",
                ContentBuilder::new()
                    .schema(Some(RefOr::Ref(Ref::from_schema_name("RestErrorResponse"))))
                    .build(),
            )
            .build(),
    );

    for (path, path_item) in openapi.paths.paths.iter_mut() {
        if !path.starts_with("/api/") {
            continue;
        }

        for operation in [
            path_item.get.as_mut(),
            path_item.put.as_mut(),
            path_item.post.as_mut(),
            path_item.delete.as_mut(),
            path_item.options.as_mut(),
            path_item.head.as_mut(),
            path_item.patch.as_mut(),
            path_item.trace.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            operation
                .responses
                .responses
                .entry("429".to_string())
                .or_insert_with(|| rate_limited_response.clone());
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
                WebhookOutEventGroupEnum::Plan => {
                    let event = WebhookOutPlanEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutPlanEventData {
                            plan_id: PlanId::default(),
                            name: "Enterprise Plan".to_string(),
                            description: Some("Our enterprise tier".to_string()),
                            plan_type: crate::api_rest::plans::model::PlanTypeEnum::Standard,
                            status: crate::api_rest::plans::model::PlanStatusEnum::Active,
                            currency: "USD".to_string(),
                            version: 1,
                            created_at: Default::default(),
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::Product => {
                    let event = WebhookOutProductEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutProductEventData {
                            product_id: ProductId::default(),
                            name: "API Calls Product".to_string(),
                            description: Some("Usage-based API calls product".to_string()),
                            fee_type: ProductFeeTypeEnum::Usage,
                            product_family_id: ProductFamilyId::default(),
                            created_at: Default::default(),
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::Coupon => {
                    let event = WebhookOutCouponEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutCouponEventData {
                            coupon_id: CouponId::default(),
                            code: "WELCOME20".to_string(),
                            description: "20% off for new customers".to_string(),
                            discount: crate::api_rest::coupons::model::CouponDiscount::Percentage(
                                crate::api_rest::coupons::model::PercentageDiscount {
                                    percentage: "20".to_string(),
                                },
                            ),
                            expires_at: None,
                            redemption_limit: Some(100),
                            recurring_value: Some(3),
                            reusable: false,
                            disabled: false,
                            created_at: Default::default(),
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::AddOn => {
                    let event = WebhookOutAddOnEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutAddOnEventData {
                            add_on_id: AddOnId::default(),
                            name: "Extra Storage".to_string(),
                            description: Some("Additional storage capacity".to_string()),
                            product_id: ProductId::default(),
                            price_id: common_domain::ids::PriceId::default(),
                            fee_type: Some(ProductFeeTypeEnum::Usage),
                            self_serviceable: true,
                            max_instances_per_subscription: Some(5),
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
