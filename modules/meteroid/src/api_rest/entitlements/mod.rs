use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub(crate) mod mapping;
pub mod model;
pub mod router;

pub fn feature_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_features))
        .routes(routes!(router::create_feature))
        .routes(routes!(router::get_feature))
        .routes(routes!(router::update_feature))
        .routes(routes!(router::set_feature_status))
        .routes(routes!(router::list_entitlements_by_feature))
        .routes(routes!(router::create_feature_entitlement))
}

pub fn entity_entitlement_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            router::list_plan_entitlements,
            router::create_plan_entitlement
        ))
        .routes(routes!(router::create_plan_version_entitlement))
        .routes(routes!(router::create_add_on_entitlement))
        .routes(routes!(router::create_subscription_entitlement))
        .routes(routes!(router::create_quote_entitlement))
}

pub fn entitlement_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(
        router::update_entitlement,
        router::delete_entitlement
    ))
}

pub fn effective_entitlement_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::get_effective_entitlements))
        .routes(routes!(router::get_effective_entitlement))
}
