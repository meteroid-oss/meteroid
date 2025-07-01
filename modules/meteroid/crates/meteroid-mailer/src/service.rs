use crate::config::MailerConfig;
use crate::errors::MailerServiceError;
use crate::model::{Email, EmailValidationLink, InvoicePaid, InvoiceReady, ResetPasswordLink};
use crate::template::{EmailValidationLinkTemplate, InvoicePaidTemplate, InvoiceReadyTemplate, ResetPasswordLinkTemplate};
use async_trait::async_trait;
use error_stack::Report;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::stub::AsyncStubTransport;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use sailfish::TemplateSimple;
use secrecy::ExposeSecret;
use std::sync::Arc;

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
#[async_trait]
pub trait MailerService: Send + Sync {
    async fn send(&self, email: Email) -> error_stack::Result<(), MailerServiceError>;
    async fn send_reset_password_link(
        &self,
        link: ResetPasswordLink,
    ) -> error_stack::Result<(), MailerServiceError>;

    async fn send_email_validation_link(
        &self,
        link: EmailValidationLink,
    ) -> error_stack::Result<(), MailerServiceError>;

    async fn send_invoice_ready_for_payment(
        &self,
        link: InvoiceReady,
    ) -> error_stack::Result<(), MailerServiceError>;

    async fn send_invoice_paid(
        &self,
        link: InvoicePaid,
    ) -> error_stack::Result<(), MailerServiceError>;
}

pub struct LettreMailerService<T: AsyncTransport> {
    pub transport: Arc<T>,
    pub config: MailerConfig,
}

#[async_trait]
impl<T: AsyncTransport + Send + Sync> MailerService for LettreMailerService<T>
where
    T::Error: Into<MailerServiceError>,
{
    async fn send(&self, email: Email) -> error_stack::Result<(), MailerServiceError> {
        let message: Message = email.try_into()?;
        let _ = self
            .transport
            .send(message)
            .await
            .map_err(|e| Report::new(e.into()))?;

        Ok(())
    }

    async fn send_reset_password_link(
        &self,
        link: ResetPasswordLink,
    ) -> error_stack::Result<(), MailerServiceError> {
        let tpl = ResetPasswordLinkTemplate::from(link.clone());

        let body_html = tpl.render_once().map_err(|e| Report::new(e.into()))?;

        let email = Email {
            from: self.config.from.clone(),
            reply_to: Some("No Reply <no-reply@meteroid.com>".into()),
            to: vec![link.recipient.clone()],
            subject: "Reset password".into(),
            body_html,
            attachments: vec![],
        };
        self.send(email).await
    }

    async fn send_email_validation_link(
        &self,
        link: EmailValidationLink,
    ) -> error_stack::Result<(), MailerServiceError> {
        let tpl = EmailValidationLinkTemplate::from(link.clone());

        let body_html = tpl.render_once().map_err(|e| Report::new(e.into()))?;

        let email = Email {
            from: self.config.from.clone(),
            reply_to: Some("No Reply <no-reply@meteroid.com>".into()),
            to: vec![link.recipient.clone()],
            subject: "Validate your email".into(),
            body_html,
            attachments: vec![],
        };
        self.send(email).await
    }

    async fn send_invoice_ready_for_payment(
        &self,
        data: InvoiceReady,
    ) -> error_stack::Result<(), MailerServiceError> {
        let tpl = InvoiceReadyTemplate::from(data.clone()).tpl;

        let title = tpl.title.clone();
        let from = format!(
            "{} (via Meteroid.com) <billing@meteroid.com>",
            data.company_name
        );
        let body_html = tpl.render_once().map_err(|e| Report::new(e.into()))?;

        let email = Email {
            from ,
            reply_to: Some("Meteroid <support@meteroid.com>".into()), // TODO allow custom reply email
            to: data.recipients.clone(),
            subject: title.into(),
            body_html,
            attachments: vec![data.attachment],
        };
        self.send(email).await

    }

    async fn send_invoice_paid(
        &self,
        data: InvoicePaid,
    ) -> error_stack::Result<(), MailerServiceError> {
        let tpl = InvoicePaidTemplate::from(data.clone()).tpl;

        let title = tpl.title.clone();
        let from = format!(
            "{} (via Meteroid.com) <billing@meteroid.com>",
            data.company_name
        );
        let body_html = tpl.render_once().map_err(|e| Report::new(e.into()))?;

        let email = Email {
            from ,
            reply_to: Some("Meteroid <support@meteroid.com>".into()), // TODO allow custom reply email
            to: data.recipients.clone(),
            subject: title.into(),
            body_html,
            attachments: data.attachments,
        };
        self.send(email).await

    }
}

pub fn mailer_service(cfg: MailerConfig) -> Arc<dyn MailerService> {
    if let (Some(host), Some(username), Some(password)) = (
        cfg.smtp_host.as_ref(),
        cfg.smtp_username.as_ref(),
        cfg.smtp_password.as_ref(),
    ) {
        let creds = Credentials::new(
            username.expose_secret().to_string(),
            password.expose_secret().to_string(),
        );

        let transport = if cfg.smtp_tls {
            log::info!("Starting mailer service with TLS to host {}", host);
            // TODO pool ?
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
                .unwrap()
                .credentials(creds)
                .timeout(Some(std::time::Duration::from_secs(10)))
                .build()
        } else {
            log::info!("Starting unsecure mailer service to host {}", host);
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host.to_string())
                .credentials(creds)
                .timeout(Some(std::time::Duration::from_secs(10)))
                .build()
        };

        Arc::new(LettreMailerService {
            transport: Arc::new(transport),
            config: cfg,
        })
    } else {
        Arc::new(LettreMailerService {
            transport: Arc::new(AsyncStubTransport::new_ok()),
            config: cfg,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::config::MailerConfig;
    use crate::model::{Email, EmailAttachment, EmailAttachmentType, EmailRecipient};
    use crate::service::mailer_service;

    #[tokio::test]
    async fn test_dummy() -> Result<(), Box<dyn std::error::Error>> {
        let cfg = MailerConfig::dummy();

        let pdf_data = vec![0; 8];

        let email = Email {
            from: "NoBody <hey@pp.com>".to_string(),
            reply_to: None,
            to: vec![EmailRecipient {
                email: "aa@g.com".into(),
                first_name: None,
                last_name: None,
            }],
            subject: "Happy new year buddy".to_string(),
            body_html: "Please find the attached PDF! \n".to_string(),
            attachments: vec![EmailAttachment {
                filename: "example.pdf".to_string(),
                content: pdf_data,
                type_: EmailAttachmentType::Pdf,
            }],
        };

        let srv = mailer_service(cfg);

        srv.send(email).await?;

        Ok(())
    }
}
