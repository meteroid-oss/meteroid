use serde::{Deserialize, Serialize};

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
    EmailValidation { invite_key: Option<String> },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    WebApi,
    EmailValidation,
    ResetPassword,
}
impl Audience {
    pub fn as_str(&self) -> &str {
        match self {
            Self::WebApi => "web_api",
            Self::EmailValidation => "email_validation",
            Self::ResetPassword => "reset_password",
        }
    }
}
