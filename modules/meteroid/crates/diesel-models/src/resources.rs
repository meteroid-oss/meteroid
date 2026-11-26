use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, ResourceId, ResourceTypeId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Clone, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::resource_type)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ResourceTypeRow {
    pub id: ResourceTypeId,
    pub tenant_id: TenantId,
    pub code: String,
    pub name: String,
    pub plural_name: String,
    pub parent_type_id: Option<ResourceTypeId>,
    /// `{"<attribute>": ["<value>", ...]}`.
    pub attributes: serde_json::Value,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::resource_type)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ResourceTypeRowNew {
    pub id: ResourceTypeId,
    pub tenant_id: TenantId,
    pub code: String,
    pub name: String,
    pub plural_name: String,
    pub attributes: serde_json::Value,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = crate::schema::resource_type)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ResourceTypeRowPatch {
    pub name: Option<String>,
    pub plural_name: Option<String>,
    pub attributes: Option<serde_json::Value>,
    pub archived_at: Option<Option<DateTime<Utc>>>,
    pub updated_at: DateTime<Utc>,
}

/// Position 0 is the canonical key, the others are aliases.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::resource_type_key)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ResourceTypeKeyRow {
    pub tenant_id: TenantId,
    pub resource_type_id: ResourceTypeId,
    pub key: String,
    pub position: i32,
}

#[derive(Debug, Clone, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::resource)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ResourceRow {
    pub id: ResourceId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub resource_type_id: ResourceTypeId,
    pub key_value: String,
    pub name: Option<String>,
    pub attributes: serde_json::Value,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::resource)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ResourceRowNew {
    pub id: ResourceId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub resource_type_id: ResourceTypeId,
    pub key_value: String,
    pub name: Option<String>,
    pub attributes: serde_json::Value,
    pub archived_at: Option<DateTime<Utc>>,
}
