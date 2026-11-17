use crate::utils::{backoff_duration, sleep_with_jitter};
use futures::FutureExt;
use meteroid_store::Services;
use std::sync::Arc;
use std::time::Duration;

/// Past each customer's own run timeout (inside its lease), a pass that hangs.
const TIMEOUT: Duration = Duration::from_secs(600);
const IDLE_POLL: Duration = Duration::from_secs(2);
const CLAIM: i64 = 16;

/// Runs the customers whose billing is due, as long as there are some (every replica: the claim
/// leases each customer to one).
pub async fn run_worker(services: Arc<Services>) {
    let mut consecutive_errors: u32 = 0;
    loop {
        // A panic past the controller's own catch is one failed pass, never the end of the loop.
        let pass = std::panic::AssertUnwindSafe(tokio::time::timeout(
            TIMEOUT,
            services.run_billing_controller(CLAIM),
        ))
        .catch_unwind()
        .await;
        let Ok(pass) = pass else {
            consecutive_errors += 1;
            log::error!("billing controller pass panicked");
            sleep_with_jitter(backoff_duration(consecutive_errors)).await;
            continue;
        };
        match pass {
            Ok(Ok(run)) => {
                consecutive_errors = 0;
                if run.failed > 0 {
                    log::warn!(
                        "billing controller: {} of {} customers failed",
                        run.failed,
                        run.customers
                    );
                }
                if run.customers == 0 {
                    sleep_with_jitter(IDLE_POLL).await;
                }
            }
            Ok(Err(err)) => {
                consecutive_errors += 1;
                log::error!("billing controller failed: {err:?}");
                sleep_with_jitter(backoff_duration(consecutive_errors)).await;
            }
            Err(_) => {
                consecutive_errors += 1;
                log::error!(
                    "billing controller timed out after {} seconds",
                    TIMEOUT.as_secs()
                );
                sleep_with_jitter(backoff_duration(consecutive_errors)).await;
            }
        }
    }
}
