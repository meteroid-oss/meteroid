use chrono::NaiveDateTime;
use common_domain::ids::{ProductId, SubscriptionId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

use crate::enums::AdjustmentKindEnum;

#[derive(Queryable, Debug, Identifiable, Selectable, Clone)]
#[diesel(table_name = crate::schema::subscription_adjustment)]
#[diesel(primary_key(subscription_id, kind))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionAdjustmentRow {
    pub subscription_id: SubscriptionId,
    pub kind: AdjustmentKindEnum,
    pub amount_subunit: i64,
    pub includes_all_components: bool,
    /// TRUE when this term was set by an operator override or sourced from a quote_adjustment.
    /// Plan changes preserve overrides; plain plan snapshots (`is_override = false`) get
    /// re-snapped to the new plan version's adjustment.
    pub is_override: bool,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::subscription_adjustment)]
pub struct SubscriptionAdjustmentRowNew {
    pub subscription_id: SubscriptionId,
    pub kind: AdjustmentKindEnum,
    pub amount_subunit: i64,
    pub includes_all_components: bool,
    pub is_override: bool,
}

#[derive(Queryable, Debug, Identifiable, Selectable, Clone)]
#[diesel(table_name = crate::schema::subscription_adjustment_component)]
#[diesel(primary_key(subscription_id, kind, product_id))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionAdjustmentComponentRow {
    pub subscription_id: SubscriptionId,
    pub kind: AdjustmentKindEnum,
    pub product_id: ProductId,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::subscription_adjustment_component)]
pub struct SubscriptionAdjustmentComponentRowNew {
    pub subscription_id: SubscriptionId,
    pub kind: AdjustmentKindEnum,
    pub product_id: ProductId,
}
