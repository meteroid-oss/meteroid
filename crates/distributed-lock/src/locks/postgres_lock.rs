use crate::errors::LockError;
use crate::locks::DistributedLock;
use diesel::sql_types::Bool;
use diesel::{QueryableByName, sql_query};
use diesel_async::{AsyncPgConnection, RunQueryDsl};

pub struct PostgresLock<'a> {
    client: &'a mut AsyncPgConnection,
    lock_key: LockKey,
}

impl<'a> PostgresLock<'a> {
    pub fn new(client: &'a mut AsyncPgConnection, lock_key: LockKey) -> Self {
        PostgresLock { client, lock_key }
    }
}

#[derive(QueryableByName)]
struct LockResult {
    #[diesel(sql_type = Bool)]
    acquired: bool,
}

// Use Postgres advisory locks for our locking mechanism.
// This is just an example and can be enhanced further.
#[async_trait::async_trait]
impl DistributedLock for PostgresLock<'_> {
    async fn acquire(&mut self) -> Result<bool, LockError> {
        sql_query("SELECT pg_try_advisory_lock($1) as acquired")
            .bind::<diesel::sql_types::BigInt, _>(self.lock_key.get())
            .get_result::<LockResult>(self.client)
            .await
            .map(|row| row.acquired)
            .map_err(|e| {
                log::error!("Failed to acquire lock: {e:?}");
                LockError::AcquireError
            })
    }

    async fn release(&mut self) -> Result<(), LockError> {
        sql_query("SELECT pg_advisory_unlock($1)")
            .bind::<diesel::sql_types::BigInt, _>(self.lock_key.get())
            .execute(self.client)
            .await
            .map(|_| ())
            .map_err(|e| {
                log::error!("Failed to release lock: {e:?}");
                LockError::ReleaseError
            })
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
