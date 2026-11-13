pub mod http;
pub mod outcome;

use std::sync::Arc;
use std::time::Duration;

use common_logging::GLOBAL_METER;
use egress_http::{EgressClient, EgressError};
use meteroid_store::Store;
use meteroid_store::domain::webhooks_out::WebhookDelivery;
use meteroid_store::repositories::WebhookOutInterface;
use meteroid_store::services::webhooks::out::schedule::{
    DEFAULT_AUTO_DISABLE_AFTER_HOURS, EndpointHealth, random_jitter,
};
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram};
use secrecy::SecretString;
use tokio::task::JoinSet;

use crate::utils::sleep_with_jitter;
use crate::webhook_delivery::http::{DeliveryRequest, EgressUnavailable};
use crate::webhook_delivery::outcome::{AttemptContext, cancelled_outcome, decide_outcome};
use meteroid_app::services::egress::{Egress, EgressPurpose};

/// How long a claimed delivery stays IN_FLIGHT before another worker may reclaim it.
const LEASE_SECS: i32 = 90;
/// Fallback poll while idle; a busy worker claims again as soon as a slot frees up.
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const PURGE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// Lets a fresh process settle (and a rolling deploy finish) before the first purge.
const FIRST_PURGE_DELAY: Duration = Duration::from_secs(10 * 60);
/// Retention deletes in bounded steps so it never locks the table for minutes.
const PURGE_CHUNK_SIZE: i64 = 2_000;
/// Retry delay while the egress proxy is down.
const EGRESS_RETRY_DELAY: Duration = Duration::from_secs(30);
/// Shown on the delivery while it waits; the proxy's details stay in the server log.
const EGRESS_UNAVAILABLE_MESSAGE: &str =
    "delivery delayed: outbound delivery is temporarily unavailable";

static DELIVERIES_TOTAL: std::sync::LazyLock<Counter<u64>> = std::sync::LazyLock::new(|| {
    GLOBAL_METER
        .u64_counter("webhook_deliveries_total")
        .with_description("Outbound webhook delivery attempts by outcome")
        .build()
});

static DELIVERY_DURATION: std::sync::LazyLock<Histogram<f64>> = std::sync::LazyLock::new(|| {
    GLOBAL_METER
        .f64_histogram("webhook_delivery_duration_ms")
        .with_description("Outbound webhook attempt duration in milliseconds")
        .with_unit("ms")
        .build()
});

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TickStats {
    pub reclaimed: usize,
    pub claimed: usize,
    pub succeeded: usize,
    pub failed: usize,
}

pub struct WebhookDeliveryWorker {
    store: Arc<Store>,
    egress: EgressClient,
    concurrency: usize,
    retention_days: i64,
    /// False when private destinations are allowed: a tenant could then point an
    /// endpoint at an internal service and read its answers back from the delivery log.
    keep_response_bodies: bool,
}

impl WebhookDeliveryWorker {
    pub fn new(store: Arc<Store>, egress: &Egress) -> Result<Self, EgressError> {
        let config = egress.config();
        let egress = egress.client(EgressPurpose::Webhooks)?;
        let keep_response_bodies = !egress.allows_private_destinations();

        Ok(Self {
            store,
            egress,
            concurrency: config.delivery_concurrency.max(1),
            retention_days: config.retention_days.0,
            keep_response_bodies,
        })
    }

