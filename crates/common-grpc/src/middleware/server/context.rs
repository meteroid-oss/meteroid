use std::task::{Context, Poll};

use common_domain::ids::{BaseId, CustomerId, InvoiceId, OrganizationId, SubscriptionId, TenantId};
use http::Request;
use tower::{Layer, Service};
use tracing::Instrument;
use tracing::instrument::Instrumented;

use super::auth::{AuthorizedAsPortalUser, AuthorizedAsTenant, AuthorizedState, ResourceAccess};

/// Correlation ids of an authorized request, recorded on a `request_context` span so the
/// request's logs and trace can be filtered by them (see `common_logging::context`).
#[derive(Clone, Copy, Debug, Default)]
pub struct RequestContext {
    pub tenant_id: Option<TenantId>,
    pub organization_id: Option<OrganizationId>,
    pub customer_id: Option<CustomerId>,
    pub subscription_id: Option<SubscriptionId>,
    pub invoice_id: Option<InvoiceId>,
}

impl RequestContext {
    pub fn of_tenant(tenant: &AuthorizedAsTenant) -> Self {
        Self {
            tenant_id: Some(tenant.tenant_id),
            organization_id: Some(tenant.organization_id),
            ..Self::default()
        }
    }

    pub fn of_portal(portal: &AuthorizedAsPortalUser) -> Self {
        let (customer_id, invoice_id) = match portal.resource_access {
            ResourceAccess::CustomerPortal(id) => (Some(id), None),
            ResourceAccess::InvoicePortal(id) => (None, Some(id)),
            ResourceAccess::CheckoutSession(_)
            | ResourceAccess::QuotePortal { .. }
            | ResourceAccess::DashboardPortal(_) => (None, None),
        };
        Self {
            tenant_id: Some(portal.tenant_id),
            customer_id,
            invoice_id,
            ..Self::default()
        }
    }

    pub fn of_state(state: &AuthorizedState) -> Option<Self> {
        match state {
            AuthorizedState::Tenant(tenant) => Some(Self::of_tenant(tenant)),
            AuthorizedState::Organization {
                organization_id, ..
            } => Some(Self {
                organization_id: Some(*organization_id),
                ..Self::default()
            }),
            AuthorizedState::Shared(portal) => Some(Self::of_portal(portal)),
            AuthorizedState::User { .. } => None,
        }
    }

    /// Fills the ids still unknown from the segments of a REST path, which carry them in their
    /// prefixed public form (`/subscriptions/sub_...`). Ids from the credentials are kept.
    pub fn with_ids_from_path(mut self, path: &str) -> Self {
        for segment in path.split('/') {
            if self.customer_id.is_none() {
                self.customer_id = CustomerId::parse_base62(segment).ok();
            }
            if self.subscription_id.is_none() {
                self.subscription_id = SubscriptionId::parse_base62(segment).ok();
            }
            if self.invoice_id.is_none() {
                self.invoice_id = InvoiceId::parse_base62(segment).ok();
            }
        }
        self
    }

    pub fn span(&self) -> tracing::Span {
        tracing::info_span!(
            "request_context",
            tenant_id = self.tenant_id.map(tracing::field::display),
            organization_id = self.organization_id.map(tracing::field::display),
            customer_id = self.customer_id.map(tracing::field::display),
            subscription_id = self.subscription_id.map(tracing::field::display),
            invoice_id = self.invoice_id.map(tracing::field::display),
        )
    }
}

/// Runs each authorized gRPC call inside its [`RequestContext`] span. Must sit inside the auth
/// layer, which puts the `AuthorizedState` in the request extensions, and inside the OTel layer,
/// so the span joins the request's trace.
#[derive(Clone, Copy, Debug, Default)]
pub struct RequestContextLayer;

impl<S> Layer<S> for RequestContextLayer {
    type Service = RequestContextService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestContextService { inner }
    }
}

#[derive(Clone, Debug)]
pub struct RequestContextService<S> {
    inner: S,
}

impl<S, ReqBody> Service<Request<ReqBody>> for RequestContextService<S>
where
    S: Service<Request<ReqBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Instrumented<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        let span = request
            .extensions()
            .get::<AuthorizedState>()
            .and_then(RequestContext::of_state)
            .map_or_else(tracing::Span::none, |ctx| ctx.span());
        self.inner.call(request).instrument(span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_ids_fill_the_unknown_ids_only() {
        let customer = CustomerId::new();
        let subscription = SubscriptionId::new();
        let from_token = CustomerId::new();

        let ctx = RequestContext::default().with_ids_from_path(&format!(
            "/api/v1/customers/{customer}/subscriptions/{subscription}"
        ));
        assert_eq!(ctx.customer_id, Some(customer));
        assert_eq!(ctx.subscription_id, Some(subscription));
        assert_eq!(ctx.invoice_id, None);

        let ctx = RequestContext {
            customer_id: Some(from_token),
            ..RequestContext::default()
        }
        .with_ids_from_path(&format!("/api/v1/customers/{customer}"));
        assert_eq!(ctx.customer_id, Some(from_token));

        let ctx = RequestContext::default().with_ids_from_path("/api/v1/customers/my-alias");
        assert_eq!(ctx.customer_id, None);
    }
}
