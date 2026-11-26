use crate::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub mod mapping;
pub mod model;
#[cfg(test)]
mod parity;
pub mod router;

pub fn feature_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_features))
        .routes(routes!(router::create_feature))
        .routes(routes!(router::get_feature))
        .routes(routes!(router::update_feature))
        .routes(routes!(router::archive_feature))
        .routes(routes!(router::unarchive_feature))
}

pub fn entitlement_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::get_entitlement))
        .routes(routes!(router::update_entitlement))
        .routes(routes!(router::delete_entitlement))
        .routes(routes!(router::get_entitlement_history))
}

pub fn entity_entitlement_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_plan_version_entitlements))
        .routes(routes!(router::create_plan_version_entitlements))
        .routes(routes!(router::list_add_on_entitlements))
        .routes(routes!(router::create_add_on_entitlements))
        .routes(routes!(router::list_subscription_entitlements))
        .routes(routes!(router::list_product_entitlements))
        .routes(routes!(router::create_product_entitlements))
}

pub fn effective_entitlement_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::get_entitlement_snapshot))
        .routes(routes!(router::create_customer_entitlements))
        .routes(routes!(router::report_usage))
}

/// The check, core since ADR 0011 §3.14: its usage is read, or the live engine's when a module adds it.
pub fn check_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::check_entitlement))
        .routes(routes!(router::check_entitlement_get))
}
