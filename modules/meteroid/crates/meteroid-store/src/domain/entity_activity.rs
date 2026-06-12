use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, EntityActivityId, TenantId};
use diesel_models::entity_activity::{EntityActivityRow, EntityActivityRowNew};
use o2o::o2o;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use uuid::Uuid;

pub use common_domain::actor::{Actor, ActorType};

use crate::domain::outbox_event::OutboxEvent;

#[derive(Debug, Clone, Copy, Display, EnumString, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    AddOn,
    BillableMetric,
    Coupon,
    ApiToken,
    Connector,
    CreditNote,
    Customer,
    Invoice,
    InvoicingEntity,
    PaymentTransaction,
    Plan,
    Product,
    Quote,
    Subscription,
    Tenant,
    User,
    WebhookEndpoint,
}

impl EntityType {
    pub fn id_as_proto(&self, raw: uuid::Uuid) -> String {
        use common_domain::ids::*;
        match self {
            Self::AddOn => AddOnId::from(raw).as_base62(),
            Self::BillableMetric => BillableMetricId::from(raw).as_base62(),
            Self::Connector => ConnectorId::from(raw).as_base62(),
            Self::Coupon => CouponId::from(raw).as_base62(),
            Self::CreditNote => CreditNoteId::from(raw).as_base62(),
            Self::Customer => CustomerId::from(raw).as_base62(),
            Self::Invoice => InvoiceId::from(raw).as_base62(),
            Self::InvoicingEntity => InvoicingEntityId::from(raw).as_base62(),
            Self::PaymentTransaction => PaymentTransactionId::from(raw).as_base62(),
            Self::Plan => PlanId::from(raw).as_base62(),
            Self::Product => ProductId::from(raw).as_base62(),
            Self::Quote => QuoteId::from(raw).as_base62(),
            Self::Subscription => SubscriptionId::from(raw).as_base62(),
            Self::Tenant => TenantId::from(raw).as_base62(),
            Self::ApiToken => common_domain::ids::ApiTokenId::from(raw).as_base62(),
            Self::User => common_domain::ids::UserId::from(raw).as_base62(),
            Self::WebhookEndpoint => raw.to_string(),
        }
    }

    pub fn parse_id_proto(&self, raw: &str) -> Result<uuid::Uuid, String> {
        use common_domain::ids::*;
        use std::str::FromStr;
        match self {
            Self::AddOn => AddOnId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::BillableMetric => BillableMetricId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::Connector => ConnectorId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::Coupon => CouponId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::CreditNote => CreditNoteId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::Customer => CustomerId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::Invoice => InvoiceId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::InvoicingEntity => InvoicingEntityId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::PaymentTransaction => PaymentTransactionId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::Plan => PlanId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::Product => ProductId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::Quote => QuoteId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::Subscription => SubscriptionId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::Tenant => TenantId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::ApiToken => common_domain::ids::ApiTokenId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::User => common_domain::ids::UserId::from_str(raw)
                .map(|i| i.as_uuid())
                .map_err(|e| e.to_string()),
            Self::WebhookEndpoint => uuid::Uuid::parse_str(raw).map_err(|e| e.to_string()),
        }
    }
}

/// Builds the client-facing actor id string from the stored columns: UUID-keyed
/// actors become their prefixed base62 id, `QuoteRecipient` surfaces its alias,
/// `System` has none.
pub fn actor_id_as_proto(
    actor_type: ActorType,
    actor_uuid: Option<Uuid>,
    actor_alias: Option<&str>,
) -> Option<String> {
    use common_domain::ids::*;
    match actor_type {
        ActorType::User => actor_uuid.map(|u| UserId::from(u).as_base62()),
        ActorType::ApiToken => actor_uuid.map(|u| ApiTokenId::from(u).as_base62()),
        ActorType::Customer => actor_uuid.map(|u| CustomerId::from(u).as_base62()),
        ActorType::QuoteRecipient => actor_alias.map(|s| s.to_string()),
        ActorType::System => None,
    }
}

