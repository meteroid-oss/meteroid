use chrono::NaiveDateTime;
use common_domain::ids::{OrganizationId, SsoConnectionId};
use diesel_models::sso::OrganizationSsoConnectionRow;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

/// SSO protocol. Only OIDC today; the discriminator keeps SAML a code-only
/// addition later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, Serialize, Deserialize)]
pub enum SsoProvider {
    #[strum(serialize = "oidc")]
    #[serde(rename = "oidc")]
    Oidc,
}

/// A configured organization SSO connection, as surfaced to admins. The client
/// secret is intentionally absent — it stays encrypted at rest and is only
/// decrypted on the sign-in path (see the sign-in helpers on `SsoInterface`).
#[derive(Debug, Clone)]
pub struct SsoConnection {
    pub id: SsoConnectionId,
    pub organization_id: OrganizationId,
    pub provider: SsoProvider,
    /// Domains this IdP serves. A domain routes/enforces only once it's verified
    /// in `organization_domain` — see the OrganizationDomain primitive.
    pub domains: Vec<String>,
    pub issuer_url: String,
    pub authorize_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub client_id: String,
    pub scopes: String,
    pub enabled: bool,
    pub enforced: bool,
    pub jit_provisioning: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
}

impl From<OrganizationSsoConnectionRow> for SsoConnection {
    fn from(row: OrganizationSsoConnectionRow) -> Self {
        SsoConnection {
            id: row.id,
            organization_id: row.organization_id,
            provider: row.provider.parse().unwrap_or(SsoProvider::Oidc),
            domains: row.domains.into_iter().flatten().collect(),
            issuer_url: row.issuer_url,
            authorize_url: row.authorize_url,
            token_url: row.token_url,
            userinfo_url: row.userinfo_url,
            client_id: row.client_id,
            scopes: row.scopes,
            enabled: row.enabled,
            enforced: row.enforced,
            jit_provisioning: row.jit_provisioning,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
