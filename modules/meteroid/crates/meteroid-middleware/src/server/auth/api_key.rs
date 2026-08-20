use cached::proc_macro::cached;
use common_domain::ids::{ApiTokenId, OrganizationId, TenantId};
use common_grpc::middleware::server::auth::TenantEnv;
use common_grpc::middleware::server::auth::api_token_validator::{
    ApiTokenValidator, CredentialFingerprint,
};
use meteroid_store::Store;
use meteroid_store::domain::TenantEnvironmentEnum;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::api_tokens::ApiTokensInterface;
use tracing::error;

/// Why a presented API key was rejected.
///
/// Deliberately coarse: an unknown token id and a wrong secret are both `Unauthorized`, so the
/// response cannot be used to confirm that a guessed token id exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeyAuthError {
    /// Absent, non-ASCII, or unparseable credential.
    Malformed,
    /// Parsed, but authorizes nobody.
    Unauthorized,
}

#[derive(Debug, Clone)]
pub struct VerifiedApiKey {
    pub id: ApiTokenId,
    pub organization_id: OrganizationId,
    pub tenant_id: TenantId,
    pub tenant_env: TenantEnv,
}

/// Resolves a raw API key to its tenant, for both the gRPC and REST entry points.
///
/// Callers must not reimplement this: the cache below is only safe because every caller reaches
/// it through the same credential-fingerprint key.
pub async fn verify_api_key(
    api_key: &str,
    store: &Store,
) -> Result<VerifiedApiKey, ApiKeyAuthError> {
    let validator =
        ApiTokenValidator::parse_api_key(api_key).map_err(|_| ApiKeyAuthError::Malformed)?;

    let id = ApiTokenId::from_const(
        validator
            .extract_identifier()
            .map_err(|_| ApiKeyAuthError::Malformed)?,
    );

    let (organization_id, tenant_id, tenant_env) =
        resolve_api_token_cached(store, &validator, &id).await?;

    Ok(VerifiedApiKey {
        id,
        organization_id,
        tenant_id,
        tenant_env,
    })
}

/// Keyed by a digest of the whole credential rather than by token id, so a warm entry can only
/// ever authorize the exact secret that already passed Argon2 verification. The key is derived
/// from the arguments themselves, so no caller can decouple it from the credential being checked.
///
/// The capacity is far above the number of keys a healthy deployment keeps warm. Entries are
/// per-credential now, so a smaller cache would let an attacker replaying forged secrets against
/// one known token id evict every legitimate entry and force an Argon2 verification on real
/// traffic. `result = true` keeps failures out of the cache entirely.
#[cached(
    result = true,
    size = 10000,
    time = 120, // 2 min
    key = "CredentialFingerprint",
    convert = r#"{ validator.credential_fingerprint(api_key_id) }"#
)]
async fn resolve_api_token_cached(
    store: &Store,
    validator: &ApiTokenValidator,
    api_key_id: &ApiTokenId,
) -> Result<(OrganizationId, TenantId, TenantEnv), ApiKeyAuthError> {
    let res = store
        .get_api_token_by_id_for_validation(api_key_id)
        .await
        .map_err(|err| {
            match err.current_context() {
                StoreError::ValueNotFound(_) => {}
                other => error!("Failed to resolve api key: {:?}", other),
            }
            ApiKeyAuthError::Unauthorized
        })?;

    validator
        .validate_hash(&res.hash)
        .map_err(|_| ApiKeyAuthError::Unauthorized)?;

    let tenant_env = if res.environment == TenantEnvironmentEnum::Production {
        TenantEnv::Production
    } else {
        TenantEnv::NonProduction
    };

    Ok((res.organization_id, res.tenant_id, tenant_env))
}
