use common_grpc::middleware::server::auth::{AuthorizedAsTenant, TenantEnv};
use rate_limiter::middleware::axum::RateLimitState;
use rate_limiter::{RateLimitConfig, RateLimitResult, RateLimiter};

use crate::AppState;
use crate::client::{AuthorizedAsCustomer, AuthorizedAsPublishable, CLIENT_API_PREFIX};
use meteroid_app::config::RestRateLimitConfig;
use meteroid_app::errors::RestApiError;
use meteroid_app::features::Entitlements;

/// Rate limiters for a single tenant environment (prod or non-prod).
#[derive(Clone)]
pub struct TenantEnvRateLimiters {
    pub api: RateLimiter,
    pub metering_api: RateLimiter,
    pub metering_eps: RateLimiter,
    /// Per customer, on the client API.
    pub client: RateLimiter,
    /// Per publishable key, on the client API's public routes.
    pub publishable: RateLimiter,
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
const METERING_EPS_BUCKET: &str = "rest:metering:eps";
const NON_PROD_METERING_EPS_BUCKET: &str = "rest:non_prod:metering:eps";

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
                    .with_bucket(METERING_EPS_BUCKET),
            ),
            client: base.with_config(
                RateLimitConfig::per_second(config.client_prod_rps).with_bucket("rest:client"),
            ),
            publishable: base.with_config(
                RateLimitConfig::per_second(config.publishable_prod_rps)
                    .with_bucket("rest:client:public"),
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
                    .with_bucket(NON_PROD_METERING_EPS_BUCKET),
            ),
            client: base.with_config(
                RateLimitConfig::per_second(config.client_non_prod_rps)
                    .with_bucket("rest:non_prod:client"),
            ),
            publishable: base.with_config(
                RateLimitConfig::per_second(config.publishable_non_prod_rps)
                    .with_bucket("rest:non_prod:client:public"),
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
            // v2 forwards v1's event routes, under the same limits.
            let path = req.uri().path();
            let is_metering =
                path.starts_with("/api/v1/events/") || path.starts_with("/api/v2/events/");

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

/// Client API requests are limited per customer, as they carry no tenant credential; the public
/// routes per publishable key, which every visitor of a site shares.
pub fn build_client_rate_limit_state(limiters: &RateLimiters) -> RateLimitState<String> {
    let limiters = limiters.clone();
    RateLimitState::new(move |req| {
        let customer = req.extensions().get::<AuthorizedAsCustomer>().map(|auth| {
            let limiter = match auth.tenant_env {
                TenantEnv::Production => &limiters.prod.client,
                TenantEnv::NonProduction => &limiters.non_prod.client,
            };
            (
                format!("{}:{}", auth.tenant_id, auth.customer_id),
                limiter.clone(),
            )
        });
        customer.or_else(|| {
            req.extensions()
                .get::<AuthorizedAsPublishable>()
                .map(|auth| {
                    let limiter = match auth.tenant_env {
                        TenantEnv::Production => &limiters.prod.publishable,
                        TenantEnv::NonProduction => &limiters.non_prod.publishable,
                    };
                    (format!("pk:{}", auth.key_id), limiter.clone())
                })
        })
    })
    .with_filter(|path| path.starts_with(CLIENT_API_PREFIX))
}

/// Check the per-event rate limit for the events ingestion endpoint: the environment's default,
/// unless the organization's `metering_eps_rate_limit` entitlement sets its own.
///
/// Consumes `event_count` tokens from the EPS bucket for the tenant's
/// environment, independently of the per-request RPS bucket.
pub async fn rate_limit_eps(
    limiters: &RateLimiters,
    entitlements: &dyn Entitlements,
    auth: &AuthorizedAsTenant,
    event_count: usize,
) -> Result<(), RestApiError> {
    let (limiter, bucket) = match auth.tenant_env {
        TenantEnv::Production => (&limiters.prod.metering_eps, METERING_EPS_BUCKET),
        TenantEnv::NonProduction => (
            &limiters.non_prod.metering_eps,
            NON_PROD_METERING_EPS_BUCKET,
        ),
    };
    let per_second = entitlements
        .resolve(auth.organization_id)
        .await
        .metering_eps_rate_limit();
    let limiter = org_limiter(limiter, bucket, per_second.map(RateLimitConfig::per_second));

    match limiter.check_n(&auth.tenant_id, event_count as f64).await {
        RateLimitResult::Allowed { .. } => Ok(()),
        RateLimitResult::RateLimited { retry_after, .. } => {
            Err(RestApiError::RetryAfter(retry_after))
        }
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
    let limiter = org_limiter(limiter, bucket, per_minute.map(RateLimitConfig::per_minute));

    match limiter.check(&auth.tenant_id).await {
        RateLimitResult::Allowed { .. } => Ok(()),
        RateLimitResult::RateLimited { retry_after, .. } => {
            Err(RestApiError::RetryAfter(retry_after))
        }
    }
}

/// The environment's limiter, or one at the organization's own rate in the same bucket.
fn org_limiter(default: &RateLimiter, bucket: &str, rate: Option<RateLimitConfig>) -> RateLimiter {
    match rate {
        Some(rate) => default.with_config(rate.with_bucket(bucket)),
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

        let kept = org_limiter(default, USAGE_COSTS_BUCKET, None);
        assert_eq!(kept.config().capacity, 600.0);

        let overridden = org_limiter(
            default,
            USAGE_COSTS_BUCKET,
            Some(RateLimitConfig::per_minute(1_200)),
        );
        assert_eq!(overridden.config().capacity, 1_200.0);
        assert_eq!(overridden.config().refill_rate, 20.0);
        assert_eq!(
            overridden.config().bucket.as_deref(),
            default.config().bucket.as_deref()
        );
    }

    #[test]
    fn an_organization_eps_replaces_the_default_in_the_same_bucket() {
        let limiters =
            build_rest_rate_limiters(&RateLimiter::default(), &RestRateLimitConfig::default());
        let default = &limiters.prod.metering_eps;
        assert_eq!(default.config().capacity, 5_000.0);

        let overridden = org_limiter(
            default,
            METERING_EPS_BUCKET,
            Some(RateLimitConfig::per_second(50_000)),
        );
        assert_eq!(overridden.config().capacity, 50_000.0);
        assert_eq!(overridden.config().refill_rate, 50_000.0);
        assert_eq!(
            overridden.config().bucket.as_deref(),
            default.config().bucket.as_deref()
        );
    }
}
