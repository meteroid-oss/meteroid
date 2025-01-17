use crate::config::MailerConfig;
use crate::errors::MailerServiceError;
use async_trait::async_trait;
use error_stack::Report;
use lettre::message::header::{ContentDisposition, ContentType};
use lettre::message::{MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::stub::AsyncStubTransport;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use secrecy::ExposeSecret;
use std::sync::Arc;

pub struct Email {
    pub from: String,
    pub to: String,
    pub reply_to: String,
    pub subject: String,
    pub body_html: String,
    pub attachments: Vec<EmailAttachment>,
}

impl Email {
    pub fn include_attachments(&self) -> bool {
        !self.attachments.is_empty()
    }
}

pub struct EmailAttachment {
    pub filename: String,
    pub content: Vec<u8>,
    pub type_: EmailAttachmentType,
}

pub enum EmailAttachmentType {
    Pdf,
}

#[async_trait]
pub trait MailerService {
    async fn send_email(&self, email: Email) -> error_stack::Result<(), MailerServiceError>;
}

pub struct LettreMailerService<T: AsyncTransport> {
    pub transport: Arc<T>,
}

#[async_trait]
impl<T: AsyncTransport + Send + Sync> MailerService for LettreMailerService<T>
where
    T::Error: Into<MailerServiceError>,
{
    async fn send_email(&self, email: Email) -> error_stack::Result<(), MailerServiceError> {
        let message: Message = email.try_into()?;
        let _ = self
            .transport
            .send(message)
            .await
            .map_err(|e| Report::new(e.into()))?;

        Ok(())
    }
}

pub fn mailer_service(cfg: &MailerConfig) -> Arc<dyn MailerService> {
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
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
                .unwrap()
                .credentials(creds)
                .build()
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host.to_string())
                .credentials(creds)
                .build()
        };

        Arc::new(LettreMailerService {
            transport: Arc::new(transport),
        })
    } else {
        Arc::new(LettreMailerService {
            transport: Arc::new(AsyncStubTransport::new_ok()),
        })
    }
}

impl TryInto<Message> for Email {
    type Error = Report<MailerServiceError>;

    fn try_into(self) -> Result<Message, Self::Error> {
        let builder = Message::builder()
            .from(self.from.parse().expect("Invalid from address"))
            .reply_to(self.reply_to.parse().expect("Invalid reply-to address"))
            .to(self.to.parse().expect("Invalid to address"))
            .subject(self.subject.clone());

        if self.include_attachments() {
            let mut multi_builder =
                MultiPart::mixed().singlepart(SinglePart::plain(self.body_html));

            for attachment in self.attachments.into_iter() {
                multi_builder = multi_builder.singlepart(attachment.into());
            }

            builder
                .multipart(multi_builder)
                .map_err(|e| Report::new(e.into()))
        } else {
            builder
                .header(ContentType::TEXT_HTML)
                .body(self.body_html)
                .map_err(|e| Report::new(e.into()))
        }
    }
}

impl From<lettre::transport::smtp::Error> for MailerServiceError {
    fn from(e: lettre::transport::smtp::Error) -> Self {
        MailerServiceError::Transport(Box::new(e))
    }
}

impl From<lettre::transport::stub::Error> for MailerServiceError {
    fn from(e: lettre::transport::stub::Error) -> Self {
        MailerServiceError::Transport(Box::new(e))
    }
}

impl From<lettre::error::Error> for MailerServiceError {
    fn from(e: lettre::error::Error) -> Self {
        MailerServiceError::EmailContent(Box::new(e))
    }
}

impl From<EmailAttachment> for SinglePart {
    fn from(attachment: EmailAttachment) -> Self {
        let content_type = match attachment.type_ {
            EmailAttachmentType::Pdf => ContentType::parse("application/pdf").unwrap(),
        };

        SinglePart::builder()
            .header(content_type)
            .header(ContentDisposition::attachment(attachment.filename.as_str()))
            .body(attachment.content)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::MailerConfig;
    use crate::services::mailer::{Email, EmailAttachment, EmailAttachmentType};

    #[tokio::test]
    async fn test_dummy() -> Result<(), Box<dyn std::error::Error>> {
        let cfg = MailerConfig::dummy();

        let pdf_data = vec![0; 8];

        let email = Email {
            from: "NoBody <hey@pp.com>".to_string(),
            reply_to: "NoBody <hey@pp.com>".to_string(),
            to: "Hei <a.b@gmail.com>".to_string(),
            subject: "Happy new year buddy".to_string(),
            body_html: "Please find the attached PDF! \n".to_string(),
            attachments: vec![EmailAttachment {
                filename: "example.pdf".to_string(),
                content: pdf_data,
                type_: EmailAttachmentType::Pdf,
            }],
        };

        let srv = crate::services::mailer::mailer_service(&cfg);

        srv.send_email(email).await?;

        Ok(())
    }
}
