use super::model::{
    CustomerUsageQuery, GroupedUsage, MetricUsage, SubscriptionUsageQuery, UsageResponse,
    UsageSummaryQuery,
};
use crate::api_rest::AppState;
use crate::api_rest::QueryParams;
use crate::api_rest::error::RestErrorResponse;
use crate::errors::RestApiError;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{AliasOr, BillableMetricId, CustomerId, SubscriptionId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::domain::{BillableMetric, Period};
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use rust_decimal::Decimal;

/// Get customer usage
///
/// Retrieve aggregated usage data for a customer over a specified period.
#[utoipa::path(
    get,
    tag = "Usage",
    path = "/api/v1/usage/customer/{customer_id}",
    params(
        ("customer_id" = String, Path, description = "Customer ID or alias"),
        CustomerUsageQuery,
    ),
    responses(
        (status = 200, description = "Customer usage data", body = UsageResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn get_customer_usage(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Path(customer_id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
    Valid(QueryParams(query)): Valid<QueryParams<CustomerUsageQuery>>,
) -> Result<impl IntoResponse, RestApiError> {
    let customer = app_state
        .store
        .find_customer_by_id_or_alias(customer_id_or_alias, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error resolving customer: {e}");
            RestApiError::from(e)
        })?;

    let metrics = load_metrics(&app_state, authorized_state.tenant_id, query.metric_id).await?;

    let period = Period {
        start: query.start_date,
        end: query.end_date,
    };

    let usage = fetch_usage_for_metrics(
        &app_state,
        &authorized_state.tenant_id,
        Some(&customer.id),
        &metrics,
        period.clone(),
    )
    .await?;

    Ok(Json(UsageResponse {
        period_start: period.start,
        period_end: period.end,
        usage,
    }))
}

/// Get subscription usage
///
/// Retrieve aggregated usage data for a subscription's usage-based components.
/// If start_date/end_date are omitted, defaults to the current billing period.
#[utoipa::path(
    get,
    tag = "Usage",
    path = "/api/v1/usage/subscription/{subscription_id}",
    params(
        ("subscription_id" = SubscriptionId, Path, description = "Subscription ID"),
        SubscriptionUsageQuery,
    ),
    responses(
        (status = 200, description = "Subscription usage data", body = UsageResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Subscription not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn get_subscription_usage(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(subscription_id): Path<SubscriptionId>,
    Valid(QueryParams(query)): Valid<QueryParams<SubscriptionUsageQuery>>,
) -> Result<impl IntoResponse, RestApiError> {
    let details = app_state
        .store
        .get_subscription_details(authorized_state.tenant_id, subscription_id)
        .await
        .map_err(|e| {
            log::error!("Error fetching subscription details for usage: {e}");
            RestApiError::from(e)
        })?;

    let period_start = query
        .start_date
        .unwrap_or(details.subscription.current_period_start);
    let period_end = query.end_date.unwrap_or_else(|| {
        details
            .subscription
            .current_period_end
            .unwrap_or_else(|| chrono::Utc::now().date_naive() + chrono::Duration::days(1))
    });

    // Filter subscription metrics by optional metric_id
    let metrics: Vec<BillableMetric> = match query.metric_id {
        Some(mid) => details
            .metrics
            .into_iter()
            .filter(|m| m.id == mid)
            .collect(),
        None => details.metrics,
    };

    let period = Period {
        start: period_start,
        end: period_end,
    };

    let usage = fetch_usage_for_metrics(
        &app_state,
        &authorized_state.tenant_id,
        Some(&details.subscription.customer_id),
        &metrics,
        period.clone(),
    )
    .await?;

    Ok(Json(UsageResponse {
        period_start: period.start,
        period_end: period.end,
        usage,
    }))
}

/// Get usage summary
///
/// Retrieve aggregated usage data across all customers for the tenant.
#[utoipa::path(
    get,
    tag = "Usage",
    path = "/api/v1/usage/summary",
    params(
        UsageSummaryQuery,
    ),
    responses(
        (status = 200, description = "Usage summary", body = UsageResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn get_usage_summary(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(QueryParams(query)): Valid<QueryParams<UsageSummaryQuery>>,
) -> Result<impl IntoResponse, RestApiError> {
    let metrics = load_metrics(&app_state, authorized_state.tenant_id, query.metric_id).await?;

    let period = Period {
        start: query.start_date,
        end: query.end_date,
    };

    let usage = fetch_usage_for_metrics(
        &app_state,
        &authorized_state.tenant_id,
        None,
        &metrics,
        period.clone(),
    )
    .await?;

    Ok(Json(UsageResponse {
        period_start: period.start,
        period_end: period.end,
        usage,
    }))
}

async fn load_metrics(
    app_state: &AppState,
    tenant_id: common_domain::ids::TenantId,
    metric_id: Option<BillableMetricId>,
) -> Result<Vec<BillableMetric>, RestApiError> {
    match metric_id {
        Some(id) => {
            let metric = app_state
                .store
                .find_billable_metric_by_id(id, tenant_id)
                .await
                .map_err(|e| {
                    log::error!("Error loading metric {id}: {e}");
                    RestApiError::from(e)
                })?;
            Ok(vec![metric])
        }
        None => app_state
            .store
            .list_active_billable_metrics(tenant_id)
            .await
            .map_err(|e| {
                log::error!("Error listing metrics: {e}");
                RestApiError::from(e)
            }),
    }
}

async fn fetch_usage_for_metrics(
    app_state: &AppState,
    tenant_id: &common_domain::ids::TenantId,
    customer_id: Option<&CustomerId>,
    metrics: &[BillableMetric],
    period: Period,
) -> Result<Vec<MetricUsage>, RestApiError> {
    let mut usage_items = Vec::with_capacity(metrics.len());

    for metric in metrics {
        let usage_data = app_state
            .services
            .usage_clients()
            .fetch_usage_summary(tenant_id, customer_id, metric, period.clone())
            .await
            .map_err(|e| {
                log::error!("Error fetching usage for metric {}: {e}", metric.id);
                RestApiError::from(e)
            })?;

        let total_value = usage_data
            .data
            .iter()
            .fold(Decimal::ZERO, |acc, g| acc + g.value);

        let grouped_usage = usage_data
            .data
            .into_iter()
            .map(|g| GroupedUsage {
                value: g.value,
                dimensions: g.dimensions,
            })
            .collect();

        usage_items.push(MetricUsage {
            metric_id: metric.id,
            metric_name: metric.name.clone(),
            metric_code: metric.code.clone(),
            total_value,
            grouped_usage,
        });
    }

    Ok(usage_items)
}
