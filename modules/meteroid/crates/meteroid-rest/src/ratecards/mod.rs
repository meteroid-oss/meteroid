use crate::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod mapping;
pub mod model;
mod router;

pub fn rate_card_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_rate_cards, router::create_rate_card))
        .routes(routes!(router::get_rate_card))
        .routes(routes!(router::set_rate_card_rates))
}
