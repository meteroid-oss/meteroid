use meteroid_store::Store;
use meteroid_store::repositories::dead_letter::DeadLetterInterface;
use std::sync::Arc;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(30);
const COOLDOWN: Duration = Duration::from_secs(300); // 5 minutes per queue
const ADVISORY_LOCK_ID: i64 = 0x4D_544F_444C_5100; // "MTODLQ\0" as i64

type AlertResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub async fn run_dlq_alert_worker(store: Arc<Store>, webhook_url: String) {
    log::info!("Starting DLQ alert worker (webhook configured)");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("Failed to create HTTP client for DLQ alerts");

    loop {
        if let Err(e) = check_and_alert(&store, &client, &webhook_url).await {
            log::warn!("DLQ alert check failed: {e}");
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn check_and_alert(
    store: &Store,
    client: &reqwest::Client,
    webhook_url: &str,
) -> AlertResult {
    let mut conn = store
        .get_conn()
        .await
        .map_err(|e| format!("{e:?}"))?;

    // Try to acquire advisory lock — only one instance runs this at a time
    let locked: bool = diesel_async::RunQueryDsl::get_result(
        diesel::sql_query(format!(
            "SELECT pg_try_advisory_lock({ADVISORY_LOCK_ID})"
        )),
        &mut *conn,
    )
    .await
    .map(|row: AdvisoryLockResult| row.pg_try_advisory_lock)
    .unwrap_or(false);

    if !locked {
        return Ok(());
    }

    let result = do_alert_check(store, client, webhook_url).await;

    let _ = diesel_async::RunQueryDsl::execute(
        diesel::sql_query(format!(
            "SELECT pg_advisory_unlock({ADVISORY_LOCK_ID})"
        )),
        &mut *conn,
    )
    .await;

    result
}

async fn do_alert_check(
    store: &Store,
    client: &reqwest::Client,
    webhook_url: &str,
) -> AlertResult {
    let now = chrono::Utc::now().naive_utc();
    let cooldown_threshold = now - chrono::Duration::seconds(COOLDOWN.as_secs() as i64);

    let stats = store
        .dead_letter_queue_stats()
        .await
        .map_err(|e| format!("{e:?}"))?;

    for stat in stats {
        if stat.pending_count == 0 {
            continue;
        }

        let last_alerted = store
            .get_dead_letter_alert_state(&stat.queue)
            .await
            .unwrap_or(None);

        if let Some(last) = last_alerted
            && last > cooldown_threshold {
                continue;
            }

        let message = format!(
            "**[Dead Letter Queue]** `{}`: **{}** pending message(s) ({} requeued, {} discarded total)",
            stat.queue, stat.pending_count, stat.requeued_count, stat.discarded_count,
        );

        let payload = serde_json::json!({ "content": message });

        match client.post(webhook_url).json(&payload).send().await {
            Ok(resp) if resp.status().is_success() => {
                log::info!("DLQ alert sent for queue {}", stat.queue);
                let _ = store.upsert_dead_letter_alert_state(&stat.queue).await;
            }
            Ok(resp) => {
                log::warn!(
                    "DLQ alert webhook returned {}: {}",
                    resp.status(),
                    resp.text().await.unwrap_or_default()
                );
            }
            Err(e) => {
                log::warn!("DLQ alert webhook failed for queue {}: {e}", stat.queue);
            }
        }
    }

    Ok(())
}

#[derive(diesel::QueryableByName)]
struct AdvisoryLockResult {
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pg_try_advisory_lock: bool,
}
