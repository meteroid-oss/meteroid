use crate::config::{MailerConfig, MailerSenders};
use crate::errors::MailerServiceError;
use crate::model::{
    AlertTriggered, Email, EmailAttachment, EmailRecipient, EmailValidationLink, OrgInvite,
    RenderedEmail, ResetPasswordLink,
};
use crate::render::Rendered;
use crate::template::{
    AlertTriggeredTemplate, EmailValidationLinkTemplate, OrgInviteTemplate,
    ResetPasswordLinkTemplate,
};
use async_trait::async_trait;
use error_stack::Report;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::stub::AsyncStubTransport;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use sailfish::TemplateSimple;
use secrecy::ExposeSecret;
use std::fmt::Write;
use std::sync::Arc;

/// Envelope of a customer email: `from` is a full mailbox (`Acme <billing@…>`, see
/// [`SenderCtx::from_mailbox`]), `bcc` the entity's audit copies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SenderCtx {
    pub from: String,
    pub reply_to: Option<String>,
    pub bcc: Vec<String>,
}

impl SenderCtx {
    /// `"Acme, Inc." <billing@acme.example>`: the display name is quoted as RFC 5322 requires.
    /// Control characters (a CR/LF would be a header injection) are dropped from the name.
    pub fn from_mailbox(
        display_name: &str,
        addr: &str,
    ) -> Result<String, Report<MailerServiceError>> {
        let address = addr.parse().map_err(|e: lettre::address::AddressError| {
            Report::new(MailerServiceError::InvalidAddress(format!(
                "from {addr:?}: {e}"
            )))
        })?;
        let name: String = display_name.chars().filter(|c| !c.is_control()).collect();
        let name = Some(name.trim().to_string()).filter(|n| !n.is_empty());
        let mut from = String::new();
        write!(from, "{}", Mailbox::new(name, address)).map_err(|_| {
            Report::new(MailerServiceError::InvalidAddress(format!(
                "from name {display_name:?}"
            )))
        })?;
        Ok(from)
    }

    fn recipient(email: String) -> EmailRecipient {
        EmailRecipient {
            email,
            first_name: None,
            last_name: None,
        }
    }

    pub fn email(
        self,
        to: Vec<EmailRecipient>,
        rendered: Rendered,
        attachments: Vec<EmailAttachment>,
    ) -> Email {
        Email {
            from: self.from,
            to,
            reply_to: self.reply_to,
            bcc: self.bcc.into_iter().map(Self::recipient).collect(),
            subject: rendered.subject,
            body_html: rendered.body_html,
            attachments,
        }
    }
}

#[async_trait]
pub trait MailerService: Send + Sync {
    /// Bare address customer emails are sent from (`MAILER_CUSTOMER_FROM_ADDR` or `MAILER_FROM`).
    fn customer_from_addr(&self) -> &str;

    async fn send(&self, email: Email) -> Result<bool, Report<MailerServiceError>>;
    async fn send_reset_password_link(
        &self,
        link: ResetPasswordLink,
    ) -> Result<(), Report<MailerServiceError>>;

    async fn send_email_validation_link(
        &self,
        link: EmailValidationLink,
    ) -> Result<(), Report<MailerServiceError>>;

    async fn send_org_invite(&self, data: OrgInvite) -> Result<(), Report<MailerServiceError>>;

    async fn send_alert_triggered(
        &self,
        data: AlertTriggered,
    ) -> Result<RenderedEmail, Report<MailerServiceError>>;
}

pub struct LettreMailerService<T: AsyncTransport> {
    pub transport: Arc<T>,
    pub senders: MailerSenders,
}

