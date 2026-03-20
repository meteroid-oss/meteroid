use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod mapping;
pub mod model;
pub mod router;

pub fn coupon_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_coupons))
        .routes(routes!(router::create_coupon))
        .routes(routes!(router::get_coupon))
        .routes(routes!(router::update_coupon))
        .routes(routes!(router::archive_coupon))
        .routes(routes!(router::unarchive_coupon))
        .routes(routes!(router::disable_coupon))
        .routes(routes!(router::enable_coupon))
}
