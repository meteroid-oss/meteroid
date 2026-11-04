//! Signed links to a batch job's files, shared by the gRPC and REST APIs.

use crate::sharable::ShareableEntityClaims;
use common_domain::ids::BaseId;
use secrecy::{ExposeSecret, SecretString};

pub fn generate_input_file_token(
    job_id: common_domain::ids::BatchJobId,
    tenant_id: common_domain::ids::TenantId,
    jwt_secret: &SecretString,
) -> Result<String, jsonwebtoken::errors::Error> {
    let exp = (chrono::Utc::now() + chrono::Duration::days(7)).timestamp() as usize;
    let claims = ShareableEntityClaims {
        exp,
        sub: format!("input-{}", job_id),
        entity_id: job_id.as_uuid(),
        tenant_id,
    };
    meteroid_store::jwt::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(jwt_secret.expose_secret().as_bytes()),
    )
}

pub fn generate_error_csv_token(
    job_id: common_domain::ids::BatchJobId,
    tenant_id: common_domain::ids::TenantId,
    jwt_secret: &SecretString,
) -> Result<String, jsonwebtoken::errors::Error> {
    let exp = (chrono::Utc::now() + chrono::Duration::days(7)).timestamp() as usize;
    let claims = ShareableEntityClaims {
        exp,
        sub: job_id.to_string(),
        entity_id: job_id.as_uuid(),
        tenant_id,
    };
    meteroid_store::jwt::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(jwt_secret.expose_secret().as_bytes()),
    )
}

pub fn generate_error_csv_url(
    job_id: common_domain::ids::BatchJobId,
    tenant_id: common_domain::ids::TenantId,
    jwt_secret: &SecretString,
) -> Result<String, jsonwebtoken::errors::Error> {
    let token = generate_error_csv_token(job_id, tenant_id, jwt_secret)?;
    Ok(format!(
        "/files/v1/batch-job/errors/{}?token={}",
        job_id, token
    ))
}

pub fn generate_output_token(
    job_id: common_domain::ids::BatchJobId,
    tenant_id: common_domain::ids::TenantId,
    jwt_secret: &SecretString,
) -> Result<String, jsonwebtoken::errors::Error> {
    let exp = (chrono::Utc::now() + chrono::Duration::days(7)).timestamp() as usize;
    let claims = ShareableEntityClaims {
        exp,
        sub: format!("output-{}", job_id),
        entity_id: job_id.as_uuid(),
        tenant_id,
    };
    meteroid_store::jwt::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(jwt_secret.expose_secret().as_bytes()),
    )
}

pub fn generate_output_url(
    job_id: common_domain::ids::BatchJobId,
    tenant_id: common_domain::ids::TenantId,
    jwt_secret: &SecretString,
) -> Result<String, jsonwebtoken::errors::Error> {
    let token = generate_output_token(job_id, tenant_id, jwt_secret)?;
    Ok(format!(
        "/files/v1/tax-report/output/{}?token={}",
        job_id, token
    ))
}
