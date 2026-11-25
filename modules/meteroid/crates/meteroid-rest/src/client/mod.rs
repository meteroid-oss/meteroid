use crate::AppState;
use crate::openapi::{
    ClientApiDoc, add_problem_rate_limit_responses, apply_sorted_tags,
    discriminate_tagged_variants, label_beta_operations,
};
use axum::response::Response;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod auth;
mod mapping;
pub mod model;
mod public;
mod router;

pub use auth::client_auth_middleware;
pub use meteroid_rest_common::client::{AuthorizedAsCustomer, AuthorizedAsPublishable};

/// Routes under this prefix authenticate with a customer portal token or, on the public routes,
/// a publishable key; never a secret API key.
pub const CLIENT_API_PREFIX: &str = "/api/client/";

/// The public routes: the tenant's public data (its pricing pages), read with a publishable key
/// and only with one. Every other client route is the customer's own, read with its token.
const PUBLIC_ROUTE_PREFIXES: &[&str] = &["/api/client/v1/pricing-pages/"];

/// A public route: a `GET` of one resource under a public prefix. Anything else is a customer
/// route, which a publishable key never reaches.
pub fn is_public_route(method: &http::Method, path: &str) -> bool {
    *method == http::Method::GET
        && PUBLIC_ROUTE_PREFIXES.iter().any(|prefix| {
            path.strip_prefix(prefix)
                .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
        })
}

/// Marks the refusal of a browser origin a publishable key does not list.
#[derive(Clone, Copy, Debug)]
pub struct OriginRefused;

/// Outside the CORS layer, which reflects any origin: a refused origin's answer leaves without
/// the headers that would let its page read it.
pub async fn strip_refused_cors(mut response: Response) -> Response {
    if response.extensions().get::<OriginRefused>().is_some() {
        let headers = response.headers_mut();
        headers.remove(http::header::ACCESS_CONTROL_ALLOW_ORIGIN);
        headers.remove(http::header::ACCESS_CONTROL_ALLOW_CREDENTIALS);
        headers.remove(http::header::ACCESS_CONTROL_EXPOSE_HEADERS);
    }
    response
}

/// The browser-facing API, with its own OpenAPI document, which the edition's modules' client
/// operations (`module_doc`) join; an edition that names its own license publishes under it.
pub fn client_api(
    module_doc: utoipa::openapi::OpenApi,
) -> (axum::Router<AppState>, utoipa::openapi::OpenApi) {
    let (router, mut open_api) = OpenApiRouter::with_openapi(ClientApiDoc::openapi())
        .routes(routes!(router::get_customer))
        .routes(routes!(router::list_entitlements))
        .routes(routes!(router::list_subscriptions))
        .routes(routes!(public::get_pricing_page))
        .split_for_parts();
    if let Some(license) = module_doc.info.license.clone() {
        open_api.info.license = Some(license);
    }
    open_api.merge(module_doc);
    meteroid_rest_common::spec::mark_operations(
        &mut open_api,
        meteroid_rest_common::spec::OperationEdition::Oss,
    );
    add_problem_rate_limit_responses(&mut open_api);
    discriminate_tagged_variants(&mut open_api);
    apply_sorted_tags(&mut open_api);
    label_beta_operations(&mut open_api);
    (router, open_api)
}

#[cfg(test)]
mod tests {
    use super::is_public_route;
    use http::Method;

    #[test]
    fn only_a_get_of_one_public_resource_is_public() {
        assert!(is_public_route(
            &Method::GET,
            "/api/client/v1/pricing-pages/cloud"
        ));
        for (method, path) in [
            (Method::POST, "/api/client/v1/pricing-pages/cloud"),
            (Method::GET, "/api/client/v1/pricing-pages/"),
            (Method::GET, "/api/client/v1/pricing-pages/cloud/x"),
            (Method::GET, "/api/client/v1/pricing-pages"),
            (Method::GET, "/api/client/v1/customer"),
            (Method::GET, "/api/client/v1/entitlements"),
            (Method::GET, "/api/client/v1/subscriptions"),
            (Method::GET, "/api/client/v1/entitlements/sso/check"),
            (Method::GET, "/api/v2/pricing-pages/cloud"),
            (Method::GET, "/api/client/v2/pricing-pages/cloud"),
        ] {
            assert!(!is_public_route(&method, path), "{method} {path}");
        }
    }
}
