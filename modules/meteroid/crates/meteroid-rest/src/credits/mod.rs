use crate::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub mod model;
pub mod router;
pub mod writes;

pub fn credit_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_wallets))
        .routes(routes!(writes::patch_wallet))
        .routes(routes!(router::list_ledger_entries))
        .routes(routes!(
            router::list_credit_grants,
            writes::create_credit_grant
        ))
        .routes(routes!(writes::create_top_up))
}
