use crate::model::{EmailValidationLink, InvoicePaid, InvoiceReady, QuoteReady, ResetPasswordLink};
use sailfish::TemplateSimple;
use secrecy::ExposeSecret;

#[derive(TemplateSimple)]
#[template(path = "reset_password_link.html")]
pub struct ResetPasswordLinkTemplate {
    pub reset_url: String,
    pub url_expires_in: String,
}

const METEROID_WORDMARK_URL: &str =
    "https://static.meteroid.com/assets/mail-images/meteroid-logo-wordmark--light.png";

impl From<ResetPasswordLink> for ResetPasswordLinkTemplate {
    fn from(link: ResetPasswordLink) -> Self {
        ResetPasswordLinkTemplate {
            reset_url: link.url.expose_secret().to_string(),
            url_expires_in: format_duration(link.url_expires_in),
        }
    }
}

#[derive(TemplateSimple)]
#[template(path = "email_validation_link.stpl")]
pub struct EmailValidationLinkContent {
    pub validation_url: String,
    pub url_expires_in: String,
    pub user: String,
}

pub struct EmailValidationLinkTemplate {
    pub tpl: LayoutTemplate<EmailValidationLinkContent>,
}

impl From<EmailValidationLink> for EmailValidationLinkTemplate {
    fn from(link: EmailValidationLink) -> Self {
        let header = HeaderTemplate {
            company_name: String::new(),
            logo_url: Some(METEROID_WORDMARK_URL.to_string()),
        };
        let footer = FooterTemplate {};

        let user = link.recipient.first_name.unwrap_or(
            link.recipient
                .email
                .split('@')
                .next()
                .map(std::string::ToString::to_string)
                .unwrap_or(link.recipient.email),
        );

        let content = EmailValidationLinkContent {
            validation_url: link.url.expose_secret().to_string(),
            url_expires_in: format_duration(link.url_expires_in),
            user,
        };
        EmailValidationLinkTemplate {
            tpl: LayoutTemplate {
                lang: "en".to_string(),
                title: "Confirm your email".to_string(),
                header,
                footer,
                content,
            },
        }
    }
}

fn format_duration(duration: chrono::Duration) -> String {
    if duration.num_days() > 1 {
        format!("{} days", duration.num_days())
    } else if duration.num_hours() > 1 {
        format!("{} hours", duration.num_hours())
    } else {
        format!("{} minutes", duration.num_minutes())
    }
}

fn format_date(date: chrono::NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn format_currency(amount: i64, currency: &str) -> String {
    let cur = rusty_money::iso::find(currency).unwrap_or(rusty_money::iso::USD);
    let amount = rusty_money::Money::from_minor(amount, cur);
    format!("{amount}")
}

#[derive(TemplateSimple)]
#[template(path = "layout.stpl")]
pub struct LayoutTemplate<A: TemplateSimple> {
    pub lang: String,
    pub title: String,
    pub header: HeaderTemplate,
    pub footer: FooterTemplate,
    pub content: A,
}

#[derive(TemplateSimple)]
#[template(path = "header.stpl")]
pub struct HeaderTemplate {
    pub company_name: String,
    pub logo_url: Option<String>,
}

#[derive(TemplateSimple)]
#[template(path = "footer.stpl")]
pub struct FooterTemplate {}

#[derive(TemplateSimple)]
#[template(path = "invoice_ready.stpl")]
pub struct InvoiceReadyContent {
    pub invoice_number: String,
    pub invoice_date: String,
    pub invoice_due_date: String,
    pub label: String,
    pub amount_due: String,
    pub payment_url: String,
}

pub struct InvoiceReadyTemplate {
    pub tpl: LayoutTemplate<InvoiceReadyContent>,
}

impl From<InvoiceReady> for InvoiceReadyTemplate {
    fn from(data: InvoiceReady) -> Self {
        let header = HeaderTemplate {
            company_name: data.company_name,
            logo_url: data.logo_url,
        };
        let footer = FooterTemplate {};
        let content = InvoiceReadyContent {
            invoice_number: data.invoice_number,
            invoice_date: format_date(data.invoice_date),
            invoice_due_date: format_date(data.invoice_due_date),
            label: data.label,
            amount_due: format_currency(data.amount_due, &data.currency),
            payment_url: data.payment_url,
        };
        InvoiceReadyTemplate {
            tpl: LayoutTemplate {
                lang: "en".to_string(),
                title: format!("Your {} invoice is ready", header.company_name),
                header,
                footer,
                content,
            },
        }
    }
}

#[derive(TemplateSimple)]
#[template(path = "invoice_receipt.stpl")]
pub struct InvoicePaidContent {
    pub invoice_number: String,
    pub invoice_date: String,
    pub invoice_due_date: String,
    pub label: String,
    pub amount_paid: String,
}

pub struct InvoicePaidTemplate {
    pub tpl: LayoutTemplate<InvoicePaidContent>,
}

impl From<InvoicePaid> for InvoicePaidTemplate {
    fn from(data: InvoicePaid) -> Self {
        let header = HeaderTemplate {
            company_name: data.company_name,
            logo_url: data.logo_url,
        };
        let footer = FooterTemplate {};
        let content = InvoicePaidContent {
            invoice_number: data.invoice_number,
            invoice_date: format_date(data.invoice_date),
            invoice_due_date: format_date(data.invoice_due_date),
            label: data.label,
            amount_paid: format_currency(data.amount_paid, &data.currency),
        };
        InvoicePaidTemplate {
            tpl: LayoutTemplate {
                lang: "en".to_string(),
                title: format!("Your {} receipt", header.company_name),
                header,
                footer,
                content,
            },
        }
    }
}

#[derive(TemplateSimple)]
#[template(path = "quote_ready.stpl")]
pub struct QuoteReadyContent {
    pub quote_number: String,
    pub expires_at: Option<String>,
    pub portal_url: String,
    pub custom_message: Option<String>,
}

pub struct QuoteReadyTemplate {
    pub tpl: LayoutTemplate<QuoteReadyContent>,
}

impl From<QuoteReady> for QuoteReadyTemplate {
    fn from(data: QuoteReady) -> Self {
        let header = HeaderTemplate {
            company_name: data.company_name.clone(),
            logo_url: data.logo_url,
        };
        let footer = FooterTemplate {};
        let content = QuoteReadyContent {
            quote_number: data.quote_number,
            expires_at: data.expires_at.map(format_date),
            portal_url: data.portal_url,
            custom_message: data.custom_message,
        };
        QuoteReadyTemplate {
            tpl: LayoutTemplate {
                lang: "en".to_string(),
                title: format!("Quote {} from {}", content.quote_number, data.company_name),
                header,
                footer,
                content,
            },
        }
    }
}
