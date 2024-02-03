use super::AuthenticatedState;
use cached::proc_macro::cached;

use common_repository::Pool;
use http::HeaderMap;
use tonic::Status;

use crate::middleware::common::auth::API_KEY_HEADER;
use crate::middleware::server::auth::api_token_validator::ApiTokenValidator;
use crate::middleware::server::utils::get_connection;
use crate::GrpcServiceMethod;
use meteroid_repository as db;

use uuid::Uuid;

const FORBIDDEN_SERVICES: [&str; 4] = [
    "meteroid.api.organizations.v1.OrganizationsService",
    "meteroid.api.users.v1.UsersService",
    "meteroid.api.apitokens.v1.ApiTokensService",
    "meteroid.api.tenants.v1.TenantsService",
];

#[cached(
    result = true,
    size = 100,
    time = 120, // 2 min
    key = "String",
    convert = r#"{ api_key_id.to_string() }"#
)]
async fn validate_api_token_by_id_cached(
    conn: &common_repository::Object,
    validator: &ApiTokenValidator,
    api_key_id: &Uuid,
) -> Result<Uuid, Status> {
    let res = db::api_tokens::get_api_token_by_id()
        .bind(conn, &api_key_id)
        .one()
        .await
        .map_err(|_| Status::permission_denied("Failed to retrieve api key"))?;

    validator
        .validate_hash(&res.hash)
        .map_err(|_| Status::permission_denied("Unauthorized"))?;

    Ok(res.tenant_id)
}

pub async fn validate_api_key(
    header_map: &mut HeaderMap,
    pool: &Pool,
    gm: &GrpcServiceMethod,
) -> Result<AuthenticatedState, Status> {
    if FORBIDDEN_SERVICES.contains(&gm.service.as_str()) {
        return Err(Status::permission_denied("Forbidden"));
    }

    let api_key_header = header_map
        .remove(API_KEY_HEADER);

    let api_key = api_key_header
        .as_ref()
        .ok_or(Status::unauthenticated("Missing API key"))?
        .to_str()
        .map_err(|_| Status::permission_denied("Invalid API key"))?;

    let validator = ApiTokenValidator::parse_api_key(api_key)
        .map_err(|_| Status::permission_denied("Invalid API key format."))?;

    let id = validator.extract_identifier().map_err(|_| {
        Status::permission_denied("Invalid API key format. Failed to extract identifier")
    })?;

    let conn = get_connection(pool).await?;

    let tenant_id = validate_api_token_by_id_cached(&conn, &validator, &id).await?;

    Ok(AuthenticatedState::ApiKey { id, tenant_id })
}
