pub const HMAC_SIGNATURE_HEADER: &str = "x-hmac-signature";
pub const HMAC_TIMESTAMP_HEADER: &str = "x-hmac-timestamp";
// pub const HMAC_NONCE_HEADER: &str = "x-hmac-nonce";

pub const API_KEY_HEADER: &str = "x-api-key";
pub const PORTAL_KEY_HEADER: &str = "x-portal-token";

pub const BEARER_AUTH_HEADER: &str = "Authorization";

// used by frontend clients to specify organization/tenant, when a JWT is used (api key already provide these infos). Context is validated against jwt's user.
pub const INTERNAL_API_CONTEXT_HEADER: &str = "x-md-context";
