use cached::proc_macro::cached;
use http::HeaderMap;
use jsonwebtoken::DecodingKey;
use secrecy::{ExposeSecret, SecretString};
use tonic::Status;
use uuid::Uuid;

use common_grpc::middleware::common::auth::{BEARER_AUTH_HEADER, TENANT_SLUG_HEADER};
use common_grpc::middleware::common::jwt::Claims;
use common_grpc::middleware::server::auth::AuthenticatedState;
use common_grpc::middleware::server::AuthorizedState;
use common_grpc::GrpcServiceMethod;
use meteroid_store::domain::enums::OrganizationUserRole;
use meteroid_store::repositories::users::UserInterface;
use meteroid_store::repositories::TenantInterface;
use meteroid_store::Store;

pub fn validate_jwt(
    header_map: &HeaderMap,
    jwt_secret: SecretString,
) -> Result<AuthenticatedState, Status> {
    let header = header_map
        .get(BEARER_AUTH_HEADER)
        .ok_or(Status::unauthenticated("Missing JWT"))?
        .to_str()
        .map_err(|_| Status::permission_denied("Invalid JWT"))?;

    let token = header
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
async fn get_tenant_id_by_slug_cached(store: Store, tenant_slug: String) -> Result<Uuid, Status> {
    let res = store
        .find_tenant_by_slug(tenant_slug)
        .await
        .map_err(|_| Status::permission_denied("Failed to retrieve tenant"))?;

    Ok(res.id)
}

#[cached(
    result = true,
    size = 50,
    time = 300, // 5 min. With RBAC, use redis backend instead & invalidate on change
    key = "Uuid",
    convert = r#"{ *user_id }"#
)]
async fn get_user_role_oss_cached(
    store: Store,
    user_id: &Uuid,
) -> Result<OrganizationUserRole, Status> {
    let res = store
        .find_user_by_id(user_id.clone(), user_id.clone())
        .await
        .map_err(|_| Status::permission_denied("Failed to retrieve user role"))
        .map(|x| x.role)?;

    Ok(res)
}

// 3 authorization possibilities : User, Organization, Tenant
pub async fn authorize_user(
    header_map: &HeaderMap,
    user_id: Uuid,
    store: Store,
    gm: GrpcServiceMethod,
) -> Result<AuthorizedState, Status> {
    let role = get_user_role_oss_cached(store.clone(), &user_id).await?;

    // if we have a tenant header, we resolve role via tenant (validating tenant access at the same time)
    let (role, state) = if let Some(tenant_slug) = header_map.get(TENANT_SLUG_HEADER) {
        let tenant_slug = tenant_slug
            .to_str()
            .map_err(|_| Status::permission_denied("Unauthorized"))?;

        let tenant_id =
            get_tenant_id_by_slug_cached(store.clone(), tenant_slug.to_string()).await?;

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
    if role == OrganizationUserRole::Member && OWNER_ONLY_METHODS.contains(&gm.method.as_str()) {
        return Err(Status::permission_denied("Unauthorized"));
    }

    Ok(state)
}
