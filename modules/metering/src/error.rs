use crate::connectors::errors::ConnectorError;
use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use error_stack::Report;
use std::error::Error;
use thiserror::Error;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum MeteringApiError {
    #[error(transparent)]
    #[code(Internal)]
    ConnectorError(#[from] Box<dyn Error>),
}

impl From<Report<ConnectorError>> for MeteringApiError {
    fn from(value: Report<ConnectorError>) -> Self {
        log::error!("{value:?}");

        let err = Box::new(value.into_error());
        Self::ConnectorError(err)
    }
}
