use uuid::Uuid;

use crate::enums::SubscriptionFeeBillingPeriod;
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::subscription_component)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionComponent {
    pub id: Uuid,
    pub name: String,
    pub subscription_id: Uuid,
    pub price_component_id: Option<Uuid>,
    pub product_item_id: Option<Uuid>,
    pub period: SubscriptionFeeBillingPeriod,
    // pub mrr_value: Option<Decimal>,
    pub fee: serde_json::Value,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::subscription_component)]
pub struct SubscriptionComponentNew {
    pub id: Uuid,
    pub name: String,
    pub subscription_id: Uuid,
    pub price_component_id: Option<Uuid>,
    pub product_item_id: Option<Uuid>,
    pub period: SubscriptionFeeBillingPeriod,
    // pub mrr_value: Option<Decimal>,
    pub fee: serde_json::Value,
}
