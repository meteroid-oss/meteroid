use hyper::{Request, Response};
use std::sync::Arc;

use secrecy::SecretString;

use crate::server::auth::SharedSupportGrantReader;
use crate::server::auth::rbac::RoutePolicy;
use crate::server::auth::strategies::api_key_strategy::validate_api_key;
use crate::server::auth::strategies::jwt_strategy::{
    authorize_user, is_user_active_cached, validate_jwt,
};
use crate::server::auth::strategies::portal_jwt_strategy::{authorize_portal, validate_portal_jwt};
use cached::proc_macro::cached;
use common_domain::ids::{OrganizationId, UserId};
use common_domain::rbac::ResolvedPermissions;
use common_grpc::GrpcServiceMethod;
use common_grpc::middleware::common::auth::{
    API_KEY_HEADER, BEARER_AUTH_HEADER, PORTAL_KEY_HEADER,
};
use common_grpc::middleware::common::filters::Filter;
use common_grpc::middleware::server::AuthorizedState;
use common_grpc::middleware::server::auth::{
    AuthenticatedState, AuthorizedAsTenant, DelegatedCredentials, TenantActor,
};
use futures_util::TryFutureExt;
use futures_util::future::BoxFuture;
use http::StatusCode;
use meteroid_store::Store;
use meteroid_store::repositories::OrganizationsInterface;
use std::task::{Context, Poll};
use tonic::Status;
use tonic::body::Body;
use tower::{BoxError, Service};
use tower_layer::Layer;
use tracing::log;

/// Services that express organizations (managed accounts created by a platform) are allowed to access.
/// Secure by default: new services are blocked for express unless explicitly added here.
const EXPRESS_ALLOWED_SERVICES: &[&str] = &[
    "InstanceService",
    "UsersService",
    "OrganizationsService",
    "TenantsService",
    "InvoicingEntitiesService",
    "InvoiceConsolidationService",
    "BankAccountsService",
    "TaxesService",
    "ConnectorsService",
    "InvoicesService",
    "CustomersService",
    "StatsService",
    // Subscriptions and Invoices list pages render metric strips (ListMetrics + Explore) for
    // express orgs too; the Intelligence screens themselves are hidden from express.
    "AnalyticsService",
    "ConnectClientService",
    "PlansService",
    "SubscriptionsService",
    "BillableMetricsService",
    "CustomerEmailsService",
    "PaymentRemindersService",
    // A catalog primitive a plan's prices can name, same footing as BillableMetricsService: an
    // express org must be able to define the units it prices in, not just the plans that use them.
    "CreditUnitsService",
];

fn is_express_allowed_service(sm: &GrpcServiceMethod) -> bool {
    EXPRESS_ALLOWED_SERVICES
        .iter()
        .any(|s| sm.service.ends_with(s))
}

#[cached(
    result = true,
    size = 100,
    time = 300,
    key = "OrganizationId",
    convert = r#"{ org_id }"#
)]
async fn is_organization_express_cached(
    store: &Store,
    org_id: OrganizationId,
) -> Result<bool, BoxError> {
    let org = store.get_organization_by_id(org_id).await.map_err(|e| {
        BoxError::from(Status::internal(format!(
            "Failed to get organization: {}",
            e
        )))
    })?;
    Ok(org.is_express)
}

async fn ensure_user_active(store: &Store, user_id: UserId) -> Result<(), Status> {
    if is_user_active_cached(store, user_id).await? {
        Ok(())
    } else {
        Err(Status::unauthenticated("Account is deleted"))
    }
}

#[derive(Clone)]
pub struct ApiAuthMiddleware<S> {
    inner: S,
    filter: Option<Filter>,
    jwt_secret: SecretString,
    store: Store,
    support: Option<SharedSupportGrantReader>,
    module_policies: Arc<[RoutePolicy]>,
}

#[derive(Clone)]
pub struct ApiAuthLayer {
    jwt_secret: SecretString,
    store: Store,
    filter: Option<Filter>,
    support: Option<SharedSupportGrantReader>,
    module_policies: Arc<[RoutePolicy]>,
}

impl ApiAuthLayer {
    #[allow(clippy::new_without_default)]
    pub fn new(
        jwt_secret: SecretString,
        store: Store,
        support: Option<SharedSupportGrantReader>,
    ) -> Self {
        ApiAuthLayer {
            jwt_secret,
            filter: None,
            store,
            support,
            module_policies: Arc::new([]),
        }
    }

