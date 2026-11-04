use chrono::NaiveDateTime;

use crate::enums::BillingPeriodEnum;
use common_domain::ids::{CreditUnitId, PriceId, ProductId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use rust_decimal::Decimal;

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
    pub archived_at: Option<NaiveDateTime>,
    pub catalog: bool,
    /// Set together: `pricing` is in this unit's credits, each worth `credit_unit_rate` major
    /// units of `currency`.
    pub credit_unit_id: Option<CreditUnitId>,
    pub credit_unit_rate: Option<Decimal>,
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
    pub catalog: bool,
    pub credit_unit_id: Option<CreditUnitId>,
    pub credit_unit_rate: Option<Decimal>,
}
