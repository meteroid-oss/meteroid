#[derive(Debug, thiserror::Error)]
pub enum OpenstackAdapterError {
    #[error("Error running the rabbitmq consumer: {0:?}")]
    LapinError(#[source] lapin::Error),
    #[error("Serialization error: {0}")]
    SerializationError(String, #[source] serde_json::Error),
    #[error("Error processing events: {0}")]
    HandlerError(String),
    #[error("Error sinking events: {0}")]
    GrpcError(#[from] tonic::Status),
}
