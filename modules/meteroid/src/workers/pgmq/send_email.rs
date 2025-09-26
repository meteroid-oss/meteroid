use crate::services::storage::{ObjectStoreService, Prefix};
use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::PgmqHandler;
use common_domain::ids::BaseId;
use common_domain::ids::TenantId;
use common_domain::pgmq::MessageId;
use error_stack::ResultExt;
use itertools::Itertools;
use meteroid_mailer::model::{EmailAttachmentType, EmailRecipient, InvoicePaid, InvoiceReady};
use meteroid_mailer::service::MailerService;
use meteroid_store::domain::pgmq::{PgmqMessage, SendEmailRequest};
use meteroid_store::errors::StoreError;
use meteroid_store::jwt_claims::{ResourceAccess, generate_portal_token};
use meteroid_store::repositories::TenantInterface;
use meteroid_store::{Store, StoreResult};
use std::sync::Arc;

#[derive(Clone)]
pub struct EmailSender {
    mailer: Arc<dyn MailerService>,
    public_url: String,
    rest_api_url: String,
    object_store: Arc<dyn ObjectStoreService>,
    jwt_secret: secrecy::SecretString,
    store: Arc<Store>,
}

impl EmailSender {
    pub(crate) fn new(
        mailer: Arc<dyn MailerService>,
        object_store: Arc<dyn ObjectStoreService>,
        public_url: String,
        rest_api_url: String,
        jwt_secret: secrecy::SecretString,
        store: Arc<Store>,
    ) -> Self {
        Self {
            mailer,
            public_url,
            rest_api_url,
            jwt_secret,
            object_store,
            store,
        }
    }

    fn get_tenant_id_from_request(
        &self,
        request: &SendEmailRequest,
    ) -> error_stack::Result<TenantId, StoreError> {
        match request {
            SendEmailRequest::InvoiceReady { tenant_id, .. } => Ok(*tenant_id),
            SendEmailRequest::InvoicePaid { tenant_id, .. } => Ok(*tenant_id),
            SendEmailRequest::PaymentReminder { tenant_id, .. } => Ok(*tenant_id),
            SendEmailRequest::PaymentRejected { tenant_id, .. } => Ok(*tenant_id),
        }
    }

    async fn is_email_disabled_for_tenant(
        &self,
        tenant_id: TenantId,
    ) -> error_stack::Result<bool, StoreError> {
        let tenant = self.store.find_tenant_by_id(tenant_id).await?;
        Ok(tenant.tenant.disable_emails)
    }

    fn convert_to_events(
        &self,
        msgs: &[PgmqMessage],
    ) -> PgmqResult<Vec<(SendEmailRequest, MessageId)>> {
        msgs.iter()
            .map(|msg| {
                let evt: StoreResult<SendEmailRequest> = msg.try_into();
                evt.map(|evt| (evt, msg.msg_id))
            })
            .collect::<StoreResult<Vec<_>>>()
            .change_context(PgmqError::HandleMessages)
    }

