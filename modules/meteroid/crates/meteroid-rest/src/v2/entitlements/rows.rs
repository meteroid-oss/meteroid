//! `/api/v2/entitlements/{id}`: any row, whoever owns it, by its lineage id, and its versions.
use super::model::{Codes, Entitlement};
use crate::AppState;
use crate::v2::extract::{Path, Query};
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery, keyset};
use crate::v2::problem::Problem;
use axum::Extension;
use axum::Json;
use axum::extract::State;
use common_domain::ids::{BaseId, EntitlementId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::repositories::entitlements::EntitlementsInterface;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

crate::v2_list!(EntitlementHistory, Entitlement);

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_entitlement))
        .routes(routes!(get_entitlement_history))
}

pub(crate) fn entitlement_id(raw: &str) -> Result<EntitlementId, Problem> {
    ids::plain_id(raw, "entitlement")
}

/// Get an entitlement
///
/// The open version of a row, whatever owns it: a feature's defaults, a plan version, an add-on,
/// a subscription or a customer. A revoked row is not found; its history still reads.
#[utoipa::path(
    get,
    tag = "Entitlements",
    path = "/api/v2/entitlements/{id}",
    params(("id" = String, Path, description = "Entitlement id")),
    responses(
        (status = 200, description = "The entitlement", body = Entitlement),
        (status = 404, description = "Not found, or revoked", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_entitlement(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Entitlement>, Problem> {
    let id = entitlement_id(&id)?;
    let row = state.store.get_entitlement(id, auth.tenant_id).await?;
    let codes = Codes::of_rows(&state, auth.tenant_id, std::slice::from_ref(&row)).await?;
    Ok(Json(codes.entitlement(row)))
}

/// List an entitlement's history
///
/// Every version of the row, newest first: each edit revoked the version before it, and a
/// revoked row's last version carries `revoked_at` and `revoked_by`.
#[utoipa::path(
    get,
    tag = "Entitlements",
    path = "/api/v2/entitlements/{id}/history",
    params(("id" = String, Path, description = "Entitlement id"), PageQuery),
    responses(
        (status = 200, description = "A page of the row's versions", body = EntitlementHistory),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "The cursor or limit is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_entitlement_history(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<Json<EntitlementHistory>, Problem> {
    let id = entitlement_id(&id)?;
    let limit = query.limit()?;
    let cursor = query.cursor()?;
    // Newest first, by keyset over the version's id (UUIDv7: its creation).
    let versions = state
        .store
        .entitlement_history_before(
            auth.tenant_id,
            id,
            cursor.map(|Cursor(before)| EntitlementId::from(before)),
            limit + 1,
        )
        .await?;
    if versions.is_empty() && cursor.is_none() {
        return Err(Problem::not_found(format!("entitlement {id}")));
    }
    let paged = keyset(versions, limit, |version| version.version_id.as_uuid());
    let codes = Codes::of_rows(&state, auth.tenant_id, &paged.data).await?;
    Ok(Json(EntitlementHistory {
        data: paged
            .data
            .into_iter()
            .map(|row| codes.entitlement(row))
            .collect(),
        next_cursor: paged.next_cursor,
        has_more: paged.has_more,
    }))
}
