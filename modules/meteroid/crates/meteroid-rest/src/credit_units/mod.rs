use crate::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod mapping;
pub mod model;
mod router;

pub fn credit_unit_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            router::list_credit_units,
            router::create_credit_unit
        ))
        .routes(routes!(router::get_credit_unit, router::update_credit_unit))
        .routes(routes!(router::list_credit_unit_prices))
        .routes(routes!(
            router::set_credit_unit_price,
            router::archive_credit_unit_price
        ))
}
