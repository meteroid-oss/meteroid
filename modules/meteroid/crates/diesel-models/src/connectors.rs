use chrono::NaiveDateTime;

use crate::enums::ConnectorProviderEnum;
use crate::enums::ConnectorTypeEnum;
use common_domain::ids::{ConnectorId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Selectable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::connector)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ConnectorRow {
    pub id: ConnectorId,
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
    pub id: ConnectorId,
    pub tenant_id: TenantId,
    pub alias: String,
    pub connector_type: ConnectorTypeEnum,
    pub provider: ConnectorProviderEnum,
    pub data: Option<serde_json::Value>,
    pub sensitive: Option<String>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::connector)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ConnectorRowPatch {
    pub id: ConnectorId,
    pub data: Option<Option<serde_json::Value>>,
}
