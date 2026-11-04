use async_trait::async_trait;
use error_stack::Report;
use meteroid_mailer::errors::MailerServiceError;
use meteroid_mailer::model::{
    AlertTriggered, Email, EmailValidationLink, OrgInvite, RenderedEmail, ResetPasswordLink,
};

mockall::mock! {
    pub MailerService {}

    #[async_trait]
    impl meteroid_mailer::service::MailerService for MailerService {
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
}
