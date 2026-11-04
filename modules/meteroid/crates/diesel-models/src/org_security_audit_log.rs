use chrono::NaiveDateTime;
use common_domain::ids::{OrgSecurityAuditLogId, OrganizationId};
use diesel::{Insertable, Queryable, Selectable};
use uuid::Uuid;

use crate::enums::ActorTypeEnum;

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::org_security_audit_log)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OrgSecurityAuditLogRow {
    pub id: OrgSecurityAuditLogId,
    pub organization_id: OrganizationId,
    pub event_type: String,
    pub actor_type: ActorTypeEnum,
    pub actor_id: Option<Uuid>,
    pub actor_email: Option<String>,
    pub target_user_id: Option<Uuid>,
    pub target_email: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub ua_browser: Option<String>,
    pub ua_os: Option<String>,
    pub ua_device: Option<String>,
    pub occurred_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::org_security_audit_log)]
pub struct OrgSecurityAuditLogRowNew {
    pub id: OrgSecurityAuditLogId,
    pub organization_id: OrganizationId,
    pub event_type: String,
    pub actor_type: ActorTypeEnum,
    pub actor_id: Option<Uuid>,
    pub actor_email: Option<String>,
    pub target_user_id: Option<Uuid>,
    pub target_email: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub ua_browser: Option<String>,
    pub ua_os: Option<String>,
    pub ua_device: Option<String>,
}
