//! Sends the renewal notices due (R-157, R-161), hourly: term renewals everywhere, the other
//! renewals for tenants whose plan opens the retention gate.
use common_domain::ids::{OrganizationId, TenantId};
use meteroid_app::features::{Entitlements, gates, tenant_gate_open};
use meteroid_store::Store;
use meteroid_store::services::renewals::{self, RetentionGate};
use std::sync::Arc;
use std::time::Duration;

const INTERVAL_SECS: u64 = 3600;

struct Gate(Arc<dyn Entitlements>);

#[async_trait::async_trait]
impl RetentionGate for Gate {
    async fn open(&self, tenant_id: TenantId, organization_id: OrganizationId) -> bool {
        tenant_gate_open(
            self.0.as_ref(),
            organization_id,
            tenant_id,
            gates::RETENTION,
        )
        .await
    }
}

pub async fn run_renewal_notices_worker(store: Arc<Store>, entitlements: Arc<dyn Entitlements>) {
    let gate = Gate(entitlements);
    loop {
        let today = common_clock::now().date_naive();
        match renewals::sweep(&store, &gate, today).await {
            Ok(report) if report.team + report.customer == 0 => {}
            Ok(report) => log::info!(
                "Renewal notices: {} to the team, {} to customers",
                report.team,
                report.customer
            ),
            Err(err) => log::error!("Renewal notices failed: {err:?}"),
        }
        let jitter = Duration::from_secs(rand::random::<u64>() % 300);
        tokio::time::sleep(Duration::from_secs(INTERVAL_SECS) + jitter).await;
    }
}
