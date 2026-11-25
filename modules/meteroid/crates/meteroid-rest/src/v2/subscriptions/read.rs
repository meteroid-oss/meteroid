//! `GET /api/v2/subscriptions` and `GET /api/v2/subscriptions/{id_or_external_id}`: lists page by
//! keyset in SQL, subscriptions being an unbounded collection (ADR-0013 §3).
use super::model::{Subscription, SubscriptionList, SubscriptionListQuery};
use super::render;
use super::resolve::not_found_at;
use crate::AppState;
use crate::v2::extract::{Path, Query};
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery};
use crate::v2::problem::Problem;
use axum::Extension;
use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, SubscriptionId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::repositories::SubscriptionPagesInterface;
use meteroid_store::repositories::subscription_pages::SubscriptionPageFilter;

/// List subscriptions
///
/// The tenant's subscriptions in creation order, paged by cursor, without what is read on a
/// single one (charges, quantities, add-ons, coupons, schedule).
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions",
    params(SubscriptionListQuery),
    responses(
        (status = 200, description = "A page of subscriptions", body = SubscriptionList),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A filter is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_subscriptions(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<SubscriptionListQuery>,
) -> Result<Json<SubscriptionList>, Problem> {
    let tenant_id = auth.tenant_id;
    let page = PageQuery {
        limit: query.limit,
        after: query.after.clone(),
    };
    let limit = page.limit()?;
    let cursor = page.cursor()?;
    let customer_id = match query.customer.as_deref() {
        Some(raw) => Some(
            ids::customer(&state, tenant_id, raw)
                .await
                .map_err(not_found_at("customer".to_owned(), "customer", raw))?
                .id,
        ),
        None => None,
    };
    let plan_id = match query.plan.as_deref() {
        Some(raw) => Some(
            ids::plan_id(&state, tenant_id, raw)
                .await
                .map_err(not_found_at("plan".to_owned(), "plan", raw))?,
        ),
        None => None,
    };
    let updated_after = query
        .updated_after
        .as_deref()
        .map(|raw| {
            DateTime::parse_from_rfc3339(raw.trim())
                .map(|at| at.with_timezone(&Utc))
                .map_err(|_| {
                    Problem::field(
                        "updated_after",
                        "invalid",
                        "An RFC 3339 date-time, like 2026-12-01T00:00:00Z",
                    )
                })
        })
        .transpose()?;
    let mut items = state
        .store
        .list_subscriptions_after(
            tenant_id,
            SubscriptionPageFilter {
                customer_id,
                plan_id,
                status: query.status.map(|status| vec![status.into()]),
                updated_after,
            },
            cursor.map(|Cursor(after)| SubscriptionId::from(after)),
            limit + 1,
        )
        .await?;
    let has_more = items.len() > limit;
    items.truncate(limit);
    let next_cursor = if has_more {
        items
            .last()
            .map(|item| Cursor(item.subscription.id.as_uuid()).encode())
    } else {
        None
    };
    let data = render::summaries(&state, tenant_id, items).await?;
    Ok(Json(SubscriptionList {
        data,
        next_cursor,
        has_more,
    }))
}

/// Get a subscription
///
/// By id, or by the external id it was created with.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    responses(
        (status = 200, description = "The subscription", body = Subscription),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A charge uses what v2 cannot express: read it with v1", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_subscription(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
) -> Result<Json<Subscription>, Problem> {
    let id = ids::subscription_id(&state, auth.tenant_id, &id_or_external_id).await?;
    Ok(Json(
        render::subscription(&state, auth.tenant_id, id).await?,
    ))
}

/// Get a subscription's summary
///
/// The subscription as lists read it: its own fields (status, dates, period, MRR, invoicing),
/// without what is read beside on a single one (charges, quantities, levels, add-ons, coupons,
/// schedule, minimum). Cheaper than the whole read, and never refused for a charge v2 cannot
/// express.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/summary",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    responses(
        (status = 200, description = "The subscription, as lists read it", body = Subscription),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_subscription_summary(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
) -> Result<Json<Subscription>, Problem> {
    let id = ids::subscription_id(&state, auth.tenant_id, &id_or_external_id).await?;
    Ok(Json(render::summary(&state, auth.tenant_id, id).await?))
}
