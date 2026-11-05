use crate::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;
use serde::Deserialize;

#[derive(Deserialize)]
struct RecoverPortalLinkRequest {
    token: String,
    email: String,
}

/// Public: the caller only proves it holds an expired link.
pub fn portal_link_routes() -> Router<AppState> {
    Router::new().route("/v1/portal/links/recover", post(recover_portal_link))
}

/// Always 202, whatever happened, so the endpoint can't be used to probe addresses.
async fn recover_portal_link(
    State(app_state): State<AppState>,
    Json(req): Json<RecoverPortalLinkRequest>,
) -> StatusCode {
    if let Err(e) = app_state
        .store
        .recover_portal_link(req.token.trim(), req.email.trim())
        .await
    {
        log::error!("Portal link recovery failed: {e:?}");
    }
    StatusCode::ACCEPTED
}
