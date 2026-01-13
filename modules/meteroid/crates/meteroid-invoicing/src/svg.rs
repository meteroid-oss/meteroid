use crate::credit_note_model::CreditNote;
use crate::credit_note_render::TypstCreditNoteRenderer;
use crate::errors::InvoicingResult;
use crate::model::Invoice;
use crate::typst_render::TypstInvoiceRenderer;
use async_trait::async_trait;
use typst::layout::PagedDocument;

fn generate_svg_from_document(document: &PagedDocument) -> Vec<String> {
    document
        .pages
        .iter()
        .map(typst_svg::svg)
        .collect::<Vec<String>>()
}

#[async_trait]
pub trait SvgGenerator: Send + Sync {
    async fn generate_svg(&self, invoice: &Invoice) -> InvoicingResult<Vec<String>>;
}

pub struct TypstSvgGenerator {
    renderer: TypstInvoiceRenderer,
}

// this is used for previews in the frontend
impl TypstSvgGenerator {
    pub fn new() -> InvoicingResult<Self> {
        let renderer = TypstInvoiceRenderer::new()?;
        Ok(TypstSvgGenerator { renderer })
    }
}

#[async_trait]
impl SvgGenerator for TypstSvgGenerator {
    async fn generate_svg(&self, invoice: &Invoice) -> InvoicingResult<Vec<String>> {
        let result = self.renderer.render_invoice(invoice)?;
        Ok(generate_svg_from_document(&result))
    }
}

#[async_trait]
pub trait CreditNoteSvgGenerator: Send + Sync {
    async fn generate_credit_note_svg(
        &self,
        credit_note: &CreditNote,
    ) -> InvoicingResult<Vec<String>>;
}

pub struct TypstCreditNoteSvgGenerator {
    renderer: TypstCreditNoteRenderer,
}

impl TypstCreditNoteSvgGenerator {
    pub fn new() -> InvoicingResult<Self> {
        let renderer = TypstCreditNoteRenderer::new()?;
        Ok(TypstCreditNoteSvgGenerator { renderer })
    }
}

#[async_trait]
impl CreditNoteSvgGenerator for TypstCreditNoteSvgGenerator {
    async fn generate_credit_note_svg(
        &self,
        credit_note: &CreditNote,
    ) -> InvoicingResult<Vec<String>> {
        let result = self.renderer.render_credit_note(credit_note)?;
        Ok(generate_svg_from_document(&result))
    }
}
