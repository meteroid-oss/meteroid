use tonic::Status;
use uuid::Uuid;

pub use admin_layer::AdminAuthLayer;
pub use admin_layer::AdminAuthService;
use common_config::auth::InternalAuthConfig;
use common_domain::actor::Actor;
use common_domain::auth::OrgMemberRole;
use common_domain::ids::{
    ApiTokenId, BaseId, CheckoutSessionId, CustomerId, InvoiceId, OrganizationId, QuoteId,
    TenantId, UserId,
};

mod admin_layer;
pub mod api_token_validator;

pub fn create_admin(config: &InternalAuthConfig) -> AdminAuthLayer {
    AdminAuthLayer::new(config)
}

pub trait RequestExt {
    fn actor(&self) -> Result<Uuid, Status> {
        self.actor_typed()?
            .as_uuid()
            .ok_or_else(|| Status::unauthenticated("Invalid actor type"))
    }
    fn actor_user(&self) -> Result<UserId, Status> {
        match self.actor_typed()? {
            Actor::User { id } => Ok(id),
            _ => Err(Status::unauthenticated(
                "User actor is not available in this context.",
            )),
        }
    }
    fn actor_typed(&self) -> Result<Actor, Status>;
    fn tenant(&self) -> Result<TenantId, Status>;
    fn organization(&self) -> Result<OrganizationId, Status>;
    fn actor_role(&self) -> Result<OrgMemberRole, Status>;
    fn require_admin(&self) -> Result<(), Status>;
    fn portal_resource(&self) -> Result<AuthorizedAsPortalUser, Status>;
}

impl<T> RequestExt for tonic::Request<T> {
    fn actor_typed(&self) -> Result<Actor, Status> {
        extract_actor_typed(self.extensions().get::<AuthorizedState>())
    }

    fn tenant(&self) -> Result<TenantId, Status> {
        extract_tenant(self.extensions().get::<AuthorizedState>())
    }

    fn organization(&self) -> Result<OrganizationId, Status> {
        extract_organization(self.extensions().get::<AuthorizedState>())
    }

    fn actor_role(&self) -> Result<OrgMemberRole, Status> {
        extract_actor_role(self.extensions().get::<AuthorizedState>())
    }

    fn require_admin(&self) -> Result<(), Status> {
        require_admin_role(self.extensions().get::<AuthorizedState>())
    }

    fn portal_resource(&self) -> Result<AuthorizedAsPortalUser, Status> {
        extract_portal(self.extensions().get::<AuthorizedState>())
    }
}

impl<T> RequestExt for http::Request<T> {
    fn actor_typed(&self) -> Result<Actor, Status> {
        extract_actor_typed(self.extensions().get::<AuthorizedState>())
    }

    fn tenant(&self) -> Result<TenantId, Status> {
        extract_tenant(self.extensions().get::<AuthorizedState>())
    }

    fn organization(&self) -> Result<OrganizationId, Status> {
        extract_organization(self.extensions().get::<AuthorizedState>())
    }

    fn actor_role(&self) -> Result<OrgMemberRole, Status> {
        extract_actor_role(self.extensions().get::<AuthorizedState>())
    }

    fn require_admin(&self) -> Result<(), Status> {
        require_admin_role(self.extensions().get::<AuthorizedState>())
    }

    fn portal_resource(&self) -> Result<AuthorizedAsPortalUser, Status> {
        extract_portal(self.extensions().get::<AuthorizedState>())
    }
}

pub fn extract_actor_typed(maybe_auth: Option<&AuthorizedState>) -> Result<Actor, Status> {
    let authorized = maybe_auth.ok_or(Status::unauthenticated(
        "Missing authorized state in request extensions",
    ))?;

    match authorized {
        AuthorizedState::Tenant(t) => Ok(t.as_actor()),
        // API tokens are tenant-scoped, so org-/user-level auth is always a user.
        AuthorizedState::Organization {
            user_id: actor_id, ..
        } => Ok(Actor::User { id: *actor_id }),
        AuthorizedState::User { user_id } => Ok(Actor::User { id: *user_id }),
        AuthorizedState::Shared(state) => match &state.resource_access {
            ResourceAccess::CustomerPortal(id) => Ok(Actor::Customer { id: *id }),
            ResourceAccess::QuotePortal {
                recipient_email, ..
            } => Ok(Actor::QuoteRecipient {
                email: recipient_email.clone(),
            }),
            ResourceAccess::CheckoutSession(_) | ResourceAccess::InvoicePortal(_) => Err(
                Status::invalid_argument("Actor is not available for this portal session."),
            ),
        },
    }
}

pub fn extract_actor_role(maybe_auth: Option<&AuthorizedState>) -> Result<OrgMemberRole, Status> {
    let authorized = maybe_auth.ok_or(Status::unauthenticated(
        "Missing authorized state in request extensions",
    ))?;

    match authorized {
        AuthorizedState::Organization { role, .. } => Ok(*role),
        AuthorizedState::Tenant(t) => t
            .actor
            .role()
            .ok_or_else(|| Status::permission_denied("Role is not available in this context.")),
        _ => Err(Status::permission_denied(
            "Role is not available in this context.",
        )),
    }
}

pub fn require_admin_role(maybe_auth: Option<&AuthorizedState>) -> Result<(), Status> {
    if extract_actor_role(maybe_auth)? != OrgMemberRole::Admin {
        return Err(Status::permission_denied(
            "Only organization admins can perform this action.",
        ));
    }
    Ok(())
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
        id: ApiTokenId,
        tenant_id: TenantId,
        organization_id: OrganizationId,
        tenant_env: TenantEnv,
    },
    User {
        id: UserId,
    },
    Shared {
        tenant_id: TenantId,
        resource_access: ResourceAccess,
    },
}

#[derive(Clone, Copy)]
pub enum TenantActor {
    ApiKey(ApiTokenId),
    User { id: UserId, role: OrgMemberRole },
}

impl TenantActor {
    pub fn id(&self) -> Uuid {
        match self {
            TenantActor::ApiKey(id) => id.as_uuid(),
            TenantActor::User { id, .. } => id.as_uuid(),
        }
    }

    pub fn role(&self) -> Option<OrgMemberRole> {
        match self {
            TenantActor::ApiKey(_) => None,
            TenantActor::User { role, .. } => Some(*role),
        }
    }
}

#[derive(Clone)]
pub struct AuthorizedAsTenant {
    pub actor: TenantActor,
    pub tenant_id: TenantId,
    pub organization_id: OrganizationId,
    pub tenant_env: TenantEnv,
}

impl AuthorizedAsTenant {
    pub fn as_actor(&self) -> Actor {
        match self.actor {
            TenantActor::User { id, .. } => Actor::User { id },
            TenantActor::ApiKey(id) => Actor::ApiToken { id },
        }
    }
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
        user_id: UserId,
        organization_id: OrganizationId,
        role: OrgMemberRole,
    },
    User {
        user_id: UserId,
    },
    Shared(AuthorizedAsPortalUser),
}

#[derive(Clone)]
pub enum TenantEnv {
    Production,
    NonProduction,
}

impl TenantEnv {
    pub fn is_production(&self) -> bool {
        matches!(self, TenantEnv::Production)
    }
}