    /// Keeps up to `concurrency` deliveries in flight; a slow endpoint only occupies
    /// its own slot and never delays claiming work for the others.
    pub async fn run(self) {
        log::info!(
            "Starting native webhook delivery worker (concurrency {})",
            self.concurrency
        );

        tokio::spawn(run_retention(self.store.clone(), self.retention_days));

        let mut tasks: JoinSet<bool> = JoinSet::new();

        loop {
            while let Some(result) = tasks.try_join_next() {
                count_task_result(result, &mut TickStats::default());
            }

            let available = self.concurrency.saturating_sub(tasks.len());
            let claimed = if available > 0 {
                self.claim_batch(available).await.1
            } else {
                Vec::new()
            };
            let idle = available == 0 || claimed.len() < available;

            for delivery in claimed {
                self.spawn_delivery(&mut tasks, delivery);
            }

            if idle {
                tokio::select! {
                    Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                        count_task_result(result, &mut TickStats::default());
                    }
                    _ = sleep_with_jitter(POLL_INTERVAL) => {}
                }
            }
        }
    }

    /// One poll: reclaim expired leases, claim what is due, deliver it and wait for every
    /// delivery to finish. Exposed so tests can drive the worker without racing the loop.
    pub async fn run_tick(&self) -> TickStats {
        let mut stats = TickStats::default();

        let (reclaimed, claimed) = self.claim_batch(self.concurrency).await;
        stats.reclaimed = reclaimed;
        stats.claimed = claimed.len();

        let mut tasks: JoinSet<bool> = JoinSet::new();
        for delivery in claimed {
            self.spawn_delivery(&mut tasks, delivery);
        }

        while let Some(result) = tasks.join_next().await {
            count_task_result(result, &mut stats);
        }

        stats
    }

    async fn claim_batch(&self, limit: usize) -> (usize, Vec<WebhookDelivery>) {
        let reclaimed = match self.store.reclaim_expired_webhook_deliveries().await {
            Ok(count) => count,
            Err(e) => {
                log::error!("Failed to reclaim expired webhook deliveries: {e:?}");
                0
            }
        };

        let claimed = match self
            .store
            .claim_due_webhook_deliveries(limit as i64, LEASE_SECS)
            .await
        {
            Ok(claimed) => claimed,
            Err(e) => {
                log::error!("Failed to claim due webhook deliveries: {e:?}");
                Vec::new()
            }
        };

        (reclaimed, claimed)
    }

    fn spawn_delivery(&self, tasks: &mut JoinSet<bool>, delivery: WebhookDelivery) {
        let store = self.store.clone();
        let egress = self.egress.clone();

        tasks.spawn(attempt_webhook_delivery(
            store,
            egress,
            delivery,
            self.keep_response_bodies,
        ));
    }
}

/// Off the claim loop: a long purge must never delay deliveries.
async fn run_retention(store: Arc<Store>, retention_days: i64) {
    tokio::time::sleep(FIRST_PURGE_DELAY).await;

    loop {
        purge_expired_messages(&store, retention_days).await;
        sleep_with_jitter(PURGE_INTERVAL).await;
    }
}

async fn purge_expired_messages(store: &Store, retention_days: i64) {
    let cutoff = common_domain::clock::wall().naive_utc() - chrono::Duration::days(retention_days);
    match store
        .purge_webhook_messages_older_than(cutoff, PURGE_CHUNK_SIZE)
        .await
    {
        Ok(0) => {}
        Ok(count) => log::info!("Purged {count} webhook message(s) older than {cutoff}"),
        Err(e) => log::error!("Failed to purge old webhook messages: {e:?}"),
    }
}

fn count_task_result(result: Result<bool, tokio::task::JoinError>, stats: &mut TickStats) {
    match result {
        Ok(true) => stats.succeeded += 1,
        Ok(false) => stats.failed += 1,
        Err(e) => {
            stats.failed += 1;
            log::error!("Webhook delivery task panicked: {e:?}");
        }
    }
}

