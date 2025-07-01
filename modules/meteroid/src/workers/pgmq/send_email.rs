use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::PgmqHandler;
use cached::proc_macro::cached;
use common_domain::ids::{BaseId, ConnectorId, CustomerId, TenantId};
use common_domain::pgmq::MessageId;
use common_logging::unwrapper::UnwrapLogger;
use error_stack::{ResultExt, report};
use hubspot_client::associations::AssociationsApi;
use hubspot_client::client::HubspotClient;
use hubspot_client::companies::{CompaniesApi, CompanyAddress, NewCompany};
use hubspot_client::deals::{DealsApi, NewDeal};
use hubspot_client::model::CompanyId;
use hubspot_client::properties::PropertiesApi;
use itertools::Itertools;
use meteroid_oauth::model::{OauthAccessToken, OauthProvider};
use meteroid_store::domain::ConnectorProviderEnum;
use meteroid_store::domain::connectors::{
    Connector, HubspotPublicData, ProviderData, ProviderSensitiveData,
};
use meteroid_store::domain::outbox_event::{CustomerEvent, SubscriptionEvent};
use meteroid_store::domain::pgmq::{HubspotSyncCustomerDomain, HubspotSyncRequestEvent, HubspotSyncSubscription, PennylaneSyncRequestEvent, PgmqMessage, SendEmailRequest};
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::oauth::OauthInterface;
use meteroid_store::repositories::{CustomersInterface, SubscriptionInterface};
use meteroid_store::{Services, Store, StoreResult};
use moka::Expiry;
use moka::future::Cache;
use secrecy::SecretString;
use std::sync::Arc;
use std::time::{Duration, Instant};
use prost::Message;
use meteroid_mailer::model::{EmailAttachmentType, EmailRecipient, InvoicePaid, InvoiceReady};
use meteroid_mailer::service::MailerService;
use meteroid_store::errors::StoreError;
use meteroid_store::jwt_claims::{generate_portal_token, ResourceAccess};
use crate::services::storage::{ObjectStoreService, Prefix};

#[derive(Clone)]
pub struct EmailSender {
    store: Arc<Store>,
    mailer: Arc<dyn MailerService>,
    public_url: String,
    rest_api_url: String,
    object_store: Arc<dyn ObjectStoreService>,
    jwt_secret: secrecy::SecretString,

}

impl EmailSender {

    pub(crate) fn new(store: Arc<Store>,
                      mailer: Arc<dyn MailerService>,
                      object_store: Arc<dyn ObjectStoreService>,
                      public_url: String,
                      rest_api_url: String,
                      jwt_secret: secrecy::SecretString) -> Self {
        Self { store,  mailer,  public_url, rest_api_url, jwt_secret, object_store }
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
                invoice_pdf_id
            } => {
                let invoice_token =     generate_portal_token(
                    &self.jwt_secret,
                    tenant_id,
                    ResourceAccess::Invoice(invoice_id)
                )?;

                let payment_url = format!(
                    "{}/i/pay?token={}",
                    self.public_url,
                    invoice_token
                );

                if invoicing_emails.is_empty() {
                    log::warn!("No invoicing emails found for invoice {}", invoice_id);
                    return Ok(());
                }

                let recipients = invoicing_emails
                    .into_iter()
                    .map(|email| {
                        EmailRecipient {
                            email,
                            first_name: None,
                            last_name: None
                        }
                    })
                    .collect_vec();

                let data = self.object_store.retrieve(invoice_pdf_id, Prefix::InvoicePdf).await
                    .change_context(StoreError::ObjectStoreError)
                    ?;

                let sanitized_company_name = company_name.to_lowercase() // onlky a-z0-9
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>();

                let attachment = meteroid_mailer::model::EmailAttachment {
                    filename: format!("invoice_{}-{}.pdf", sanitized_company_name, invoice_number),
                    content: data.to_vec(),
                    type_: EmailAttachmentType::Pdf,
                };

                let logo_url = logo_attachment_id.map(
                    |logo_attachment_id|
                        format!("{}/files/v1/logo/{}",
                            self.rest_api_url,
                            logo_attachment_id.to_string()
                        )
                );

                self.mailer.send_invoice_ready_for_payment(InvoiceReady {
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
                }).await
                    .change_context(StoreError::MailServiceError)
            },
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
                receipt_pdf_id
            } => {

                if invoicing_emails.is_empty() {
                    log::warn!("No invoicing emails found for invoice {}", invoice_id);
                    return Ok(());
                }
                let recipients = invoicing_emails
                    .into_iter()
                    .map(|email| {
                        EmailRecipient {
                            email,
                            first_name: None,
                            last_name: None
                        }
                    })
                    .collect_vec();

                let sanitized_company_name = company_name.to_lowercase() // onlky a-z0-9
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>();


                let mut attachments = vec![];

                let invoice_data = self.object_store.retrieve(invoice_pdf_id, Prefix::InvoicePdf).await
                    .change_context(StoreError::ObjectStoreError)
                    ?;

                attachments.push(
                    meteroid_mailer::model::EmailAttachment {
                        filename: format!("invoice_{}-{}.pdf", sanitized_company_name, invoice_number),
                        content: invoice_data.to_vec(),
                        type_: EmailAttachmentType::Pdf,
                    }
                );

                if let Some(receipt_pdf_id) = receipt_pdf_id {
                    let receipt_data = self.object_store.retrieve(receipt_pdf_id, Prefix::ReceiptPdf).await
                        .change_context(StoreError::ObjectStoreError) ?;

                    attachments.push(
                        meteroid_mailer::model::EmailAttachment {
                            filename: format!("receipt_{}-{}.pdf", sanitized_company_name, receipt_pdf_id.to_string()),
                            content: receipt_data.to_vec(),
                            type_: EmailAttachmentType::Pdf,
                        }
                    );
                }


                let logo_url = logo_attachment_id.map(
                    |logo_attachment_id|
                        format!(
                            "{}/files/v1/logo/{}",
                            self.rest_api_url,
                            logo_attachment_id.to_string() // TODO check it's base62 everywhere
                        )
                );

                self.mailer.send_invoice_paid(InvoicePaid {
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
                }).await
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
                    async move { value.send_email(ev).await .map(|_| id)  }

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




