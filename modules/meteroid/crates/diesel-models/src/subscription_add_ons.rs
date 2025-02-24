use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::SubscriptionFeeBillingPeriod;
use common_domain::ids::SubscriptionId;
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::subscription_add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionAddOnRow {
    pub id: Uuid,
    pub name: String,
    pub subscription_id: SubscriptionId,
    pub add_on_id: Uuid,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: serde_json::Value,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::subscription_add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionAddOnRowNew {
    pub id: Uuid,
    pub name: String,
    pub subscription_id: SubscriptionId,
    pub add_on_id: Uuid,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: serde_json::Value,
}