#[async_trait]
impl<T: AsyncTransport + Send + Sync> MailerService for LettreMailerService<T>
where
    T::Error: Into<MailerServiceError>,
{
    fn customer_from_addr(&self) -> &str {
        &self.senders.customer_from_addr
    }

    async fn send(&self, email: Email) -> Result<bool, Report<MailerServiceError>> {
        let to = email
            .to
            .iter()
            .map(|r| r.email.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let Some(email) = email.without_test_recipients() else {
            log::debug!("Skipping sending email to {to} as it is a test email");
            return Ok(false);
        };

        let message: Message = email.try_into()?;
        let _ = self
            .transport
            .send(message)
            .await
            .map_err(|e| Report::new(e.into()))?;

        Ok(true)
    }

    async fn send_reset_password_link(
        &self,
        link: ResetPasswordLink,
    ) -> Result<(), Report<MailerServiceError>> {
        let tpl = ResetPasswordLinkTemplate::from(link.clone());

        let body_html = tpl.render_once().map_err(|e| Report::new(e.into()))?;

        let email = Email {
            from: self.senders.from.clone(),
            reply_to: self.senders.reply_to.clone(),
            to: vec![link.recipient.clone()],
            bcc: vec![],
            subject: "Reset password".into(),
            body_html,
            attachments: vec![],
        };
        self.send(email).await.map(|_| ())
    }

    async fn send_email_validation_link(
        &self,
        link: EmailValidationLink,
    ) -> Result<(), Report<MailerServiceError>> {
        let tpl = EmailValidationLinkTemplate::from(link.clone()).tpl;

        let body_html = tpl.render_once().map_err(|e| Report::new(e.into()))?;

        let email = Email {
            from: self.senders.from.clone(),
            reply_to: self.senders.reply_to.clone(),
            to: vec![link.recipient.clone()],
            bcc: vec![],
            subject: "Confirm your email".into(),
            body_html,
            attachments: vec![],
        };
        self.send(email).await.map(|_| ())
    }

    async fn send_org_invite(&self, data: OrgInvite) -> Result<(), Report<MailerServiceError>> {
        let tpl = OrgInviteTemplate::from(data.clone()).tpl;
        let body_html = tpl.render_once().map_err(|e| Report::new(e.into()))?;
        let email = Email {
            from: self.senders.from.clone(),
            reply_to: self.senders.reply_to.clone(),
            to: vec![data.recipient],
            bcc: vec![],
            subject: format!("You've been invited to join {} on Meteroid", data.org_name),
            body_html,
            attachments: vec![],
        };
        self.send(email).await.map(|_| ())
    }

    async fn send_alert_triggered(
        &self,
        data: AlertTriggered,
    ) -> Result<RenderedEmail, Report<MailerServiceError>> {
        let tpl = AlertTriggeredTemplate::from(data.clone()).tpl;
        let subject = tpl.title.clone();
        let from = self.senders.from.clone();
        let reply_to = self.senders.reply_to.clone();
        let body_html = tpl.render_once().map_err(|e| Report::new(e.into()))?;
        let recipients = data.recipients.clone();

        let email = Email {
            from: from.clone(),
            reply_to: reply_to.clone(),
            to: recipients.clone(),
            bcc: vec![],
            subject: subject.clone(),
            body_html: body_html.clone(),
            attachments: vec![],
        };
        let delivered = self.send(email).await?;
        Ok(RenderedEmail {
            subject,
            from,
            reply_to,
            recipients,
            body_html,
            attachment_filenames: vec![],
            delivered,
        })
    }
}

/// Panics on invalid senders, so a misconfigured process fails at startup.
pub fn mailer_service(cfg: MailerConfig) -> Arc<dyn MailerService> {
    let senders = cfg
        .senders()
        .unwrap_or_else(|e| panic!("Invalid mailer configuration: {e}"));
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
            log::info!("Starting mailer service with TLS to host {host}");
            // TODO pool ?
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
                .unwrap()
                .credentials(creds)
                .timeout(Some(std::time::Duration::from_secs(10)))
                .build()
        } else {
            log::info!("Starting unsecure mailer service to host {host}");
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host.to_string())
                .credentials(creds)
                .timeout(Some(std::time::Duration::from_secs(10)))
                .build()
        };

        Arc::new(LettreMailerService {
            transport: Arc::new(transport),
            senders,
        })
    } else {
        Arc::new(LettreMailerService {
            transport: Arc::new(AsyncStubTransport::new_ok()),
            senders,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::config::MailerConfig;
    use crate::model::{Email, EmailAttachment, EmailAttachmentType, EmailRecipient};
    use crate::service::{SenderCtx, mailer_service};

    #[test]
    fn from_mailbox_drops_control_characters_instead_of_panicking() {
        let from = SenderCtx::from_mailbox("Acme\r\nBcc: x@evil.example", "billing@acme.example")
            .expect("a valid mailbox");
        assert!(!from.contains(['\r', '\n']), "{from}");
        assert!(from.ends_with("<billing@acme.example>"), "{from}");
        assert_eq!(
            SenderCtx::from_mailbox("\n\t", "billing@acme.example").unwrap(),
            "billing@acme.example"
        );
        assert_eq!(
            SenderCtx::from_mailbox("Acme, Inc.", "billing@acme.example").unwrap(),
            "\"Acme, Inc.\" <billing@acme.example>"
        );
    }

    #[tokio::test]
    async fn only_test_recipients_are_not_delivered() {
        let srv = mailer_service(MailerConfig::dummy());
        let email = |to: &[&str]| Email {
            from: "NoBody <hey@pp.com>".to_string(),
            reply_to: None,
            to: to
                .iter()
                .map(|e| EmailRecipient {
                    email: e.to_string(),
                    first_name: None,
                    last_name: None,
                })
                .collect(),
            bcc: vec![],
            subject: "s".to_string(),
            body_html: "<p>b</p>".to_string(),
            attachments: vec![],
        };
        assert!(!srv.send(email(&["a@acme.test"])).await.unwrap());
        assert!(
            srv.send(email(&["a@acme.test", "b@acme.example"]))
                .await
                .unwrap()
        );
    }

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
            bcc: vec![],
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

    #[tokio::test]
    async fn test_alert_triggered_renders() -> Result<(), Box<dyn std::error::Error>> {
        use crate::model::AlertTriggered;

        let srv = mailer_service(MailerConfig::dummy());

        let rendered = srv
            .send_alert_triggered(AlertTriggered {
                alert_name: "MRR drop".to_string(),
                severity: "CRITICAL".to_string(),
                summary: "[CRITICAL] MRR drop: value 12,345 on 2026-06-14".to_string(),
                metric_key: "mrr".to_string(),
                window_date: chrono::NaiveDate::from_ymd_opt(2026, 6, 14).unwrap(),
                alerts_url: "https://app.meteroid.com/acme/prod/alerts".to_string(),
                recipients: vec![EmailRecipient {
                    email: "ops@example.com".into(),
                    first_name: None,
                    last_name: None,
                }],
            })
            .await?;

        // The body should carry the deep link and the human summary.
        assert!(
            rendered
                .body_html
                .contains("https://app.meteroid.com/acme/prod/alerts")
        );
        assert!(rendered.body_html.contains("MRR drop"));
        assert!(rendered.subject.contains("CRITICAL"));

        Ok(())
    }
}
