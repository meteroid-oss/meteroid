use std::sync::Arc;

use uuid::Uuid;

#[async_trait::async_trait]
pub trait EventBus<E>: Send + Sync {
    async fn subscribe(&self, handler: Arc<dyn EventHandler<E>>);
    async fn publish(&self, event: E) -> Result<(), EventBusError>;
}

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
            event_id: Uuid::now_v7(),
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

    pub fn organization_created(actor: Uuid, organization_id: Uuid) -> Self {
        Self::new(
            EventData::OrganizationCreated(EventDataDetails {
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

    pub fn product_family_created(
        actor: Option<Uuid>,
        product_family_id: Uuid,
        tenant_id: Uuid,
    ) -> Self {
        Self::new(
            EventData::ProductFamilyCreated(TenantEventDataDetails {
                tenant_id,
                entity_id: product_family_id,
            }),
            actor,
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

    pub fn user_updated(
        actor: Uuid,
        user_id: Uuid,
        department: Option<String>,
        know_us_from: Option<String>,
    ) -> Self {
        Self::new(
            EventData::UserUpdated(EventDataWithMetadataDetails {
                entity_id: user_id,
                metadata: vec![
                    (
                        "department".to_string(),
                        department.unwrap_or("undefined".to_string()),
                    ),
                    (
                        "know_us_from".to_string(),
                        know_us_from.unwrap_or("undefined".to_string()),
                    ),
                ],
            }),
            Some(actor),
        )
    }
}

#[derive(Debug, Clone)]
pub enum EventData {
    ApiTokenCreated(EventDataDetails),
    BillableMetricCreated(TenantEventDataDetails),
    CustomerCreated(TenantEventDataDetails),
    CustomerPatched(TenantEventDataDetails),
    OrganizationCreated(EventDataDetails),
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
    UserUpdated(EventDataWithMetadataDetails),
}

#[derive(Debug, Clone)]
pub struct EventDataDetails {
    pub entity_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct EventDataWithMetadataDetails {
    pub entity_id: Uuid,
    pub metadata: Vec<(String, String)>,
}

impl EventDataWithMetadataDetails {
    pub fn metadata_as_hashmap(&self) -> std::collections::HashMap<String, String> {
        self.metadata.iter().cloned().collect()
    }
}

#[derive(Debug, Clone)]
pub struct TenantEventDataDetails {
    pub tenant_id: Uuid,
    pub entity_id: Uuid,
}
