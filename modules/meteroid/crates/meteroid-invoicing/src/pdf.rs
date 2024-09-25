use crate::errors::{InvoicingError, InvoicingResult};
use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;

#[async_trait]
pub trait PdfGenerator: Send + Sync {
    async fn generate_pdf(&self, invoice_html: &str) -> InvoicingResult<Bytes>;
}

pub struct GotenbergPdfGenerator {
    gotenberg_url: String,
    client: Client,
}

impl GotenbergPdfGenerator {
    pub fn new(gotenberg_url: String) -> Self {
        let client = reqwest::Client::new();
        GotenbergPdfGenerator {
            gotenberg_url,
            client,
        }
    }
}

#[async_trait]
impl PdfGenerator for GotenbergPdfGenerator {
    async fn generate_pdf(&self, invoice_html: &str) -> InvoicingResult<Bytes> {
        let html_part = reqwest::multipart::Part::text(invoice_html.to_owned())
            .file_name("index.html")
            .mime_str("text/html")
            .map_err(|_| {
                InvoicingError::PdfGenerationError(
                    "Failed to create HTML part for Gotenberg".to_string(),
                )
            })?;

        let footer_part =
            reqwest::multipart::Part::text(crate::footer_render::render_footer().into_string())
                .file_name("footer.html")
                .mime_str("text/html")
                .map_err(|_| {
                    InvoicingError::PdfGenerationError(
                        "Failed to create footer part for Gotenberg".to_string(),
                    )
                })?;

        let form = reqwest::multipart::Form::new()
            .part("files", html_part)
            .part("files", footer_part)
            .text("scale", "1")
            .text("marginTop", "0.2")
            .text("marginBottom", "0.2")
            .text("marginLeft", "0.2")
            .text("marginRight", "0.2")
            .text("pdfa", "PDF/A-3b");

        let response = self
            .client
            .post(format!(
                "{}/forms/chromium/convert/html",
                self.gotenberg_url
            ))
            .multipart(form)
            .send()
            .await
            .map_err(|_| {
                InvoicingError::PdfGenerationError(
                    "Failed to send request to gotenberg".to_string(),
                )
            })?;

        if response.status().is_success() {
            Ok(response.bytes().await.map_err(|_| {
                InvoicingError::PdfGenerationError("Failed to read Gotenberg response".to_string())
            })?)
        } else {
            Err(InvoicingError::PdfGenerationError(format!(
                "Gotenberg returned status code {} and body {}",
                response.status(),
                response.text().await.unwrap()
            )))
        }
    }
}
