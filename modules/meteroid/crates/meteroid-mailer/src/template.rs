use crate::model::{EmailValidationLink, ResetPasswordLink};
use sailfish::TemplateSimple;
use secrecy::ExposeSecret;

#[derive(TemplateSimple)]
#[template(path = "reset_password_link.html")]
pub struct ResetPasswordLinkTemplate {
    pub reset_url: String,
    pub url_expires_in: String,
}

impl From<ResetPasswordLink> for ResetPasswordLinkTemplate {
    fn from(link: ResetPasswordLink) -> Self {
        ResetPasswordLinkTemplate {
            reset_url: link.url.expose_secret().clone(),
            url_expires_in: format_duration(link.url_expires_in),
        }
    }
}

#[derive(TemplateSimple)]
#[template(path = "email_validation_link.html")]
pub struct EmailValidationLinkTemplate {
    pub validation_url: String,
    pub url_expires_in: String,
}

impl From<EmailValidationLink> for EmailValidationLinkTemplate {
    fn from(link: EmailValidationLink) -> Self {
        EmailValidationLinkTemplate {
            validation_url: link.url.expose_secret().clone(),
            url_expires_in: format_duration(link.url_expires_in),
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
