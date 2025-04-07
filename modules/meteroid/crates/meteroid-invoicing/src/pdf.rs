use crate::errors::{InvoicingError, InvoicingResult};
use crate::model::Invoice;
use crate::typst_render::TypstInvoiceRenderer;
use async_trait::async_trait;
use bytes::Bytes;
use typst::foundations::Smart;
use typst_pdf::{self, PdfOptions, PdfStandard, PdfStandards};

#[async_trait]
pub trait PdfGenerator: Send + Sync {
    async fn generate_pdf(&self, invoice: &Invoice) -> InvoicingResult<Bytes>;
}

pub struct TypstPdfGenerator {
    renderer: TypstInvoiceRenderer,
}

impl TypstPdfGenerator {
    pub fn new() -> InvoicingResult<Self> {
        let renderer = TypstInvoiceRenderer::new()?;
        Ok(TypstPdfGenerator { renderer })
    }
}
#[async_trait]
impl PdfGenerator for TypstPdfGenerator {
    // Direct method to generate PDF from an Invoice object
    async fn generate_pdf(&self, invoice: &Invoice) -> InvoicingResult<Bytes> {
        let result = self.renderer.render_invoice(invoice)?;

        // Generate PDF with proper standards - reuse PDF standards when possible
        let pdf_standard = [PdfStandard::A_3b]; // PDF/A-3b is required for e-invoicing
        let pdf_standards = PdfStandards::new(&pdf_standard).map_err(|_| {
            InvoicingError::PdfGenerationError("Failed to create PDF standards".to_string())
        })?;

        let pdf_options = PdfOptions {
            standards: pdf_standards,
            page_ranges: None,
            timestamp: None,
            ident: Smart::Auto,
        };

        let pdf = typst_pdf::pdf(&result, &pdf_options).map_err(|e| {
            InvoicingError::PdfGenerationError(format!("Failed to generate PDF: {:?}", e))
        })?;

        Ok(Bytes::from(pdf))
    }
}
