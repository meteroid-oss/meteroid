//! `GET /api/v2/customers` and `GET /api/v2/customers/{id_or_alias}`: lists page by keyset in
//! SQL, customers being an unbounded collection (ADR-0013 §3).
use super::model::{Customer, CustomerList, CustomerListQuery};
use crate::AppState;
use crate::v2::extract::{Path, Query};
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery};
use crate::v2::problem::Problem;
use axum::Extension;
use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, CustomerId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::repositories::CustomerPagesInterface;
use meteroid_store::repositories::customer_pages::CustomerPageFilter;

/// The customer as v2 reads it, by id, archived or not.
pub(super) async fn render(
    state: &AppState,
    tenant_id: TenantId,
    id: CustomerId,
) -> Result<Customer, Problem> {
    let listed = state.store.get_listed_customer(tenant_id, id).await?;
    Ok(Customer::new(listed.customer, listed.updated_at))
}

/// List customers
///
/// The tenant's live customers in creation order, paged by cursor; the archived ones with
/// `archived=true`.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers",
    params(CustomerListQuery),
    responses(
        (status = 200, description = "A page of customers", body = CustomerList),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A filter is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_customers(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<CustomerListQuery>,
) -> Result<Json<CustomerList>, Problem> {
    let page = PageQuery {
        limit: query.limit,
        after: query.after.clone(),
    };
    let limit = page.limit()?;
    let cursor = page.cursor()?;
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
        .list_customers_after(
            auth.tenant_id,
            CustomerPageFilter {
                search: query.search.clone(),
                archived: query.archived.unwrap_or(false),
                updated_after,
            },
            cursor.map(|Cursor(after)| CustomerId::from(after)),
            limit + 1,
        )
        .await?;
    let has_more = items.len() > limit;
    items.truncate(limit);
    let next_cursor = if has_more {
        items
            .last()
            .map(|item| Cursor(item.customer.id.as_uuid()).encode())
    } else {
        None
    };
    Ok(Json(CustomerList {
        data: items
            .into_iter()
            .map(|item| Customer::new(item.customer, item.updated_at))
            .collect(),
        next_cursor,
        has_more,
    }))
}

/// Get a customer
///
/// By id, or by its alias as `ext:<alias>`; an archived one too.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}",
    params(("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`")),
    responses(
        (status = 200, description = "The customer", body = Customer),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_customer(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
) -> Result<Json<Customer>, Problem> {
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    Ok(Json(render(&state, auth.tenant_id, customer.id).await?))
}
