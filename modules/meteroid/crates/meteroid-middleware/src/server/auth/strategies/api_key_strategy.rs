use common_grpc::GrpcServiceMethod;
use common_grpc::middleware::common::auth::API_KEY_HEADER;
use common_grpc::middleware::server::auth::AuthenticatedState;
use http::HeaderMap;
use meteroid_store::Store;
use tonic::Status;

use crate::server::auth::api_key::{ApiKeyAuthError, verify_api_key};

const FORBIDDEN_SERVICES: [&str; 5] = [
    "meteroid.api.organizations.v1.OrganizationsService",
    "meteroid.api.users.v1.UsersService",
    "meteroid.api.apitokens.v1.ApiTokensService",
    "meteroid.api.tenants.v1.TenantsService",
    "meteroid.api.instance.v1.InstanceService",
];

pub async fn validate_api_key(
    header_map: &HeaderMap,
    store: &Store,
    gm: &GrpcServiceMethod,
) -> Result<AuthenticatedState, Status> {
    if FORBIDDEN_SERVICES.contains(&gm.service.as_str()) {
        return Err(Status::permission_denied("Forbidden"));
    }

    let api_key = header_map
        .get(API_KEY_HEADER)
        .ok_or(Status::unauthenticated("Missing API key"))?
        .to_str()
        .map_err(|_| Status::unauthenticated("Invalid API key"))?;

    let verified = verify_api_key(api_key, store)
        .await
        .map_err(|err| match err {
            ApiKeyAuthError::Malformed => Status::unauthenticated("Invalid API key format."),
            ApiKeyAuthError::Unauthorized => Status::unauthenticated("Invalid API key"),
        })?;

    Ok(AuthenticatedState::ApiKey {
        id: verified.id,
        tenant_id: verified.tenant_id,
        organization_id: verified.organization_id,
        tenant_env: verified.tenant_env,
    })
}
