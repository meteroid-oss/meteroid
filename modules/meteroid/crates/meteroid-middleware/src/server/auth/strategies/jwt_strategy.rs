use cached::proc_macro::cached;
use common_domain::auth::{Audience, JwtClaims};
use common_domain::ids::{OrganizationId, TenantId};
use common_grpc::GrpcServiceMethod;
use common_grpc::middleware::common::auth::{BEARER_AUTH_HEADER, INTERNAL_API_CONTEXT_HEADER};
use common_grpc::middleware::server::AuthorizedState;
use common_grpc::middleware::server::auth::{AuthenticatedState, AuthorizedAsTenant};
use http::HeaderMap;
use jsonwebtoken::DecodingKey;
use meteroid_store::Store;
use meteroid_store::domain::enums::OrganizationUserRole;
use meteroid_store::repositories::users::UserInterface;
use meteroid_store::repositories::{OrganizationsInterface, TenantInterface};
use secrecy::{ExposeSecret, SecretString};
use std::time::Duration;
use tonic::Status;
use uuid::Uuid;

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

    let mut validation = jsonwebtoken::Validation::default();
    validation.set_audience(&[Audience::WebApi.as_str()]);

    let decoded = jsonwebtoken::decode::<JwtClaims>(token, &decoding_key, &validation)
        .map_err(|_| Status::permission_denied("Invalid JWT"))?;

    let user_id = Uuid::parse_str(&decoded.claims.sub)
        .map_err(|_| Status::permission_denied("Invalid JWT"))?;

    // check expiry
    if decoded.claims.exp < chrono::Utc::now().timestamp() as usize {
        return Err(Status::permission_denied("JWT expired"));
    }

    Ok(AuthenticatedState::User { id: user_id })
}

// const OWNER_ONLY_METHODS: [&str; 1] = ["CreateTenant"];
const OWNER_ONLY_METHODS: [&str; 0] = [];

#[cached(
    result = true,
    size = 100,
    time = 86400, // 1 day
    key = "(String, Option<String>)",
    convert = r#"{ (organization_slug.clone(), tenant_slug.clone()) }"#
)]
async fn resolve_slugs_cached(
    store: Store,
    organization_slug: String,
    tenant_slug: Option<String>,
) -> Result<(OrganizationId, Option<TenantId>), Status> {
    let org_and_tenant_ids = if let Some(tenant_slug) = tenant_slug {
        let res = store
            .find_tenant_by_slug_and_organization_slug(
                tenant_slug.clone(),
                organization_slug.clone(),
            )
            .await
            .map_err(|_| {
                Status::permission_denied(format!(
                    "Failed to retrieve tenant for slug {} and organization slug {}",
                    &tenant_slug, &organization_slug
                ))
            })?;

        (res.organization_id, Some(res.id))
    } else {
        let org_id = store
            .get_organizations_by_slug(organization_slug)
            .await
            .map_err(|_| Status::permission_denied("Failed to retrieve organization"))?
            .id;

        (org_id, None)
    };

    Ok(org_and_tenant_ids)
}

pub async fn invalidate_resolve_slugs_cache(organization_slug: &str, tenant_slug: &str) {
    {
        use cached::Cached;
        let mut cache = self::RESOLVE_SLUGS_CACHED.lock().await;
        cache.cache_remove(&(organization_slug.to_string(), Some(tenant_slug.to_string())));
    }
}

#[cached(
    result = true,
    size = 150,
    time = 300, // 5 min. With RBAC, use redis backend instead & invalidate on change
    key = "(Uuid, OrganizationId)",
    convert = r#"{ (*user_id, org_id) }"#
)]
async fn get_user_role_oss_cached(
    store: Store,
    user_id: &Uuid,
    org_id: OrganizationId,
) -> Result<OrganizationUserRole, Status> {
    let res = store
        .find_user_by_id_and_organization(*user_id, org_id)
        .await
        .map_err(|_| {
            Status::permission_denied(format!(
                "Failed to retrieve user role for organization {org_id} and user {user_id}"
            ))
        })
        .map(|x| x.role)?;

    Ok(res)
}

fn extract_context(header_map: &HeaderMap) -> Result<(String, Option<String>), Status> {
    let context = header_map
        .get(INTERNAL_API_CONTEXT_HEADER)
        .ok_or(Status::permission_denied(
            "Unauthorized. Missing org/tenant context",
        ))?
        .to_str()
        .map_err(|_| Status::permission_denied("Unauthorized. Invalid context"))?
        .split('/')
        .collect::<Vec<&str>>();

    if context.len() != 2 {
        return Err(Status::permission_denied(
            "Invalid auth context. Too many parts",
        ));
    }

    Ok((
        context[0].to_string(),
        if context[1].is_empty() {
            None
        } else {
            Some(context[1].to_string())
        },
    ))
}

pub async fn authorize_user(
    header_map: &HeaderMap,
    user_id: Uuid,
    store: Store,
    gm: GrpcServiceMethod,
) -> Result<AuthorizedState, Status> {
    let (org_slug, tenant_slug) = extract_context(header_map)?;
    let (organization_id, tenant_id) =
        resolve_slugs_cached(store.clone(), org_slug, tenant_slug).await?;

    let role = get_user_role_oss_cached(store.clone(), &user_id, organization_id).await?;

    // if we have a tenant header, we resolve role via tenant (validating tenant access at the same time)
    let (role, state) = if let Some(tenant_id) = tenant_id {
        (
            role,
            AuthorizedState::Tenant(AuthorizedAsTenant {
                tenant_id,
                organization_id,
                actor_id: user_id,
            }),
        )
    } else {
        (
            role,
            AuthorizedState::Organization {
                organization_id,
                actor_id: user_id,
            },
        )
    };
    if role == OrganizationUserRole::Member && OWNER_ONLY_METHODS.contains(&gm.method.as_str()) {
        return Err(Status::permission_denied(
            "Unauthorized. Only organization owners can perform this action.",
        ));
    }

    Ok(state)
}
