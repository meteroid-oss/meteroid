use crate::errors::InvoicingResult;
use crate::model::Invoice;
use crate::typst_render::TypstInvoiceRenderer;
use async_trait::async_trait;
use typst::layout::Abs;

#[async_trait]
pub trait SvgGenerator: Send + Sync {
    async fn generate_svg(&self, invoice: &Invoice) -> InvoicingResult<String>;
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
    async fn generate_svg(&self, invoice: &Invoice) -> InvoicingResult<String> {
        let result = self.renderer.render_invoice(invoice)?;

        // TODO maybe separate
        let svg = typst_svg::svg_merged(&result, Abs::mm(10.0));
        Ok(svg)
    }
}
