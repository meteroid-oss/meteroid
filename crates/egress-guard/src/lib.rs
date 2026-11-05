//! The destination guard of outbound HTTP (SSRF): https only, ports 80 and 443, and no private
//! or reserved address, checked on the URL and on every DNS answer a connection uses.

mod dns;
mod ip_rules;
mod rules;

use std::sync::Arc;
use std::time::Duration;

use egress_http::{DestinationPolicy, EgressError, Lookup, Url};
use hickory_resolver::TokioResolver;
use meteroid_app::services::egress::{EgressGuard, EgressPurpose};

/// `WEBHOOK_ALLOW_PRIVATE_DESTINATIONS` and its SSO override.
pub struct DestinationGuard {
    allow_private_destinations: bool,
    sso_allow_private_destinations: Option<bool>,
}

impl DestinationGuard {
    pub fn new(
        allow_private_destinations: bool,
        sso_allow_private_destinations: Option<bool>,
    ) -> Self {
        Self {
            allow_private_destinations,
            sso_allow_private_destinations,
        }
    }

    /// Alerts follow the webhook setting; SSO defaults to it too, but can be relaxed on
    /// its own so an internal IdP can be allowed without opening webhooks.
    pub fn allows_private_destinations(&self, purpose: EgressPurpose) -> bool {
        match purpose {
            EgressPurpose::Webhooks | EgressPurpose::Alerts => self.allow_private_destinations,
            EgressPurpose::Sso => self
                .sso_allow_private_destinations
                .unwrap_or(self.allow_private_destinations),
        }
    }
}

impl EgressGuard for DestinationGuard {
    fn policy(
        &self,
        purpose: EgressPurpose,
        connect_timeout: Duration,
    ) -> Result<Arc<dyn DestinationPolicy>, EgressError> {
        Ok(Arc::new(GuardedDestinations::new(
            self.allows_private_destinations(purpose),
            connect_timeout,
        )?))
    }
}

/// The policy of one client. With private destinations allowed, every IP-level check is
/// skipped: self-hosters delivering inside their own network, local development and tests.
pub struct GuardedDestinations {
    resolver: Arc<TokioResolver>,
    allow_private_destinations: bool,
}

impl GuardedDestinations {
    pub fn new(allow_private_destinations: bool, timeout: Duration) -> Result<Self, EgressError> {
        Ok(Self {
            resolver: dns::resolver(timeout)?,
            allow_private_destinations,
        })
    }
}

impl DestinationPolicy for GuardedDestinations {
    fn allows_private_destinations(&self) -> bool {
        self.allow_private_destinations
    }

    fn check_destination(&self, url: &Url) -> Result<(), EgressError> {
        rules::check_destination(url, self.allow_private_destinations)
    }

