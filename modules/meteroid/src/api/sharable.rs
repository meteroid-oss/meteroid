use common_domain::ids::TenantId;
use meteroid_store::StoreResult;
use meteroid_store::errors::StoreError;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Use to create secure shareable URLs
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ShareableEntityClaims {
    pub sub: String,
    pub exp: usize,
    pub tenant_id: TenantId,
    pub entity_id: Uuid,
}

pub fn generate_entity_share_key(
    entity_id: Uuid,
    tenant_id: TenantId,
    jwt_secret: &SecretString,
    exp: usize,
) -> StoreResult<String> {
    let claims = ShareableEntityClaims {
        exp,
        sub: entity_id.to_string(),
        entity_id,
        tenant_id,
    };

    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(jwt_secret.expose_secret().as_bytes()),
    )
    .map_err(|_| {
        error_stack::Report::new(StoreError::CryptError(
            "Failed to encode shareable claims".to_string(),
        ))
    })
}
