use cached::proc_macro::cached;
use common_domain::auth::{Audience, JwtClaims, OrgMemberRole};
use common_domain::ids::{OrganizationId, TenantId, UserId};
use common_domain::rbac::RolePermissions;
use common_grpc::GrpcServiceMethod;
use common_grpc::middleware::common::auth::{BEARER_AUTH_HEADER, INTERNAL_API_CONTEXT_HEADER};
use common_grpc::middleware::server::AuthorizedState;
use common_grpc::middleware::server::auth::{
    AuthenticatedState, AuthorizedAsTenant, TenantActor, TenantEnv,
};
use http::HeaderMap;
use jsonwebtoken::DecodingKey;
use meteroid_store::Store;
use meteroid_store::domain::TenantEnvironmentEnum;
use meteroid_store::domain::enums::OrganizationUserRole;
use meteroid_store::domain::roles::MemberAccess;
use meteroid_store::repositories::{
    OrganizationsInterface, RolesInterface, TenantInterface, UserDeletionInterface,
};

use crate::server::auth::SharedSupportGrantReader;
use crate::server::auth::rbac;
use secrecy::{ExposeSecret, SecretString};
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
        .map_err(|_| Status::unauthenticated("Invalid JWT"))?;

    let token = header
        .strip_prefix("Bearer ")
        .ok_or(Status::unauthenticated("Missing JWT"))?;

    let decoding_key = DecodingKey::from_secret(jwt_secret.expose_secret().as_bytes());

    let mut validation = jsonwebtoken::Validation::default();
    validation.set_audience(&[Audience::WebApi.as_str()]);

    let decoded = meteroid_store::jwt::decode::<JwtClaims>(token, &decoding_key, &validation)
        .map_err(|_| Status::unauthenticated("Invalid JWT"))?;

    let user_id = UserId::from_const(
        Uuid::parse_str(&decoded.claims.sub).map_err(|_| Status::unauthenticated("Invalid JWT"))?,
    );

    // check expiry
    if decoded.claims.exp < chrono::Utc::now().timestamp() as usize {
        return Err(Status::unauthenticated("JWT expired"));
    }

    Ok(AuthenticatedState::User { id: user_id })
}

// Method-level authorization is enforced centrally via `rbac::method_auth`.

async fn resolve_slugs(
    store: &Store,
    organization_slug: String,
    tenant_slug: Option<String>,
) -> Result<(OrganizationId, Option<TenantId>), Status> {
    if let Some(ids) = store
        .slug_ids
        .get(&organization_slug, tenant_slug.as_deref())
    {
        return Ok(ids);
    }
    let ids = if let Some(tenant_slug) = &tenant_slug {
        let res = store
            .find_tenant_by_slug_and_organization_slug(
                tenant_slug.clone(),
                organization_slug.clone(),
            )
            .await
            .map_err(|_| {
                Status::permission_denied(format!(
                    "Failed to retrieve tenant for slug {} and organization slug {}",
                    tenant_slug, &organization_slug
                ))
            })?;

        (res.organization_id, Some(res.id))
    } else {
        let org_id = store
            .get_organizations_by_slug(organization_slug.clone())
            .await
            .map_err(|_| Status::permission_denied("Failed to retrieve organization"))?
            .id;

        (org_id, None)
    };
    store.slug_ids.insert(organization_slug, tenant_slug, ids);

    Ok(ids)
}

#[cached(
    result = true,
    size = 150,
    time = 300, // 5 min. TODO: redis-backed cache invalidated on role change.
    key = "(UserId, OrganizationId)",
    convert = r#"{ (*user_id, org_id) }"#
)]
async fn get_member_access_cached(
    store: &Store,
    user_id: &UserId,
    org_id: OrganizationId,
) -> Result<MemberAccess, Status> {
    store
        .resolve_member_access(*user_id, org_id)
        .await
        .map_err(|_| {
            Status::permission_denied(format!(
                "Failed to resolve access for organization {org_id} and user {user_id}"
            ))
        })
}

/// Drop the cached access for one member so a role change takes effect on their
/// next request instead of after the 5-minute TTL.
pub async fn invalidate_member_access_cache(user_id: UserId, org_id: OrganizationId) {
    use cached::Cached;
    let mut cache = GET_MEMBER_ACCESS_CACHED.lock().await;
    cache.cache_remove(&(user_id, org_id));
}

