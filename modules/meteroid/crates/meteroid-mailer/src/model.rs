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
    pub subject: String,
    pub body_html: String,
    pub attachments: Vec<EmailAttachment>,
}

impl Email {
    pub fn include_attachments(&self) -> bool {
        !self.attachments.is_empty()
    }
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

impl TryInto<Message> for Email {
    type Error = Report<MailerServiceError>;

    fn try_into(self) -> Result<Message, Self::Error> {
        let mut builder = Message::builder()
            .from(self.from.parse().expect("Invalid from address"))
            .subject(self.subject.clone());

        for recipient in self.to.iter() {
            let mailbox: Mailbox = recipient.clone().try_into()?;
            builder = builder.to(mailbox);
        }

        if let Some(reply_to) = self.reply_to.as_ref() {
            builder = builder.reply_to(reply_to.parse().expect("Invalid reply-to address"));
        }

        if self.include_attachments() {
            let mut multi_builder = MultiPart::mixed().singlepart(SinglePart::html(self.body_html));

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

#[derive(Clone)]
pub struct InvoiceLine {
    pub description: String,
    pub quantity: String,
    pub unit_price: String,
    pub total: String,
    pub from_date: Option<NaiveDate>,
    pub to_date: Option<NaiveDate>,
}

#[derive(Clone)]
pub struct InvoiceReady {
    pub invoice_number: String,
    pub invoice_date: NaiveDate,
    pub invoice_due_date: NaiveDate,
    pub label: String,
    pub amount_due: i64,
    pub currency: String,
    pub payment_url: String,
    pub company_name: String,
    pub logo_url: Option<String>,
    pub recipients: Vec<EmailRecipient>,
    pub attachment: EmailAttachment,
    pub account: String,
}

#[derive(Clone)]
pub struct InvoicePaid {
    pub invoice_number: String,
    pub invoice_date: NaiveDate,
    pub invoice_due_date: NaiveDate,
    pub label: String,
    pub amount_paid: i64,
    pub currency: String,
    pub company_name: String,
    pub logo_url: Option<String>,
    pub recipients: Vec<EmailRecipient>,
    pub attachments: Vec<EmailAttachment>,
    //
    pub lines: Vec<InvoiceLine>,
    pub account: String, // is checkout
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
