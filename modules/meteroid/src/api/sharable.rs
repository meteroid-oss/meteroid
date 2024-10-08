use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Use to create secure shareable URLs
#[derive(Debug, Serialize, Deserialize)]
pub struct ShareableEntityClaims {
    pub sub: String,
    pub exp: usize,
    pub tenant_id: Uuid,
    pub entity_id: Uuid,
}
