//! Worker that hard-deletes `PendingDeletion` tenants: purges their ClickHouse
//! (metering, then the edition's analytics) and object-store data, deletes their Postgres rows (cascade), then removes
//! any archived organization left with no tenants. Schedule is configurable.

use std::sync::Arc;

use common_domain::ids::TenantId;
use distributed_lock::LeaderElection;
use error_stack::ResultExt;
use meteroid_store::Store;
use meteroid_store::clients::usage::UsageClient;
use meteroid_store::errors::StoreError;
use meteroid_store::hooks::AnalyticsHook;
use meteroid_store::repositories::OrganizationsInterface;
use meteroid_store::repositories::tenants::TenantInterface;

use meteroid_app::config::ParsedCron;
use meteroid_app::services::storage::ObjectStoreService;

/// Tenants purged per batch; the worker drains until a short batch comes back.
const BATCH_SIZE: i64 = 1000;

pub async fn run_tenant_deletion_cleanup_worker(
    store: Arc<Store>,
    usage_client: Arc<dyn UsageClient>,
    analytics: Arc<dyn AnalyticsHook>,
    object_store: Arc<dyn ObjectStoreService>,
    elector: Arc<dyn LeaderElection>,
    schedule: ParsedCron,
) {
    super::run_scheduled("tenant deletion cleanup", schedule, elector, move || {
        let store = store.clone();
        let usage_client = usage_client.clone();
        let analytics = analytics.clone();
        let object_store = object_store.clone();
        async move { run_once(&store, &usage_client, &analytics, &object_store).await }
    })
    .await;
}

async fn run_once(
    store: &Arc<Store>,
    usage_client: &Arc<dyn UsageClient>,
    analytics: &Arc<dyn AnalyticsHook>,
    object_store: &Arc<dyn ObjectStoreService>,
) -> meteroid_store::StoreResult<()> {
    // A failing batch propagates out and retries next run instead of looping.
    loop {
        let tenant_ids = store.list_tenants_pending_deletion(BATCH_SIZE).await?;
        if tenant_ids.is_empty() {
            break;
        }

        purge_deleted_tenants(store, usage_client, analytics, object_store, &tenant_ids).await?;

        if (tenant_ids.len() as i64) < BATCH_SIZE {
            break;
        }
    }

    // Remove archived organizations whose tenants are all gone.
    store.delete_empty_archived_organizations().await?;
    Ok(())
}

#[tracing::instrument(skip_all)]
async fn purge_deleted_tenants(
    store: &Store,
    usage_client: &Arc<dyn UsageClient>,
    analytics: &Arc<dyn AnalyticsHook>,
    object_store: &Arc<dyn ObjectStoreService>,
    tenant_ids: &[TenantId],
) -> meteroid_store::StoreResult<()> {
    // External stores (idempotent) before Postgres, so a failure leaves the
    // tenants pending rather than orphaning ClickHouse/object-store data.
    usage_client.purge_tenants(tenant_ids).await?;
    analytics.tenants_purged(tenant_ids).await?;
    for tenant_id in tenant_ids {
        object_store
            .delete_tenant(*tenant_id)
            .await
            .change_context(StoreError::ObjectStoreError)?;
    }
    store.hard_delete_tenants(tenant_ids).await?;
    for tenant_id in tenant_ids {
        tracing::info!(tenant_id = %tenant_id, "tenant deletion cleanup: purged tenant {tenant_id}");
    }
    log::info!(
        "tenant deletion cleanup: purged {} tenant(s)",
        tenant_ids.len()
    );
    Ok(())
}
