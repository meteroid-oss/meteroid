//! Construction of the shared outbound HTTP clients. Every outbound call to a tenant- or
//! admin-supplied destination goes through one; they share the `WEBHOOK_*` egress settings, with
//! a per-purpose override for the proxy, and the edition's destination guard when it has one.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use egress_http::{DestinationPolicy, EgressClient, EgressClientConfig, EgressError};
use meteroid_store::services::webhooks::out::signing::USER_AGENT as WEBHOOKS_USER_AGENT;

use crate::config::WebhookOutConfig;

pub const ALERTS_USER_AGENT: &str = "Meteroid-Alerts/1.0";
pub const SSO_USER_AGENT: &str = "Meteroid-SSO/1.0";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EgressPurpose {
    Webhooks,
    Alerts,
    Sso,
}

impl EgressPurpose {
    fn user_agent(self) -> &'static str {
        match self {
            EgressPurpose::Webhooks => WEBHOOKS_USER_AGENT,
            EgressPurpose::Alerts => ALERTS_USER_AGENT,
            EgressPurpose::Sso => SSO_USER_AGENT,
        }
    }

    /// SSO identity providers do not allowlist our IPs, so logins skip the proxy unless
    /// told otherwise and keep working through a proxy outage.
    pub fn proxy_url(self, config: &WebhookOutConfig) -> Option<String> {
        match self {
            EgressPurpose::Webhooks | EgressPurpose::Alerts => config.egress_proxy_url.clone(),
            EgressPurpose::Sso => config
                .sso_use_proxy
                .then(|| config.egress_proxy_url.clone())
                .flatten(),
        }
    }
}

/// Restricts where each purpose's clients may send requests.
pub trait EgressGuard: Send + Sync {
    fn policy(
        &self,
        purpose: EgressPurpose,
        connect_timeout: Duration,
    ) -> Result<Arc<dyn DestinationPolicy>, EgressError>;
}

/// The process's outbound HTTP clients: the `WEBHOOK_*` egress settings and the edition's
/// destination guard. Built once by the process and passed down; clones share one instance.
#[derive(Clone)]
pub struct Egress(Arc<EgressInner>);

struct EgressInner {
    config: WebhookOutConfig,
    guard: Option<Arc<dyn EgressGuard>>,
    alerts: OnceLock<EgressClient>,
}

impl Egress {
    pub fn new(config: WebhookOutConfig, guard: Option<Arc<dyn EgressGuard>>) -> Self {
        Self(Arc::new(EgressInner {
            config,
            guard,
            alerts: OnceLock::new(),
        }))
    }

    pub fn config(&self) -> &WebhookOutConfig {
        &self.0.config
    }

    pub fn client(&self, purpose: EgressPurpose) -> Result<EgressClient, EgressError> {
        let policy = self
            .0
            .guard
            .as_ref()
            .map(|guard| guard.policy(purpose, CONNECT_TIMEOUT))
            .transpose()?;
        EgressClient::new(EgressClientConfig {
            policy,
            proxy_url: purpose.proxy_url(&self.0.config),
            connect_timeout: CONNECT_TIMEOUT,
            timeout: TIMEOUT,
            user_agent: purpose.user_agent().to_string(),
        })
    }

    /// For call sites that run once per tenant, so each call does not rebuild a resolver and a
    /// connection pool.
    pub fn shared_alerts_client(&self) -> Result<EgressClient, EgressError> {
        if let Some(client) = self.0.alerts.get() {
            return Ok(client.clone());
        }
        let client = self.client(EgressPurpose::Alerts)?;
        Ok(self.0.alerts.get_or_init(|| client).clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sso_skips_the_proxy_unless_opted_in() {
        let mut config = WebhookOutConfig {
            egress_proxy_url: Some("http://egress-proxy:3128".to_string()),
            ..WebhookOutConfig::default()
        };
        assert_eq!(EgressPurpose::Sso.proxy_url(&config), None);
        for purpose in [EgressPurpose::Webhooks, EgressPurpose::Alerts] {
            assert_eq!(
                purpose.proxy_url(&config),
                config.egress_proxy_url,
                "{purpose:?}"
            );
        }

        config.sso_use_proxy = true;
        assert_eq!(
            EgressPurpose::Sso.proxy_url(&config),
            config.egress_proxy_url
        );
    }

    #[test]
    fn without_a_guard_every_destination_is_reachable() {
        let egress = Egress::new(WebhookOutConfig::default(), None);
        for purpose in [
            EgressPurpose::Webhooks,
            EgressPurpose::Alerts,
            EgressPurpose::Sso,
        ] {
            let client = egress.client(purpose).expect("egress client");
            assert!(client.allows_private_destinations(), "{purpose:?}");
        }
    }
}
