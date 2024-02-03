use super::{AuthenticatedState, AuthorizedState};
use crate::middleware::common::auth::{BEARER_AUTH_HEADER, TENANT_SLUG_HEADER};
use crate::middleware::server::utils::get_connection;
use crate::GrpcServiceMethod;

use common_repository::Pool;
use http::HeaderMap;
use jsonwebtoken::DecodingKey;
use meteroid_repository as db;
use meteroid_repository::OrganizationUserRole;
use secrecy::{ExposeSecret, SecretString};
use tonic::Status;

use crate::middleware::common::jwt::Claims;
use uuid::Uuid;

use cached::proc_macro::cached;

pub fn validate_jwt(
    header_map: &mut HeaderMap,
    jwt_secret: SecretString,
) -> Result<AuthenticatedState, Status> {

    let header = header_map
        .remove(BEARER_AUTH_HEADER);

    let bearer = header
        .as_ref()
        .ok_or(Status::unauthenticated("Missing JWT"))?
        .to_str()
        .map_err(|_| Status::permission_denied("Invalid JWT"))?;

    let token = bearer
        .strip_prefix("Bearer ")
        .ok_or(Status::unauthenticated("Missing JWT"))?;

    let decoding_key = DecodingKey::from_secret(jwt_secret.expose_secret().as_bytes());
    let decoded =
        jsonwebtoken::decode::<Claims>(token, &decoding_key, &jsonwebtoken::Validation::default())
            .map_err(|_| Status::permission_denied("Invalid JWT"))?;

    let user_id = Uuid::parse_str(&decoded.claims.sub)
        .map_err(|_| Status::permission_denied("Invalid JWT"))?;

    // check expiry
    if decoded.claims.exp < chrono::Utc::now().timestamp() as usize {
        return Err(Status::permission_denied("JWT expired"));
    }

    Ok(AuthenticatedState::User { id: user_id })
}

const OWNER_ONLY_METHODS: [&str; 1] = ["CreateTenant"];

#[cached(
    result = true,
    size = 20,
    time = 86400, // 1 day
    key = "String",
    convert = r#"{ tenant_slug.to_string() }"#
)]
async fn get_tenant_id_by_slug_cached(
    conn: &common_repository::Object,
    tenant_slug: &str,
) -> Result<Uuid, Status> {
    let res = db::tenants::get_tenant_by_slug()
        .bind(conn, &tenant_slug)
        .one()
        .await
        .map_err(|_| Status::permission_denied("Failed to retrieve tenant"))?;

    Ok(res.id)
}

#[cached(
    result = true,
    size = 50,
    time = 300, // 5 min. With RBAC, use redis backend instead & invalidate on change
    key = "String",
    convert = r#"{ user_id.to_string() }"#
)]
async fn get_user_role_oss_cached(
    conn: &common_repository::Object,
    user_id: &Uuid,
) -> Result<OrganizationUserRole, Status> {
    let res = db::users::get_user_role_oss()
        .bind(conn, &user_id)
        .one()
        .await
        .map_err(|_| Status::permission_denied("Failed to obtain user role"))?;

    Ok(res)
}

// 3 authorization possibilities : User, Organization, Tenant
pub async fn authorize_user(
    header_map: &HeaderMap,
    user_id: Uuid,
    pool: &Pool,
    gm: GrpcServiceMethod,
) -> Result<AuthorizedState, Status> {
    let conn = get_connection(pool).await?;

    let role = get_user_role_oss_cached(&conn, &user_id).await?;

    // if we have a tenant header, we resolve role via tenant (validating tenant access at the same time)
    let (role, state) = if let Some(tenant_slug) = header_map.get(TENANT_SLUG_HEADER) {
        let tenant_slug = tenant_slug
            .to_str()
            .map_err(|_| Status::permission_denied("Unauthorized"))?;

        let tenant_id = get_tenant_id_by_slug_cached(&conn, &tenant_slug).await?;

        (
            role,
            AuthorizedState::Tenant {
                tenant_id,
                actor_id: user_id,
            },
        )
    }
    // else, no org in OSS
    else {
        (role, AuthorizedState::User { user_id })
    };
    if role == OrganizationUserRole::MEMBER && OWNER_ONLY_METHODS.contains(&gm.method.as_str()) {
        return Err(Status::permission_denied("Unauthorized"));
    }

    Ok(state)
}
