#[derive(Debug, thiserror::Error, PartialEq, Clone)]
pub enum InvoicingError {
    #[error("Error during internationalization: {0}")]
    I18nError(String),
    #[error("Failed to generate invoice document: {0}")]
    InvoiceGenerationError(String),
    #[error("Failed to generate PDF: {0}")]
    PdfGenerationError(String),
    #[error("Failed to generate SVG: {0}")]
    SvgGenerationError(String),
    #[error("Failed to generate XML: {0}")]
    XmlGenerationError(String),
    #[error("Failed to store PDF: {0}")]
    StorageError(String),
}

pub type InvoicingResult<T> = std::result::Result<T, InvoicingError>;