#[tracing::instrument(skip_all, fields(
    tenant_id = %delivery.tenant_id,
    delivery_id = %delivery.id,
    endpoint_id = %delivery.endpoint_id,
    message_id = %delivery.message_id,
))]
async fn attempt_webhook_delivery(
    store: Arc<Store>,
    egress: EgressClient,
    delivery: WebhookDelivery,
    keep_response_body: bool,
) -> bool {
    let attempt_number = delivery.attempt_count;
    let Some(lease_expires_at) = delivery.lease_expires_at else {
        log::error!(
            "Webhook delivery {} was claimed without a lease",
            delivery.id
        );
        return false;
    };

    let endpoint = match store
        .get_webhook_endpoint_for_delivery(delivery.endpoint_id)
        .await
    {
        Ok(Some(endpoint)) => endpoint,
        Ok(None) => {
            return record(
                &store,
                cancelled_outcome(
                    delivery.id,
                    delivery.endpoint_id,
                    attempt_number,
                    lease_expires_at,
                    "endpoint no longer exists",
                ),
            )
            .await;
        }
        Err(e) => {
            log::error!(
                "Failed to load endpoint {} for webhook delivery {}: {e:?}",
                delivery.endpoint_id,
                delivery.id
            );
            return false;
        }
    };

    if !endpoint.endpoint.is_deliverable() {
        return record(
            &store,
            cancelled_outcome(
                delivery.id,
                delivery.endpoint_id,
                attempt_number,
                lease_expires_at,
                "endpoint is disabled or archived",
            ),
        )
        .await;
    }

    let message = match store.get_webhook_message(delivery.message_id).await {
        Ok(message) => message,
        Err(e) => {
            log::error!(
                "Failed to load message {} for webhook delivery {}: {e:?}",
                delivery.message_id,
                delivery.id
            );
            return false;
        }
    };

    let mut secrets: Vec<SecretString> = vec![endpoint.secret.clone()];
    secrets.extend(endpoint.previous_secret.clone());

    let attempt = match http::deliver(
        &egress,
        DeliveryRequest {
            url: endpoint.endpoint.url.clone(),
            message_id: message.id,
            payload: message.payload,
            secrets,
            svix_headers: endpoint.endpoint.legacy_svix_headers,
            keep_response_body,
            custom_headers: endpoint.headers,
        },
    )
    .await
    {
        Ok(attempt) => attempt,
        Err(EgressUnavailable(reason)) => {
            return defer_for_egress(&store, &delivery, lease_expires_at, &reason).await;
        }
    };

    let jitter = random_jitter(&mut rand::rng());

    let outcome = decide_outcome(
        AttemptContext {
            delivery_id: delivery.id,
            endpoint_id: delivery.endpoint_id,
            attempt_number,
            lease_expires_at,
            duration_ms: attempt.duration_ms,
            health: EndpointHealth {
                consecutive_failures: endpoint.endpoint.consecutive_failures,
                failing_since: endpoint.endpoint.failing_since,
                unreachable_streak: endpoint.endpoint.unreachable_streak,
            },
            now: common_domain::clock::wall().naive_utc(),
            jitter,
            auto_disable_after_hours: DEFAULT_AUTO_DISABLE_AFTER_HOURS,
        },
        attempt.result,
    );

    let succeeded = outcome.succeeded;
    let status_label = outcome
        .response_status
        .map(|s| s.to_string())
        .unwrap_or_else(|| "none".to_string());

    DELIVERIES_TOTAL.add(
        1,
        &[
            KeyValue::new("outcome", if succeeded { "success" } else { "failure" }),
            KeyValue::new("status", status_label),
        ],
    );
    DELIVERY_DURATION.record(attempt.duration_ms as f64, &[]);

    record(&store, outcome).await
}

async fn defer_for_egress(
    store: &Store,
    delivery: &WebhookDelivery,
    lease_expires_at: chrono::NaiveDateTime,
    reason: &str,
) -> bool {
    log::error!(
        "Webhook delivery {} deferred, the egress proxy is unavailable: {reason}",
        delivery.id
    );
    DELIVERIES_TOTAL.add(
        1,
        &[
            KeyValue::new("outcome", "deferred"),
            KeyValue::new("status", "egress_unavailable"),
        ],
    );

    let delay = EGRESS_RETRY_DELAY.mul_f64(1.0 + random_jitter(&mut rand::rng()));
    let next_attempt_at = common_domain::clock::wall().naive_utc()
        + chrono::Duration::from_std(delay).unwrap_or_default();

    if let Err(e) = store
        .defer_webhook_delivery(
            delivery.id,
            delivery.attempt_count,
            lease_expires_at,
            next_attempt_at,
            EGRESS_UNAVAILABLE_MESSAGE.to_string(),
        )
        .await
    {
        log::error!("Failed to defer webhook delivery {}: {e:?}", delivery.id);
    }
    false
}

async fn record(
    store: &Arc<Store>,
    outcome: meteroid_store::domain::webhooks_out::WebhookAttemptOutcome,
) -> bool {
    let delivery_id = outcome.delivery_id;
    let succeeded = outcome.succeeded;

    match store.record_webhook_attempt(outcome).await {
        Ok(()) => succeeded,
        Err(e) => {
            log::error!("Failed to record webhook attempt for delivery {delivery_id}: {e:?}");
            false
        }
    }
}
