use sailfish::RenderError;
use std::error::Error;

#[derive(Debug, thiserror::Error)]
pub enum MailerServiceError {
    #[error("Failed to initialize mailer service")]
    Initialization,

    #[error("Failed to convert email")]
    EmailContent(#[source] Box<dyn Error + Send + Sync>),

    #[error("Failed to send email")]
    Transport(#[source] Box<dyn Error + Send + Sync>),
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

impl From<lettre::address::AddressError> for MailerServiceError {
    fn from(e: lettre::address::AddressError) -> Self {
        MailerServiceError::EmailContent(Box::new(e))
    }
}

impl From<RenderError> for MailerServiceError {
    fn from(e: RenderError) -> Self {
        MailerServiceError::EmailContent(Box::new(e))
    }
}
