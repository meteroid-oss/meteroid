//! Promotional-code tries counted in Postgres, for when the shared rate limiter cannot answer: a
//! customer gets a budget of failed tries either way, never an unlimited one. The two budgets are
//! independent, so a customer whose Redis budget is spent gets this one while Redis flaps.
use crate::StoreResult;
use crate::services::ServicesEdge;
use chrono::{DateTime, Duration, Utc};
use common_domain::ids::{CustomerId, TenantId};
use diesel_models::promo_code_attempts::PromoCodeAttemptRowNew;
use diesel_models::query::promo_code_attempts::{
    attempts_since, delete_attempt, lock_customer_attempts,
};
use scoped_futures::ScopedFutureExt;
use uuid::Uuid;

/// Failed tries a customer may make per rolling window: the Redis budget's burst, and its
/// long-run rate of 10 an hour.
pub const PROMO_CODE_ATTEMPTS: usize = 5;
pub const PROMO_CODE_WINDOW_MINUTES: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromoCodeTry {
    /// One try taken; release it once the code proves usable.
    Taken(Uuid),
    Limited {
        retry_after_seconds: i64,
    },
}

/// The answer for a customer whose tries in the window are `attempts` (oldest first).
fn judge(attempts: &[DateTime<Utc>], now: DateTime<Utc>) -> Option<i64> {
    if attempts.len() < PROMO_CODE_ATTEMPTS {
        return None;
    }
    let frees_at = attempts[attempts.len() - PROMO_CODE_ATTEMPTS]
        + Duration::minutes(PROMO_CODE_WINDOW_MINUTES);
    Some((frees_at - now).num_seconds().max(1))
}

impl ServicesEdge {
    /// Takes one try under the customer's try lock, so concurrent tries cannot all pass the count.
    pub async fn take_promo_code_attempt(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<PromoCodeTry> {
        self.store
            .transaction(|conn| {
                async move {
                    let now = common_domain::clock::now();
                    lock_customer_attempts(conn, tenant_id, customer_id).await?;
                    let attempts = attempts_since(
                        conn,
                        tenant_id,
                        customer_id,
                        now - Duration::minutes(PROMO_CODE_WINDOW_MINUTES),
                    )
                    .await?;
                    if let Some(retry_after_seconds) = judge(&attempts, now) {
                        return Ok(PromoCodeTry::Limited {
                            retry_after_seconds,
                        });
                    }
                    let id = Uuid::now_v7();
                    PromoCodeAttemptRowNew {
                        id,
                        tenant_id,
                        customer_id,
                        attempted_at: now,
                    }
                    .insert(conn)
                    .await?;
                    Ok(PromoCodeTry::Taken(id))
                }
                .scope_boxed()
            })
            .await
    }

    pub async fn release_promo_code_attempt(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        attempt: Uuid,
    ) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;
        Ok(delete_attempt(&mut conn, tenant_id, customer_id, attempt).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_frees_a_try_when_its_oldest_counted_try_ages_out() {
        let now = common_domain::clock::now();
        let tries = |ages_minutes: &[i64]| {
            ages_minutes
                .iter()
                .map(|age| now - Duration::minutes(*age))
                .collect::<Vec<_>>()
        };
        assert_eq!(judge(&tries(&[20, 10, 5, 1]), now), None);
        assert_eq!(judge(&tries(&[20, 10, 5, 2, 1]), now), Some(10 * 60));
        assert_eq!(judge(&tries(&[29, 28, 27, 26, 25, 1]), now), Some(2 * 60));
    }
}
