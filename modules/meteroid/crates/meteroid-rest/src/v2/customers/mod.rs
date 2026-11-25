//! `/api/v2/customers` (ADR-0013 §11): customers listed by keyset in SQL, read and written as
//! v2 models them (a merge patch for an update), archived (`DELETE` too, as v1) and restored, with their portal token,
//! their entitlements and their credit (wallets, grants, ledger, top-ups), each on the store call
//! v1 uses. A customer is named by its id, or by its alias as `ext:<alias>`; its subscriptions are
//! `GET /subscriptions?customer=`.
use crate::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod credits;
pub(crate) mod entitlements;
pub mod model;
mod read;
mod write;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(read::list_customers, write::create_customer))
        .routes(routes!(read::get_customer, write::update_customer))
        .routes(routes!(write::archive_customer))
        .routes(routes!(write::unarchive_customer))
        .routes(routes!(write::create_portal_token))
        .routes(routes!(entitlements::list_entitlements))
        .routes(routes!(entitlements::get_entitlement))
        .routes(routes!(credits::list_wallets))
        .routes(routes!(credits::update_wallet))
        .routes(routes!(
            credits::list_credit_grants,
            credits::create_credit_grant
        ))
        .routes(routes!(credits::list_ledger))
        .routes(routes!(credits::create_top_up))
}
