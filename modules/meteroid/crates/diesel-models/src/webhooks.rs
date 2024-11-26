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
}
