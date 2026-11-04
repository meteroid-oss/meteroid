use crate::AppState;
use axum::Router;
use axum::routing::post;

mod challenge;

/// Test gateway routes. Always mounted: the handler refuses any connector that is not the mock,
/// and mock connectors only exist where the instance allowed them.
pub fn mock_routes() -> Router<AppState> {
    Router::new().route("/v1/portal/mock/3ds", post(challenge::handle))
}
