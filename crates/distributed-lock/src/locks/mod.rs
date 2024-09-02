use crate::errors::LockError;
use async_trait::async_trait;

#[async_trait]
pub trait DistributedLock {
    async fn acquire(&mut self) -> Result<bool, LockError>;
    async fn release(&mut self) -> Result<(), LockError>;
}

#[cfg(feature = "postgres-support")]
mod postgres_lock;
#[cfg(feature = "postgres-support")]
pub use postgres_lock::LockKey;
#[cfg(feature = "postgres-support")]
pub use postgres_lock::PostgresLock;
