//! The customer's own routes, read with its token: who it is, what it may use, what it is
//! subscribed to. Each reads what v2 reads, rendered by v2.
use crate::AppState;
use crate::client::AuthorizedAsCustomer;
use crate::client::mapping;
use crate::client::model::{ClientCustomer, ClientSubscriptionList};
use crate::v2::customers::entitlements::{
    CustomerEntitlementList, EntitlementListQuery, codes, to_api,
};
use crate::v2::extract::Query;
use crate::v2::pagination::{Cursor, PageQuery, keyset, page};
use crate::v2::problem::Problem;
use crate::v2::subscriptions::render;
use axum::Extension;
use axum::Json;
use axum::extract::State;
use common_domain::ids::{BaseId, SubscriptionId};
use meteroid_store::domain::enums::SubscriptionStatusEnum;
use meteroid_store::repositories::SubscriptionPagesInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::subscription_pages::SubscriptionPageFilter;

/// Terminal subscriptions (cancelled, completed, aborted) are left out.
const CURRENT_SUBSCRIPTION_STATUSES: [SubscriptionStatusEnum; 4] = [
    SubscriptionStatusEnum::PendingActivation,
    SubscriptionStatusEnum::TrialActive,
    SubscriptionStatusEnum::Active,
    SubscriptionStatusEnum::TrialExpired,
];

/// Get the customer
///
/// The customer the token was minted for.
#[utoipa::path(
    get,
    tag = "Customer",
    path = "/api/client/v1/customer",
    responses(
        (status = 200, description = "Customer", body = ClientCustomer),
        (status = 401, description = "Missing, invalid or expired token (`token_expired`)", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = "The token does not grant read access", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "Customer not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("customer_token" = []))
)]
pub async fn get_customer(
    Extension(auth): Extension<AuthorizedAsCustomer>,
    State(app_state): State<AppState>,
) -> Result<Json<ClientCustomer>, Problem> {
    let customer = app_state
        .store
        .find_customer_by_id(auth.customer_id, auth.tenant_id)
        .await?;

    Ok(Json(mapping::customer_to_client(customer)))
}

/// List entitlements
///
/// Every feature the customer is entitled to, with live usage, in feature order and paged by
/// cursor: the customer's entitlements as API v2 reads them.
#[utoipa::path(
    get,
    tag = "Entitlements",
    path = "/api/client/v1/entitlements",
    params(EntitlementListQuery),
    responses(
        (status = 200, description = "A page of the customer's entitlements", body = CustomerEntitlementList),
        (status = 401, description = "Missing, invalid or expired token (`token_expired`)", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = "The token does not grant read access", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "The cursor or limit is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("customer_token" = []))
)]
pub async fn list_entitlements(
    Extension(auth): Extension<AuthorizedAsCustomer>,
    State(app_state): State<AppState>,
    Query(query): Query<EntitlementListQuery>,
) -> Result<Json<CustomerEntitlementList>, Problem> {
    let entitlements = app_state
        .services
        .get_effective_entitlements(auth.customer_id, auth.tenant_id)
        .await?;
    let paged = page(
        entitlements,
        |entitlement| entitlement.feature.id.as_uuid(),
        &PageQuery {
            limit: query.limit,
            after: query.after,
        },
    )?;
    let refs = codes(&app_state, auth.tenant_id, &paged.data).await?;
    Ok(Json(
        paged.map(|entitlement| to_api(entitlement, &refs)).into(),
    ))
}

/// List subscriptions
///
/// The customer's current subscriptions in creation order, paged by cursor; cancelled, completed
/// and aborted ones are left out. Each reads as API v2's summary of a subscription.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/client/v1/subscriptions",
    params(PageQuery),
    responses(
        (status = 200, description = "A page of the customer's subscriptions", body = ClientSubscriptionList),
        (status = 401, description = "Missing, invalid or expired token (`token_expired`)", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = "The token does not grant read access", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "The cursor or limit is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("customer_token" = []))
)]
pub async fn list_subscriptions(
    Extension(auth): Extension<AuthorizedAsCustomer>,
    State(app_state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<ClientSubscriptionList>, Problem> {
    let limit = query.limit()?;
    let cursor = query.cursor()?;
    let listed = app_state
        .store
        .list_subscriptions_after(
            auth.tenant_id,
            SubscriptionPageFilter {
                customer_id: Some(auth.customer_id),
                plan_id: None,
                status: Some(CURRENT_SUBSCRIPTION_STATUSES.to_vec()),
                updated_after: None,
            },
            cursor.map(|Cursor(after)| SubscriptionId::from(after)),
            limit + 1,
        )
        .await?;
    let paged = keyset(listed, limit, |item| item.subscription.id.as_uuid());
    let plan_names: Vec<String> = paged
        .data
        .iter()
        .map(|item| item.subscription.plan_name.clone())
        .collect();
    let summaries = render::summaries(&app_state, auth.tenant_id, paged.data).await?;
    Ok(Json(ClientSubscriptionList {
        data: summaries
            .into_iter()
            .zip(plan_names)
            .map(|(summary, plan_name)| mapping::subscription_to_client(summary, plan_name))
            .collect(),
        next_cursor: paged.next_cursor,
        has_more: paged.has_more,
    }))
}
