use crate::enums::{CustomPropertyEntityTypeEnum, CustomPropertyTypeEnum};
use chrono::NaiveDateTime;
use common_domain::ids::{CustomPropertyDefinitionId, TenantId};
use diesel::{Identifiable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable, Clone)]
#[diesel(table_name = crate::schema::custom_property_definition)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomPropertyDefinitionRow {
    pub id: CustomPropertyDefinitionId,
    pub tenant_id: TenantId,
    pub entity_type: CustomPropertyEntityTypeEnum,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub property_type: CustomPropertyTypeEnum,
    pub config: serde_json::Value,
    pub required: bool,
    pub default_value: Option<serde_json::Value>,
    pub display_order: i32,
    pub archived_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
}
