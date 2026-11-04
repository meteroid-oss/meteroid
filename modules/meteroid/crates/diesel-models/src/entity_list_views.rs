use chrono::NaiveDateTime;

use crate::enums::EntityListTypeEnum;
use common_domain::ids::{EntityListViewId, TenantId, UserId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Selectable, Debug, Clone, Identifiable)]
#[diesel(table_name = crate::schema::entity_list_view)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EntityListViewRow {
    pub id: EntityListViewId,
    pub tenant_id: TenantId,
    pub entity_type: EntityListTypeEnum,
    pub name: String,
    pub config: serde_json::Value,
    pub shared: bool,
    pub created_by: Option<UserId>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::entity_list_view)]
pub struct EntityListViewRowNew {
    pub id: EntityListViewId,
    pub tenant_id: TenantId,
    pub entity_type: EntityListTypeEnum,
    pub name: String,
    pub config: serde_json::Value,
    pub shared: bool,
    pub created_by: Option<UserId>,
}

/// Partial update. `None` fields are left untouched.
#[derive(Debug, Default, AsChangeset)]
#[diesel(table_name = crate::schema::entity_list_view)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EntityListViewRowPatch {
    pub name: Option<String>,
    pub config: Option<serde_json::Value>,
    pub shared: Option<bool>,
    pub updated_at: Option<NaiveDateTime>,
}

#[derive(Queryable, Selectable, Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::entity_list_view_default)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EntityListViewDefaultRow {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub entity_type: EntityListTypeEnum,
    pub view_id: EntityListViewId,
}
