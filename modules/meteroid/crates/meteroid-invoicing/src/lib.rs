use crate::errors::InvoicingResult;

use std::sync::Arc;

pub mod errors;
pub mod html_render;
pub mod model;
pub mod pdf;
pub mod storage;

pub struct InvoicingConfig {
    gotenberg_url: String, // optionally add basic auth, or works through url ?
    s3_uri: String,
    s3_prefix: Option<String>,
}

pub struct InvoicingPdfService {
    storage: Arc<dyn storage::Storage>,
    pdf: Arc<dyn pdf::PdfGenerator>,
}

impl InvoicingPdfService {
    pub fn live(config: InvoicingConfig) -> InvoicingResult<Self> {
        let pdf_generator = Arc::new(pdf::GotenbergPdfGenerator::new(
            config.gotenberg_url.clone(),
        ));
        let s3_storage = Arc::new(storage::S3Storage::create(
            config.s3_uri.clone(),
            config.s3_prefix.clone(),
        )?);

        Ok(Self {
            storage: s3_storage,
            pdf: pdf_generator,
        })
    }

    pub fn preview_invoice(&self, invoice: model::Invoice) -> InvoicingResult<String> {
        InvoicingHtmlService::render_invoice(invoice)
    }

    pub async fn generate_invoice_document(
        &self,
        invoice: model::Invoice,
    ) -> InvoicingResult<String> {
        let html = html_render::render_invoice(&invoice)?;
        let pdf = self.pdf.generate_pdf(&html.into_string()).await?;
        self.storage.store_pdf(pdf, None).await
    }
}


pub struct InvoicingHtmlService;

impl InvoicingHtmlService {
    pub fn render_invoice(invoice: model::Invoice) -> InvoicingResult<String> {
        html_render::render_invoice(&invoice).map(|html| html.into_string())
    }
}