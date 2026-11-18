use crate::StoreResult;
use crate::errors::StoreError;
use common_domain::ids::{BaseId, TenantId};
use error_stack::Report;
use uuid::Uuid;

/// A public API write keyed by the caller's Idempotency-Key. The fingerprint is a digest of the
/// request, so the same key sent with a different request is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiIdempotency {
    pub operation: &'static str,
    pub key: String,
    pub fingerprint: [u8; 32],
}

impl ApiIdempotency {
    pub fn new(operation: &'static str, key: &str, fingerprint: [u8; 32]) -> StoreResult<Self> {
        let key = key.trim();
        if key.is_empty() || key.len() > 255 {
            return Err(Report::new(StoreError::InvalidArgument(
                "an idempotency key is 1 to 255 bytes".to_owned(),
            )));
        }
        Ok(Self {
            operation,
            key: key.to_owned(),
            fingerprint,
        })
    }

    /// Stable per tenant, operation and key, so a retry addresses what its first attempt made.
    pub fn derived_id(&self, tenant_id: TenantId) -> Uuid {
        Uuid::new_v5(
            &tenant_id.as_uuid(),
            format!("api-write:{}:{}", self.operation, self.key).as_bytes(),
        )
    }
}
