use crate::enums::SubscriptionFeeBillingPeriod;
use common_domain::ids::{
    PriceComponentId, PriceId, ProductId, SubscriptionId, SubscriptionPriceComponentId,
};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::subscription_component)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionComponentRow {
    pub id: SubscriptionPriceComponentId,
    pub name: String,
    pub subscription_id: SubscriptionId,
    pub price_component_id: Option<PriceComponentId>,
    pub product_id: Option<ProductId>,
    pub period: SubscriptionFeeBillingPeriod,
    pub legacy_fee: Option<serde_json::Value>,
    pub price_id: Option<PriceId>,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::subscription_component)]
pub struct SubscriptionComponentRowNew {
    pub id: SubscriptionPriceComponentId,
    pub name: String,
    pub subscription_id: SubscriptionId,
    pub price_component_id: Option<PriceComponentId>,
    pub product_id: Option<ProductId>,
    pub period: SubscriptionFeeBillingPeriod,
    pub legacy_fee: Option<serde_json::Value>,
    pub price_id: Option<PriceId>,
}
