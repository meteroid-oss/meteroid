use std::fmt::Debug;
use std::sync::Arc;

use uuid::Uuid;

use crate::api::utils::uuid_gen;
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
                    let country = match crate::eventbus::analytics_handler::get_geoip().await {
                        Ok(geoip) => Some(geoip.country),
                        Err(err) => {
                            log::warn!("Failed to obtain data for analytics: {}", err);
                            None
                        }
                    };

                    bus.subscribe(Arc::new(analytics_handler::AnalyticsHandler::new(
                        config.analytics.clone(),
                        pool.clone(),
                        country,
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
    pub fn customer_patched(actor: Uuid, customer_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(
            EventData::CustomerPatched(TenantEventDataDetails {
                tenant_id,
                entity_id: customer_id,
            }),
            Some(actor),
        )
    }

    pub fn instance_inited(actor: Uuid, organization_id: Uuid) -> Self {
        Self::new(
            EventData::InstanceInited(EventDataDetails {
                entity_id: organization_id,
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

    pub fn plan_created_draft(actor: Uuid, plan_version_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(
            EventData::PlanCreatedDraft(TenantEventDataDetails {
                tenant_id,
                entity_id: plan_version_id,
            }),
            Some(actor),
        )
    }

    pub fn plan_published_version(actor: Uuid, plan_version_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(
            EventData::PlanPublishedVersion(TenantEventDataDetails {
                tenant_id,
                entity_id: plan_version_id,
            }),
            Some(actor),
        )
    }

    pub fn plan_discarded_version(actor: Uuid, plan_version_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(
            EventData::PlanDiscardedVersion(TenantEventDataDetails {
                tenant_id,
                entity_id: plan_version_id,
            }),
            Some(actor),
        )
    }

    pub fn price_component_created(actor: Uuid, price_component_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(
            EventData::PriceComponentCreated(TenantEventDataDetails {
                tenant_id,
                entity_id: price_component_id,
            }),
            Some(actor),
        )
    }

    pub fn price_component_edited(actor: Uuid, price_component_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(
            EventData::PriceComponentEdited(TenantEventDataDetails {
                tenant_id,
                entity_id: price_component_id,
            }),
            Some(actor),
        )
    }

    pub fn price_component_removed(actor: Uuid, price_component_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(
            EventData::PriceComponentRemoved(TenantEventDataDetails {
                tenant_id,
                entity_id: price_component_id,
            }),
            Some(actor),
        )
    }

    pub fn product_family_created(actor: Uuid, product_family_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(
            EventData::ProductFamilyCreated(TenantEventDataDetails {
                tenant_id,
                entity_id: product_family_id,
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

    pub fn subscription_canceled(actor: Uuid, subscription_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(
            EventData::SubscriptionCanceled(TenantEventDataDetails {
                tenant_id,
                entity_id: subscription_id,
            }),
            Some(actor),
        )
    }

    pub fn user_created(actor: Option<Uuid>, user_id: Uuid) -> Self {
        Self::new(
            EventData::UserCreated(EventDataDetails { entity_id: user_id }),
            actor,
        )
    }
}

#[derive(Debug, Clone)]
pub enum EventData {
    ApiTokenCreated(EventDataDetails),
    BillableMetricCreated(TenantEventDataDetails),
    CustomerCreated(TenantEventDataDetails),
    CustomerPatched(TenantEventDataDetails),
    InstanceInited(EventDataDetails),
    InvoiceCreated(TenantEventDataDetails),
    InvoiceFinalized(TenantEventDataDetails),
    PlanCreatedDraft(TenantEventDataDetails),
    PlanPublishedVersion(TenantEventDataDetails),
    PlanDiscardedVersion(TenantEventDataDetails),
    PriceComponentCreated(TenantEventDataDetails),
    PriceComponentEdited(TenantEventDataDetails),
    PriceComponentRemoved(TenantEventDataDetails),
    ProductFamilyCreated(TenantEventDataDetails),
    SubscriptionCreated(TenantEventDataDetails),
    SubscriptionCanceled(TenantEventDataDetails),
    TenantCreated(TenantEventDataDetails),
    UserCreated(EventDataDetails),
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
