use crate::AppState;
use axum::Router;
use axum::routing::get;

pub mod router;

pub fn sso_routes() -> Router<AppState> {
    Router::new()
        // Public login-side endpoints for the org SSO handshake. Config
        // management lives on the authenticated gRPC `SsoService`.
        .route("/sso/discover", get(router::discover))
        .route("/sso/authorize", get(router::authorize))
        .route("/sso/callback", get(router::callback))
}
