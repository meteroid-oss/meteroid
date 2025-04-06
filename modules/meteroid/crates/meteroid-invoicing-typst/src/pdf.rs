// src/pdf.rs (updated to include Typst PDF generator)
use crate::errors::{InvoicingError, InvoicingResult};
use crate::model::Invoice;
use crate::typst_render::TypstInvoiceRenderer;
use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;

#[async_trait]
pub trait PdfGenerator: Send + Sync {
    async fn generate_pdf(&self, invoice_html: &str) -> InvoicingResult<Bytes>;
}

pub struct TypstPdfGenerator {
    renderer: TypstInvoiceRenderer,
}

impl TypstPdfGenerator {
    pub fn new() -> InvoicingResult<Self> {
        let renderer = TypstInvoiceRenderer::new()?;
        Ok(TypstPdfGenerator { renderer })
    }

    // Direct method to generate PDF from an Invoice object
    pub async fn generate_pdf_from_invoice(&self, invoice: &Invoice) -> InvoicingResult<Bytes> {
        let pdf_data = self.renderer.render_invoice(invoice)?;
        Ok(Bytes::from(pdf_data))
    }
}

// We need to implement a compatibility layer for the PdfGenerator trait
#[async_trait]
impl PdfGenerator for TypstPdfGenerator {
    // This implementation is just a compatibility layer for the existing interface
    // It doesn't actually use the invoice_html parameter
    async fn generate_pdf(&self, _invoice_html: &str) -> InvoicingResult<Bytes> {
        Err(InvoicingError::PdfGenerationError(
            "This method is for compatibility only. Use generate_pdf_from_invoice instead.".to_string(),
        ))
    }
}
