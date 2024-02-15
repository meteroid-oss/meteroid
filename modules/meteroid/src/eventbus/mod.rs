use std::fmt::Debug;
use std::sync::Arc;

use uuid::Uuid;

use crate::api::services::utils::uuid_gen;
use crate::config::Config;
use crate::repo::get_pool;

pub mod analytics_handler;
pub mod memory;
pub mod webhook_handler;

static CONFIG: tokio::sync::OnceCell<Arc<dyn EventBus<Event>>> = tokio::sync::OnceCell::const_new();

#[derive(thiserror::Error, Debug, Clone)]
pub enum EventBusError {
    #[error("Failed to publish event")]
    PublishFailed,
    #[error("Failed to handle event: {0}")]
    EventHandlerFailed(String),
}

#[async_trait::async_trait]
pub trait EventHandler<E>: Send + Sync {
    async fn handle(&self, event: E) -> Result<(), EventBusError>;
}

#[async_trait::async_trait]
pub trait EventBus<E>: Send + Sync {
    async fn subscribe(&self, handler: Arc<dyn EventHandler<E>>);
    async fn publish(&self, event: E) -> Result<(), EventBusError>;
}

pub struct EventBusStatic;

impl EventBusStatic {
    pub async fn get() -> &'static Arc<dyn EventBus<Event>> {
        CONFIG
            .get_or_init(|| async {
                let config = Config::get();
                let pool = get_pool();

                let bus: Arc<dyn EventBus<Event>> = Arc::new(memory::InMemory::new());

                bus.subscribe(Arc::new(webhook_handler::WebhookHandler::new(
                    pool.clone(),
                    config.secrets_crypt_key.clone(),
                    true,
                )))
                .await;

                bus.subscribe(Arc::new(webhook_handler::WebhookHandler::new(
                    pool.clone(),
                    config.secrets_crypt_key.clone(),
                    true,
                )))
                .await;

                if config.analytics.enabled {
                    bus.subscribe(Arc::new(analytics_handler::AnalyticsHandler::new(
                        config.analytics.clone(),
                        pool.clone(),
                    )))
                    .await;
                } else {
                    log::info!("Analytics is disabled");
                }

                bus
            })
            .await
    }
}

#[derive(Debug, Clone)]
pub struct Event {
    pub event_id: Uuid,
    pub event_timestamp: chrono::DateTime<chrono::Utc>,
    pub event_data: EventData,
    pub actor: Option<Uuid>,
}

impl Event {
    pub fn new(event_data: EventData, actor: Option<Uuid>) -> Self {
        Self {
            event_id: uuid_gen::v7(),
            event_timestamp: chrono::Utc::now(),
            event_data,
            actor,
        }
    }

    pub fn api_token_created(actor: Uuid, api_token_id: Uuid) -> Self {
        Self::new(
            EventData::ApiTokenCreated(EventDataDetails {
                entity_id: api_token_id,
            }),
            Some(actor),
        )
    }

    pub fn billable_metric_created(actor: Uuid, billable_metric_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(
            EventData::BillableMetricCreated(TenantEventDataDetails {
                tenant_id,
                entity_id: billable_metric_id,
            }),
            Some(actor),
        )
    }

    pub fn customer_created(actor: Uuid, customer_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(
            EventData::CustomerCreated(TenantEventDataDetails {
                tenant_id,
                entity_id: customer_id,
            }),
            Some(actor),
        )
    }

    pub fn subscription_created(actor: Uuid, subscription_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(
            EventData::SubscriptionCreated(TenantEventDataDetails {
                tenant_id,
                entity_id: subscription_id,
            }),
            Some(actor),
        )
    }

    pub fn invoice_created(invoice_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(
            EventData::InvoiceCreated(TenantEventDataDetails {
                tenant_id,
                entity_id: invoice_id,
            }),
            None,
        )
    }

    pub fn invoice_finalized(invoice_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(
            EventData::InvoiceFinalized(TenantEventDataDetails {
                tenant_id,
                entity_id: invoice_id,
            }),
            None,
        )
    }
}

#[derive(Debug, Clone)]
pub enum EventData {
    ApiTokenCreated(EventDataDetails),
    BillableMetricCreated(TenantEventDataDetails),
    OrganizationCreated(EventDataDetails),
    TenantCreated(TenantEventDataDetails),
    CustomerCreated(TenantEventDataDetails),
    SubscriptionCreated(TenantEventDataDetails),
    InvoiceCreated(TenantEventDataDetails),
    InvoiceFinalized(TenantEventDataDetails),
}

#[derive(Debug, Clone)]
pub struct EventDataDetails {
    pub entity_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct TenantEventDataDetails {
    pub tenant_id: Uuid,
    pub entity_id: Uuid,
}
