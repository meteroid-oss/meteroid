use crate::enums::SubscriptionFeeBillingPeriod;
use common_domain::ids::{AddOnId, PriceId, ProductId, QuoteAddOnId, QuoteId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::quote_add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteAddOnRow {
    pub id: QuoteAddOnId,
    pub name: String,
    pub quote_id: QuoteId,
    pub add_on_id: AddOnId,
    pub period: SubscriptionFeeBillingPeriod,
    pub legacy_fee: Option<serde_json::Value>,
    pub product_id: Option<ProductId>,
    pub price_id: Option<PriceId>,
    pub quantity: i32,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::quote_add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteAddOnRowNew {
    pub id: QuoteAddOnId,
    pub name: String,
    pub quote_id: QuoteId,
    pub add_on_id: AddOnId,
    pub period: SubscriptionFeeBillingPeriod,
    pub legacy_fee: Option<serde_json::Value>,
    pub product_id: Option<ProductId>,
    pub price_id: Option<PriceId>,
    pub quantity: i32,
}
