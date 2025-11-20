use http::HeaderMap;
use jsonwebtoken::DecodingKey;
use secrecy::{ExposeSecret, SecretString};
use tonic::Status;
use tracing::log;

use common_domain::ids::TenantId;
use common_grpc::GrpcServiceMethod;
use common_grpc::middleware::common::auth::PORTAL_KEY_HEADER;
use common_grpc::middleware::server::AuthorizedState;
use common_grpc::middleware::server::auth::{AuthenticatedState, AuthorizedAsPortalUser};
use meteroid_store::jwt_claims::{PortalJwtClaims, ResourceAccess};
use tap::tap::TapFallible;

pub fn validate_portal_jwt(
    header_map: &HeaderMap,
    jwt_secret: SecretString,
) -> Result<AuthenticatedState, Status> {
    let token = header_map
        .get(PORTAL_KEY_HEADER)
        .ok_or(Status::unauthenticated("Missing JWT"))?
        .to_str()
        .map_err(|_| Status::permission_denied("Invalid JWT1"))?;

    let decoding_key = DecodingKey::from_secret(jwt_secret.expose_secret().as_bytes());
    let mut validation = jsonwebtoken::Validation::default();
    validation.set_required_spec_claims(&Vec::<String>::new());
    let decoded = jsonwebtoken::decode::<PortalJwtClaims>(token, &decoding_key, &validation)
        .tap_err(|err| log::error!("Error decoding JWT: {err:?}"))
        .map_err(|_| Status::permission_denied("Invalid JWT2"))?
        .claims;

    // check expiry
    if let Some(exp) = decoded.exp
        && exp < chrono::Utc::now().timestamp() as usize
    {
        return Err(Status::permission_denied("JWT expired"));
    }

    Ok(AuthenticatedState::Shared {
        tenant_id: decoded.tenant_id,
        resource_access: match decoded.resource {
            ResourceAccess::SubscriptionCheckout(id) => {
                common_grpc::middleware::server::auth::ResourceAccess::SubscriptionCheckout(id)
            }
            ResourceAccess::Customer(id) => {
                common_grpc::middleware::server::auth::ResourceAccess::CustomerPortal(id)
            }
            ResourceAccess::Invoice(id) => {
                common_grpc::middleware::server::auth::ResourceAccess::InvoicePortal(id)
            }
            ResourceAccess::Quote {
                quote_id,
                recipient_email,
            } => common_grpc::middleware::server::auth::ResourceAccess::QuotePortal {
                quote_id,
                recipient_email,
            },
        },
    })
}

pub async fn authorize_portal(
    tenant_id: TenantId,
    resource_access: common_grpc::middleware::server::auth::ResourceAccess,
    gm: GrpcServiceMethod,
) -> Result<AuthorizedState, Status> {
    match resource_access {
        common_grpc::middleware::server::auth::ResourceAccess::QuotePortal { .. } => {
            if !gm.service.starts_with("meteroid.portal.quotes.") {
                return Err(Status::permission_denied("Unauthorized"));
            }
        }
        common_grpc::middleware::server::auth::ResourceAccess::InvoicePortal { .. } => {
            if !gm.service.starts_with("meteroid.portal.invoice.") && !gm.service.starts_with("meteroid.portal.shared.") {
                return Err(Status::permission_denied("Unauthorized"));
            }
        }
        common_grpc::middleware::server::auth::ResourceAccess::SubscriptionCheckout { .. } => {
            if !gm.service.starts_with("meteroid.portal.checkout.") && !gm.service.starts_with("meteroid.portal.shared.") {
                return Err(Status::permission_denied("Unauthorized"));
            }
        }
        _ =>   if !gm.service.starts_with("meteroid.portal.invoice.")
            && !gm.service.starts_with("meteroid.portal.checkout.")
            && !gm.service.starts_with("meteroid.portal.shared.")
            && !gm.service.starts_with("meteroid.portal.customer.") {
            return Err(Status::permission_denied("Unauthorized"));
        }
    }

    Ok(AuthorizedState::Shared(AuthorizedAsPortalUser {
        tenant_id,
        resource_access,
    }))
}
