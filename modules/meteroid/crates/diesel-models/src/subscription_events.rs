use chrono::{NaiveDate, NaiveDateTime};
use uuid::Uuid;

use crate::enums::SubscriptionEventType;
use common_domain::ids::SubscriptionId;
use diesel::{Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Insertable, Selectable)]
#[diesel(table_name = crate::schema::subscription_event)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionEventRow {
    pub id: Uuid,
    pub mrr_delta: Option<i64>,
    pub event_type: SubscriptionEventType,
    pub created_at: NaiveDateTime,
    pub applies_to: NaiveDate,
    pub subscription_id: SubscriptionId,
    pub bi_mrr_movement_log_id: Option<Uuid>,
    pub details: Option<serde_json::Value>,
}

/// One page of `list_incomplete_settling_endings` for one event type; binds (after day, after
/// subscription, limit, now). `page` walks the type's partial index in order and stops at the
/// limit: keep its predicate identical to the index's.
macro_rules! incomplete_settling_page {
    ($event_type:literal) => {
        concat!(
            "\
WITH page AS ( \
  SELECT e.applies_to, e.subscription_id, \
         max(e.outgoing_checked_at) AS checked_at, \
         max(e.outgoing_next_check_at) AS next_check_at \
  FROM subscription_event e \
  WHERE e.event_type = '",
            $event_type,
            "' \
    AND e.details ->> 'outgoing' = 'settled' \
    AND NOT (e.details ? 'outgoing_complete') \
    AND (e.applies_to, e.subscription_id) > ($1, $2) \
  GROUP BY e.applies_to, e.subscription_id \
  ORDER BY e.applies_to, e.subscription_id \
  LIMIT $3 \
), changed AS ( \
  SELECT i.subscription_id, \
         max(GREATEST(CASE WHEN i.status <> 'DRAFT' THEN i.updated_at END, parent.updated_at)) AS changed_at \
  FROM invoice i \
  JOIN (SELECT DISTINCT subscription_id FROM page) p ON p.subscription_id = i.subscription_id \
  LEFT JOIN invoice parent ON parent.id = i.consolidated_into_invoice_id AND parent.status <> 'DRAFT' \
  WHERE i.status <> 'DRAFT' OR parent.id IS NOT NULL \
  GROUP BY i.subscription_id \
) \
SELECT s.tenant_id, s.customer_id, p.subscription_id, p.applies_to, \
       COALESCE(c.changed_at > p.checked_at AT TIME ZONE 'UTC', false) AS invoices_changed, \
       (p.next_check_at IS NULL OR p.next_check_at <= $4 \
        OR COALESCE(c.changed_at > p.checked_at AT TIME ZONE 'UTC', false)) AS due \
FROM page p \
JOIN subscription s ON s.id = p.subscription_id \
LEFT JOIN changed c ON c.subscription_id = p.subscription_id \
ORDER BY p.applies_to, p.subscription_id"
        )
    };
}

/// Walks idx_subscription_event_incomplete_switch.
pub const INCOMPLETE_SETTLING_SWITCHES_PAGE: &str = incomplete_settling_page!("SWITCH");
/// Walks idx_subscription_event_incomplete_cancellation.
pub const INCOMPLETE_SETTLING_CANCELLATIONS_PAGE: &str = incomplete_settling_page!("CANCELLED");

/// What ended a subscription's lines mid-period and settles them, now or once their invoice is
/// issued: an immediate plan change (its SWITCH event) or cancellation (its CANCELLED event).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlingEnding {
    PlanChange,
    Cancellation,
}

impl SettlingEnding {
    pub fn event_type(self) -> &'static str {
        match self {
            Self::PlanChange => "SWITCH",
            Self::Cancellation => "CANCELLED",
        }
    }

    pub fn incomplete_page(self) -> &'static str {
        match self {
            Self::PlanChange => INCOMPLETE_SETTLING_SWITCHES_PAGE,
            Self::Cancellation => INCOMPLETE_SETTLING_CANCELLATIONS_PAGE,
        }
    }
}

/// A plan switch or cancellation the sweep visits, checked now when `due`; `invoices_changed`: an issued invoice
/// of its subscription changed (was issued, voided, paid) after the last check.
#[derive(Debug, Clone, Copy)]
pub struct SettlingSwitchDue {
    pub tenant_id: common_domain::ids::TenantId,
    pub customer_id: common_domain::ids::CustomerId,
    pub subscription_id: SubscriptionId,
    pub applies_to: NaiveDate,
    pub invoices_changed: bool,
    pub due: bool,
}

impl SubscriptionEventRow {
    /// An ENDED lifecycle event: a counted (activated) subscription leaving the active set at its
    /// natural end. Mirrors CANCELLED — decrements the active count and churns the given MRR.
    pub fn ended(
        subscription_id: SubscriptionId,
        applies_to: NaiveDate,
        mrr_delta: Option<i64>,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            subscription_id,
            event_type: SubscriptionEventType::Ended,
            details: None,
            created_at: common_domain::clock::now().naive_utc(),
            mrr_delta,
            bi_mrr_movement_log_id: None,
            applies_to,
        }
    }
}