    /// The access the edition's modules require on the services whose protos they own.
    #[must_use]
    pub fn module_policies(mut self, policies: Vec<RoutePolicy>) -> Self {
        self.module_policies = policies.into();
        self
    }

    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filter = Some(filter);
        self
    }
}

impl<S> Layer<S> for ApiAuthLayer {
    type Service = ApiAuthMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ApiAuthMiddleware {
            inner,
            filter: self.filter,
            store: self.store.clone(),
            jwt_secret: self.jwt_secret.clone(),
            support: self.support.clone(),
            module_policies: self.module_policies.clone(),
        }
    }
}

// services that don't require authentication
const ANONYMOUS_SERVICES: [&str; 12] = [
    "/meteroid.api.instance.v1.InstanceService/GetInstance",
    "/meteroid.api.instance.v1.InstanceService/GetInviteDetails",
    "/meteroid.api.instance.v1.InstanceService/GetCountries",
    "/meteroid.api.users.v1.UsersService/InitRegistration",
    "/meteroid.api.users.v1.UsersService/CompleteRegistration",
    "/meteroid.api.users.v1.UsersService/InitResetPassword",
    "/meteroid.api.users.v1.UsersService/ResetPassword",
    "/meteroid.api.users.v1.UsersService/Login",
    "/meteroid.api.users.v1.UsersService/VerifyMfa",
    "/meteroid.api.connect.v1.ConnectClientService/GetOnboardingInfo",
    // Purchase links: callers hold a link token only. Rate-limited per client IP.
    "/meteroid.portal.purchase.v1.PurchaseService/GetPurchaseOffer",
    "/meteroid.portal.purchase.v1.PurchaseService/StartPurchase",
];

// services require authentication but no authorization (no organization/tenant)
// MFA enrollment/management is user-global and must work before the user holds
// an active tenant context (e.g. an invited user gated on forced enrollment).
// Preferences are keyed by user id alone, so they belong here for the same reason.
const UNAUTHORIZED_SERVICES: [&str; 14] = [
    "/meteroid.api.organizations.v1.OrganizationsService/ListOrganizations",
    "/meteroid.api.organizations.v1.OrganizationsService/CreateOrganization",
    "/meteroid.api.users.v1.UsersService/Me",
    "/meteroid.api.users.v1.UsersService/AcceptInvite",
    "/meteroid.api.users.v1.UsersService/OnboardMe",
    "/meteroid.api.users.v1.UsersService/UpdateUserPreferences",
    "/meteroid.api.users.v1.UsersService/InitMfaEnrollment",
    "/meteroid.api.users.v1.UsersService/ConfirmMfaEnrollment",
    "/meteroid.api.users.v1.UsersService/DisableMfa",
    "/meteroid.api.users.v1.UsersService/RegenerateMfaRecoveryCodes",
    "/meteroid.api.connect.v1.ConnectClientService/CompleteOnboarding",
    "/meteroid.api.connect.v1.ConnectClientService/GetAuthorizationRequest",
    "/meteroid.api.connect.v1.ConnectClientService/ApproveAuthorization",
    "/meteroid.api.connect.v1.ConnectClientService/DenyAuthorization",
];

