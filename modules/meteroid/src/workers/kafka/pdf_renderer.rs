use crate::api::utils::parse_uuid;
use crate::services::invoice_rendering::{GenerateResult, PdfRenderingService};
use crate::workers::kafka::outbox::{parse_outbox_event, EventType};
use crate::workers::kafka::processor::MessageHandler;
use async_trait::async_trait;
use uuid::Uuid;

pub struct PdfRendererHandler {
    pdf_service: PdfRenderingService,
}

impl PdfRendererHandler {
    pub fn new(pdf_service: PdfRenderingService) -> Self {
        Self { pdf_service }
    }
}

#[async_trait]
impl MessageHandler for PdfRendererHandler {
    async fn handle(
        &self,
        message: &rdkafka::message::BorrowedMessage<'_>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(event) = parse_outbox_event(message) {
            log::info!("Processing message: {:?}", event);

            match event.event_type {
                EventType::InvoiceFinalized(_) | EventType::InvoicePdfRequested => {
                    let invoice_id: Uuid = parse_uuid(event.aggregate_id.as_str(), "aggregate_id")?;

                    let result = self.pdf_service.generate_pdfs(vec![invoice_id]).await;

                    match result {
                        Ok(results) => {
                            results.into_iter().for_each(|x| match x {
                                GenerateResult::Success {
                                    invoice_id,
                                    pdf_url,
                                } => {
                                    log::info!(
                                        "Generated pdf for invoice {} at url {}",
                                        invoice_id,
                                        pdf_url
                                    )
                                }
                                GenerateResult::Failure { invoice_id, error } => {
                                    log::error!(
                                        "Failed to generate pdf for invoice {}: {}",
                                        invoice_id,
                                        error
                                    )
                                }
                            });
                        }
                        Err(e) => log::error!(
                            "Failed to generate pdf for invoice {}: {:?}",
                            invoice_id,
                            e
                        ),
                    }
                }
                _ => (),
            }
        } else {
            log::debug!("Skipping message");
        }

        Ok(())
    }
}
