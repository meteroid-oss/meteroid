use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub(crate) mod mapping;
pub mod model;
pub mod router;

pub fn feature_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_features))
        .routes(routes!(router::get_feature))
}

pub fn entity_entitlement_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_plan_version_entitlements))
        .routes(routes!(router::list_add_on_entitlements))
        .routes(routes!(router::list_subscription_entitlements))
        .routes(routes!(router::list_product_entitlements))
}

pub fn effective_entitlement_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(router::get_effective_entitlements))
}
