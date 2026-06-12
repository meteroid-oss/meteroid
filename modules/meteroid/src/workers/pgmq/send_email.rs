use crate::services::storage::{ObjectStoreService, Prefix};
use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::actor::Actor;
use common_domain::ids::BaseId;
use common_domain::ids::TenantId;
use common_domain::pgmq::MessageId;
use error_stack::{Report, ResultExt};
use itertools::Itertools;
use meteroid_mailer::model::{EmailAttachmentType, EmailRecipient, InvoicePaid, InvoiceReady};
use meteroid_mailer::service::MailerService;
use meteroid_store::domain::entity_activity::EntityType;
use meteroid_store::domain::pgmq::{PgmqMessage, SendEmailRequest};
use meteroid_store::errors::StoreError;
use meteroid_store::jwt_claims::{ResourceAccess, generate_portal_token};
use meteroid_store::repositories::TenantInterface;
use meteroid_store::repositories::entity_activity::{
    EntityActivityInterfaceEmail, SentEmailAttachment, SentEmailAttachmentKind, SentEmailNew,
};
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
            object_store,
            jwt_secret,
            store,
        }
    }

    fn get_tenant_id_from_request(
        &self,
        request: &SendEmailRequest,
    ) -> Result<TenantId, Report<StoreError>> {
        match request {
            SendEmailRequest::InvoiceReady { tenant_id, .. } => Ok(*tenant_id),
            SendEmailRequest::InvoicePaid { tenant_id, .. } => Ok(*tenant_id),
            SendEmailRequest::PaymentReminder { tenant_id, .. } => Ok(*tenant_id),
            SendEmailRequest::PaymentRejected { tenant_id, .. } => Ok(*tenant_id),
            SendEmailRequest::QuoteReady { tenant_id, .. } => Ok(*tenant_id),
        }
    }

    async fn is_email_disabled_for_tenant(
        &self,
        tenant_id: TenantId,
    ) -> Result<bool, Report<StoreError>> {
        let _tenant = self.store.find_tenant_by_id(tenant_id).await?;
        //Ok(tenant.tenant.disable_emails)
        Ok(false)
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

    /// Returns `Some(SentEmailNew)` only when an email was actually delivered.
    /// Skips (`None`) on tenant disable_emails / no recipients / test address.
    async fn send_email(
        &self,
        ev: SendEmailRequest,
    ) -> Result<Option<SentEmailNew>, Report<StoreError>> {
        let tenant_id = self.get_tenant_id_from_request(&ev)?;
        if self.is_email_disabled_for_tenant(tenant_id).await? {
            return Ok(None);
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
                agg_customer_id,
                agg_subscription_id,
            } => {
                if invoicing_emails.is_empty() {
                    log::warn!("No invoicing emails found for invoice {invoice_id}");
                    return Ok(None);
                }

                let invoice_token = generate_portal_token(
                    &self.jwt_secret,
                    tenant_id,
                    ResourceAccess::Invoice(invoice_id),
                )?;
                let payment_url = format!(
                    "{}/portal/invoice-payment?token={}",
                    self.public_url, invoice_token
                );

                let recipients = invoicing_emails
                    .into_iter()
                    .map(|email| EmailRecipient {
                        email,
                        first_name: None,
                        last_name: None,
                    })
                    .collect_vec();

                let sanitized_company_name = sanitize_company_name(&company_name);
                let attachment_filename =
                    format!("invoice_{sanitized_company_name}-{invoice_number}.pdf");

                let data = self
                    .object_store
                    .retrieve(invoice_pdf_id, Prefix::InvoicePdf)
                    .await
                    .change_context(StoreError::ObjectStoreError)?;

                let attachment = meteroid_mailer::model::EmailAttachment {
                    filename: attachment_filename.clone(),
                    content: data.to_vec(),
                    type_: EmailAttachmentType::Pdf,
                };

                let sent_attachments = vec![SentEmailAttachment {
                    filename: attachment_filename,
                    id: invoice_pdf_id,
                    kind: SentEmailAttachmentKind::InvoicePdf,
                }];

                let logo_url = logo_attachment_id.map(|logo_attachment_id| {
                    format!("{}/files/v1/logo/{}", self.rest_api_url, logo_attachment_id)
                });

                let rendered = self
                    .mailer
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
                        recipients,
                        attachment,
                        account: invoicing_entity_id.as_base62(),
                    })
                    .await
                    .change_context(StoreError::MailServiceError)?;

                if !rendered.delivered {
                    return Ok(None);
                }
                Ok(Some(rendered_into_sent(
                    rendered,
                    tenant_id,
                    EntityType::Invoice,
                    invoice_id.as_uuid(),
                    "invoice_ready",
                    agg_customer_id,
                    agg_subscription_id,
                    sent_attachments,
                )))
            }
            SendEmailRequest::InvoicePaid {
                tenant_id,
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
                agg_customer_id,
                agg_subscription_id,
            } => {
                if invoicing_emails.is_empty() {
                    log::warn!("No invoicing emails found for invoice {invoice_id}");
                    return Ok(None);
                }

                let recipients = invoicing_emails
                    .into_iter()
                    .map(|email| EmailRecipient {
                        email,
                        first_name: None,
                        last_name: None,
                    })
                    .collect_vec();

                let sanitized_company_name = sanitize_company_name(&company_name);
                let mut attachments = vec![];
                let mut sent_attachments = vec![];

                let invoice_data = self
                    .object_store
                    .retrieve(invoice_pdf_id, Prefix::InvoicePdf)
                    .await
                    .change_context(StoreError::ObjectStoreError)?;

                let invoice_filename =
                    format!("invoice_{sanitized_company_name}-{invoice_number}.pdf");
                attachments.push(meteroid_mailer::model::EmailAttachment {
                    filename: invoice_filename.clone(),
                    content: invoice_data.to_vec(),
                    type_: EmailAttachmentType::Pdf,
                });
                sent_attachments.push(SentEmailAttachment {
                    filename: invoice_filename,
                    id: invoice_pdf_id,
                    kind: SentEmailAttachmentKind::InvoicePdf,
                });

                if let Some(receipt_pdf_id) = receipt_pdf_id {
                    let receipt_data = self
                        .object_store
                        .retrieve(receipt_pdf_id, Prefix::ReceiptPdf)
                        .await
                        .change_context(StoreError::ObjectStoreError)?;

                    let receipt_filename =
                        format!("receipt_{sanitized_company_name}-{receipt_pdf_id}.pdf");
                    attachments.push(meteroid_mailer::model::EmailAttachment {
                        filename: receipt_filename.clone(),
                        content: receipt_data.to_vec(),
                        type_: EmailAttachmentType::Pdf,
                    });
                    sent_attachments.push(SentEmailAttachment {
                        filename: receipt_filename,
                        id: receipt_pdf_id,
                        kind: SentEmailAttachmentKind::ReceiptPdf,
                    });
                }

                let logo_url = logo_attachment_id.map(|logo_attachment_id| {
                    format!("{}/files/v1/logo/{}", self.rest_api_url, logo_attachment_id)
                });

                let rendered = self
                    .mailer
                    .send_invoice_paid(InvoicePaid {
                        invoice_number,
                        invoice_date,
                        invoice_due_date,
                        label,
                        amount_paid,
                        currency,
                        company_name,
                        logo_url,
                        recipients,
                        attachments,
                        lines: vec![], // TODO
                        account: invoicing_entity_id.as_base62(),
                    })
                    .await
                    .change_context(StoreError::MailServiceError)?;

                if !rendered.delivered {
                    return Ok(None);
                }
                Ok(Some(rendered_into_sent(
                    rendered,
                    tenant_id,
                    EntityType::Invoice,
                    invoice_id.as_uuid(),
                    "invoice_paid",
                    agg_customer_id,
                    agg_subscription_id,
                    sent_attachments,
                )))
            }

            SendEmailRequest::QuoteReady {
                invoicing_entity_id,
                quote_number,
                expires_at,
                company_name,
                logo_attachment_id,
                recipient_emails,
                portal_url,
                custom_message,
                ..
            } => {
                if recipient_emails.is_empty() {
                    log::warn!("No recipient emails found for quote {quote_number}");
                    return Ok(None);
                }

                let recipients = recipient_emails
                    .into_iter()
                    .map(|email| EmailRecipient {
                        email,
                        first_name: None,
                        last_name: None,
                    })
                    .collect_vec();

                let logo_url = logo_attachment_id.map(|logo_attachment_id| {
                    format!("{}/files/v1/logo/{}", self.rest_api_url, logo_attachment_id)
                });

                let _ = self
                    .mailer
                    .send_quote_ready(meteroid_mailer::model::QuoteReady {
                        quote_number,
                        expires_at,
                        company_name,
                        logo_url,
                        recipients,
                        portal_url,
                        custom_message,
                        account: invoicing_entity_id.as_base62(),
                    })
                    .await
                    .change_context(StoreError::MailServiceError)?;

                // QuoteSent audit is recorded by the producer.
                Ok(None)
            }

            _ => Ok(None),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn rendered_into_sent(
    r: meteroid_mailer::model::RenderedEmail,
    tenant_id: TenantId,
    entity_type: EntityType,
    entity_id: uuid::Uuid,
    kind: &'static str,
    agg_customer_id: Option<common_domain::ids::CustomerId>,
    agg_subscription_id: Option<common_domain::ids::SubscriptionId>,
    attachments: Vec<SentEmailAttachment>,
) -> SentEmailNew {
    SentEmailNew {
        tenant_id,
        entity_type,
        entity_id,
        agg_customer_id,
        agg_subscription_id,
        kind: kind.to_string(),
        subject: r.subject,
        from_addr: r.from,
        reply_to: r.reply_to,
        recipients: r.recipients.into_iter().map(|x| x.email).collect(),
        body_html: r.body_html,
        attachments,
    }
}

fn sanitize_company_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

#[async_trait::async_trait]
impl PgmqHandler for EmailSender {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let events = self.convert_to_events(msgs)?;

        let mut result = HandleResult {
            succeeded: vec![],
            failed: vec![],
        };

        let tasks = events
            .into_iter()
            .map(|(ev, id)| {
                tokio::spawn({
                    let value = self.clone();
                    async move { (id, value.send_email(ev).await) }
                })
            })
            .collect_vec();

        for task in tasks {
            match task.await {
                Ok((id, Ok(sent))) => {
                    result.succeeded.push(id);
                    if let Some(sent) = sent {
                        // Audit best-effort: don't fail the queue message — the email already went out,
                        // so a missed audit row is better than a redelivery.
                        if let Err(e) = self.store.record_email_sent(Actor::System, sent).await {
                            log::warn!("Failed to record sent_email: {e:?}");
                        }
                    }
                }
                Ok((id, Err(e))) => {
                    log::warn!("Failed to send email: {e:?}");
                    result.failed.push(HandleResult::fail(id, &e));
                }
                Err(e) => {
                    log::warn!("Email send task panicked: {e:?}");
                }
            }
        }

        Ok(result)
    }
}
