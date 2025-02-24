use tonic::Status;
use uuid::Uuid;

pub use admin_layer::AdminAuthLayer;
pub use admin_layer::AdminAuthService;
use common_config::auth::InternalAuthConfig;
use common_domain::ids::{OrganizationId, TenantId};

mod admin_layer;
pub mod api_token_validator;

pub fn create_admin(config: &InternalAuthConfig) -> AdminAuthLayer {
    AdminAuthLayer::new(config)
}

pub trait RequestExt {
    fn actor(&self) -> Result<Uuid, Status>;
    fn tenant(&self) -> Result<TenantId, Status>;
    fn organization(&self) -> Result<OrganizationId, Status>;
}

impl<T> RequestExt for tonic::Request<T> {
    fn actor(&self) -> Result<Uuid, Status> {
        extract_actor(self.extensions().get::<AuthorizedState>())
    }

    fn tenant(&self) -> Result<TenantId, Status> {
        extract_tenant(self.extensions().get::<AuthorizedState>())
    }

    fn organization(&self) -> Result<OrganizationId, Status> {
        extract_organization(self.extensions().get::<AuthorizedState>())
    }
}

impl<T> RequestExt for http::Request<T> {
    fn actor(&self) -> Result<Uuid, Status> {
        extract_actor(self.extensions().get::<AuthorizedState>())
    }

    fn tenant(&self) -> Result<TenantId, Status> {
        extract_tenant(self.extensions().get::<AuthorizedState>())
    }

    fn organization(&self) -> Result<OrganizationId, Status> {
        extract_organization(self.extensions().get::<AuthorizedState>())
    }
}

pub fn extract_actor(maybe_auth: Option<&AuthorizedState>) -> Result<Uuid, Status> {
    let authorized = maybe_auth.ok_or(Status::unauthenticated(
        "Missing authorized state in request extensions",
    ))?;

    let res = match authorized {
        AuthorizedState::Tenant(tenant) => tenant.actor_id,
        AuthorizedState::Organization { actor_id, .. } => *actor_id,
        AuthorizedState::User { user_id } => *user_id,
    };

    Ok(res)
}

pub fn extract_tenant(maybe_auth: Option<&AuthorizedState>) -> Result<TenantId, Status> {
    let authorized = maybe_auth.ok_or(Status::unauthenticated(
        "Missing authorized state in request extensions",
    ))?;

    let res = match authorized {
        AuthorizedState::Tenant(tenant) => { Ok(tenant.tenant_id) }
        AuthorizedState::Organization { .. }  => { Err(Status::invalid_argument("Tenant is absent from the authorized state. This indicates an incomplete x-md-context header.")) }
        AuthorizedState::User { .. }  => { Err(Status::invalid_argument("Tenant is absent from the authorized state. This indicates a missing x-md-context header.")) }
    }?;
    Ok(res)
}

pub fn extract_organization(
    maybe_auth: Option<&AuthorizedState>,
) -> Result<OrganizationId, Status> {
    let authorized = maybe_auth.ok_or(Status::unauthenticated(
        "Missing authorized state in request extensions",
    ))?;

    let res = match authorized {
        AuthorizedState::Tenant(tenant) => { Ok(tenant.organization_id) }
        AuthorizedState::Organization { organization_id, .. } => { Ok(*organization_id) }
        AuthorizedState::User { .. } => { Err(Status::invalid_argument("Organization is absent from the authorized state. This indicates a missing x-md-context header.")) }
    }?;
    Ok(res)
}

pub enum AuthenticatedState {
    ApiKey {
        id: Uuid,
        tenant_id: TenantId,
        organization_id: OrganizationId,
    },
    User {
        id: Uuid,
    },
}

#[derive(Clone)]
pub struct AuthorizedAsTenant {
    pub actor_id: Uuid,
    pub tenant_id: TenantId,
    pub organization_id: OrganizationId,
}

#[derive(Clone)]
pub enum AuthorizedState {
    Tenant(AuthorizedAsTenant),
    Organization {
        actor_id: Uuid,
        organization_id: OrganizationId,
    },
    User {
        user_id: Uuid,
    },
}
