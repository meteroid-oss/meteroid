use cached::proc_macro::cached;
use common_domain::ids::{OrganizationId, TenantId};
use common_grpc::GrpcServiceMethod;
use common_grpc::middleware::common::auth::API_KEY_HEADER;
use common_grpc::middleware::server::auth::AuthenticatedState;
use common_grpc::middleware::server::auth::api_token_validator::ApiTokenValidator;
use http::HeaderMap;
use meteroid_store::Store;
use meteroid_store::repositories::api_tokens::ApiTokensInterface;
use tonic::Status;
use uuid::Uuid;

const FORBIDDEN_SERVICES: [&str; 5] = [
    "meteroid.api.organizations.v1.OrganizationsService",
    "meteroid.api.users.v1.UsersService",
    "meteroid.api.apitokens.v1.ApiTokensService",
    "meteroid.api.tenants.v1.TenantsService",
    "meteroid.api.instance.v1.InstanceService",
];

#[cached(
    result = true,
    size = 100,
    time = 120, // 2 min
    key = "Uuid",
    convert = r#"{ *api_key_id }"#
)]
async fn validate_api_token_by_id_cached(
    store: &Store,
    validator: &ApiTokenValidator,
    api_key_id: &Uuid,
) -> Result<(OrganizationId, TenantId), Status> {
    let res = store
        .get_api_token_by_id_for_validation(api_key_id)
        .await
        .map_err(|_| Status::permission_denied("Failed to retrieve api key"))?;

    validator
        .validate_hash(&res.hash)
        .map_err(|_| Status::permission_denied("Unauthorized"))?;

    Ok((res.organization_id, res.tenant_id))
}

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
        .map_err(|_| Status::permission_denied("Invalid API key"))?;

    let validator = ApiTokenValidator::parse_api_key(api_key)
        .map_err(|_| Status::permission_denied("Invalid API key format."))?;

    let id = validator.extract_identifier().map_err(|_| {
        Status::permission_denied("Invalid API key format. Failed to extract identifier")
    })?;

    let (organization_id, tenant_id) =
        validate_api_token_by_id_cached(store, &validator, &id).await?;

    Ok(AuthenticatedState::ApiKey {
        id,
        tenant_id,
        organization_id,
    })
}
