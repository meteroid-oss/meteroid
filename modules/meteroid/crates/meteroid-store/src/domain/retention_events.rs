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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RenewalMode {
    /// An annual or semiannual subscription renews on the date.
    Renewing,
    /// A fixed term ends on the date.
    Ending,
    /// The subscription is set to cancel on the date.
    Cancelling,
}

impl RenewalMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RenewalMode::Renewing => "RENEWING",
            RenewalMode::Ending => "ENDING",
            RenewalMode::Cancelling => "CANCELLING",
        }
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenewalUpcomingEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: SubscriptionId,
    pub renewal_date: NaiveDate,
    pub mode: RenewalMode,
    /// The checkpoint reached: how many days before the date this notice is.
    pub days_before: u32,
    pub plan_name: String,
    pub currency: String,
    pub mrr_cents: i64,
    pub notified_at: DateTime<Utc>,
}

impl RenewalUpcomingEvent {
    pub const ROUTING_KEY: &'static str = "retention.renewal_upcoming";

    pub fn event_id(
        subscription_id: SubscriptionId,
        renewal_date: NaiveDate,
        days_before: u32,
        mode: RenewalMode,
    ) -> EventId {
        let key = format!(
            "{}:{renewal_date}:{days_before}:{}",
            subscription_id.as_uuid(),
            mode.as_str()
        );
        EventId::from(Uuid::new_v5(&EVENT_NAMESPACE, key.as_bytes()))
    }
}
