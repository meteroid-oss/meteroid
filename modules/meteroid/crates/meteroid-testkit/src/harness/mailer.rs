//! A `MailerService` that keeps every email instead of sending it.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use error_stack::Report;
use meteroid_mailer::config::MailerConfig;
use meteroid_mailer::errors::MailerServiceError;
use meteroid_mailer::model::{
    AlertTriggered, Email, EmailAttachment, EmailAttachmentType, EmailValidationLink, OrgInvite,
    RenderedEmail, ResetPasswordLink,
};
use meteroid_mailer::service::{MailerService, mailer_service};

/// Captures every email the real service would have handed to the transport; `.test` /
/// `.invalid` recipients are skipped like in production. `fail_next` and `delay` simulate a
/// flaky and a slow transport.
pub struct CapturingMailer {
    inner: Arc<dyn MailerService>,
    sent: Mutex<Vec<Email>>,
    fail_next: AtomicU32,
    delay: Mutex<Option<Duration>>,
}

impl Default for CapturingMailer {
    fn default() -> Self {
        Self::new()
    }
}

impl CapturingMailer {
    pub fn new() -> Self {
        Self {
            inner: mailer_service(MailerConfig::dummy()),
            sent: Mutex::new(Vec::new()),
            fail_next: AtomicU32::new(0),
            delay: Mutex::new(None),
        }
    }

    /// The next `n` sends fail with a transport error instead of being captured.
    pub fn fail_next(&self, n: u32) {
        self.fail_next.store(n, Ordering::SeqCst);
    }

    /// Every send sleeps this long before capturing (to hold a claim in flight).
    pub fn set_delay(&self, delay: Option<Duration>) {
        *self.delay.lock().expect("delay poisoned") = delay;
    }

    pub fn sent(&self) -> MutexGuard<'_, Vec<Email>> {
        self.sent.lock().expect("captured emails poisoned")
    }

    pub fn take(&self) -> Vec<Email> {
        std::mem::take(&mut *self.sent())
    }
}

#[async_trait]
impl MailerService for CapturingMailer {
    fn customer_from_addr(&self) -> &str {
        self.inner.customer_from_addr()
    }

    async fn send(&self, email: Email) -> Result<bool, Report<MailerServiceError>> {
        let Some(email) = email.without_test_recipients() else {
            return Ok(false);
        };
        let delay = *self.delay.lock().expect("delay poisoned");
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        if self
            .fail_next
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(Report::new(MailerServiceError::Transport(Box::new(
                std::io::Error::other("simulated transport failure"),
            ))));
        }
        self.sent().push(email);
        Ok(true)
    }

    async fn send_reset_password_link(
        &self,
        link: ResetPasswordLink,
    ) -> Result<(), Report<MailerServiceError>> {
        self.inner.send_reset_password_link(link).await
    }

    async fn send_email_validation_link(
        &self,
        link: EmailValidationLink,
    ) -> Result<(), Report<MailerServiceError>> {
        self.inner.send_email_validation_link(link).await
    }

    async fn send_org_invite(&self, data: OrgInvite) -> Result<(), Report<MailerServiceError>> {
        self.inner.send_org_invite(data).await
    }

    async fn send_alert_triggered(
        &self,
        data: AlertTriggered,
    ) -> Result<RenderedEmail, Report<MailerServiceError>> {
        let rendered = self.inner.send_alert_triggered(data).await?;
        if rendered.delivered {
            self.sent().push(Email {
                from: rendered.from.clone(),
                to: rendered.recipients.clone(),
                reply_to: rendered.reply_to.clone(),
                bcc: vec![],
                subject: rendered.subject.clone(),
                body_html: rendered.body_html.clone(),
                attachments: rendered
                    .attachment_filenames
                    .iter()
                    .map(|filename| EmailAttachment {
                        filename: filename.clone(),
                        content: Vec::new(),
                        type_: EmailAttachmentType::Pdf,
                    })
                    .collect(),
            });
        }
        Ok(rendered)
    }
}
