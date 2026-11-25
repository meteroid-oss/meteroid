//! `GET /api/v2/subscriptions/{id_or_external_id}/entitlements`: what one subscription grants,
//! read as a customer's entitlements read (`customers::entitlements`), its metered usage counted
//! over the subscription's current period.
use crate::AppState;
use crate::v2::customers::entitlements::{
    CustomerEntitlementList, EntitlementListQuery, codes, to_api,
};
use crate::v2::extract::{Path, Query};
use crate::v2::ids;
use crate::v2::pagination::{PageQuery, page};
use crate::v2::problem::Problem;
use axum::Extension;
use axum::Json;
use axum::extract::State;
use common_domain::ids::BaseId;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;

/// List a subscription's entitlements
///
/// Every feature the subscription grants, its value merged from its plan version, its add-ons
/// and its overrides over the feature's default, in feature order; the usage of a metered limit
/// counted in the subscription's current period. What the customer may use across all its
/// subscriptions is `GET /customers/{id_or_alias}/entitlements`.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/entitlements",
    params(
        ("id_or_external_id" = String, Path, description = "Subscription id or external id"),
        EntitlementListQuery,
    ),
    responses(
        (status = 200, description = "A page of the subscription's entitlements", body = CustomerEntitlementList),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "The cursor or limit is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_subscription_entitlements(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
    Query(query): Query<EntitlementListQuery>,
) -> Result<Json<CustomerEntitlementList>, Problem> {
    let id = ids::subscription_id(&state, auth.tenant_id, &id_or_external_id).await?;
    let entitlements = state
        .services
        .get_effective_entitlements_for_subscription(id, auth.tenant_id)
        .await?;
    let paged = page(
        entitlements,
        |entitlement| entitlement.feature.id.as_uuid(),
        &PageQuery {
            limit: query.limit,
            after: query.after,
        },
    )?;
    let refs = codes(&state, auth.tenant_id, &paged.data).await?;
    Ok(Json(
        paged.map(|entitlement| to_api(entitlement, &refs)).into(),
    ))
}
