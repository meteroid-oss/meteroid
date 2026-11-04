use distributed_lock::LeaderElection;
use meteroid_app::config::ParsedCron;
use meteroid_store::Services;
use std::sync::Arc;

/// How stale a draft's `data_updated_at` must be before the sweep recomputes it. Matches the
/// cron cadence: with an hourly run, a draft refreshed on the previous pass is skipped on the
/// next one only if that pass ran late, which is the intent.
const STALE_AFTER_HOURS: i32 = 1;
/// Rows per page. Small enough that a page's worth of metering queries stays bounded, large
/// enough that a healthy tenant drains in one query.
const PAGE_SIZE: u32 = 100;
/// Ceiling for one pass. A backlog (a tenant switching a large book to manual finalization)
/// drains across consecutive runs instead of holding a connection for the whole hour.
const MAX_INVOICES_PER_RUN: usize = 2_000;
/// How long past its `invoice_date` a draft stays worth recomputing. The usage window is closed,
/// so once late events stop arriving the figure cannot move and every further pass is a metering
/// query for nothing — and an `auto_advance = false` draft lingers indefinitely. Comfortably
/// covers a normal grace period (hours) plus late imports. Raise it only for a tenant whose
/// grace runs longer than this; a draft past the horizon still shows a stale preview, never a
/// stale bill, since finalization recomputes from current usage.
const REFRESH_HORIZON_DAYS: i64 = 7;

/// Hourly recompute of subscription-period drafts so their usage lines track what has been
/// ingested since the draft was created.
///
/// The case this exists for is `auto_advance = false`: those drafts get no scheduled
/// finalization (see `bill.rs`), so nothing ever recomputes them and late usage stays invisible
/// until an operator finalizes. Drafts inside the grace window are refreshed at grace expiry
/// anyway; the sweep only makes the displayed figure honest in the meantime.
///
/// This moves what an operator *sees*, never what is billed: every finalization path recomputes
/// from current usage on its own.
///
/// Leader-elected: the scan takes no claim, so every replica would recompute the same drafts and
/// multiply metering traffic by the replica count.
pub async fn run_draft_refresh_worker(
    services: Arc<Services>,
    elector: Arc<dyn LeaderElection>,
    schedule: ParsedCron,
) {
    super::run_scheduled("draft refresh", schedule, elector, move || {
        let services = services.clone();
        async move { run_once(&services).await }
    })
    .await;
}

async fn run_once(services: &Arc<Services>) -> meteroid_store::StoreResult<()> {
    let outcome = services
        .refresh_stale_draft_invoices(
            STALE_AFTER_HOURS,
            REFRESH_HORIZON_DAYS,
            PAGE_SIZE,
            MAX_INVOICES_PER_RUN,
        )
        .await?;

    if outcome.did_work() {
        log::info!(
            "Draft refresh: {} refreshed, {} failed, out of {} scanned{}",
            outcome.refreshed,
            outcome.failed,
            outcome.scanned,
            if outcome.truncated {
                " (capped, backlog continues next run)"
            } else {
                ""
            }
        );
    }

    Ok(())
}
