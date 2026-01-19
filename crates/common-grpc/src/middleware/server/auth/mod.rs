use tonic::Status;
use uuid::Uuid;

pub use admin_layer::AdminAuthLayer;
pub use admin_layer::AdminAuthService;
use common_config::auth::InternalAuthConfig;
use common_domain::ids::{
    CheckoutSessionId, CustomerId, InvoiceId, OrganizationId, QuoteId, TenantId,
};

mod admin_layer;
pub mod api_token_validator;

pub fn create_admin(config: &InternalAuthConfig) -> AdminAuthLayer {
    AdminAuthLayer::new(config)
}

pub trait RequestExt {
    fn actor(&self) -> Result<Uuid, Status>;
    fn tenant(&self) -> Result<TenantId, Status>;
    fn organization(&self) -> Result<OrganizationId, Status>;
    fn portal_resource(&self) -> Result<AuthorizedAsPortalUser, Status>;
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

    fn portal_resource(&self) -> Result<AuthorizedAsPortalUser, Status> {
        extract_portal(self.extensions().get::<AuthorizedState>())
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

    fn portal_resource(&self) -> Result<AuthorizedAsPortalUser, Status> {
        extract_portal(self.extensions().get::<AuthorizedState>())
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
        AuthorizedState::Shared { .. } => {
            return Err(Status::invalid_argument(
                "Actor is not available for portal events.",
            ));
        }
    };

    Ok(res)
}

pub fn extract_portal(
    maybe_auth: Option<&AuthorizedState>,
) -> Result<AuthorizedAsPortalUser, Status> {
    let authorized = maybe_auth.ok_or(Status::unauthenticated(
        "Missing authorized state in request extensions",
    ))?;

    let res = match authorized {
        AuthorizedState::Shared(state) => Ok(state.clone()),
        _ => Err(Status::invalid_argument(
            "Portal state is only available in portal apis.",
        )),
    }?;
    Ok(res)
}

pub fn extract_tenant(maybe_auth: Option<&AuthorizedState>) -> Result<TenantId, Status> {
    let authorized = maybe_auth.ok_or(Status::unauthenticated(
        "Missing authorized state in request extensions",
    ))?;

    let res = match authorized {
        AuthorizedState::Tenant(tenant) => Ok(tenant.tenant_id),
        AuthorizedState::Organization { .. } => Err(Status::invalid_argument(
            "Tenant is absent from the authorized state. This indicates an incomplete x-md-context header.",
        )),
        AuthorizedState::User { .. } => Err(Status::invalid_argument(
            "Tenant is absent from the authorized state. This indicates a missing x-md-context header.",
        )),
        AuthorizedState::Shared(state) => Ok(state.tenant_id),
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
        AuthorizedState::Tenant(tenant) => Ok(tenant.organization_id),
        AuthorizedState::Organization {
            organization_id, ..
        } => Ok(*organization_id),
        AuthorizedState::User { .. } => Err(Status::invalid_argument(
            "Organization is absent from the authorized state. This indicates a missing x-md-context header.",
        )),
        AuthorizedState::Shared(_) => Err(Status::invalid_argument(
            "Organization is not available in authorized state for portal apis.",
        )),
    }?;
    Ok(res)
}

#[derive(Clone)]
pub enum ResourceAccess {
    CheckoutSession(CheckoutSessionId),
    CustomerPortal(CustomerId),
    InvoicePortal(InvoiceId),
    QuotePortal {
        quote_id: QuoteId,
        recipient_email: String,
    },
}
#[derive(Clone)]
pub enum AuthenticatedState {
    ApiKey {
        id: Uuid,
        tenant_id: TenantId,
        organization_id: OrganizationId,
    },
    User {
        id: Uuid,
    },
    Shared {
        tenant_id: TenantId,
        resource_access: ResourceAccess,
    },
}

#[derive(Clone)]
pub struct AuthorizedAsTenant {
    pub actor_id: Uuid,
    pub tenant_id: TenantId,
    pub organization_id: OrganizationId,
}

#[derive(Clone)]
pub struct AuthorizedAsPortalUser {
    pub tenant_id: TenantId,
    pub resource_access: ResourceAccess,
}

impl AuthorizedAsPortalUser {
    pub fn checkout_session(&self) -> Result<CheckoutSessionId, Status> {
        match self.resource_access {
            ResourceAccess::CheckoutSession(id) => Ok(id),
            _ => Err(Status::invalid_argument(
                "Resource is not a checkout session.",
            )),
        }
    }

    pub fn customer(&self) -> Result<CustomerId, Status> {
        match self.resource_access {
            ResourceAccess::CustomerPortal(id) => Ok(id),
            _ => Err(Status::invalid_argument(
                "Resource is not a customer portal.",
            )),
        }
    }

    pub fn invoice(&self) -> Result<InvoiceId, Status> {
        match self.resource_access {
            ResourceAccess::InvoicePortal(id) => Ok(id),
            _ => Err(Status::invalid_argument(
                "Resource is not an invoice portal.",
            )),
        }
    }

    pub fn quote(&self) -> Result<QuoteId, Status> {
        match &self.resource_access {
            ResourceAccess::QuotePortal { quote_id, .. } => Ok(*quote_id),
            _ => Err(Status::invalid_argument("Resource is not a quote portal.")),
        }
    }

    pub fn quote_recipient_email(&self) -> Result<String, Status> {
        match &self.resource_access {
            ResourceAccess::QuotePortal {
                recipient_email, ..
            } => Ok(recipient_email.clone()),
            _ => Err(Status::invalid_argument("Resource is not a quote portal.")),
        }
    }
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
    Shared(AuthorizedAsPortalUser),
}
