use crate::errors::MailerServiceError;
use chrono::NaiveDate;
use error_stack::Report;
use itertools::Itertools;
use lettre::message::header::{ContentDisposition, ContentType};
use lettre::message::{Mailbox, MultiPart, SinglePart};
use lettre::{Address, Message};
use secrecy::SecretString;
use std::str::FromStr;

pub struct Email {
    pub from: String,
    pub to: Vec<EmailRecipient>,
    pub reply_to: Option<String>,
    pub bcc: Vec<EmailRecipient>,
    pub subject: String,
    pub body_html: String,
    pub attachments: Vec<EmailAttachment>,
}

impl Email {
    pub fn include_attachments(&self) -> bool {
        !self.attachments.is_empty()
    }

    /// `.test` / `.invalid` recipients (RFC 2606) are never handed to the transport; `None`
    /// when no other recipient is left, the email is then not sent at all.
    pub fn without_test_recipients(mut self) -> Option<Email> {
        self.to.retain(|r| !is_test_address(&r.email));
        (!self.to.is_empty()).then_some(self)
    }
}

/// RFC 2606 reserved TLDs: seeded and sandbox customers use them, nothing is ever delivered.
pub fn is_test_address(email: &str) -> bool {
    let email = email.trim().to_ascii_lowercase();
    email.ends_with(".test") || email.ends_with(".invalid")
}

/// `delivered = false` means the transport short-circuited (e.g. test address);
/// the body was rendered but never sent.
#[derive(Clone)]
pub struct RenderedEmail {
    pub subject: String,
    pub from: String,
    pub reply_to: Option<String>,
    pub recipients: Vec<EmailRecipient>,
    pub body_html: String,
    pub attachment_filenames: Vec<String>,
    pub delivered: bool,
}

#[derive(Clone)]
pub struct EmailAttachment {
    pub filename: String,
    pub content: Vec<u8>,
    pub type_: EmailAttachmentType,
}

#[derive(Clone)]
pub enum EmailAttachmentType {
    Pdf,
}

/// A bare address (`billing@acme.example`), as stored for reply-to and bcc.
pub fn validate_email_address(value: &str) -> Result<(), String> {
    Address::from_str(value)
        .map(|_| ())
        .map_err(|e| format!("{value:?}: {e}"))
}

fn parse_mailbox(field: &str, value: &str) -> Result<Mailbox, Report<MailerServiceError>> {
    value.parse().map_err(|e: lettre::address::AddressError| {
        Report::new(MailerServiceError::InvalidAddress(format!(
            "{field} {value:?}: {e}"
        )))
    })
}

impl TryInto<Message> for Email {
    type Error = Report<MailerServiceError>;

