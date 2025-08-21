use crate::config::Config;
use secrecy::ExposeSecret;
use std::sync::Arc;
use svix::api::Svix;

pub fn new_svix(config: &Config) -> Option<Arc<Svix>> {
    config.svix_server_url.clone().map(|x| {
        Arc::new(Svix::new(
            config.svix_jwt_token.expose_secret().clone(),
            Some(svix::api::SvixOptions {
                debug: true,
                server_url: Some(x),
                timeout: Some(std::time::Duration::from_secs(30)),
                num_retries: Some(3),
                retry_schedule: None,
                proxy_address: None,
            }),
        ))
    })
}
