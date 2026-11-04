use crate::domain::enums::OrganizationUserRole;
use common_domain::ids::{RoleId, UserId};
use secrecy::SecretString;

use crate::domain::Organization;
use diesel_models::users::{UserRow, UserWithRoleRow};

#[derive(Clone, Debug)]
pub struct User {
    pub id: UserId,
    pub email: String,
    pub onboarded: bool,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub department: Option<String>,
    pub mfa_enabled: bool,
    /// Opaque per-user preferences bag, owned and shaped by the frontend.
    pub preferences: serde_json::Value,
}

impl From<UserRow> for User {
    fn from(row: UserRow) -> Self {
        User {
            id: row.id,
            email: row.email,
            onboarded: row.onboarded,
            first_name: row.first_name,
            last_name: row.last_name,
            department: row.department,
            mfa_enabled: row.totp_enabled,
            preferences: row.preferences,
        }
    }
}

#[derive(Clone, Debug)]
pub struct UserWithRole {
    pub id: UserId,
    pub email: String,
    pub role: OrganizationUserRole,
    pub onboarded: bool,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub department: Option<String>,
    pub mfa_enabled: bool,
    pub custom_role_id: Option<RoleId>,
}

impl From<UserWithRoleRow> for UserWithRole {
    fn from(row: UserWithRoleRow) -> Self {
        UserWithRole {
            id: row.id,
            email: row.email,
            role: row.role.into(),
            onboarded: row.onboarded,
            first_name: row.first_name,
            last_name: row.last_name,
            department: row.department,
            mfa_enabled: row.totp_enabled,
            custom_role_id: row.role_id,
        }
    }
}

impl From<UserWithRole> for User {
    fn from(val: UserWithRole) -> Self {
        User {
            id: val.id,
            email: val.email,
            onboarded: val.onboarded,
            first_name: val.first_name,
            last_name: val.last_name,
            department: val.department,
            mfa_enabled: val.mfa_enabled,
            // UserWithRole (admin/list views) doesn't carry preferences.
            preferences: serde_json::json!({}),
        }
    }
}

pub struct Me {
    pub user: User,
    pub organizations: Vec<Organization>,
    /// True when the user belongs to at least one organization that enforces
    /// MFA but they have not yet enrolled. The frontend should gate the UI to
    /// force enrollment before any further action.
    pub mfa_enrollment_required: bool,
}

#[derive(Clone, Debug)]
pub struct LoginUserRequest {
    pub email: String,
    pub password: SecretString,
    pub request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
}

/// Result of a login attempt. Either an authenticated session token is
/// issued, or a short-lived MFA challenge token is returned and the
/// caller must complete the challenge via `verify_mfa_login`.
#[derive(Clone, Debug)]
pub enum LoginUserResponse {
    Authenticated { token: SecretString, user: User },
    MfaChallenge { challenge_token: SecretString },
}

#[derive(Clone, Debug)]
pub struct MfaEnrollmentInit {
    pub secret_base32: String,
    pub otpauth_uri: SecretString,
}

#[derive(Clone, Debug)]
pub struct MfaActivated {
    /// Plaintext one-time-use recovery codes. Shown to the user ONCE; only hashes are persisted.
    pub recovery_codes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct RegisterUserRequest {
    pub email: String,
    pub password: Option<SecretString>,
    pub invite_key: Option<SecretString>,
    pub email_validation_token: Option<SecretString>,
}

#[derive(Clone, Debug)]
pub struct InitRegistrationResponse {
    pub validation_required: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct RegisterUserRequestInternal {
    pub email: String,
    pub password: Option<SecretString>,
    pub invite_key: Option<SecretString>,
    pub signup_source: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RegisterUserResponse {
    pub token: SecretString,
    pub user: User,
}

#[derive(Debug)]
pub struct UpdateUser {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub department: Option<String>,
    pub heard_from: Option<String>,
}

#[derive(Debug)]
pub struct UpdateUserRole {
    pub role: OrganizationUserRole,
    pub user_id: UserId,
}
