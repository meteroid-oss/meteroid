use crate::credit_note_model::CreditNote;
use crate::credit_note_render::TypstCreditNoteRenderer;
use crate::errors::{InvoicingError, InvoicingResult};
use crate::model::Invoice;
use crate::typst_render::TypstInvoiceRenderer;
use async_trait::async_trait;
use bytes::Bytes;
use typst::foundations::Smart;
use typst::layout::PagedDocument;
use typst_pdf::{self, PdfOptions, PdfStandard, PdfStandards};

fn generate_pdf_from_document(document: &PagedDocument) -> InvoicingResult<Bytes> {
    let pdf_standard = [PdfStandard::A_3b]; // PDF/A-3b is required for e-invoicing
    let pdf_standards = PdfStandards::new(&pdf_standard).map_err(|_| {
        InvoicingError::PdfGenerationError("Failed to create PDF standards".to_string())
    })?;

    let pdf_options = PdfOptions {
        standards: pdf_standards,
        page_ranges: None,
        timestamp: None,
        ident: Smart::Auto,
        tagged: true,
    };

    let pdf = typst_pdf::pdf(document, &pdf_options).map_err(|e| {
        InvoicingError::PdfGenerationError(format!("Failed to generate PDF: {e:?}"))
    })?;

    Ok(Bytes::from(pdf))
}

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
    async fn generate_pdf(&self, invoice: &Invoice) -> InvoicingResult<Bytes> {
        let result = self.renderer.render_invoice(invoice)?;
        generate_pdf_from_document(&result)
    }
}

#[async_trait]
pub trait CreditNotePdfGenerator: Send + Sync {
    async fn generate_credit_note_pdf(&self, credit_note: &CreditNote) -> InvoicingResult<Bytes>;
}

pub struct TypstCreditNotePdfGenerator {
    renderer: TypstCreditNoteRenderer,
}

impl TypstCreditNotePdfGenerator {
    pub fn new() -> InvoicingResult<Self> {
        let renderer = TypstCreditNoteRenderer::new()?;
        Ok(TypstCreditNotePdfGenerator { renderer })
    }
}

#[async_trait]
impl CreditNotePdfGenerator for TypstCreditNotePdfGenerator {
    async fn generate_credit_note_pdf(&self, credit_note: &CreditNote) -> InvoicingResult<Bytes> {
        let result = self.renderer.render_credit_note(credit_note)?;
        generate_pdf_from_document(&result)
    }
}