    fn try_into(self) -> Result<Message, Self::Error> {
        let mut builder = Message::builder()
            .from(parse_mailbox("from", &self.from)?)
            .subject(self.subject.clone());

        for recipient in &self.to {
            let mailbox: Mailbox = recipient.clone().try_into()?;
            builder = builder.to(mailbox);
        }

        for recipient in &self.bcc {
            let mailbox: Mailbox = recipient.clone().try_into()?;
            builder = builder.bcc(mailbox);
        }

        if let Some(reply_to) = self.reply_to.as_ref() {
            builder = builder.reply_to(parse_mailbox("reply-to", reply_to)?);
        }

        if self.include_attachments() {
            let mut multi_builder = MultiPart::mixed().singlepart(SinglePart::html(self.body_html));

            for attachment in self.attachments {
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

#[derive(Clone)]
pub struct EmailRecipient {
    pub email: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}

#[derive(Clone)]
pub struct ResetPasswordLink {
    pub url: SecretString,
    pub url_expires_in: chrono::Duration,
    pub recipient: EmailRecipient,
}

#[derive(Clone)]
pub struct EmailValidationLink {
    pub url: SecretString,
    pub url_expires_in: chrono::Duration,
    pub recipient: EmailRecipient,
}

/// A triggered analytics alert, delivered as a concise notification email that
/// mirrors the Slack/Discord summary and deep-links back to the in-app alerts
/// feed for the full details.
#[derive(Clone)]
pub struct AlertTriggered {
    /// Human alert name, e.g. "MRR drop".
    pub alert_name: String,
    /// Severity label already rendered for display: "INFO" / "WARNING" / "CRITICAL".
    pub severity: String,
    /// One-line summary, e.g. "[CRITICAL] MRR drop: value 12,345 on 2026-06-14".
    pub summary: String,
    /// Catalog metric key the alert watches, e.g. "mrr".
    pub metric_key: String,
    /// The window date the firing is attributed to.
    pub window_date: NaiveDate,
    /// Deep link to the in-app alerts feed for the owning tenant.
    pub alerts_url: String,
    pub recipients: Vec<EmailRecipient>,
}

#[derive(Clone)]
pub struct OrgInvite {
    pub org_name: String,
    pub inviter_name: String,
    pub role: String,
    pub invite_url: String,
    pub expires_in: String,
    pub recipient: EmailRecipient,
}

impl TryInto<Mailbox> for EmailRecipient {
    type Error = Report<MailerServiceError>;

    fn try_into(self) -> Result<Mailbox, Self::Error> {
        let address = Address::from_str(self.email.as_str()).map_err(|e| Report::new(e.into()))?;

        let name = [self.first_name.as_ref(), self.last_name.as_ref()]
            .into_iter()
            .flatten()
            .join(" ");

        let name = if name.is_empty() { None } else { Some(name) };

        Ok(Mailbox::new(name, address))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipient(email: &str) -> EmailRecipient {
        EmailRecipient {
            email: email.into(),
            first_name: None,
            last_name: None,
        }
    }

    fn email(from: &str, reply_to: Option<&str>, bcc: Vec<&str>) -> Email {
        Email {
            from: from.into(),
            to: vec![recipient("to@example.com")],
            reply_to: reply_to.map(str::to_string),
            bcc: bcc.into_iter().map(recipient).collect(),
            subject: "s".into(),
            body_html: "<p>b</p>".into(),
            attachments: vec![],
        }
    }

    #[test]
    fn test_recipients_are_dropped_and_the_rest_still_receive() {
        let mut mixed = email("a@example.com", None, vec![]);
        mixed.to = vec![
            recipient("ops@acme.test"),
            recipient("billing@acme.example"),
            recipient("x@y.INVALID"),
        ];
        let kept = mixed.without_test_recipients().expect("a real recipient");
        let to: Vec<&str> = kept.to.iter().map(|r| r.email.as_str()).collect();
        assert_eq!(to, vec!["billing@acme.example"]);

        let mut only_test = email("a@example.com", None, vec![]);
        only_test.to = vec![recipient("a@acme.test"), recipient("b@acme.invalid")];
        assert!(only_test.without_test_recipients().is_none());
        assert!(!is_test_address("someone@testing.com"));
    }

    #[test]
    fn invalid_from_is_an_error_not_a_panic() {
        let err: Result<Message, _> = email("not an address", None, vec![]).try_into();
        assert!(matches!(
            err.unwrap_err().current_context(),
            MailerServiceError::InvalidAddress(m) if m.starts_with("from ")
        ));
    }

    #[test]
    fn invalid_reply_to_is_an_error() {
        let err: Result<Message, _> = email("a@example.com", Some("<<nope"), vec![]).try_into();
        assert!(matches!(
            err.unwrap_err().current_context(),
            MailerServiceError::InvalidAddress(m) if m.starts_with("reply-to ")
        ));
    }

    #[test]
    fn invalid_bcc_is_an_error() {
        let err: Result<Message, _> = email("a@example.com", None, vec!["nope"]).try_into();
        assert!(matches!(
            err.unwrap_err().current_context(),
            MailerServiceError::EmailContent(_)
        ));
    }

    #[test]
    fn bcc_lands_in_the_message_envelope() {
        let message: Message = email(
            "\"Acme, Inc.\" <billing@example.com>",
            Some("ar@example.com"),
            vec!["audit@example.com", "boss@example.com"],
        )
        .try_into()
        .unwrap();
        let bcc: Vec<String> = message
            .envelope()
            .to()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            bcc,
            vec!["to@example.com", "audit@example.com", "boss@example.com"]
        );
    }
}
