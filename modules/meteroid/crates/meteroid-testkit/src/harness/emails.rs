//! Email worker helpers.

use std::sync::Arc;

use meteroid::workers::pgmq::processors::run_once_send_email;
use meteroid_mailer::model::Email;

use super::TestEnv;

impl TestEnv {
    /// Run the email worker until the queue is empty and return everything it sent.
    /// A worker error fails the test instead of reading as "nothing was sent".
    #[allow(dead_code)]
    pub async fn drain_email_queue(&self) -> Vec<Email> {
        let config = &self.setup.config;
        loop {
            let read = run_once_send_email(
                Arc::new(self.store().clone()),
                Arc::new(self.services().clone()),
                self.mailer.clone(),
                self.setup.object_store.clone(),
                config.public_url.clone(),
                config.rest_api_external_url.clone(),
            )
            .await
            .expect("email worker failed");
            if read == 0 {
                break;
            }
        }
        self.mailer.take()
    }
}
