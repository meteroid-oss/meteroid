use crate::AppState;
use crate::openapi::{
    ClientApiDoc, add_rate_limit_responses, apply_sorted_tags, discriminate_tagged_variants,
    label_beta_operations,
};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod auth;
mod mapping;
pub mod model;
mod router;

pub use auth::client_auth_middleware;
pub use meteroid_rest_common::client::AuthorizedAsCustomer;

/// Routes under this prefix authenticate with a customer portal token, never an API key.
pub const CLIENT_API_PREFIX: &str = "/api/client/";

/// The browser-facing API, with its own OpenAPI document, which the edition's modules' client
/// operations (`module_doc`) join; an edition that names its own license publishes under it.
pub fn client_api(
    module_doc: utoipa::openapi::OpenApi,
) -> (axum::Router<AppState>, utoipa::openapi::OpenApi) {
    let (router, mut open_api) = OpenApiRouter::with_openapi(ClientApiDoc::openapi())
        .routes(routes!(router::get_customer))
        .routes(routes!(router::list_entitlements))
        .routes(routes!(router::list_subscriptions))
        .split_for_parts();
    if let Some(license) = module_doc.info.license.clone() {
        open_api.info.license = Some(license);
    }
    open_api.merge(module_doc);
    add_rate_limit_responses(&mut open_api);
    discriminate_tagged_variants(&mut open_api);
    apply_sorted_tags(&mut open_api);
    label_beta_operations(&mut open_api);
    (router, open_api)
}
