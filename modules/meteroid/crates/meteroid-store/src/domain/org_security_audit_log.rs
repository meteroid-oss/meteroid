use common_domain::ids::OrganizationId;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use uuid::Uuid;

pub use common_domain::actor::{Actor, ActorType};

/// Stable, dot-separated event identifiers. Renames are a breaking API
/// change — auditors and downstream SIEM consumers grep these strings.
/// Format: `<domain>.<noun>.<verb>`.
#[derive(Debug, Clone, Copy, Display, EnumString, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SecurityEventType {
    // -- Authentication --
    #[strum(serialize = "auth.login.succeeded")]
    #[serde(rename = "auth.login.succeeded")]
    AuthLoginSucceeded,
    #[strum(serialize = "auth.login.failed.password")]
    #[serde(rename = "auth.login.failed.password")]
    AuthLoginFailedPassword,
    #[strum(serialize = "auth.login.failed.unknown_user")]
    #[serde(rename = "auth.login.failed.unknown_user")]
    AuthLoginFailedUnknownUser,
    #[strum(serialize = "auth.oauth.login.succeeded")]
    #[serde(rename = "auth.oauth.login.succeeded")]
    AuthOauthLoginSucceeded,
    #[strum(serialize = "auth.mfa.challenge.succeeded")]
    #[serde(rename = "auth.mfa.challenge.succeeded")]
    AuthMfaChallengeSucceeded,
    #[strum(serialize = "auth.mfa.challenge.failed")]
    #[serde(rename = "auth.mfa.challenge.failed")]
    AuthMfaChallengeFailed,
    #[strum(serialize = "auth.mfa.recovery_code.consumed")]
    #[serde(rename = "auth.mfa.recovery_code.consumed")]
    AuthMfaRecoveryCodeConsumed,
    #[strum(serialize = "auth.password_reset.requested")]
    #[serde(rename = "auth.password_reset.requested")]
    AuthPasswordResetRequested,
    #[strum(serialize = "auth.password_reset.completed")]
    #[serde(rename = "auth.password_reset.completed")]
    AuthPasswordResetCompleted,

    // -- MFA lifecycle (per-user) --
    #[strum(serialize = "user.mfa.enrolled")]
    #[serde(rename = "user.mfa.enrolled")]
    UserMfaEnrolled,
    #[strum(serialize = "user.mfa.disabled")]
    #[serde(rename = "user.mfa.disabled")]
    UserMfaDisabled,
    #[strum(serialize = "user.mfa.recovery_codes.regenerated")]
    #[serde(rename = "user.mfa.recovery_codes.regenerated")]
    UserMfaRecoveryCodesRegenerated,

    // -- Org-wide policy --
    #[strum(serialize = "organization.mfa_required.enabled")]
    #[serde(rename = "organization.mfa_required.enabled")]
    OrganizationMfaRequiredEnabled,
    #[strum(serialize = "organization.mfa_required.disabled")]
    #[serde(rename = "organization.mfa_required.disabled")]
    OrganizationMfaRequiredDisabled,

    // -- SSO (Single Sign-On) --
    #[strum(serialize = "auth.sso.login.succeeded")]
    #[serde(rename = "auth.sso.login.succeeded")]
    AuthSsoLoginSucceeded,
    #[strum(serialize = "sso.connection.created")]
    #[serde(rename = "sso.connection.created")]
    SsoConnectionCreated,
    #[strum(serialize = "sso.connection.updated")]
    #[serde(rename = "sso.connection.updated")]
    SsoConnectionUpdated,
    #[strum(serialize = "sso.connection.deleted")]
    #[serde(rename = "sso.connection.deleted")]
    SsoConnectionDeleted,
    #[strum(serialize = "sso.enforced.enabled")]
    #[serde(rename = "sso.enforced.enabled")]
    SsoEnforcedEnabled,
    #[strum(serialize = "sso.enforced.disabled")]
    #[serde(rename = "sso.enforced.disabled")]
    SsoEnforcedDisabled,

    // -- Organization domains (ownership primitive shared by SSO, white-label…) --
    #[strum(serialize = "domain.added")]
    #[serde(rename = "domain.added")]
    DomainAdded,
    #[strum(serialize = "domain.verified")]
    #[serde(rename = "domain.verified")]
    DomainVerified,
    #[strum(serialize = "domain.removed")]
    #[serde(rename = "domain.removed")]
    DomainRemoved,

    // -- Membership & invites --
    #[strum(serialize = "member.invited")]
    #[serde(rename = "member.invited")]
    MemberInvited,
    #[strum(serialize = "member.invite.resent")]
    #[serde(rename = "member.invite.resent")]
    MemberInviteResent,
    #[strum(serialize = "member.invite.revoked")]
    #[serde(rename = "member.invite.revoked")]
    MemberInviteRevoked,
    #[strum(serialize = "member.invite.accepted")]
    #[serde(rename = "member.invite.accepted")]
    MemberInviteAccepted,
    #[strum(serialize = "member.removed")]
    #[serde(rename = "member.removed")]
    MemberRemoved,
    #[strum(serialize = "member.left")]
    #[serde(rename = "member.left")]
    MemberLeft,
    #[strum(serialize = "member.role.changed")]
    #[serde(rename = "member.role.changed")]
    MemberRoleChanged,
    #[strum(serialize = "user.deleted")]
    #[serde(rename = "user.deleted")]
    UserDeleted,

    // -- OAuth apps & API tokens --
    #[strum(serialize = "oauth_app.created")]
    #[serde(rename = "oauth_app.created")]
    OauthAppCreated,
    #[strum(serialize = "oauth_app.revoked")]
    #[serde(rename = "oauth_app.revoked")]
    OauthAppRevoked,
    #[strum(serialize = "api_token.created")]
    #[serde(rename = "api_token.created")]
    ApiTokenCreated,
    #[strum(serialize = "api_token.revoked")]
    #[serde(rename = "api_token.revoked")]
    ApiTokenRevoked,
    #[strum(serialize = "support.access.granted")]
    #[serde(rename = "support.access.granted")]
    SupportAccessGranted,
    #[strum(serialize = "support.access.revoked")]
    #[serde(rename = "support.access.revoked")]
    SupportAccessRevoked,

    // -- Destructive lifecycle --
    #[strum(serialize = "tenant.deleted")]
    #[serde(rename = "tenant.deleted")]
    TenantDeleted,
    #[strum(serialize = "organization.deletion_scheduled")]
    #[serde(rename = "organization.deletion_scheduled")]
    OrganizationDeletionScheduled,
}

/// Optional request-context (IP, user agent) captured by the API layer at
/// the edge and threaded through to the store. Repository methods that
/// emit a security event accept this as an argument; it's threaded from
/// the gRPC/REST handler via a tonic request extension.
#[derive(Debug, Clone, Default)]
pub struct SecurityRequestContext {
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub ua_browser: Option<String>,
    pub ua_os: Option<String>,
    pub ua_device: Option<String>,
}

/// Inputs for `SecurityAuditInterface::record(...)`. Either an
/// already-resolved actor user_id+email pair, or "no actor" for
/// pre-authentication events like login attempts by unknown users.
#[derive(Debug, Clone)]
pub struct RecordSecurityEvent {
    pub organization_id: OrganizationId,
    pub event_type: SecurityEventType,
    pub actor: Actor,
    /// Snapshotted at write-time so the trail survives user deletion.
    pub actor_email: Option<String>,
    pub target_user_id: Option<Uuid>,
    pub target_email: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub context: SecurityRequestContext,
}
