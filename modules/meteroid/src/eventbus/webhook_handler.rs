use cached::proc_macro::cached;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use uuid::Uuid;

use common_eventbus::{Event, EventData, TenantEventDataDetails};
use common_eventbus::{EventBusError, EventHandler};
use meteroid_store::domain::enums::WebhookOutEventTypeEnum;
use meteroid_store::domain::webhooks::WebhookOutEventNew;
use meteroid_store::domain::DetailedInvoice;
use meteroid_store::repositories::webhooks::WebhooksInterface;
use meteroid_store::repositories::{CustomersInterface, InvoiceInterface, SubscriptionInterface};
use meteroid_store::{crypt, Store};

use crate::webhook;
use crate::webhook::Webhook;

const ENDPOINT_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const ENDPOINT_RETRIES: u32 = 3;

pub struct WebhookHandler {
    pub store: Store,
    pub crypt_key: SecretString,
    pub client: ClientWithMiddleware,
    pub cache_enabled: bool,
}

impl WebhookHandler {
    pub fn new(store: Store, crypt_key: SecretString, cache_enabled: bool) -> Self {
        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(ENDPOINT_RETRIES);
        let client = ClientBuilder::new(reqwest::Client::new())
            // Retry failed requests.
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        WebhookHandler {
            store,
            crypt_key,
            client,
            cache_enabled,
        }
    }

    #[tracing::instrument(skip_all)]
    async fn send_webhook_event(
        &self,
        event: &Event,
        webhook_event: &WebhookEvent,
        webhook_event_payload: &Vec<u8>,
        endpoint: &Endpoint,
    ) -> Result<reqwest::Response, EventBusError> {
        log::debug!(
            "Sending event {} to endpoint {}",
            event.event_id,
            endpoint.url
        );

        let webhook = Webhook::new(endpoint.secret.as_str()).map_err(|e| {
            EventBusError::EventHandlerFailed(format!("Invalid webhook signature: {}", e))
        })?;

        let signature = webhook
            .sign(
                event.event_id.to_string().as_str(),
                event.event_timestamp.timestamp(),
                webhook_event_payload.as_slice(),
            )
            .map_err(|e| {
                EventBusError::EventHandlerFailed(format!("Failed to sign event: {}", e))
            })?;

        self.client
            .post(&endpoint.url)
            .timeout(ENDPOINT_REQUEST_TIMEOUT)
            .header(webhook::HEADER_WEBHOOK_ID, event.event_id.to_string())
            .header(
                webhook::HEADER_WEBHOOK_TIMESTAMP,
                event.event_timestamp.timestamp(),
            )
            .header(webhook::HEADER_WEBHOOK_SIGNATURE, signature)
            .json(&webhook_event)
            .send()
            .await
            .map_err(|e| {
                EventBusError::EventHandlerFailed(format!(
                    "Failed to send event to endpoint: {}",
                    e
                ))
            })
    }

