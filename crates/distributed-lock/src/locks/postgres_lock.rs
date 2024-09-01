use crate::errors::LockError;
use crate::locks::DistributedLock;
use diesel::sql_types::Bool;
use diesel::{sql_query, QueryableByName};
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

#[async_trait::async_trait]
impl<'a> DistributedLock for PostgresLock<'a> {
    async fn acquire(&mut self) -> Result<bool, LockError> {
        // Create the raw SQL query using Diesel's `sql_query`
        sql_query("SELECT pg_try_advisory_lock($1)")
            .bind::<diesel::sql_types::BigInt, _>(self.lock_key.get())
            .get_result::<LockResult>(self.client)
            .await
            .map(|row| row.acquired) // Extract the first (and only) field from the tuple
            .map_err(|_| LockError::AcquireError)
    }

    async fn release(&mut self) -> Result<(), LockError> {
        sql_query("SELECT pg_advisory_unlock($1)")
            .bind::<diesel::sql_types::BigInt, _>(self.lock_key.get())
            .execute(self.client)
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
