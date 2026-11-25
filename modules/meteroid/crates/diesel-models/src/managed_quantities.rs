use chrono::{DateTime, Utc};
use common_domain::ids::{ManagedQuantityId, TenantId};
use diesel::{AsChangeset, Insertable, Queryable, Selectable};

use crate::enums::{QuantityDecreaseEnum, QuantityIncreaseEnum};

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::managed_quantity)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ManagedQuantityRow {
    pub id: ManagedQuantityId,
    pub tenant_id: TenantId,
    pub code: String,
    pub name: String,
    pub on_increase: QuantityIncreaseEnum,
    pub on_decrease: QuantityDecreaseEnum,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::managed_quantity)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ManagedQuantityRowNew {
    pub id: ManagedQuantityId,
    pub tenant_id: TenantId,
    pub code: String,
    pub name: String,
}

/// `None` leaves a column alone. The code is immutable: seats are keyed by it.
#[derive(Debug, Clone, Default, AsChangeset)]
#[diesel(table_name = crate::schema::managed_quantity)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ManagedQuantityRowPatch {
    pub name: Option<String>,
    pub on_increase: Option<QuantityIncreaseEnum>,
    pub on_decrease: Option<QuantityDecreaseEnum>,
}