/// Short TTL: invalidation only reaches the replica that deleted the user, so this bounds how
/// long a deleted user's session survives on the others.
#[cached(
    result = true,
    size = 10_000,
    time = 60,
    key = "UserId",
    convert = r#"{ user_id }"#
)]
pub async fn is_user_active_cached(store: &Store, user_id: UserId) -> Result<bool, Status> {
    store
        .is_user_active(user_id)
        .await
        .map_err(|e| Status::internal(format!("Failed to check user: {e}")))
}

pub async fn invalidate_user_active_cache(user_id: UserId) {
    use cached::Cached;
    let mut cache = IS_USER_ACTIVE_CACHED.lock().await;
    cache.cache_remove(&user_id);
}

/// Clear all cached member access. Used when a custom role's permissions change
/// or the role is deleted, which can affect any number of members at once.
pub async fn invalidate_all_member_access_cache() {
    use cached::Cached;
    let mut cache = GET_MEMBER_ACCESS_CACHED.lock().await;
    cache.cache_clear();
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
    user_id: UserId,
    store: &Store,
    support: Option<&SharedSupportGrantReader>,
    gm: GrpcServiceMethod,
    module_policies: &[rbac::RoutePolicy],
) -> Result<(AuthorizedState, RolePermissions), Status> {
    let (org_slug, tenant_slug) = extract_context(header_map)?;
    let (organization_id, tenant_id) = resolve_slugs(store, org_slug, tenant_slug).await?;

    let access = match get_member_access_cached(store, &user_id, organization_id).await {
        Ok(access) => access,
        // Not a member: allow a platform admin holding a live support grant to
        // proceed at the granted level. Any other case keeps the original denial.
        Err(denied) => resolve_support_access(store, support, &user_id, organization_id)
            .await
            .ok_or(denied)?,
    };
    let role = access.role;

    // if we have a tenant header, we resolve role via tenant (validating tenant access at the same time)
    let (_role, state) = if let Some(tenant_id) = tenant_id {
        let tenant_env = get_tenant_env(store, tenant_id).await?;
        (
            role,
            AuthorizedState::Tenant(AuthorizedAsTenant {
                tenant_id,
                organization_id,
                actor: TenantActor::User {
                    id: user_id,
                    role: match role {
                        OrganizationUserRole::Admin => OrgMemberRole::Admin,
                        OrganizationUserRole::Member => OrgMemberRole::Member,
                    },
                },
                tenant_env,
            }),
        )
    } else {
        (
            role,
            AuthorizedState::Organization {
                organization_id,
                user_id,
                role: match role {
                    OrganizationUserRole::Admin => OrgMemberRole::Admin,
                    OrganizationUserRole::Member => OrgMemberRole::Member,
                },
            },
        )
    };
    rbac::enforce_with(&gm, &access.permissions, module_policies)?;

    Ok((state, access.permissions))
}

/// A platform admin can reach an org they're not a member of, at the granted level,
/// while a support-access grant is live. Returns `None` (→ deny) unless the caller is a
/// platform admin AND a grant exists. Fail-closed and cheap: the platform-admin
/// check reuses the in-memory member-access cache, so Redis is consulted only for
/// an actual platform admin on a membership miss (never on normal traffic).
async fn resolve_support_access(
    store: &Store,
    support: Option<&SharedSupportGrantReader>,
    user_id: &UserId,
    org_id: OrganizationId,
) -> Option<MemberAccess> {
    let reader = support?;
    let admin_org = store.settings.admin_organization?;

    if get_member_access_cached(store, user_id, admin_org)
        .await
        .is_err()
    {
        return None;
    }
    let mode = reader.grant_mode(*user_id, org_id).await?;
    Some(mode.member_access())
}

async fn get_tenant_env(store: &Store, tenant_id: TenantId) -> Result<TenantEnv, Status> {
    let tenant = store
        // this one is cached already
        .find_tenant_by_id(tenant_id)
        .await
        .map_err(|_| Status::permission_denied(format!("Failed to retrieve tenant {tenant_id}")))?;

    match tenant.tenant.environment {
        TenantEnvironmentEnum::Production => Ok(TenantEnv::Production),
        _ => Ok(TenantEnv::NonProduction),
    }
}
