use std::collections::HashMap;
use std::sync::Arc;

use common_domain::actor::Actor;
use common_domain::ids::{
    ApiTokenId, BankAccountId, BaseId, BillableMetricId, CustomerId, InvoiceId, OrganizationId,
    PlanVersionId, PriceComponentId, ProductFamilyId, SubscriptionId, TenantId, UserId,
};
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
    pub actor: Option<Actor>,
}

impl Event {
    pub fn new(event_data: EventData, actor: Option<Actor>) -> Self {
        Self {
            event_id: Uuid::now_v7(),
            event_timestamp: chrono::Utc::now(),
            event_data,
            actor,
        }
    }

    pub fn api_token_created(actor: Actor, api_token_id: ApiTokenId, tenant_id: TenantId) -> Self {
        Self::new(
            EventData::ApiTokenCreated(TenantEventDataDetails {
                entity_id: api_token_id.as_uuid(),
                tenant_id,
            }),
            Some(actor),
        )
    }

    pub fn api_token_revoked(actor: Actor, api_token_id: ApiTokenId, tenant_id: TenantId) -> Self {
        Self::new(
            EventData::ApiTokenRevoked(TenantEventDataDetails {
                entity_id: api_token_id.as_uuid(),
                tenant_id,
            }),
            Some(actor),
        )
    }

    pub fn bank_account_created(
        actor: Actor,
        bank_account_id: BankAccountId,
        tenant_id: TenantId,
    ) -> Self {
        Self::new(
            EventData::BankAccountCreated(TenantEventDataDetails {
                tenant_id,
                entity_id: bank_account_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn bank_account_edited(
        actor: Actor,
        bank_account_id: BankAccountId,
        tenant_id: TenantId,
    ) -> Self {
        Self::new(
            EventData::BankAccountEdited(TenantEventDataDetails {
                tenant_id,
                entity_id: bank_account_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn billable_metric_created(
        actor: Actor,
        billable_metric_id: BillableMetricId,
        tenant_id: TenantId,
    ) -> Self {
        Self::new(
            EventData::BillableMetricCreated(TenantEventDataDetails {
                tenant_id,
                entity_id: billable_metric_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn customer_created(actor: Actor, customer_id: CustomerId, tenant_id: TenantId) -> Self {
        Self::new(
            EventData::CustomerCreated(TenantEventDataDetails {
                tenant_id,
                entity_id: customer_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn customer_patched(actor: Actor, customer_id: CustomerId, tenant_id: TenantId) -> Self {
        Self::new(
            EventData::CustomerPatched(TenantEventDataDetails {
                tenant_id,
                entity_id: customer_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn customer_updated(actor: Actor, customer_id: CustomerId, tenant_id: TenantId) -> Self {
        Self::new(
            EventData::CustomerUpdated(TenantEventDataDetails {
                tenant_id,
                entity_id: customer_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn organization_created(actor: Actor, organization_id: OrganizationId) -> Self {
        Self::new(
            EventData::OrganizationCreated(EventDataDetails {
                entity_id: organization_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn invoice_created(invoice_id: InvoiceId, tenant_id: TenantId) -> Self {
        Self::new(
            EventData::InvoiceCreated(TenantEventDataDetails {
                tenant_id,
                entity_id: invoice_id.as_uuid(),
            }),
            None,
        )
    }

    pub fn invoice_finalized(invoice_id: InvoiceId, tenant_id: TenantId) -> Self {
        Self::new(
            EventData::InvoiceFinalized(TenantEventDataDetails {
                tenant_id,
                entity_id: invoice_id.as_uuid(),
            }),
            None,
        )
    }

    pub fn plan_created_draft(
        actor: Actor,
        plan_version_id: PlanVersionId,
        tenant_id: TenantId,
    ) -> Self {
        Self::new(
            EventData::PlanCreatedDraft(TenantEventDataDetails {
                tenant_id,
                entity_id: plan_version_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn plan_published_version(
        actor: Actor,
        plan_version_id: PlanVersionId,
        tenant_id: TenantId,
    ) -> Self {
        Self::new(
            EventData::PlanPublishedVersion(TenantEventDataDetails {
                tenant_id,
                entity_id: plan_version_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn plan_discarded_version(
        actor: Actor,
        plan_version_id: PlanVersionId,
        tenant_id: TenantId,
    ) -> Self {
        Self::new(
            EventData::PlanDiscardedVersion(TenantEventDataDetails {
                tenant_id,
                entity_id: plan_version_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn price_component_created(
        actor: Actor,
        price_component_id: PriceComponentId,
        tenant_id: TenantId,
    ) -> Self {
        Self::new(
            EventData::PriceComponentCreated(TenantEventDataDetails {
                tenant_id,
                entity_id: price_component_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn price_component_edited(
        actor: Actor,
        price_component_id: PriceComponentId,
        tenant_id: TenantId,
    ) -> Self {
        Self::new(
            EventData::PriceComponentEdited(TenantEventDataDetails {
                tenant_id,
                entity_id: price_component_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn price_component_removed(
        actor: Actor,
        price_component_id: PriceComponentId,
        tenant_id: TenantId,
    ) -> Self {
        Self::new(
            EventData::PriceComponentRemoved(TenantEventDataDetails {
                tenant_id,
                entity_id: price_component_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn product_family_created(
        actor: Option<Actor>,
        product_family_id: ProductFamilyId,
        tenant_id: TenantId,
    ) -> Self {
        Self::new(
            EventData::ProductFamilyCreated(TenantEventDataDetails {
                tenant_id,
                entity_id: product_family_id.as_uuid(),
            }),
            actor,
        )
    }

    pub fn subscription_created(
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> Self {
        Self::new(
            EventData::SubscriptionCreated(TenantEventDataDetails {
                tenant_id,
                entity_id: subscription_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn subscription_canceled(
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> Self {
        Self::new(
            EventData::SubscriptionCanceled(TenantEventDataDetails {
                tenant_id,
                entity_id: subscription_id.as_uuid(),
            }),
            Some(actor),
        )
    }

    pub fn user_created(actor: Option<Actor>, user_id: UserId) -> Self {
        Self::new(
            EventData::UserCreated(EventDataDetails {
                entity_id: user_id.as_uuid(),
            }),
            actor,
        )
    }

    pub fn user_updated(
        actor: Actor,
        user_id: UserId,
        department: Option<String>,
        know_us_from: Option<String>,
    ) -> Self {
        Self::new(
            EventData::UserUpdated(EventDataWithMetadataDetails {
                entity_id: user_id.as_uuid(),
                metadata: HashMap::from_iter(vec![
                    (
                        "department".to_string(),
                        department.unwrap_or("undefined".to_string()),
                    ),
                    (
                        "know_us_from".to_string(),
                        know_us_from.unwrap_or("undefined".to_string()),
                    ),
                ]),
            }),
            Some(actor),
        )
    }
}

#[derive(Debug, Clone)]
pub enum EventData {
    ApiTokenCreated(TenantEventDataDetails),
    ApiTokenRevoked(TenantEventDataDetails),
    BankAccountCreated(TenantEventDataDetails),
    BankAccountEdited(TenantEventDataDetails),
    BillableMetricCreated(TenantEventDataDetails),
    CustomerCreated(TenantEventDataDetails),
    CustomerPatched(TenantEventDataDetails),
    CustomerUpdated(TenantEventDataDetails),
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
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct TenantEventDataDetails {
    pub tenant_id: TenantId,
    pub entity_id: Uuid,
}
