#[derive(Debug, thiserror::Error, PartialEq, Clone)]
pub enum IngestError {
    #[error("transient error, please retry")]
    RetryableSinkError,
    #[error("maximum event size exceeded")]
    EventTooBig,
    #[error("invalid event could not be processed")]
    NonRetryableSinkError,
}
