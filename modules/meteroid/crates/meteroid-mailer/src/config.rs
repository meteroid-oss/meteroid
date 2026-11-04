use crate::errors::MailerServiceError;
use envconfig::Envconfig;
use lettre::Address;
use lettre::message::Mailbox;
use secrecy::SecretString;

/// Without an SMTP host nothing is delivered, so the sender needs no real domain.
const UNDELIVERED_FROM: &str = "Meteroid <mailer@meteroid.invalid>";

#[derive(Envconfig, Debug, Clone)]
pub struct MailerConfig {
    #[envconfig(from = "MAILER_SMTP_HOST")]
    pub smtp_host: Option<String>,
    #[envconfig(from = "MAILER_SMTP_USERNAME")]
    pub smtp_username: Option<SecretString>,
    #[envconfig(from = "MAILER_SMTP_PASSWORD")]
    pub smtp_password: Option<SecretString>,
    #[envconfig(from = "MAILER_SMTP_TLS", default = "true")]
    pub smtp_tls: bool,
    /// Sender of the account and alert emails; required once `MAILER_SMTP_HOST` is set.
    #[envconfig(from = "MAILER_FROM")]
    pub from: Option<String>,
    /// Bare address customer emails are sent from, behind the entity's sender name; unset, the
    /// address of `MAILER_FROM`.
    #[envconfig(from = "MAILER_CUSTOMER_FROM_ADDR")]
    pub customer_from_addr: Option<String>,
    /// Reply-To of the account and alert emails; none when unset.
    #[envconfig(from = "MAILER_REPLY_TO")]
    pub reply_to: Option<String>,
}

/// The validated senders of a [`MailerConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailerSenders {
    pub from: String,
    pub customer_from_addr: String,
    pub reply_to: Option<String>,
}

fn set(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

fn invalid(var: &str, value: &str, e: impl std::fmt::Display) -> MailerServiceError {
    MailerServiceError::InvalidAddress(format!("{var} {value:?}: {e}"))
}

impl MailerConfig {
    pub fn dummy() -> Self {
        MailerConfig {
            smtp_host: None,
            smtp_username: None,
            smtp_password: None,
            smtp_tls: true,
            from: Some("Meteroid <mailer@example.com>".to_string()),
            customer_from_addr: None,
            reply_to: None,
        }
    }

    pub fn smtp_enabled(&self) -> bool {
        set(self.smtp_host.as_deref()).is_some()
    }

    pub fn senders(&self) -> Result<MailerSenders, MailerServiceError> {
        let from = match set(self.from.as_deref()) {
            Some(from) => from,
            None if !self.smtp_enabled() => UNDELIVERED_FROM,
            None => {
                return Err(MailerServiceError::InvalidAddress(
                    "MAILER_FROM is required when MAILER_SMTP_HOST is set".to_string(),
                ));
            }
        };
        let from_mailbox: Mailbox = from.parse().map_err(|e| invalid("MAILER_FROM", from, e))?;
        let customer_from_addr = match set(self.customer_from_addr.as_deref()) {
            Some(addr) => addr
                .parse::<Address>()
                .map_err(|e| invalid("MAILER_CUSTOMER_FROM_ADDR", addr, e))?
                .to_string(),
            None => from_mailbox.email.to_string(),
        };
        let reply_to = set(self.reply_to.as_deref())
            .map(|r| {
                r.parse::<Mailbox>()
                    .map(|_| r.to_string())
                    .map_err(|e| invalid("MAILER_REPLY_TO", r, e))
            })
            .transpose()?;
        Ok(MailerSenders {
            from: from.to_string(),
            customer_from_addr,
            reply_to,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMTP: Option<&str> = Some("smtp.acme.example");

    fn senders(
        smtp_host: Option<&str>,
        from: Option<&str>,
        customer_from_addr: Option<&str>,
        reply_to: Option<&str>,
    ) -> Result<MailerSenders, MailerServiceError> {
        MailerConfig {
            smtp_host: smtp_host.map(str::to_string),
            from: from.map(str::to_string),
            customer_from_addr: customer_from_addr.map(str::to_string),
            reply_to: reply_to.map(str::to_string),
            ..MailerConfig::dummy()
        }
        .senders()
    }

    #[test]
    fn customer_address_defaults_to_the_address_of_mailer_from() {
        let from = Some("Acme <ops@acme.example>");
        let s = senders(SMTP, from, None, None).unwrap();
        assert_eq!(s.from, "Acme <ops@acme.example>");
        assert_eq!(s.customer_from_addr, "ops@acme.example");
        assert_eq!(s.reply_to, None);

        let s = senders(SMTP, Some("ops@acme.example"), Some(""), Some("  ")).unwrap();
        assert_eq!(s.customer_from_addr, "ops@acme.example");
        assert_eq!(s.reply_to, None);
    }

    #[test]
    fn explicit_addresses_win() {
        let from = Some("Acme <ops@acme.example>");
        let reply_to = Some("No Reply <no-reply@acme.example>");
        let s = senders(SMTP, from, Some("billing@acme.example"), reply_to).unwrap();
        assert_eq!(s.customer_from_addr, "billing@acme.example");
        assert_eq!(s.reply_to.as_deref(), reply_to);
    }

    #[test]
    fn mailer_from_is_required_only_with_smtp() {
        assert!(senders(SMTP, None, None, None).is_err());
        assert!(senders(SMTP, Some(" "), None, None).is_err());

        let s = senders(Some(""), None, None, None).unwrap();
        assert_eq!(s.from, UNDELIVERED_FROM);
        assert_eq!(s.customer_from_addr, "mailer@meteroid.invalid");
    }

    #[test]
    fn invalid_addresses_are_rejected() {
        let customer = Some("Acme <billing@acme.example>");
        assert!(senders(None, Some("not an address"), None, None).is_err());
        assert!(senders(None, None, customer, None).is_err());
        assert!(senders(None, None, None, Some("nope")).is_err());
    }
}
