use crate::services::invoice_rendering::{GenerateResult, PdfRenderingService};
use crate::workers::kafka::outbox::from_kafka_message;
use crate::workers::kafka::processor::MessageHandler;
use async_trait::async_trait;
use common_domain::ids::InvoiceId;
use meteroid_store::domain::outbox_event::OutboxEvent;

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
        if let Some(event) = from_kafka_message(message) {
            log::info!("Processing message: {:?}", event);

            if let OutboxEvent::InvoiceFinalized(evt) = event {
                let invoice_id: InvoiceId = evt.invoice_id;

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
                    Err(e) => {
                        log::error!("Failed to generate pdf for invoice {}: {:?}", invoice_id, e)
                    }
                }
            }
        } else {
            log::debug!("Skipping message");
        }

        Ok(())
    }
}
