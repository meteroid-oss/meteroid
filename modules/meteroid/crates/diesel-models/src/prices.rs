use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::BillingPeriodEnum;
use common_domain::ids::{PriceId, ProductId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::price)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PriceRow {
    pub id: PriceId,
    pub product_id: ProductId,
    pub cadence: BillingPeriodEnum,
    pub currency: String,
    pub pricing: serde_json::Value,
    pub tenant_id: TenantId,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub archived_at: Option<NaiveDateTime>,
    pub catalog: bool,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::price)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PriceRowNew {
    pub id: PriceId,
    pub product_id: ProductId,
    pub cadence: BillingPeriodEnum,
    pub currency: String,
    pub pricing: serde_json::Value,
    pub tenant_id: TenantId,
    pub created_by: Uuid,
    pub catalog: bool,
}
