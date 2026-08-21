use thiserror::Error;

#[derive(Error, Debug)]
pub enum LockError {
    #[error("Failed to acquire lock")]
    AcquireError,
    #[error("Failed to release lock")]
    ReleaseError,
}

#[derive(Error, Debug)]
pub enum LeaderError {
    #[error("Leader election failed: {0}")]
    Election(String),
}
