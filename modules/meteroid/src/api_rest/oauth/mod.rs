use crate::api_rest::AppState;
use axum::Router;
use axum::routing::get;

mod router;

pub fn oauth_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/oauth/{provider}",
            get(router::redirect_to_identity_provider),
        )
        .route("/oauth-callback/{provider}", get(router::callback))
}
