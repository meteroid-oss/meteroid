use crate::errors::LockError;
use crate::locks::DistributedLock;
use deadpool_postgres::Client;

pub struct PostgresLock<'a> {
    client: &'a Client,
    lock_key: LockKey,
}

impl<'a> PostgresLock<'a> {
    pub fn new(client: &'a Client, lock_key: LockKey) -> Self {
        PostgresLock { client, lock_key }
    }
}

#[async_trait::async_trait]
impl<'a> DistributedLock for PostgresLock<'a> {
    async fn acquire(&self) -> Result<bool, LockError> {
        // Use Postgres advisory locks for our locking mechanism.
        // This is just an example and can be enhanced further.
        self.client
            .query_one("SELECT pg_try_advisory_lock($1)", &[&self.lock_key.get()])
            .await
            .map(|row| row.get(0))
            .map_err(|_| LockError::AcquireError)
    }

    async fn release(&self) -> Result<(), LockError> {
        self.client
            .query_one("SELECT pg_advisory_unlock($1)", &[&self.lock_key.get()])
            .await
            .map(|_| ())
            .map_err(|_| LockError::ReleaseError)
    }
}

// global place to define lock keys in a single place
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy)]
pub enum LockKey {
    InvoicingDraft,
    InvoicingPendingStatus,
    InvoicingIssue,
    InvoicingFinalize,
    InvoicingPrice,
    CurrencyRates,
}

impl LockKey {
    pub fn get(&self) -> i64 {
        match self {
            LockKey::InvoicingDraft => 1000,
            LockKey::InvoicingPendingStatus => 1001,
            LockKey::InvoicingIssue => 1002,
            LockKey::InvoicingFinalize => 1003,
            LockKey::InvoicingPrice => 1004,
            LockKey::CurrencyRates => 2000,
        }
    }
}
