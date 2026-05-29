use chrono::{NaiveDate, NaiveDateTime};

use crate::enums::SubscriptionFeeBillingPeriod;
use common_domain::ids::{AddOnId, PriceId, ProductId, SubscriptionAddOnId, SubscriptionId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::subscription_add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionAddOnRow {
    pub id: SubscriptionAddOnId,
    pub name: String,
    pub subscription_id: SubscriptionId,
    pub add_on_id: AddOnId,
    pub period: SubscriptionFeeBillingPeriod,
    pub legacy_fee: Option<serde_json::Value>,
    pub created_at: NaiveDateTime,
    pub product_id: Option<ProductId>,
    pub price_id: Option<PriceId>,
    pub quantity: i32,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    /// Lineage root this add-on descends from across overrides. `None` means the
    /// row is its own root. See the `subscription_component_lineage` migration.
    pub lineage_id: Option<SubscriptionAddOnId>,
    /// True when this row was added by a manual amendment (vs. coming from the plan
    /// definition or a plan change). Drives one-time-fee billing on the effective
    /// period; recurring fees are unaffected.
    pub added_by_amendment: bool,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::subscription_add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionAddOnRowNew {
    pub id: SubscriptionAddOnId,
    pub name: String,
    pub subscription_id: SubscriptionId,
    pub add_on_id: AddOnId,
    pub period: SubscriptionFeeBillingPeriod,
    pub legacy_fee: Option<serde_json::Value>,
    pub product_id: Option<ProductId>,
    pub price_id: Option<PriceId>,
    pub quantity: i32,
    pub effective_from: NaiveDate,
    /// Lineage root this add-on descends from across overrides. `None` means the
    /// row is its own root.
    pub lineage_id: Option<SubscriptionAddOnId>,
    /// True when added by a manual amendment (see `SubscriptionAddOnRow`).
    pub added_by_amendment: bool,
}