/// Stable strings — renames are a breaking API change. Format: <entity>.<verb>.
#[derive(Debug, Clone, Copy, Display, EnumString, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ActivityType {
    #[strum(serialize = "customer.created")]
    #[serde(rename = "customer.created")]
    CustomerCreated,
    #[strum(serialize = "customer.updated")]
    #[serde(rename = "customer.updated")]
    CustomerUpdated,
    #[strum(serialize = "customer.archived")]
    #[serde(rename = "customer.archived")]
    CustomerArchived,
    #[strum(serialize = "customer.unarchived")]
    #[serde(rename = "customer.unarchived")]
    CustomerUnarchived,

    #[strum(serialize = "billable_metric.created")]
    #[serde(rename = "billable_metric.created")]
    BillableMetricCreated,
    #[strum(serialize = "billable_metric.updated")]
    #[serde(rename = "billable_metric.updated")]
    BillableMetricUpdated,
    #[strum(serialize = "billable_metric.archived")]
    #[serde(rename = "billable_metric.archived")]
    BillableMetricArchived,

    #[strum(serialize = "invoice.created")]
    #[serde(rename = "invoice.created")]
    InvoiceCreated,
    #[strum(serialize = "invoice.finalized")]
    #[serde(rename = "invoice.finalized")]
    InvoiceFinalized,
    #[strum(serialize = "invoice.paid")]
    #[serde(rename = "invoice.paid")]
    InvoicePaid,
    #[strum(serialize = "invoice.voided")]
    #[serde(rename = "invoice.voided")]
    InvoiceVoided,
    #[strum(serialize = "invoice.consolidated")]
    #[serde(rename = "invoice.consolidated")]
    InvoiceConsolidated,

    #[strum(serialize = "credit_note.created")]
    #[serde(rename = "credit_note.created")]
    CreditNoteCreated,
    #[strum(serialize = "credit_note.finalized")]
    #[serde(rename = "credit_note.finalized")]
    CreditNoteFinalized,
    #[strum(serialize = "credit_note.voided")]
    #[serde(rename = "credit_note.voided")]
    CreditNoteVoided,

    #[strum(serialize = "subscription.created")]
    #[serde(rename = "subscription.created")]
    SubscriptionCreated,

    #[strum(serialize = "quote.created")]
    #[serde(rename = "quote.created")]
    QuoteCreated,
    #[strum(serialize = "quote.updated")]
    #[serde(rename = "quote.updated")]
    QuoteUpdated,
    #[strum(serialize = "quote.accepted")]
    #[serde(rename = "quote.accepted")]
    QuoteAccepted,
    #[strum(serialize = "quote.converted")]
    #[serde(rename = "quote.converted")]
    QuoteConverted,

    #[strum(serialize = "plan.created")]
    #[serde(rename = "plan.created")]
    PlanCreated,
    #[strum(serialize = "plan.published")]
    #[serde(rename = "plan.published")]
    PlanPublished,
    #[strum(serialize = "plan.archived")]
    #[serde(rename = "plan.archived")]
    PlanArchived,

    #[strum(serialize = "product.created")]
    #[serde(rename = "product.created")]
    ProductCreated,
    #[strum(serialize = "product.updated")]
    #[serde(rename = "product.updated")]
    ProductUpdated,
    #[strum(serialize = "product.archived")]
    #[serde(rename = "product.archived")]
    ProductArchived,

    #[strum(serialize = "coupon.created")]
    #[serde(rename = "coupon.created")]
    CouponCreated,
    #[strum(serialize = "coupon.updated")]
    #[serde(rename = "coupon.updated")]
    CouponUpdated,
    #[strum(serialize = "coupon.archived")]
    #[serde(rename = "coupon.archived")]
    CouponArchived,

    #[strum(serialize = "add_on.created")]
    #[serde(rename = "add_on.created")]
    AddOnCreated,
    #[strum(serialize = "add_on.updated")]
    #[serde(rename = "add_on.updated")]
    AddOnUpdated,
    #[strum(serialize = "add_on.archived")]
    #[serde(rename = "add_on.archived")]
    AddOnArchived,

    #[strum(serialize = "customer.logged_in")]
    #[serde(rename = "customer.logged_in")]
    CustomerPortalLoggedIn,

    #[strum(serialize = "quote.published")]
    #[serde(rename = "quote.published")]
    QuotePublished,
    #[strum(serialize = "quote.declined")]
    #[serde(rename = "quote.declined")]
    QuoteDeclined,
    #[strum(serialize = "quote.viewed")]
    #[serde(rename = "quote.viewed")]
    QuoteViewed,
    #[strum(serialize = "quote.signature_added")]
    #[serde(rename = "quote.signature_added")]
    QuoteSignatureAdded,
    #[strum(serialize = "quote.cancelled")]
    #[serde(rename = "quote.cancelled")]
    QuoteCancelled,
    #[strum(serialize = "quote.sent")]
    #[serde(rename = "quote.sent")]
    QuoteSent,

    #[strum(serialize = "subscription.paused")]
    #[serde(rename = "subscription.paused")]
    SubscriptionPaused,
    #[strum(serialize = "subscription.cancellation_scheduled")]
    #[serde(rename = "subscription.cancellation_scheduled")]
    SubscriptionCancellationScheduled,
    #[strum(serialize = "subscription.cancelled")]
    #[serde(rename = "subscription.cancelled")]
    SubscriptionCancelled,
    #[strum(serialize = "subscription.cancellation_undone")]
    #[serde(rename = "subscription.cancellation_undone")]
    SubscriptionCancellationUndone,
    #[strum(serialize = "subscription.plan_change_scheduled")]
    #[serde(rename = "subscription.plan_change_scheduled")]
    SubscriptionPlanChangeScheduled,
    #[strum(serialize = "subscription.plan_change_cancelled")]
    #[serde(rename = "subscription.plan_change_cancelled")]
    SubscriptionPlanChangeCancelled,
    #[strum(serialize = "subscription.plan_changed")]
    #[serde(rename = "subscription.plan_changed")]
    SubscriptionPlanChanged,
    #[strum(serialize = "subscription.amendment_scheduled")]
    #[serde(rename = "subscription.amendment_scheduled")]
    SubscriptionAmendmentScheduled,
    #[strum(serialize = "subscription.amendment_cancelled")]
    #[serde(rename = "subscription.amendment_cancelled")]
    SubscriptionAmendmentCancelled,
    #[strum(serialize = "subscription.amended")]
    #[serde(rename = "subscription.amended")]
    SubscriptionAmended,

    #[strum(serialize = "plan.draft_discarded")]
    #[serde(rename = "plan.draft_discarded")]
    PlanDraftDiscarded,

    #[strum(serialize = "api_token.created")]
    #[serde(rename = "api_token.created")]
    ApiTokenCreated,
    #[strum(serialize = "api_token.revoked")]
    #[serde(rename = "api_token.revoked")]
    ApiTokenRevoked,

    #[strum(serialize = "connector.connected")]
    #[serde(rename = "connector.connected")]
    ConnectorConnected,
    #[strum(serialize = "connector.disconnected")]
    #[serde(rename = "connector.disconnected")]
    ConnectorDisconnected,

    /// `metadata`: `{ "field", "from", "to" }`.
    #[strum(serialize = "entity.field_changed")]
    #[serde(rename = "entity.field_changed")]
    FieldChanged,

    /// `metadata`: `{ "changes": [{"field", "from", "to"}, ...] }`.
    #[strum(serialize = "entity.updated")]
    #[serde(rename = "entity.updated")]
    EntityUpdated,

    /// `metadata`: `{ "kind", "recipients", "subject", "preview" }`.
    #[strum(serialize = "entity.email_sent")]
    #[serde(rename = "entity.email_sent")]
    EmailSent,
}

