use thiserror::Error;

#[derive(Error, Debug)]
pub enum LockError {
    #[error("Failed to acquire lock")]
    AcquireError,
    #[error("Failed to release lock")]
    ReleaseError,
}
