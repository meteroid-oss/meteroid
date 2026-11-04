//! The per-customer budget of promotional-code tries, shared by redemptions and checkout codes.
//! Redis counts it; when Redis cannot answer, Postgres does, so the budget never fails open. The
//! two counts are independent: while Redis flaps a customer may get both budgets, never more.
use crate::wallets::WalletApiError;
use common_domain::ids::{CustomerId, TenantId};
use meteroid_store::Services;
use meteroid_store::services::promo_code_attempts::PromoCodeTry;
use rate_limiter::{RateLimitConfig, RateLimitResult, RateLimiter};
use tonic::Status;
use uuid::Uuid;

/// A burst of 5 codes, then 10 an hour, per customer.
fn redeem_code_limit() -> RateLimitConfig {
    RateLimitConfig {
        capacity: 5.0,
        refill_rate: 10.0 / 3600.0,
        window_seconds: 3600,
        bucket: Some("portal:redeem_code".to_owned()),
    }
}

pub(crate) struct PromoCodeLimiter {
    redis: RateLimiter,
    services: Services,
}

/// One try taken from the budget, held until the code is judged.
pub(crate) enum CodeTry {
    Redis(String),
    Postgres(Uuid),
}

impl PromoCodeLimiter {
    pub(crate) fn new(rate_limiter: &RateLimiter, services: Services) -> Self {
        Self {
            redis: rate_limiter.with_config(redeem_code_limit()),
            services,
        }
    }

    pub(crate) async fn try_code(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> Result<CodeTry, Status> {
        let key = format!("{tenant_id}:{customer_id}");
        match self.redis.try_check(&key).await {
            Some(RateLimitResult::Allowed { .. }) => Ok(CodeTry::Redis(key)),
            Some(RateLimitResult::RateLimited { retry_after, .. }) => {
                Err(exhausted(i64::try_from(retry_after).unwrap_or(i64::MAX)))
            }
            None => match self
                .services
                .take_promo_code_attempt(tenant_id, customer_id)
                .await
                .map_err(Into::<WalletApiError>::into)?
            {
                PromoCodeTry::Taken(id) => Ok(CodeTry::Postgres(id)),
                PromoCodeTry::Limited {
                    retry_after_seconds,
                } => Err(exhausted(retry_after_seconds)),
            },
        }
    }

    /// Gives the try back when the code proved usable: only failed tries count.
    pub(crate) async fn settle(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        code_try: CodeTry,
        usable: bool,
    ) {
        if !usable {
            return;
        }
        match code_try {
            CodeTry::Redis(key) => self.redis.refund(key).await,
            CodeTry::Postgres(id) => {
                if let Err(error) = self
                    .services
                    .release_promo_code_attempt(tenant_id, customer_id, id)
                    .await
                {
                    tracing::warn!(?error, "could not give back a usable promo code try");
                }
            }
        }
    }
}

fn exhausted(retry_after_seconds: i64) -> Status {
    Status::resource_exhausted(format!(
        "Too many codes tried. Try again in {} minutes.",
        retry_after_seconds.max(1).saturating_add(59) / 60
    ))
}
