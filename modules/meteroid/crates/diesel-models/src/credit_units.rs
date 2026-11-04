use chrono::{DateTime, Utc};
use common_domain::ids::{CreditUnitId, CreditUnitPriceId, ProductId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};
use rust_decimal::Decimal;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::credit_unit)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditUnitRow {
    pub id: CreditUnitId,
    pub tenant_id: TenantId,
    pub code: String,
    pub name: String,
    pub plural_name: String,
    pub symbol: Option<String>,
    pub precision: i16,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub product_id: Option<ProductId>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::credit_unit)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditUnitRowNew {
    pub id: CreditUnitId,
    pub tenant_id: TenantId,
    pub code: String,
    pub name: String,
    pub plural_name: String,
    pub symbol: Option<String>,
    pub precision: i16,
}

/// Code and precision are immutable (trigger). `None` leaves a column alone; `Some(None)` clears it.
#[derive(Debug, Clone, Default, AsChangeset)]
#[diesel(table_name = crate::schema::credit_unit)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditUnitRowPatch {
    pub name: Option<String>,
    pub plural_name: Option<String>,
    pub symbol: Option<Option<String>>,
    pub archived_at: Option<Option<DateTime<Utc>>>,
    pub product_id: Option<Option<ProductId>>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::credit_unit_price)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditUnitPriceRow {
    pub id: CreditUnitPriceId,
    pub tenant_id: TenantId,
    pub credit_unit_id: CreditUnitId,
    pub currency: String,
    pub unit_price: Decimal,
    pub created_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::credit_unit_price)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditUnitPriceRowNew {
    pub id: CreditUnitPriceId,
    pub tenant_id: TenantId,
    pub credit_unit_id: CreditUnitId,
    pub currency: String,
    pub unit_price: Decimal,
}
