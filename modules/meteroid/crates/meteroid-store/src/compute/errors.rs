use thiserror::Error;

#[derive(Error, Debug)]
pub enum ComputeError {
    #[error("Internal Error occurred while computing invoice lines")]
    InternalError,
    #[error("Conversion Error occurred while computing invoice lines")]
    ConversionError,
    #[error("Invalid invoice date provided to the Compute service")]
    InvalidInvoiceDate,
    #[error("Invalid period provided to the Compute service")]
    InvalidPeriod,
    #[error("Metric not found")]
    MetricNotFound,
    #[error("Metering gRPC error occurred while fetching usage data")]
    MeteringGrpcError,
    #[error("Metering returned too many results")]
    TooManyResults,
}
