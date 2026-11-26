//! `/api/v2/customers` (ADR-0013 §11): customers listed by keyset in SQL, read and written as
//! v2 models them (a merge patch for an update), archived (`POST …/archive`, no `DELETE`) and
//! restored, with their portal token, their credit (wallets, grants, ledger, top-ups), and what
//! they may use (§25: the snapshot, the check, usage counted by the app, overrides, resources),
//! each on the store call v1 uses. A customer is named by its id, or by its alias as
//! `ext:<alias>`; its subscriptions are `GET /subscriptions?customer=`.
use crate::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub(crate) mod checks;
pub(crate) mod credits;
mod entitlement_overrides;
pub(crate) mod entitlements;
pub mod model;
pub(crate) mod read;
pub(crate) mod resources;
mod write;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(read::list_customers, write::create_customer))
        .routes(routes!(read::get_customer, write::update_customer))
        .routes(routes!(write::archive_customer))
        .routes(routes!(write::unarchive_customer))
        .routes(routes!(write::create_portal_token))
        .routes(routes!(entitlements::get_entitlements))
        .routes(routes!(entitlements::get_entitlement))
        .routes(routes!(entitlements::report_usage))
        .routes(routes!(checks::check_entitlement))
        .routes(routes!(checks::check_entitlement_by_query))
        .routes(routes!(
            entitlement_overrides::list_entitlement_overrides,
            entitlement_overrides::create_entitlement_override
        ))
        .routes(routes!(
            entitlement_overrides::get_entitlement_override,
            entitlement_overrides::update_entitlement_override,
            entitlement_overrides::delete_entitlement_override
        ))
        .routes(routes!(resources::list_resources))
        .routes(routes!(
            resources::get_resource,
            resources::put_resource,
            resources::delete_resource
        ))
        .routes(routes!(resources::archive_resource))
        .routes(routes!(resources::unarchive_resource))
        .routes(routes!(credits::list_wallets))
        .routes(routes!(credits::get_wallet, credits::update_wallet))
        .routes(routes!(
            credits::list_credit_grants,
            credits::create_credit_grant
        ))
        .routes(routes!(credits::get_credit_grant))
        .routes(routes!(credits::list_ledger))
        .routes(routes!(credits::create_top_up))
}
