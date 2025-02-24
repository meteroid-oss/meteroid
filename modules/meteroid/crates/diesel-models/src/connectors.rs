use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::ConnectorProviderEnum;
use crate::enums::ConnectorTypeEnum;
use common_domain::ids::TenantId;
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Selectable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::connector)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ConnectorRow {
    pub id: Uuid,
    pub created_at: NaiveDateTime,
    pub tenant_id: TenantId,
    pub alias: String,
    pub connector_type: ConnectorTypeEnum,
    pub provider: ConnectorProviderEnum,
    pub data: Option<serde_json::Value>,
    pub sensitive: Option<String>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::connector)]
pub struct ConnectorRowNew {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub alias: String,
    pub connector_type: ConnectorTypeEnum,
    pub provider: ConnectorProviderEnum,
    pub data: Option<serde_json::Value>,
    pub sensitive: Option<String>,
}
