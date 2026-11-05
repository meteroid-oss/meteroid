use chrono::{DateTime, Utc};
use common_domain::ids::{PortalTokenId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

use crate::enums::PortalResourceKindEnum;

#[derive(Queryable, Selectable, Debug, Identifiable, Clone)]
#[diesel(table_name = crate::schema::portal_token)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PortalTokenRow {
    pub id: PortalTokenId,
    pub tenant_id: TenantId,
    pub token_hash: String,
    pub resource_kind: PortalResourceKindEnum,
    pub resource_id: Uuid,
    pub recipient_email: Option<String>,
    pub scopes: Vec<Option<String>>,
    pub delegated: bool,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub last_recovery_at: Option<DateTime<Utc>>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::portal_token)]
pub struct PortalTokenRowNew {
    pub id: PortalTokenId,
    pub tenant_id: TenantId,
    pub token_hash: String,
    pub resource_kind: PortalResourceKindEnum,
    pub resource_id: Uuid,
    pub recipient_email: Option<String>,
    pub scopes: Vec<Option<String>>,
    pub delegated: bool,
    pub expires_at: Option<DateTime<Utc>>,
}
