//! `/api/v2/subscriptions` (ADR-0013 §10): a subscription created on the store's create path with
//! every code resolved, read back with its charges as plans render them, listed by keyset in SQL,
//! and changed — its plan, a seat count, a level, its charges and add-ons, its cancellation —
//! each change with a `/preview` writing nothing, each on the store service the gRPC API uses.
//! Its summary (`…/summary`) reads as lists read it; its entitlements as a customer's.
use crate::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod changes;
pub(crate) mod create;
mod entitlements;
pub mod model;
mod patch;
mod read;
pub(crate) mod render;
pub(crate) mod resolve;
mod when;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            read::list_subscriptions,
            create::create_subscription
        ))
        .routes(routes!(read::get_subscription, patch::update_subscription))
        .routes(routes!(read::get_subscription_summary))
        .routes(routes!(entitlements::list_subscription_entitlements))
        .merge(changes::routes())
}
