use super::model::{
    CustomerUsageQuery, GroupedUsage, MetricUsage, SubscriptionUsageQuery, UnattributedMetricUsage,
    UnattributedUsageQuery, UnattributedUsageResponse, UsageResponse, UsageSummaryQuery, sum_costs,
};
use crate::AppState;
use crate::rate_limit::rate_limit_usage_costs;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{AliasOr, BillableMetricId, CustomerId, SubscriptionId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_app::errors::RestApiError;
use meteroid_rest_common::QueryParams;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_store::domain::{BillableMetric, UsagePeriod, unattributed_default_range};
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
pub async fn get_customer_usage(
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

    let period = UsagePeriod {
        start: query.start_date.and_time(chrono::NaiveTime::MIN),
        end: query.end_date.and_time(chrono::NaiveTime::MIN),
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
        period_start: period.start.date(),
        period_end: period.end.date(),
        usage,
        computed_at: None,
        totals: None,
    }))
}

/// Get unattributed usage
///
/// Retrieve a customer's usage that no subscription bills. When subscriptions of the customer
/// pricing a metric set a usage filter, events matching none of the filters are billed by a
/// subscription without one; when every such subscription has a filter, those events are kept
/// and reported here instead of being billed.
#[utoipa::path(
    get,
    tag = "Usage",
    path = "/api/v1/usage/customer/{customer_id}/unattributed",
    params(
        ("customer_id" = String, Path, description = "Customer ID or alias"),
        UnattributedUsageQuery,
    ),
    responses(
        (status = 200, description = "Unattributed usage", body = UnattributedUsageResponse),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub async fn get_unattributed_usage(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Path(customer_id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
    Valid(QueryParams(query)): Valid<QueryParams<UnattributedUsageQuery>>,
) -> Result<impl IntoResponse, RestApiError> {
    let customer = app_state
        .store
        .find_customer_by_id_or_alias(customer_id_or_alias, authorized_state.tenant_id)
        .await
        .map_err(RestApiError::from)?;
    let (default_start, default_end) =
        unattributed_default_range(common_domain::clock::now().date_naive());
    let period_start = query.start_date.unwrap_or(default_start);
    let period_end = query.end_date.unwrap_or(default_end);

    let usage = app_state
        .services
        .unattributed_usage(
            authorized_state.tenant_id,
            customer.id,
            period_start.and_time(chrono::NaiveTime::MIN),
            period_end.and_time(chrono::NaiveTime::MIN),
        )
        .await
        .map_err(RestApiError::from)?;

    Ok(Json(UnattributedUsageResponse {
        period_start,
        period_end,
        usage: usage
            .into_iter()
            .map(|usage| UnattributedMetricUsage {
                metric_id: usage.metric.id,
                metric_name: usage.metric.name,
                metric_code: usage.metric.code,
                from: usage.from.into(),
                until: usage.until.into(),
                total_value: usage.quantity,
            })
            .collect(),
    }))
}

/// Get subscription usage
///
/// Retrieve aggregated usage data for a subscription's usage-based components.
/// If start_date/end_date are omitted, defaults to the current billing period, and the response
/// then carries the running total (beta): what each metric's usage adds so far to the next invoice,
/// and what they add up to, before discounts and credits. That reading is shared for up to 30
/// seconds (see `computed_at`). Counts towards the usage cost reads' rate limit.
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
        (status = 429, description = "Too many usage cost reads", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub async fn get_subscription_usage(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(subscription_id): Path<SubscriptionId>,
    Valid(QueryParams(query)): Valid<QueryParams<SubscriptionUsageQuery>>,
) -> Result<impl IntoResponse, RestApiError> {
    rate_limit_usage_costs(&app_state, &authorized_state).await?;
    let tenant_id = authorized_state.tenant_id;
    if query.start_date.is_none() && query.end_date.is_none() {
        let current = app_state
            .subscription_usage
            .try_get_with(
                (tenant_id, subscription_id),
                current_period_usage(&app_state, tenant_id, subscription_id),
            )
            .await
            .map_err(|e| (*e).clone())?;
        if current.computed_at.is_none() {
            app_state
                .subscription_usage
                .invalidate(&(tenant_id, subscription_id))
                .await;
        }
        let usage = current.only_metric(query.metric_id);
        return Ok(Json(
            if crate::usage_costs::router::usage_costs_open(&app_state, &authorized_state).await {
                usage
            } else {
                usage.without_costs()
            },
        ));
    }

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
            .unwrap_or_else(|| common_domain::clock::now().date_naive() + chrono::Duration::days(1))
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

    let period = UsagePeriod {
        start: period_start.and_time(chrono::NaiveTime::MIN),
        end: period_end.and_time(chrono::NaiveTime::MIN),
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
        period_start: period.start.date(),
        period_end: period.end.date(),
        usage,
        computed_at: None,
        totals: None,
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
pub async fn get_usage_summary(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(QueryParams(query)): Valid<QueryParams<UsageSummaryQuery>>,
) -> Result<impl IntoResponse, RestApiError> {
    let metrics = load_metrics(&app_state, authorized_state.tenant_id, query.metric_id).await?;

    let period = UsagePeriod {
        start: query.start_date.and_time(chrono::NaiveTime::MIN),
        end: query.end_date.and_time(chrono::NaiveTime::MIN),
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
        period_start: period.start.date(),
        period_end: period.end.date(),
        usage,
        computed_at: None,
        totals: None,
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
    period: UsagePeriod,
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
            costs: None,
        });
    }

    Ok(usage_items)
}

async fn current_period_usage(
    app_state: &AppState,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> Result<std::sync::Arc<UsageResponse>, RestApiError> {
    let details = app_state
        .store
        .get_subscription_details(tenant_id, subscription_id)
        .await
        .map_err(RestApiError::from)?;
    let period = UsagePeriod {
        start: details
            .subscription
            .current_period_start
            .and_time(chrono::NaiveTime::MIN),
        end: details
            .subscription
            .current_period_end
            .unwrap_or_else(|| common_domain::clock::now().date_naive() + chrono::Duration::days(1))
            .and_time(chrono::NaiveTime::MIN),
    };

    // Usage is still served when pricing it fails; the reading then carries no costs.
    let running = app_state
        .services
        .subscription_running_total(&details)
        .await
        .inspect_err(|e| {
            log::warn!("Running total of subscription {subscription_id} not computed: {e:?}")
        })
        .ok();

    // Metrics a plan change removed are still billed by the next invoice, so listed too.
    let mut metrics = details.metrics.clone();
    for priced in running.iter().flat_map(|running| &running.metrics) {
        if !metrics.iter().any(|metric| metric.id == priced.metric.id) {
            metrics.push(priced.metric.clone());
        }
    }
    let mut usage = fetch_usage_for_metrics(
        app_state,
        &tenant_id,
        Some(&details.subscription.customer_id),
        &metrics,
        period.clone(),
    )
    .await?;

    let Some(running) = running else {
        return Ok(std::sync::Arc::new(UsageResponse {
            period_start: period.start.date(),
            period_end: period.end.date(),
            usage,
            computed_at: None,
            totals: None,
        }));
    };
    for metric in &mut usage {
        metric.costs = Some(
            running
                .metrics
                .iter()
                .find(|priced| priced.metric.id == metric.metric_id)
                .map(|priced| priced.costs.iter().cloned().map(Into::into).collect())
                .unwrap_or_default(),
        );
    }
    let totals = sum_costs(
        usage
            .iter()
            .flat_map(|metric| metric.costs.iter().flatten()),
    );
    Ok(std::sync::Arc::new(UsageResponse {
        period_start: period.start.date(),
        period_end: period.end.date(),
        usage,
        computed_at: Some(running.computed_at.into()),
        totals: Some(totals),
    }))
}
