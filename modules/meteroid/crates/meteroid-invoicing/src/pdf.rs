use crate::credit_note_model::CreditNote;
use crate::credit_note_render::TypstCreditNoteRenderer;
use crate::errors::{InvoicingError, InvoicingResult};
use crate::model::Invoice;
use crate::typst_render::{EInvoiceAttachment, TypstInvoiceRenderer};
use async_trait::async_trait;
use bytes::Bytes;
use typst::foundations::{Datetime, Smart};
use typst::layout::PagedDocument;
use typst_pdf::{self, PdfOptions, PdfStandard, PdfStandards, Timestamp};

/// PDF/A demands a document date, and an embedded file carries one of its own —
/// krilla refuses to attach without it. Deriving it from the document's own date
/// rather than the clock also keeps generation reproducible: re-rendering an
/// issued invoice yields the same bytes.
fn timestamp_of(date: chrono::NaiveDate) -> Option<Timestamp> {
    use chrono::Datelike;
    Datetime::from_ymd(date.year(), date.month() as u8, date.day() as u8).map(Timestamp::new_utc)
}

fn generate_pdf_from_document(
    document: &PagedDocument,
    timestamp: Option<Timestamp>,
) -> InvoicingResult<Bytes> {
    let pdf_standard = [PdfStandard::A_3b]; // PDF/A-3b is required for e-invoicing
    let pdf_standards = PdfStandards::new(&pdf_standard).map_err(|_| {
        InvoicingError::PdfGenerationError("Failed to create PDF standards".to_string())
    })?;

    let pdf_options = PdfOptions {
        standards: pdf_standards,
        page_ranges: None,
        timestamp,
        ident: Smart::Auto,
        tagged: true,
    };

    let pdf = typst_pdf::pdf(document, &pdf_options).map_err(|e| {
        InvoicingError::PdfGenerationError(format!("Failed to generate PDF: {e:?}"))
    })?;

    Ok(Bytes::from(pdf))
}

/// Typst compilation is CPU-bound: on a multi-threaded runtime, hand this worker thread
/// over so other tasks (queue loops, HTTP handlers) keep running meanwhile.
fn run_cpu_bound<R>(f: impl FnOnce() -> R) -> R {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current().map(|h| h.runtime_flavor()) {
        Ok(RuntimeFlavor::MultiThread) => tokio::task::block_in_place(f),
        _ => f(),
    }
}

#[async_trait]
pub trait PdfGenerator: Send + Sync {
    /// `attachment` makes the result a hybrid document: the same pages, with the
    /// structured invoice embedded as a PDF/A-3 associated file.
    async fn generate_pdf(
        &self,
        invoice: &Invoice,
        attachment: Option<&EInvoiceAttachment>,
    ) -> InvoicingResult<Bytes>;
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
    async fn generate_pdf(
        &self,
        invoice: &Invoice,
        attachment: Option<&EInvoiceAttachment>,
    ) -> InvoicingResult<Bytes> {
        run_cpu_bound(|| {
            let result = self
                .renderer
                .render_invoice_with_attachment(invoice, attachment)?;

            generate_pdf_from_document(&result, timestamp_of(invoice.metadata.issue_date))
        })
    }
}

#[async_trait]
pub trait CreditNotePdfGenerator: Send + Sync {
    async fn generate_credit_note_pdf(
        &self,
        credit_note: &CreditNote,
        attachment: Option<&EInvoiceAttachment>,
    ) -> InvoicingResult<Bytes>;
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
    async fn generate_credit_note_pdf(
        &self,
        credit_note: &CreditNote,
        attachment: Option<&EInvoiceAttachment>,
    ) -> InvoicingResult<Bytes> {
        run_cpu_bound(|| {
            let result = self
                .renderer
                .render_credit_note_with_attachment(credit_note, attachment)?;
            generate_pdf_from_document(&result, timestamp_of(credit_note.metadata.issue_date))
        })
    }
}
