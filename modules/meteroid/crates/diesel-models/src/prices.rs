use chrono::{NaiveDate, NaiveDateTime};

use crate::enums::BillingPeriodEnum;
use common_domain::ids::{CreditUnitId, PriceId, ProductId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use rust_decimal::Decimal;

#[derive(Queryable, Debug, Clone, Identifiable, Selectable)]
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
    /// A successor's first day; None on a lineage's root (R-097).
    pub effective_from: Option<NaiveDate>,
    /// The root of the lineage a successor continues.
    pub supersedes: Option<PriceId>,
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

/// A dated successor of a lineage's root: its rates from `effective_from` on (R-097).
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::price)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PriceSuccessorRowNew {
    pub id: PriceId,
    pub product_id: ProductId,
    pub cadence: BillingPeriodEnum,
    pub currency: String,
    pub pricing: serde_json::Value,
    pub tenant_id: TenantId,
    pub catalog: bool,
    pub credit_unit_id: Option<CreditUnitId>,
    pub credit_unit_rate: Option<Decimal>,
    pub effective_from: NaiveDate,
    pub supersedes: PriceId,
}
