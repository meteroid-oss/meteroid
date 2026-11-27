//! Outbox payload of a finished portal cancel flow (EE retention module). The id derives from the
//! session, so a session announces its outcome once.
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{
    BaseId, CustomerId, EventId, RetentionSessionId, SubscriptionId, TenantId,
};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use uuid::Uuid;

const EVENT_NAMESPACE: Uuid = Uuid::from_u128(0x3b6e_92d4_51a7_4c0f_8e29_d71a_04f6_c83b);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RetentionOutcome {
    Saved,
    ContactRequested,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RetentionOfferKind {
    Coupon,
    PlanChange,
    Contact,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionSessionEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub session_id: RetentionSessionId,
    pub customer_id: CustomerId,
    pub subscription_id: SubscriptionId,
    pub outcome: RetentionOutcome,
    pub reason: Option<String>,
    pub comment: Option<String>,
    /// The offer accepted, or the one declined before cancelling.
    pub offer_kind: Option<RetentionOfferKind>,
    pub offer_title: Option<String>,
    /// When the accepted offer or the cancellation takes effect.
    pub effective_date: Option<NaiveDate>,
    pub completed_at: DateTime<Utc>,
}

impl RetentionSessionEvent {
    pub const ROUTING_KEY: &'static str = "retention.session_completed";

    pub fn event_id(session_id: RetentionSessionId) -> EventId {
        EventId::from(Uuid::new_v5(
            &EVENT_NAMESPACE,
            session_id.as_uuid().as_bytes(),
        ))
    }
}