#[derive(Debug, Clone)]
pub struct Activity {
    pub activity_type: ActivityType,
    pub entity_type: EntityType,
    pub entity_id: Uuid,
    pub metadata: Option<serde_json::Value>,
    /// Denormalized rollup refs so child events (invoice, credit_note, ...) surface
    /// on the customer/subscription timeline. CHECK constraint forbids setting these
    /// when the entity itself is the customer/subscription.
    pub agg_customer_id: Option<common_domain::ids::CustomerId>,
    pub agg_subscription_id: Option<common_domain::ids::SubscriptionId>,
}

impl Activity {
    pub fn new(activity_type: ActivityType, entity_type: EntityType, entity_id: Uuid) -> Self {
        Self {
            activity_type,
            entity_type,
            entity_id,
            metadata: None,
            agg_customer_id: None,
            agg_subscription_id: None,
        }
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn agg_customer(mut self, id: common_domain::ids::CustomerId) -> Self {
        self.agg_customer_id = Some(id);
        self
    }

    pub fn agg_subscription(mut self, id: common_domain::ids::SubscriptionId) -> Self {
        self.agg_subscription_id = Some(id);
        self
    }
}

/// `Outbox` writes the outbox row plus (if mappable) the derived audit row.
/// `Activity` writes the audit row only.
pub enum AuditInput {
    Outbox(OutboxEvent),
    Activity(Activity),
}

impl From<OutboxEvent> for AuditInput {
    fn from(e: OutboxEvent) -> Self {
        Self::Outbox(e)
    }
}

impl From<Activity> for AuditInput {
    fn from(a: Activity) -> Self {
        Self::Activity(a)
    }
}

/// Registry of which outbox events surface in the user-facing audit feed.
/// Returns None for internal pipeline events.
impl From<&OutboxEvent> for Option<Activity> {
    fn from(event: &OutboxEvent) -> Self {
        let activity = match event {
            OutboxEvent::CustomerCreated(e) => Activity::new(
                ActivityType::CustomerCreated,
                EntityType::Customer,
                e.customer_id.as_uuid(),
            ),
            OutboxEvent::CustomerUpdated(e) => Activity::new(
                ActivityType::CustomerUpdated,
                EntityType::Customer,
                e.customer_id.as_uuid(),
            ),
            OutboxEvent::BillableMetricCreated(e) => Activity::new(
                ActivityType::BillableMetricCreated,
                EntityType::BillableMetric,
                e.metric_id.as_uuid(),
            ),
            OutboxEvent::BillableMetricUpdated(e) => Activity::new(
                ActivityType::BillableMetricUpdated,
                EntityType::BillableMetric,
                e.metric_id.as_uuid(),
            ),
            OutboxEvent::BillableMetricArchived(e) => Activity::new(
                ActivityType::BillableMetricArchived,
                EntityType::BillableMetric,
                e.metric_id.as_uuid(),
            ),
            OutboxEvent::InvoiceCreated(e) => {
                let mut a = Activity::new(
                    ActivityType::InvoiceCreated,
                    EntityType::Invoice,
                    e.invoice_id.as_uuid(),
                )
                .agg_customer(e.customer_id);
                if let Some(sub) = e.subscription_id {
                    a = a.agg_subscription(sub);
                }
                a
            }
            OutboxEvent::InvoiceFinalized(e) => {
                let mut a = Activity::new(
                    ActivityType::InvoiceFinalized,
                    EntityType::Invoice,
                    e.invoice_id.as_uuid(),
                )
                .agg_customer(e.customer_id);
                if let Some(sub) = e.subscription_id {
                    a = a.agg_subscription(sub);
                }
                a
            }
            OutboxEvent::InvoicePaid(e) => {
                let mut a = Activity::new(
                    ActivityType::InvoicePaid,
                    EntityType::Invoice,
                    e.invoice_id.as_uuid(),
                )
                .agg_customer(e.customer_id);
                if let Some(sub) = e.subscription_id {
                    a = a.agg_subscription(sub);
                }
                a
            }
            OutboxEvent::InvoiceVoided(e) => {
                let mut a = Activity::new(
                    ActivityType::InvoiceVoided,
                    EntityType::Invoice,
                    e.invoice_id.as_uuid(),
                )
                .agg_customer(e.customer_id);
                if let Some(sub) = e.subscription_id {
                    a = a.agg_subscription(sub);
                }
                a
            }
            OutboxEvent::CreditNoteCreated(e) => {
                let mut a = Activity::new(
                    ActivityType::CreditNoteCreated,
                    EntityType::CreditNote,
                    e.credit_note_id.as_uuid(),
                )
                .agg_customer(e.customer_id);
                if let Some(sub) = e.subscription_id {
                    a = a.agg_subscription(sub);
                }
                a
            }
            OutboxEvent::CreditNoteFinalized(e) => {
                let mut a = Activity::new(
                    ActivityType::CreditNoteFinalized,
                    EntityType::CreditNote,
                    e.credit_note_id.as_uuid(),
                )
                .agg_customer(e.customer_id);
                if let Some(sub) = e.subscription_id {
                    a = a.agg_subscription(sub);
                }
                a
            }
            OutboxEvent::CreditNoteVoided(e) => {
                let mut a = Activity::new(
                    ActivityType::CreditNoteVoided,
                    EntityType::CreditNote,
                    e.credit_note_id.as_uuid(),
                )
                .agg_customer(e.customer_id);
                if let Some(sub) = e.subscription_id {
                    a = a.agg_subscription(sub);
                }
                a
            }
            OutboxEvent::SubscriptionCreated(e) => Activity::new(
                ActivityType::SubscriptionCreated,
                EntityType::Subscription,
                e.subscription_id.as_uuid(),
            )
            .agg_customer(e.customer_id),
            OutboxEvent::QuoteAccepted(e) => Activity::new(
                ActivityType::QuoteAccepted,
                EntityType::Quote,
                e.quote_id.as_uuid(),
            )
            .agg_customer(e.customer_id),
            OutboxEvent::QuoteConverted(e) => Activity::new(
                ActivityType::QuoteConverted,
                EntityType::Quote,
                e.quote_id.as_uuid(),
            )
            .agg_customer(e.customer_id)
            .agg_subscription(e.subscription_id),
            OutboxEvent::PlanCreated(e) => Activity::new(
                ActivityType::PlanCreated,
                EntityType::Plan,
                e.plan_id.as_uuid(),
            ),
            OutboxEvent::PlanPublished(e) => Activity::new(
                ActivityType::PlanPublished,
                EntityType::Plan,
                e.plan_id.as_uuid(),
            ),
            OutboxEvent::PlanArchived(e) => Activity::new(
                ActivityType::PlanArchived,
                EntityType::Plan,
                e.plan_id.as_uuid(),
            ),
            OutboxEvent::ProductCreated(e) => Activity::new(
                ActivityType::ProductCreated,
                EntityType::Product,
                e.product_id.as_uuid(),
            ),
            OutboxEvent::ProductUpdated(e) => Activity::new(
                ActivityType::ProductUpdated,
                EntityType::Product,
                e.product_id.as_uuid(),
            ),
            OutboxEvent::ProductArchived(e) => Activity::new(
                ActivityType::ProductArchived,
                EntityType::Product,
                e.product_id.as_uuid(),
            ),
            OutboxEvent::CouponCreated(e) => Activity::new(
                ActivityType::CouponCreated,
                EntityType::Coupon,
                e.coupon_id.as_uuid(),
            ),
            OutboxEvent::CouponUpdated(e) => Activity::new(
                ActivityType::CouponUpdated,
                EntityType::Coupon,
                e.coupon_id.as_uuid(),
            ),
            OutboxEvent::CouponArchived(e) => Activity::new(
                ActivityType::CouponArchived,
                EntityType::Coupon,
                e.coupon_id.as_uuid(),
            ),
            OutboxEvent::AddOnCreated(e) => Activity::new(
                ActivityType::AddOnCreated,
                EntityType::AddOn,
                e.add_on_id.as_uuid(),
            ),
            OutboxEvent::AddOnUpdated(e) => Activity::new(
                ActivityType::AddOnUpdated,
                EntityType::AddOn,
                e.add_on_id.as_uuid(),
            ),
            OutboxEvent::AddOnArchived(e) => Activity::new(
                ActivityType::AddOnArchived,
                EntityType::AddOn,
                e.add_on_id.as_uuid(),
            ),
            OutboxEvent::InvoiceAccountingPdfGenerated(_)
            | OutboxEvent::PaymentTransactionSaved(_) => return None,
        };
        Some(activity)
    }
}

#[derive(Debug, Clone, o2o)]
#[from_owned(EntityActivityRow)]
pub struct EntityActivity {
    pub id: EntityActivityId,
    pub tenant_id: TenantId,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub activity_type: String,
    #[map(@.actor_type.into())]
    pub actor_type: ActorType,
    pub actor_uuid: Option<Uuid>,
    pub actor_alias: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub occurred_at: NaiveDateTime,
    pub agg_customer_id: Option<Uuid>,
    pub agg_subscription_id: Option<Uuid>,
}

pub(crate) fn build_row_new(
    tenant_id: TenantId,
    actor: &Actor,
    activity: Activity,
) -> EntityActivityRowNew {
    // Mirror of the CHECK constraint — strip rollup ids when entity_id IS the customer/sub,
    // so the insert fails cleanly rather than as an opaque DB error.
    let agg_customer_id = match activity.entity_type {
        EntityType::Customer => None,
        _ => activity.agg_customer_id.map(|id| id.as_uuid()),
    };
    let agg_subscription_id = match activity.entity_type {
        EntityType::Subscription => None,
        _ => activity.agg_subscription_id.map(|id| id.as_uuid()),
    };

    EntityActivityRowNew {
        id: EntityActivityId::new(),
        tenant_id,
        entity_type: activity.entity_type.to_string(),
        entity_id: activity.entity_id,
        activity_type: activity.activity_type.to_string(),
        actor_type: actor.actor_type().into(),
        actor_uuid: actor.as_uuid(),
        actor_alias: actor.actor_alias(),
        metadata: activity.metadata,
        agg_customer_id,
        agg_subscription_id,
    }
}
