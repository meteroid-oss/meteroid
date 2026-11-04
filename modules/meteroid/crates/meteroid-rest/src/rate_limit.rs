use common_grpc::middleware::server::auth::{AuthorizedAsTenant, TenantEnv};
use rate_limiter::middleware::axum::RateLimitState;
use rate_limiter::{RateLimitConfig, RateLimitResult, RateLimiter};

use crate::AppState;
use crate::client::{AuthorizedAsCustomer, CLIENT_API_PREFIX};
use meteroid_app::config::RestRateLimitConfig;
use meteroid_app::errors::RestApiError;

/// Rate limiters for a single tenant environment (prod or non-prod).
#[derive(Clone)]
pub struct TenantEnvRateLimiters {
    pub api: RateLimiter,
    pub metering_api: RateLimiter,
    pub metering_eps: RateLimiter,
    /// Per customer, on the client API.
    pub client: RateLimiter,
    pub usage_costs: RateLimiter,
}

/// Holds separate rate limiters for production and non-production environments.
#[derive(Clone)]
pub struct RateLimiters {
    pub prod: TenantEnvRateLimiters,
    pub non_prod: TenantEnvRateLimiters,
}

const USAGE_COSTS_BUCKET: &str = "rest:usage_costs";
const NON_PROD_USAGE_COSTS_BUCKET: &str = "rest:non_prod:usage_costs";

/// Build the per-environment, pre-configured limiters used by both the
/// request-rate middleware and the in-handler EPS check.
pub fn build_rest_rate_limiters(base: &RateLimiter, config: &RestRateLimitConfig) -> RateLimiters {
    RateLimiters {
        prod: TenantEnvRateLimiters {
            api: base
                .with_config(RateLimitConfig::per_second(config.prod_rps).with_bucket("rest:api")),
            metering_api: base.with_config(
                RateLimitConfig::per_second(config.prod_metering_rps).with_bucket("rest:metering"),
            ),
            metering_eps: base.with_config(
                RateLimitConfig::per_second(config.prod_metering_eps)
                    .with_bucket("rest:metering:eps"),
            ),
            client: base.with_config(
                RateLimitConfig::per_second(config.client_prod_rps).with_bucket("rest:client"),
            ),
            usage_costs: base.with_config(
                RateLimitConfig::per_minute(config.prod_usage_costs_rpm)
                    .with_bucket(USAGE_COSTS_BUCKET),
            ),
        },
        non_prod: TenantEnvRateLimiters {
            api: base.with_config(
                RateLimitConfig::per_second(config.non_prod_rps).with_bucket("rest:non_prod:api"),
            ),
            metering_api: base.with_config(
                RateLimitConfig::per_second(config.non_prod_metering_rps)
                    .with_bucket("rest:non_prod:metering"),
            ),
            metering_eps: base.with_config(
                RateLimitConfig::per_second(config.non_prod_metering_eps)
                    .with_bucket("rest:non_prod:metering:eps"),
            ),
            client: base.with_config(
                RateLimitConfig::per_second(config.client_non_prod_rps)
                    .with_bucket("rest:non_prod:client"),
            ),
            usage_costs: base.with_config(
                RateLimitConfig::per_minute(config.non_prod_usage_costs_rpm)
                    .with_bucket(NON_PROD_USAGE_COSTS_BUCKET),
            ),
        },
    }
}

/// Build the axum middleware state for request-per-second rate limiting.
pub fn build_rate_limit_state(limiters: &RateLimiters) -> RateLimitState<String> {
    let limiters = limiters.clone();
    RateLimitState::new(move |req| {
        req.extensions().get::<AuthorizedAsTenant>().map(|auth| {
            let is_metering = req.uri().path().starts_with("/api/v1/events/");

            let limiter = match auth.tenant_env {
                TenantEnv::Production => {
                    if is_metering {
                        &limiters.prod.metering_api
                    } else {
                        &limiters.prod.api
                    }
                }
                TenantEnv::NonProduction => {
                    if is_metering {
                        &limiters.non_prod.metering_api
                    } else {
                        &limiters.non_prod.api
                    }
                }
            };

            (auth.tenant_id.to_string(), limiter.clone())
        })
    })
    .with_filter(|path| path.starts_with("/api/"))
}

