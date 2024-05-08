#[derive(Debug, thiserror::Error)]
pub enum SeederError {
    #[error("Initialization Error")]
    InitializationError,
    #[error("Store Error")]
    StoreError,
    #[error("Temporary Error")]
    TempError,
}
