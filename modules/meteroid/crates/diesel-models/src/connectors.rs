use chrono::NaiveDateTime;

use crate::enums::ConnectorProviderEnum;
use crate::enums::ConnectorStatusEnum;
use crate::enums::ConnectorTypeEnum;
use crate::enums::ConnectorWebhookRouteEnum;
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
    pub status: ConnectorStatusEnum,
    pub updated_at: NaiveDateTime,
    pub account: Option<serde_json::Value>,
    pub webhook_route: Option<ConnectorWebhookRouteEnum>,
    pub credentials_error: Option<String>,
    pub credentials_error_at: Option<NaiveDateTime>,
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
    pub status: ConnectorStatusEnum,
    pub account: Option<serde_json::Value>,
    pub webhook_route: Option<ConnectorWebhookRouteEnum>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::connector)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ConnectorRowPatch {
    pub id: ConnectorId,
    pub alias: Option<String>,
    pub data: Option<Option<serde_json::Value>>,
    pub sensitive: Option<Option<String>>,
    pub account: Option<Option<serde_json::Value>>,
    pub updated_at: Option<NaiveDateTime>,
}