/// Client API requests are limited per customer, as they carry no tenant credential.
pub fn build_client_rate_limit_state(limiters: &RateLimiters) -> RateLimitState<String> {
    let limiters = limiters.clone();
    RateLimitState::new(move |req| {
        req.extensions().get::<AuthorizedAsCustomer>().map(|auth| {
            let limiter = match auth.tenant_env {
                TenantEnv::Production => &limiters.prod.client,
                TenantEnv::NonProduction => &limiters.non_prod.client,
            };
            (
                format!("{}:{}", auth.tenant_id, auth.customer_id),
                limiter.clone(),
            )
        })
    })
    .with_filter(|path| path.starts_with(CLIENT_API_PREFIX))
}

/// Check the per-event rate limit for the events ingestion endpoint.
///
/// Consumes `event_count` tokens from the EPS bucket for the tenant's
/// environment, independently of the per-request RPS bucket.
pub async fn rate_limit_eps(
    limiters: &RateLimiters,
    auth: &AuthorizedAsTenant,
    event_count: usize,
) -> Result<(), RestApiError> {
    let limiter = match auth.tenant_env {
        TenantEnv::Production => &limiters.prod.metering_eps,
        TenantEnv::NonProduction => &limiters.non_prod.metering_eps,
    };

    match limiter.check_n(&auth.tenant_id, event_count as f64).await {
        RateLimitResult::Allowed { .. } => Ok(()),
        RateLimitResult::RateLimited { retry_after, .. } => Err(RestApiError::RateLimitExceeded(
            format!("Retry after {retry_after} seconds"),
        )),
    }
}

/// Check the per-tenant quota of the usage cost reads, which price live usage: the environment's
/// default, unless the organization's `usage_costs_rate_limit` entitlement sets its own.
pub async fn rate_limit_usage_costs(
    state: &AppState,
    auth: &AuthorizedAsTenant,
) -> Result<(), RestApiError> {
    let (limiter, bucket) = match auth.tenant_env {
        TenantEnv::Production => (
            &state.rest_rate_limiters.prod.usage_costs,
            USAGE_COSTS_BUCKET,
        ),
        TenantEnv::NonProduction => (
            &state.rest_rate_limiters.non_prod.usage_costs,
            NON_PROD_USAGE_COSTS_BUCKET,
        ),
    };
    let per_minute = state
        .feature_resolver
        .resolve(auth.organization_id)
        .await
        .usage_costs_rate_limit();
    let limiter = usage_costs_limiter(limiter, bucket, per_minute);

    match limiter.check(&auth.tenant_id).await {
        RateLimitResult::Allowed { .. } => Ok(()),
        RateLimitResult::RateLimited { retry_after, .. } => Err(RestApiError::RateLimitExceeded(
            format!("Retry after {retry_after} seconds"),
        )),
    }
}

/// The environment's limiter, or one at the organization's own rate in the same bucket.
fn usage_costs_limiter(
    default: &RateLimiter,
    bucket: &str,
    per_minute: Option<u64>,
) -> RateLimiter {
    match per_minute {
        Some(per_minute) => {
            default.with_config(RateLimitConfig::per_minute(per_minute).with_bucket(bucket))
        }
        None => default.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_organization_rate_replaces_the_default_in_the_same_bucket() {
        let limiters =
            build_rest_rate_limiters(&RateLimiter::default(), &RestRateLimitConfig::default());
        let default = &limiters.prod.usage_costs;

        let kept = usage_costs_limiter(default, USAGE_COSTS_BUCKET, None);
        assert_eq!(kept.config().capacity, 600.0);

        let overridden = usage_costs_limiter(default, USAGE_COSTS_BUCKET, Some(1_200));
        assert_eq!(overridden.config().capacity, 1_200.0);
        assert_eq!(overridden.config().refill_rate, 20.0);
        assert_eq!(
            overridden.config().bucket.as_deref(),
            default.config().bucket.as_deref()
        );
    }
}
