use crate::enums::SubscriptionFeeBillingPeriod;
use chrono::NaiveDate;
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
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    /// Lineage root this component descends from across overrides. `None` means the
    /// row is its own root. See the `subscription_component_lineage` migration.
    pub lineage_id: Option<SubscriptionPriceComponentId>,
    /// True when this row was added by a manual amendment (vs. coming from the plan
    /// definition or a plan change). Drives one-time-fee billing on the effective
    /// period; recurring fees are unaffected.
    pub added_by_amendment: bool,
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
    pub effective_from: NaiveDate,
    /// Lineage root this component descends from across overrides. `None` means the
    /// row is its own root.
    pub lineage_id: Option<SubscriptionPriceComponentId>,
    /// True when added by a manual amendment (see `SubscriptionComponentRow`).
    pub added_by_amendment: bool,
}
