use chrono::NaiveDateTime;
use common_domain::ids::{OrganizationId, SsoConnectionId};
use diesel::{Identifiable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Queryable, Selectable, Debug, Clone, Identifiable)]
#[diesel(table_name = crate::schema::organization_sso_connection)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OrganizationSsoConnectionRow {
    pub id: SsoConnectionId,
    pub organization_id: OrganizationId,
    pub provider: String,
    pub domains: Vec<Option<String>>,
    pub issuer_url: String,
    pub authorize_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub scopes: String,
    pub enabled: bool,
    pub enforced: bool,
    pub jit_provisioning: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub created_by: Option<Uuid>,
}
