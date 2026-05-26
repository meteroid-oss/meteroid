use chrono::NaiveDateTime;

use diesel::{Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Queryable, Identifiable, Debug, Selectable)]
#[diesel(table_name = crate::schema::webhook_in_event)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct WebhookInEventRow {
    pub id: Uuid,
    pub received_at: NaiveDateTime,
    pub action: Option<String>,
    pub key: String,
    pub processed: bool,
    pub attempts: i32,
    pub error: Option<String>,
    pub provider_config_id: Uuid,
    /// Provider-side event id (Stripe `evt_…`, GoCardless `EV…`, …). Combined
    /// with `provider_config_id` it enforces idempotency: duplicate webhook
    /// deliveries violate the unique index `(provider_config_id,
    /// provider_event_id)` and are rejected at insert. The index is not partial;
    /// rows with a NULL provider_event_id (events with no usable id) do not
    /// participate in dedup under PostgreSQL's `NULLS DISTINCT`.
    pub provider_event_id: Option<String>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::webhook_in_event)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct WebhookInEventRowNew {
    pub id: Uuid,
    pub received_at: NaiveDateTime,
    pub action: Option<String>,
    pub key: String,
    pub processed: bool,
    pub attempts: i32,
    pub error: Option<String>,
    pub provider_config_id: Uuid,
    pub provider_event_id: Option<String>,
}