    fn lookup(&self, host: String, guarded: bool) -> Lookup {
        let resolver = Arc::clone(&self.resolver);
        let allow_private = self.allow_private_destinations || !guarded;
        Box::pin(async move { dns::resolve_checked(&resolver, allow_private, &host).await })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use egress_http::{EgressClient, EgressClientConfig};
    use meteroid_app::config::WebhookOutConfig;
    use meteroid_app::services::egress::Egress;

    pub(crate) fn guarded_client(allow_private_destinations: bool) -> EgressClient {
        client_with(allow_private_destinations, None)
    }

    fn client_with(allow_private_destinations: bool, proxy_url: Option<&str>) -> EgressClient {
        EgressClient::new(EgressClientConfig {
            policy: Some(Arc::new(
                GuardedDestinations::new(allow_private_destinations, Duration::from_secs(2))
                    .expect("a resolver"),
            )),
            proxy_url: proxy_url.map(str::to_string),
            connect_timeout: Duration::from_secs(2),
            timeout: Duration::from_secs(2),
            ..EgressClientConfig::default()
        })
        .expect("a guarded client builds")
    }

    #[test]
    fn sso_and_alerts_default_to_the_webhook_setting() {
        let guard = DestinationGuard::new(true, None);
        for purpose in [
            EgressPurpose::Webhooks,
            EgressPurpose::Alerts,
            EgressPurpose::Sso,
        ] {
            assert!(guard.allows_private_destinations(purpose), "{purpose:?}");
        }
    }

    #[test]
    fn an_internal_idp_can_be_allowed_without_opening_webhooks() {
        let guard = DestinationGuard::new(false, Some(true));
        assert!(guard.allows_private_destinations(EgressPurpose::Sso));
        assert!(!guard.allows_private_destinations(EgressPurpose::Webhooks));
        assert!(!guard.allows_private_destinations(EgressPurpose::Alerts));
    }

    #[test]
    fn the_process_egress_builds_every_client_through_the_guard() {
        let guard = DestinationGuard::new(false, Some(true));
        let egress = Egress::new(WebhookOutConfig::default(), Some(Arc::new(guard)));
        let webhooks = egress.client(EgressPurpose::Webhooks).expect("client");
        let alerts = egress.shared_alerts_client().expect("client");
        let sso = egress.client(EgressPurpose::Sso).expect("client");
        assert!(!webhooks.allows_private_destinations());
        assert!(!alerts.allows_private_destinations());
        assert!(sso.allows_private_destinations());
    }

    #[test]
    fn the_guarded_client_keeps_no_response_bodies_unless_private_destinations_are_allowed() {
        assert!(!guarded_client(false).allows_private_destinations());
        assert!(guarded_client(true).allows_private_destinations());
    }

    #[tokio::test]
    async fn the_proxy_host_resolves_to_a_private_address() {
        let policy = GuardedDestinations::new(false, Duration::from_secs(2)).unwrap();
        let addresses = policy
            .lookup("localhost".to_string(), false)
            .await
            .expect("the proxy host bypasses the guard");
        assert!(addresses.iter().all(|ip| ip.is_loopback()));
    }

    #[tokio::test]
    async fn other_hosts_stay_guarded() {
        let policy = GuardedDestinations::new(false, Duration::from_secs(2)).unwrap();
        let checked = policy.lookup("localhost".to_string(), true).await;
        assert!(matches!(checked, Err(EgressError::PrivateAddress(_))));
    }

    #[tokio::test]
    async fn targets_stay_guarded_behind_a_proxy_on_a_private_host() {
        let client = client_with(false, Some("http://localhost:4750"));

        let literal = Url::parse("https://169.254.169.254/latest").unwrap();
        assert!(matches!(
            client.post_json(&literal, vec![], vec![]).await,
            Err(EgressError::PrivateAddress(_))
        ));

        let resolved = Url::parse("https://localhost/hook").unwrap();
        assert!(matches!(
            client.post_json(&resolved, vec![], vec![]).await,
            Err(EgressError::PrivateAddress(_))
        ));
    }

    #[test]
    fn oidc_endpoints_follow_the_egress_rules() {
        use meteroid_oauth::service::validate_endpoint;

        let egress = guarded_client(false);
        assert!(validate_endpoint(&egress, "issuer", "https://idp.example.com/x").is_ok());
        for url in [
            "http://insecure.example.com/.well-known/openid-configuration",
            "https://169.254.169.254/.well-known/openid-configuration",
            "https://10.0.0.5:9200/.well-known/openid-configuration",
            "https://admin:pw@idp.example.com/.well-known/openid-configuration",
        ] {
            assert!(validate_endpoint(&egress, "issuer", url).is_err(), "{url}");
        }
    }

    #[test]
    fn oidc_sso_rejects_internal_endpoints() {
        let res = meteroid_oauth::service::build_oidc_sso_service(
            &guarded_client(false),
            meteroid_oauth::model::OidcSsoConfig {
                client_id: "cid".to_string(),
                client_secret: "secret".to_string(),
                authorize_url: "https://idp.example.com/authorize".to_string(),
                token_url: "https://idp.example.com/token".to_string(),
                userinfo_url: "http://169.254.169.254/latest/meta-data/".to_string(),
                callback_url: "https://app.example.com/sso/callback".to_string(),
                scopes: vec![],
            },
        );
        assert!(res.is_err());
    }
}
