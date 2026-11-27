//! Tenant rows are cached in each process. A trigger on `tenant` notifies every change on
//! [`CHANNEL`], and each process listens, so a change made by one replica reaches them all.

use std::time::Duration;

use common_domain::ids::TenantId;
use futures::StreamExt;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_postgres::AsyncMessage;

use crate::Store;

pub const CHANNEL: &str = "meteroid_tenant_changed";

/// `None`: any tenant may have changed (the listener reconnected and may have missed some).
pub type TenantChange = Option<TenantId>;

pub fn channel() -> broadcast::Sender<TenantChange> {
    broadcast::channel(256).0
}

impl Store {
    /// Changes to forget in caches outside the store, such as the REST API keys'.
    pub fn subscribe_tenant_changes(&self) -> broadcast::Receiver<TenantChange> {
        self.tenant_changes.subscribe()
    }

    pub async fn forget_tenant(&self, change: TenantChange) {
        match change {
            Some(tenant_id) => {
                self.invalidate_tenant_cache(&tenant_id);
                self.invalidate_reporting_currency_cache(&tenant_id);
                self.keep_last_events.invalidate(&tenant_id);
            }
            None => {
                self.tenant_cache.clear();
                self.reporting_currency_cache.clear();
                self.keep_last_events.clear();
            }
        }
        let _ = self.tenant_changes.send(change);
    }
}

/// Listens for the process's lifetime, reconnecting with a capped backoff.
pub fn spawn_listener(store: Store) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = Duration::from_millis(500);
        loop {
            match listen(&store).await {
                Ok(()) => backoff = Duration::from_millis(500),
                Err(e) => log::warn!("Tenant change listener: {e}; retrying in {backoff:?}"),
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    })
}

/// Returns when the connection ends.
async fn listen(store: &Store) -> Result<(), tokio_postgres::Error> {
    let (client, mut connection) =
        tokio_postgres::connect(&store.database_url, crate::store::tls_connector()).await?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let pump = tokio::spawn(async move {
        let mut messages = futures::stream::poll_fn(move |cx| connection.poll_message(cx));
        while let Some(Ok(message)) = messages.next().await {
            if let AsyncMessage::Notification(n) = message
                && tx.send(n.payload().to_string()).is_err()
            {
                break;
            }
        }
    });
    client.batch_execute(&format!("LISTEN {CHANNEL}")).await?;
    store.forget_tenant(None).await;
    while let Some(payload) = rx.recv().await {
        match uuid::Uuid::parse_str(&payload) {
            Ok(id) => store.forget_tenant(Some(TenantId::from(id))).await,
            Err(_) => store.forget_tenant(None).await,
        }
    }
    pump.abort();
    drop(client);
    Ok(())
}
