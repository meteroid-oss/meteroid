use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::Url;

use crate::error::EgressError;

pub type Lookup = Pin<Box<dyn Future<Output = Result<Vec<IpAddr>, EgressError>> + Send>>;

/// Where a client may send requests, beyond the scheme, credentials and host checks every
/// client makes.
pub trait DestinationPolicy: Send + Sync {
    /// Plain `http` is refused unless private destinations are allowed.
    fn allows_private_destinations(&self) -> bool;

    /// Port and address rules, for a URL that passed the client's own checks.
    fn check_destination(&self, url: &Url) -> Result<(), EgressError>;

    /// The addresses connections to `host` are pinned to. `guarded` is false for the egress
    /// proxy: operator infrastructure, usually on a private address.
    fn lookup(&self, host: String, guarded: bool) -> Lookup;
}

pub(crate) struct PolicyResolver {
    pub(crate) policy: Arc<dyn DestinationPolicy>,
    pub(crate) proxy_host: Option<String>,
}

impl PolicyResolver {
    fn is_proxy_host(&self, host: &str) -> bool {
        self.proxy_host
            .as_deref()
            .is_some_and(|proxy| proxy.eq_ignore_ascii_case(host.trim_end_matches('.')))
    }
}

impl Resolve for PolicyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        let guarded = !self.is_proxy_host(&host);
        let lookup = self.policy.lookup(host, guarded);

        Box::pin(async move {
            let addresses = lookup.await?;
            let addrs: Addrs = Box::new(addresses.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use std::sync::Mutex;

    /// Records whether each lookup was guarded.
    #[derive(Default)]
    struct Recording(Mutex<Vec<(String, bool)>>);

    impl DestinationPolicy for Recording {
        fn allows_private_destinations(&self) -> bool {
            false
        }

        fn check_destination(&self, _url: &Url) -> Result<(), EgressError> {
            Ok(())
        }

        fn lookup(&self, host: String, guarded: bool) -> Lookup {
            self.0.lock().unwrap().push((host, guarded));
            Box::pin(async { Ok(vec![IpAddr::from([127, 0, 0, 1])]) })
        }
    }

    #[tokio::test]
    async fn only_the_proxy_host_resolves_unguarded() {
        let policy = Arc::new(Recording::default());
        let resolver = PolicyResolver {
            policy: policy.clone(),
            proxy_host: Some("LOCALHOST".to_string()),
        };
        for host in ["localhost", "localhost.", "proxy.internal"] {
            let _ = resolver
                .resolve(Name::from_str(host).unwrap())
                .await
                .unwrap();
        }
        assert_eq!(
            *policy.0.lock().unwrap(),
            [
                ("localhost".to_string(), false),
                ("localhost.".to_string(), false),
                ("proxy.internal".to_string(), true),
            ]
        );
    }
}