    async fn send_email(&self, ev: SendEmailRequest) -> error_stack::Result<(), StoreError> {
        // Check if emails are disabled for this tenant
        let tenant_id = self.get_tenant_id_from_request(&ev)?;
        if self.is_email_disabled_for_tenant(tenant_id).await? {
            return Ok(());
        }
        match ev {
            SendEmailRequest::InvoiceReady {
                tenant_id,
                invoice_id,
                invoicing_entity_id,
                invoice_number,
                invoice_date,
                invoice_due_date,
                label,
                amount_due,
                currency,
                company_name,
                logo_attachment_id,
                invoicing_emails,
                invoice_pdf_id,
            } => {
                let invoice_token = generate_portal_token(
                    &self.jwt_secret,
                    tenant_id,
                    ResourceAccess::Invoice(invoice_id),
                )?;

                let payment_url = format!("{}/i/pay?token={}", self.public_url, invoice_token);

                if invoicing_emails.is_empty() {
                    log::warn!("No invoicing emails found for invoice {}", invoice_id);
                    return Ok(());
                }

                let recipients = invoicing_emails
                    .into_iter()
                    .map(|email| EmailRecipient {
                        email,
                        first_name: None,
                        last_name: None,
                    })
                    .collect_vec();

                let data = self
                    .object_store
                    .retrieve(invoice_pdf_id, Prefix::InvoicePdf)
                    .await
                    .change_context(StoreError::ObjectStoreError)?;

                let sanitized_company_name = company_name
                    .to_lowercase() // onlky a-z0-9
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>();

                let attachment = meteroid_mailer::model::EmailAttachment {
                    filename: format!("invoice_{}-{}.pdf", sanitized_company_name, invoice_number),
                    content: data.to_vec(),
                    type_: EmailAttachmentType::Pdf,
                };

                let logo_url = logo_attachment_id.map(|logo_attachment_id| {
                    format!("{}/files/v1/logo/{}", self.rest_api_url, logo_attachment_id)
                });

                self.mailer
                    .send_invoice_ready_for_payment(InvoiceReady {
                        invoice_number,
                        invoice_date,
                        invoice_due_date,
                        label,
                        amount_due,
                        currency,
                        payment_url,
                        company_name,
                        logo_url,
                        recipients: recipients.clone(),
                        attachment,
                        account: invoicing_entity_id.as_base62(),
                    })
                    .await
                    .change_context(StoreError::MailServiceError)
            }
            SendEmailRequest::InvoicePaid {
                invoice_id,
                invoicing_entity_id,
                invoice_number,
                invoice_date,
                invoice_due_date,
                label,
                amount_paid,
                currency,
                company_name,
                logo_attachment_id,
                invoicing_emails,
                invoice_pdf_id,
                receipt_pdf_id,
                ..
            } => {
                if invoicing_emails.is_empty() {
                    log::warn!("No invoicing emails found for invoice {}", invoice_id);
                    return Ok(());
                }
                let recipients = invoicing_emails
                    .into_iter()
                    .map(|email| EmailRecipient {
                        email,
                        first_name: None,
                        last_name: None,
                    })
                    .collect_vec();

                let sanitized_company_name = company_name
                    .to_lowercase() // onlky a-z0-9
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>();

                let mut attachments = vec![];

                let invoice_data = self
                    .object_store
                    .retrieve(invoice_pdf_id, Prefix::InvoicePdf)
                    .await
                    .change_context(StoreError::ObjectStoreError)?;

                attachments.push(meteroid_mailer::model::EmailAttachment {
                    filename: format!("invoice_{}-{}.pdf", sanitized_company_name, invoice_number),
                    content: invoice_data.to_vec(),
                    type_: EmailAttachmentType::Pdf,
                });

                if let Some(receipt_pdf_id) = receipt_pdf_id {
                    let receipt_data = self
                        .object_store
                        .retrieve(receipt_pdf_id, Prefix::ReceiptPdf)
                        .await
                        .change_context(StoreError::ObjectStoreError)?;

                    attachments.push(meteroid_mailer::model::EmailAttachment {
                        filename: format!(
                            "receipt_{}-{}.pdf",
                            sanitized_company_name, receipt_pdf_id
                        ),
                        content: receipt_data.to_vec(),
                        type_: EmailAttachmentType::Pdf,
                    });
                }

                let logo_url = logo_attachment_id.map(|logo_attachment_id| {
                    format!(
                        "{}/files/v1/logo/{}",
                        self.rest_api_url,
                        logo_attachment_id // TODO check it's base62 everywhere
                    )
                });

                self.mailer
                    .send_invoice_paid(InvoicePaid {
                        invoice_number,
                        invoice_date,
                        invoice_due_date,
                        label,
                        amount_paid,
                        currency,
                        company_name,
                        logo_url,
                        recipients: recipients.clone(),
                        attachments,
                        lines: vec![], // TODO
                        account: invoicing_entity_id.as_base62(),
                    })
                    .await
                    .change_context(StoreError::MailServiceError)
            }

            _ => {
                todo!()
            }
        }
    }
}

#[async_trait::async_trait]
impl PgmqHandler for EmailSender {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
        let events = self.convert_to_events(msgs)?;

        let mut success_msg_ids = vec![];

        let tasks = events
            .into_iter()
            .map(|(ev, id)| {
                tokio::spawn({
                    let value = self.clone();
                    async move { value.send_email(ev).await.map(|_| id) }
                })
            })
            .collect_vec();

        for task in tasks {
            match task.await {
                Ok(Ok(id)) => {
                    success_msg_ids.push(id);
                }
                Ok(Err(e)) => {
                    log::warn!("Failed to sync connected tenant: {:?}", e);
                }
                Err(e) => {
                    log::warn!("Sync task failed: {:?}", e);
                }
            }
        }

        Ok(success_msg_ids)
    }
}