    #[tracing::instrument(skip_all)]
    async fn log_endpoint_response_to_db(
        &self,
        event: &Event,
        endpoint: &Endpoint,
        webhook_event_payload: &Vec<u8>,
        endpoint_response: Result<reqwest::Response, EventBusError>,
    ) -> Result<(), EventBusError> {
        let event_type = get_event_type(event).ok_or_else(|| {
            EventBusError::EventHandlerFailed("Failed to get event type".to_string())
        })?;

        let request_body = String::from_utf8(webhook_event_payload.clone()).map_err(|e| {
            EventBusError::EventHandlerFailed(format!("Failed to convert payload to string: {}", e))
        })?;

        let (http_status_code, response_body, error_message) = match endpoint_response {
            Ok(r) => (Some(r.status().as_u16() as i16), r.text().await.ok(), None),
            Err(e) => (None, None, Some(e.to_string())),
        };

        let _ = self
            .store
            .insert_webhook_event(WebhookOutEventNew {
                endpoint_id: endpoint.id,
                created_at: chrono::Utc::now().naive_utc(),
                event_type,
                request_body,
                response_body,
                http_status_code,
                error_message,
            })
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn get_active_endpoints(&self, event: &Event) -> Result<Vec<Endpoint>, EventBusError> {
        let event_type = get_event_type(event);
        let details = get_tenant_event_details(event);

        let endpoints = if let (Some(event_type), Some(details)) = (event_type, details) {
            let endpoints = if self.cache_enabled {
                get_active_endpoints_by_tenant_cached(
                    self.store.clone(),
                    &details.tenant_id,
                    &self.crypt_key,
                )
                .await?
            } else {
                get_active_endpoints_by_tenant(
                    self.store.clone(),
                    &details.tenant_id,
                    &self.crypt_key,
                )
                .await?
            };

            endpoints
                .into_iter()
                .filter(|e| e.event_types.contains(&event_type))
                .collect()
        } else {
            vec![]
        };

        Ok(endpoints)
    }

    #[tracing::instrument(skip_all)]
    async fn customer_created_webhook(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<WebhookEvent, EventBusError> {
        let customer = self
            .store
            .find_customer_by_id(event_data_details.entity_id)
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        let event = WebhookEvent {
            event_type: "customer.created".to_string(),
            timestamp: event.event_timestamp,
            data: to_json(CustomerData {
                name: customer.name,
                email: customer.email,
                invoicing_email: customer.invoicing_email,
                phone: customer.phone,
                balance_value_cents: customer.balance_value_cents,
            })?,
        };

        Ok(event)
    }

    #[tracing::instrument(skip_all)]
    async fn subscription_created_webhook(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<WebhookEvent, EventBusError> {
        let subscription = self
            .store
            .get_subscription_details(event_data_details.tenant_id, event_data_details.entity_id)
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        let event = WebhookEvent {
            event_type: "subscription.created".to_string(),
            timestamp: event.event_timestamp,
            data: to_json(SubscriptionData {
                customer_name: subscription.customer_name,
                billing_day: subscription.billing_day,
                billing_start_date: subscription.billing_start_date,
                billing_end_date: subscription.billing_end_date,
                currency: subscription.currency,
                net_terms: subscription.net_terms,
            })?,
        };

        Ok(event)
    }

    #[tracing::instrument(skip_all)]
    async fn invoice_draft_webhook(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<WebhookEvent, EventBusError> {
        let DetailedInvoice {
            invoice,
            customer,
            plan,
        } = self
            .store
            .find_invoice_by_id(event_data_details.tenant_id, event_data_details.entity_id)
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        let event = WebhookEvent {
            event_type: "invoice.draft".to_string(),
            timestamp: event.event_timestamp,
            data: to_json(InvoiceData {
                customer_name: customer.name,
                currency: invoice.currency,
                status: "draft".to_string(),
                invoice_date: invoice.invoice_date,
                amount_cents: Some(invoice.total),
                plan_name: plan.map(|p| p.plan_name),
            })?,
        };

        Ok(event)
    }

    #[tracing::instrument(skip_all)]
    async fn invoice_finalized_webhook(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<WebhookEvent, EventBusError> {
        let DetailedInvoice {
            invoice, customer, ..
        } = self
            .store
            .find_invoice_by_id(event_data_details.tenant_id, event_data_details.entity_id)
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        let event = WebhookEvent {
            event_type: "invoice.finalized".to_string(),
            timestamp: event.event_timestamp,
            data: to_json(InvoiceData {
                customer_name: customer.name,
                currency: invoice.currency,
                status: "finalized".to_string(),
                invoice_date: invoice.invoice_date,
                amount_cents: Some(invoice.total),
                plan_name: invoice.plan_name,
            })?,
        };

        Ok(event)
    }
}

#[async_trait::async_trait]
impl EventHandler<Event> for WebhookHandler {
    #[tracing::instrument(skip_all)]
    async fn handle(&self, event: Event) -> Result<(), EventBusError> {
        log::debug!("Handling event: {:?}", event);

        let endpoints = self.get_active_endpoints(&event).await?;

        if endpoints.is_empty() {
            log::debug!("No active endpoints found for event: {:?}", event);
            return Ok(());
        }

        let webhook_event = match &event.event_data {
            EventData::CustomerCreated(details) => {
                self.customer_created_webhook(&event, details).await?
            }
            EventData::SubscriptionCreated(details) => {
                self.subscription_created_webhook(&event, details).await?
            }
            EventData::InvoiceCreated(details) => {
                self.invoice_draft_webhook(&event, details).await?
            }
            EventData::InvoiceFinalized(details) => {
                self.invoice_finalized_webhook(&event, details).await?
            }
            _ => {
                log::debug!("Skipping event: {:?}", &event);
                return Ok(());
            }
        };

        let webhook_event_payload = serde_json::to_vec(&webhook_event).map_err(|e| {
            EventBusError::EventHandlerFailed(format!("Failed to serialize event: {}", e))
        })?;

        for endpoint in endpoints {
            let send_result = self
                .send_webhook_event(&event, &webhook_event, &webhook_event_payload, &endpoint)
                .await;

            let log_result = self
                .log_endpoint_response_to_db(&event, &endpoint, &webhook_event_payload, send_result)
                .await;

            if let Err(e) = log_result {
                log::error!("Failed to log webhook event: {}", e);
            }
        }

        Ok(())
    }
}

#[derive(Clone)]
struct Endpoint {
    pub id: Uuid,
    pub url: String,
    pub secret: String,
    pub event_types: Vec<WebhookOutEventTypeEnum>,
}

#[derive(Serialize)]
struct WebhookEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub data: serde_json::Value,
}

#[derive(Serialize)]
struct CustomerData {
    pub name: String,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i32,
}

#[derive(Serialize)]
struct SubscriptionData {
    pub customer_name: String,
    pub billing_day: i16,
    pub billing_start_date: chrono::NaiveDate,
    pub billing_end_date: Option<chrono::NaiveDate>,
    pub currency: String,
    pub net_terms: u32,
}

#[derive(Serialize)]
struct InvoiceData {
    pub customer_name: String,
    pub currency: String,
    pub status: String,
    pub invoice_date: chrono::NaiveDate,
    pub amount_cents: Option<i64>,
    pub plan_name: Option<String>,
}

fn to_json<T: Serialize>(data: T) -> Result<serde_json::Value, EventBusError> {
    serde_json::to_value(data).map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))
}

fn get_event_type(event: &Event) -> Option<WebhookOutEventTypeEnum> {
    match &event.event_data {
        EventData::CustomerCreated(_) => Some(WebhookOutEventTypeEnum::CustomerCreated),
        EventData::SubscriptionCreated(_) => Some(WebhookOutEventTypeEnum::SubscriptionCreated),
        EventData::InvoiceCreated(_) => Some(WebhookOutEventTypeEnum::InvoiceCreated),
        EventData::InvoiceFinalized(_) => Some(WebhookOutEventTypeEnum::InvoiceFinalized),
        _ => None,
    }
}

fn get_tenant_event_details(event: &Event) -> Option<&TenantEventDataDetails> {
    match &event.event_data {
        EventData::CustomerCreated(d) => Some(d),
        EventData::SubscriptionCreated(d) => Some(d),
        EventData::InvoiceCreated(d) => Some(d),
        EventData::InvoiceFinalized(d) => Some(d),
        _ => None,
    }
}

#[tracing::instrument(skip_all)]
async fn get_active_endpoints_by_tenant(
    store: Store,
    tenant_id: &Uuid,
    crypt_key: &SecretString,
) -> Result<Vec<Endpoint>, EventBusError> {
    let endpoints = store
        .list_webhook_out_endpoints(tenant_id.clone())
        .await
        .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?
        .into_iter()
        .filter_map(|e| {
            if e.enabled {
                let secret = crypt::decrypt(crypt_key, e.secret.expose_secret()).ok()?;

                Some(Endpoint {
                    id: e.id,
                    url: e.url.to_string(),
                    secret: secret.expose_secret().to_string(),
                    event_types: e.events_to_listen,
                })
            } else {
                None
            }
        })
        .collect::<Vec<Endpoint>>();

    Ok(endpoints)
}

#[tracing::instrument(skip_all)]
#[cached(
    result = true,
    size = 20,
    time = 120, // 2 min
    key = "String",
    convert = r#"{ tenant_id.to_string() }"#
)]
async fn get_active_endpoints_by_tenant_cached(
    store: Store,
    tenant_id: &Uuid,
    crypt_key: &SecretString,
) -> Result<Vec<Endpoint>, EventBusError> {
    get_active_endpoints_by_tenant(store, tenant_id, crypt_key).await
}
