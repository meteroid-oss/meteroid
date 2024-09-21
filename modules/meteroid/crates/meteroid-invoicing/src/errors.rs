#[derive(Debug, thiserror::Error, PartialEq, Clone)]
pub enum InvoicingError {
    #[error("Error during internationalization: {0}")]
    I18nError(String),
    #[error("Failed to generate PDF: {0}")]
    PdfGenerationError(String),
    #[error("Failed to store PDF: {0}")]
    StorageError(String),
}

pub type InvoicingResult<T> = std::result::Result<T, InvoicingError>;
