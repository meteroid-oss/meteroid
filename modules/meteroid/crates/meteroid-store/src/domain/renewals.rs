//! A subscription's next renewal (R-160) and the notices sent before it (R-157, R-161).
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{BaseId, CustomerId, EventId, SubscriptionId, TenantId};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use uuid::Uuid;

/// The retention module's namespace: ids of notices claimed before the merge stay the same.
const EVENT_NAMESPACE: Uuid = Uuid::from_u128(0x3b6e_92d4_51a7_4c0f_8e29_d71a_04f6_c83b);

pub const DEFAULT_CHECKPOINTS: [u32; 3] = [90, 60, 30];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RenewalMode {
    /// The subscription renews on the date: its term auto-renews, or a long period ends.
    Renewing,
    /// The subscription ends on the date: a term not renewing, or an end date.
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

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "RENEWING" => Some(RenewalMode::Renewing),
            "ENDING" => Some(RenewalMode::Ending),
            "CANCELLING" => Some(RenewalMode::Cancelling),
            _ => None,
        }
    }
}

/// What dates the renewal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RenewalSource {
    Term,
    Period,
    EndDate,
    Cancellation,
}

impl RenewalSource {
    /// Term renewals are noticed wherever a term exists; the others need the retention gate.
    pub fn needs_retention(self) -> bool {
        !matches!(self, RenewalSource::Term)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenewalTermSnapshot {
    pub auto_renew: bool,
    pub renewal_months: u32,
    pub uplift_percent: Option<Decimal>,
    pub notice_days: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renewal {
    pub tenant_id: TenantId,
    pub subscription_id: SubscriptionId,
    pub customer_id: CustomerId,
    pub customer_name: String,
    pub plan_name: String,
    pub currency: String,
    pub mrr_cents: i64,
    pub date: NaiveDate,
    pub mode: RenewalMode,
    pub source: RenewalSource,
    /// The last day the renewal can be opted out of, for a term renewing with notice.
    pub opt_out_deadline: Option<NaiveDate>,
    pub term: Option<RenewalTermSnapshot>,
}

impl Renewal {
    /// The day checkpoints count to: the opt-out deadline, else the date.
    pub fn anchor(&self) -> NaiveDate {
        self.opt_out_deadline.unwrap_or(self.date)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NoticeAudience {
    Team,
    Customer,
}

impl NoticeAudience {
    pub fn as_str(self) -> &'static str {
        match self {
            NoticeAudience::Team => "TEAM",
            NoticeAudience::Customer => "CUSTOMER",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "TEAM" => Some(NoticeAudience::Team),
            "CUSTOMER" => Some(NoticeAudience::Customer),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenewalNotice {
    pub renewal_date: NaiveDate,
    pub mode: RenewalMode,
    pub audience: NoticeAudience,
    pub days_before: u32,
    pub notified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenewalSettings {
    /// Days before the anchor the team is notified, largest first.
    pub checkpoints: Vec<u32>,
    /// Days before the anchor the customer is emailed; none emails no one.
    pub customer_reminder_days: Option<u32>,
}

impl Default for RenewalSettings {
    fn default() -> Self {
        Self {
            checkpoints: DEFAULT_CHECKPOINTS.to_vec(),
            customer_reminder_days: None,
        }
    }
}

/// An operator's acceptance of an opt-out after the notice cutoff (R-152).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeWaiver {
    pub reason: String,
}

/// `subscription.renewal_upcoming`: a renewal reached a team checkpoint.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenewalUpcomingEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: SubscriptionId,
    pub renewal_date: NaiveDate,
    pub mode: RenewalMode,
    #[serde(default = "period_source")]
    pub source: RenewalSource,
    /// The checkpoint reached: how many days before the anchor this notice is.
    pub days_before: u32,
    pub opt_out_deadline: Option<NaiveDate>,
    pub term: Option<RenewalTermSnapshot>,
    pub plan_name: String,
    pub currency: String,
    pub mrr_cents: i64,
    pub notified_at: DateTime<Utc>,
}

/// Rows queued by the retention module before the merge carry no source.
fn period_source() -> RenewalSource {
    RenewalSource::Period
}

impl RenewalUpcomingEvent {
    pub const ROUTING_KEY: &'static str = "subscription.renewal_upcoming";

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::outbox_event::{EventType, OutboxEvent};

    #[test]
    fn a_notice_queued_by_the_retention_module_reads_as_the_merged_event() {
        let id = EventId::from(Uuid::nil());
        let json = serde_json::json!({
            "RetentionRenewalUpcoming": {
                "id": id,
                "tenant_id": Uuid::nil(),
                "customer_id": Uuid::nil(),
                "subscription_id": Uuid::nil(),
                "renewal_date": "2026-12-01",
                "mode": "RENEWING",
                "days_before": 60,
                "plan_name": "Studio",
                "currency": "EUR",
                "mrr_cents": 12300,
                "notified_at": "2026-10-02T09:00:00Z"
            }
        });
        let event: OutboxEvent = serde_json::from_value(json).expect("an old row parses");
        let OutboxEvent::SubscriptionRenewalUpcoming(event) = &event else {
            panic!("merged into subscription.renewal_upcoming: {event:?}");
        };
        assert_eq!(event.source, RenewalSource::Period);
        assert_eq!(event.term, None);
        let header: EventType =
            serde_json::from_value(serde_json::json!("RetentionRenewalUpcoming")).expect("header");
        assert_eq!(header, EventType::SubscriptionRenewalUpcoming);
        assert_eq!(header.routing_key(), RenewalUpcomingEvent::ROUTING_KEY);
    }
}
