use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use egress_http::EgressError;
use hickory_resolver::TokioResolver;
use hickory_resolver::config::ResolverOpts;

use crate::ip_rules::is_disallowed_ip;

pub(crate) fn resolver(timeout: Duration) -> Result<Arc<TokioResolver>, EgressError> {
    let mut options = ResolverOpts::default();
    options.timeout = timeout;
    options.attempts = 2;
    options.try_tcp_on_error = true;

    TokioResolver::builder_tokio()
        .and_then(|builder| builder.with_options(options).build())
        .map(Arc::new)
        .map_err(|err| EgressError::ClientBuild(err.to_string()))
}

/// Refuses the whole request as soon as one answer points at a disallowed address. Connections
/// are pinned to the addresses returned here, which closes the DNS rebinding window.
pub(crate) async fn resolve_checked(
    resolver: &TokioResolver,
    allow_private_destinations: bool,
    host: &str,
) -> Result<Vec<IpAddr>, EgressError> {
    let lookup = resolver
        .lookup_ip(host)
        .await
        .map_err(|err| EgressError::DnsFailure {
            host: host.to_string(),
            message: err.to_string(),
        })?;

    let mut addresses = Vec::new();
    for ip in lookup.iter() {
        if !allow_private_destinations && is_disallowed_ip(ip) {
            return Err(EgressError::PrivateAddress(ip));
        }
        addresses.push(ip);
    }

    if addresses.is_empty() {
        return Err(EgressError::DnsFailure {
            host: host.to_string(),
            message: "no address records".to_string(),
        });
    }
    Ok(addresses)
}
