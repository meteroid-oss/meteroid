use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrgMemberRole {
    Admin,
    Member,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JwtClaims {
    pub sub: String,
    pub exp: usize,
    pub aud: Audience,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<JwtPayload>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", content = "data")]
pub enum JwtPayload {
    EmailValidation {
        invite_key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signup_source: Option<String>,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    WebApi,
    EmailValidation,
    ResetPassword,
    MfaChallenge,
}
impl Audience {
    pub fn as_str(&self) -> &str {
        match self {
            Self::WebApi => "web_api",
            Self::EmailValidation => "email_validation",
            Self::ResetPassword => "reset_password",
            Self::MfaChallenge => "mfa_challenge",
        }
    }
}

/// What a customer portal token may do. `Manage` implies `Read`.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PortalScope {
    Read,
    Manage,
}

impl PortalScope {
    pub const ALL: [PortalScope; 2] = [PortalScope::Read, PortalScope::Manage];
}
