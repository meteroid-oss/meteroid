use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{BillableMetricId, CreditUnitId, RateCardEntryId, RateCardId, TenantId};
use diesel::{Insertable, Queryable, Selectable};
use rust_decimal::Decimal;

use crate::enums::{RateCardPurposeEnum, RateCardUnmatchedEnum};

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::rate_card)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RateCardRow {
    pub id: RateCardId,
    pub tenant_id: TenantId,
    pub code: String,
    pub name: String,
    pub metric_id: BillableMetricId,
    pub dimensions: Vec<Option<String>>,
    pub credit_unit_id: Option<CreditUnitId>,
    pub on_unmatched: RateCardUnmatchedEnum,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub unit_size: Decimal,
    pub purpose: RateCardPurposeEnum,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::rate_card)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RateCardRowNew {
    pub id: RateCardId,
    pub tenant_id: TenantId,
    pub code: String,
    pub name: String,
    pub metric_id: BillableMetricId,
    pub dimensions: Vec<Option<String>>,
    pub credit_unit_id: Option<CreditUnitId>,
    pub on_unmatched: RateCardUnmatchedEnum,
    pub unit_size: Decimal,
    pub purpose: RateCardPurposeEnum,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::rate_card_entry)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RateCardEntryRow {
    pub id: RateCardEntryId,
    pub tenant_id: TenantId,
    pub rate_card_id: RateCardId,
    pub key: Vec<Option<String>>,
    pub effective_from: NaiveDate,
    /// None: the key has no rate in `currency` from its day.
    pub rate: Option<Decimal>,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub currency: Option<String>,
}
