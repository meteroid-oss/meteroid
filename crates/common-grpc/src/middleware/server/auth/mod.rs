use tonic::Status;
use uuid::Uuid;

pub use admin_layer::AdminAuthLayer;
pub use admin_layer::AdminAuthService;
use common_config::auth::InternalAuthConfig;

mod admin_layer;
pub mod api_token_validator;

pub fn create_admin(config: &InternalAuthConfig) -> AdminAuthLayer {
    AdminAuthLayer::new(config)
}

pub trait RequestExt {
    fn actor(&self) -> Result<Uuid, Status>;
    fn tenant(&self) -> Result<Uuid, Status>;
}

impl<T> RequestExt for tonic::Request<T> {
    fn actor(&self) -> Result<Uuid, Status> {
        extract_actor(self.extensions().get::<AuthorizedState>())
    }

    fn tenant(&self) -> Result<Uuid, Status> {
        extract_tenant(self.extensions().get::<AuthorizedState>())
    }
}

impl<T> RequestExt for http::Request<T> {
    fn actor(&self) -> Result<Uuid, Status> {
        extract_actor(self.extensions().get::<AuthorizedState>())
    }

    fn tenant(&self) -> Result<Uuid, Status> {
        extract_tenant(self.extensions().get::<AuthorizedState>())
    }
}

pub fn extract_actor(maybe_auth: Option<&AuthorizedState>) -> Result<Uuid, Status> {
    let authorized = maybe_auth.ok_or(Status::unauthenticated(
        "Missing authorized state in request extensions",
    ))?;

    let res = match authorized {
        AuthorizedState::Tenant { actor_id, .. } => *actor_id,
        AuthorizedState::User { user_id } => *user_id,
    };

    Ok(res)
}

pub fn extract_tenant(maybe_auth: Option<&AuthorizedState>) -> Result<Uuid, Status> {
    let authorized = maybe_auth.ok_or(Status::unauthenticated(
        "Missing authorized state in request extensions",
    ))?;

    let res = match authorized {
        AuthorizedState::Tenant { tenant_id, .. } => { Ok(*tenant_id) }
        AuthorizedState::User { .. } => { Err(Status::invalid_argument("Tenant is absent from the authorized state. This indicates a missing x-tenant-id header.")) }
    }?;
    Ok(res)
}

pub enum AuthenticatedState {
    ApiKey { id: Uuid, tenant_id: Uuid },
    User { id: Uuid },
}

pub enum AuthorizedState {
    Tenant { actor_id: Uuid, tenant_id: Uuid },
    User { user_id: Uuid },
}