impl<S, ReqBody> Service<Request<ReqBody>> for ApiAuthMiddleware<S>
where
    S: Service<Request<ReqBody>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Into<BoxError>,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, mut request: Request<ReqBody>) -> Self::Future {
        if !self.filter.is_none_or(|f| f(request.uri().path())) {
            let mut inner = self.inner.clone();
            return Box::pin(async move { inner.call(request).await.map_err(Into::into) });
        }
        if ANONYMOUS_SERVICES.contains(&request.uri().path()) {
            let mut inner = self.inner.clone();
            return Box::pin(async move { inner.call(request).await.map_err(Into::into) });
        }

        // This is necessary because tonic internally uses `tower::buffer::Buffer`.
        // See https://github.com/tower-rs/tower/issues/547#issuecomment-767629149
        // for details on why this is necessary
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        let sm = GrpcServiceMethod::extract(request.uri());

        let metadata = request.headers().clone();

        let store = self.store.clone();
        let jwt_secret = self.jwt_secret.clone();
        let support = self.support.clone();
        let module_policies = self.module_policies.clone();

        let future = async move {
            let authenticated_state = if metadata.contains_key(API_KEY_HEADER) {
                validate_api_key(&metadata, &store, &sm)
                    .await
                    .map_err(|e| BoxError::from(e) as BoxError)
            } else if metadata.contains_key(PORTAL_KEY_HEADER) {
                validate_portal_jwt(&metadata, jwt_secret)
                    .map_err(|e| BoxError::from(e) as BoxError)
            } else if metadata.contains_key(BEARER_AUTH_HEADER) {
                validate_jwt(&metadata, jwt_secret).map_err(|e| BoxError::from(e) as BoxError)
            } else {
                Err(Box::new(Status::unauthenticated("No authentication provided")) as BoxError)
            }?;

            let sm_for_express = sm.clone();
            let mut permissions = None;
            let authorized_state = match authenticated_state {
                AuthenticatedState::ApiKey {
                    tenant_id,
                    id,
                    organization_id,
                    tenant_env,
                } => Ok(AuthorizedState::Tenant(AuthorizedAsTenant {
                    tenant_id,
                    organization_id,
                    actor: TenantActor::ApiKey(id),
                    tenant_env,
                })),
                AuthenticatedState::User { id } => {
                    ensure_user_active(&store, id)
                        .await
                        .map_err(|e| BoxError::from(e) as BoxError)?;
                    if UNAUTHORIZED_SERVICES.contains(&request.uri().path()) {
                        Ok(AuthorizedState::User { user_id: id })
                    } else {
                        authorize_user(
                            &metadata,
                            id,
                            &store,
                            support.as_ref(),
                            sm,
                            &module_policies,
                        )
                        .await
                        .map(|(state, resolved)| {
                            permissions = Some(resolved);
                            state
                        })
                        .map_err(|e| BoxError::from(e) as BoxError)
                    }
                }
                AuthenticatedState::Shared {
                    resource_access,
                    tenant_id,
                    scopes,
                    delegated,
                } => authorize_portal(tenant_id, resource_access, scopes, delegated, sm)
                    .map_err(|e| BoxError::from(e) as BoxError),
            }?;

            // Check express organization restrictions.
            // Only restrict direct access (Tenant/Organization), not portal (Shared).
            let express_check_org_id = match &authorized_state {
                AuthorizedState::Tenant(tenant) => Some(tenant.organization_id),
                AuthorizedState::Organization {
                    organization_id, ..
                } => Some(*organization_id),
                _ => None,
            };
            if let Some(org_id) = express_check_org_id
                && is_organization_express_cached(&store, org_id).await?
            {
                if !is_express_allowed_service(&sm_for_express) {
                    return Err(BoxError::from(Status::permission_denied(
                        "This operation is not available for express organizations",
                    )));
                }
                request.extensions_mut().insert(DelegatedCredentials);
            }

            request.extensions_mut().insert(authorized_state);
            if let Some(permissions) = permissions {
                request
                    .extensions_mut()
                    .insert(ResolvedPermissions(permissions));
            }

            inner.call(request).await.map_err(Into::into)
        };

        let future = future.or_else(|e: BoxError| async move {
            log::warn!("Error in auth middleware: {e:?}");
            let status_code = e
                .downcast_ref::<Status>()
                .map(|s| match s.code() {
                    tonic::Code::PermissionDenied => StatusCode::FORBIDDEN,
                    tonic::Code::Internal => StatusCode::INTERNAL_SERVER_ERROR,
                    tonic::Code::NotFound => StatusCode::NOT_FOUND,
                    _ => StatusCode::UNAUTHORIZED,
                })
                .unwrap_or(StatusCode::UNAUTHORIZED);
            let response = Response::builder()
                .status(status_code)
                .body(Body::empty())
                .unwrap();
            Ok(response)
        });

        Box::pin(future)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gm(service: &str) -> GrpcServiceMethod {
        GrpcServiceMethod {
            service: service.to_string(),
            method: "Any".to_string(),
        }
    }

    /// Catalog primitives a plan's prices can name must be express-allowed together: an express
    /// org that can manage plans (`PlansService`) but not the units they price in would be unable
    /// to finish setting up a credit-priced plan at all.
    #[test]
    fn credit_units_service_is_express_allowed_like_billable_metrics() {
        assert!(is_express_allowed_service(&gm(
            "meteroid.api.creditunits.v1.CreditUnitsService"
        )));
        assert!(is_express_allowed_service(&gm(
            "meteroid.api.billablemetrics.v1.BillableMetricsService"
        )));
    }
}
