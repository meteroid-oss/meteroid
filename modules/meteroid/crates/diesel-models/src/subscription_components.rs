use crate::enums::SubscriptionFeeBillingPeriod;
use common_domain::ids::{
    PriceComponentId, ProductId, SubscriptionId, SubscriptionPriceComponentId,
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
    // pub mrr_value: Option<Decimal>,
    pub fee: serde_json::Value,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::subscription_component)]
pub struct SubscriptionComponentRowNew {
    pub id: SubscriptionPriceComponentId,
    pub name: String,
    pub subscription_id: SubscriptionId,
    pub price_component_id: Option<PriceComponentId>,
    pub product_id: Option<ProductId>,
    pub period: SubscriptionFeeBillingPeriod,
    // pub mrr_value: Option<Decimal>,
    pub fee: serde_json::Value,
}
