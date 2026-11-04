use std::time::Instant;

use chrono::Utc;
use common_domain::ids::EventId;
use egress_http::{EgressClient, EgressError, Url};
use meteroid_store::services::webhooks::out::signing::build_headers;
use secrecy::{ExposeSecret, SecretString};

use super::outcome::AttemptResult;

pub struct DeliveryRequest {
    pub url: String,
    pub message_id: EventId,
    pub payload: serde_json::Value,
    /// Current secret first, then the rotated-out one while its grace window is open.
    pub secrets: Vec<SecretString>,
    /// The endpoint's custom headers. Validated at write time never to collide with the
    /// signing headers, which are added after them.
    pub custom_headers: Vec<(String, SecretString)>,
    /// Also sign under the `svix-*` names: the endpoint was migrated from Svix.
    pub svix_headers: bool,
    /// Record the start of the response body; otherwise only its status.
    pub keep_response_body: bool,
}

pub struct Attempt {
    pub result: AttemptResult,
    pub duration_ms: i32,
}

/// The egress proxy is down: nothing reached the endpoint, so nothing is held against it.
#[derive(Debug)]
pub struct EgressUnavailable(pub String);

impl Attempt {
    fn failed(started: Instant, message: String) -> Self {
        Self {
            result: AttemptResult::Error(message),
            duration_ms: elapsed_ms(started.elapsed()),
        }
    }
}

fn elapsed_ms(duration: std::time::Duration) -> i32 {
    duration.as_millis().min(i32::MAX as u128) as i32
}

/// Signs and POSTs one delivery. Every rejection (bad URL, SSRF rule, timeout) comes back
/// as a failed attempt so it lands in the attempt log instead of killing the worker.
pub async fn deliver(
    egress: &EgressClient,
    request: DeliveryRequest,
) -> Result<Attempt, EgressUnavailable> {
    let started = Instant::now();

    let body = match serde_json::to_vec(&request.payload) {
        Ok(body) => body,
        Err(e) => {
            return Ok(Attempt::failed(
                started,
                format!("payload is not serializable: {e}"),
            ));
        }
    };

    let url = match Url::parse(&request.url) {
        Ok(url) => url,
        Err(e) => {
            return Ok(Attempt::failed(
                started,
                format!("invalid endpoint url: {e}"),
            ));
        }
    };

    let message_id = request.message_id.to_string();
    let timestamp = Utc::now().timestamp();

    let signing_headers = match build_headers(
        &request.secrets,
        &message_id,
        timestamp,
        &body,
        request.svix_headers,
    ) {
        Ok(headers) => headers,
        Err(e) => {
            return Ok(Attempt::failed(
                started,
                format!("could not sign the payload: {e}"),
            ));
        }
    };

    let headers = request
        .custom_headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_string()))
        .chain(
            signing_headers
                .into_iter()
                .map(|(name, value)| (name.to_string(), value)),
        )
        .collect();

    match egress.post_json(&url, headers, body).await {
        Ok(response) => Ok(Attempt {
            duration_ms: elapsed_ms(response.duration),
            result: AttemptResult::Response {
                status: response.status,
                body_prefix: if request.keep_response_body {
                    response.body_prefix
                } else {
                    String::new()
                },
            },
        }),
        Err(EgressError::ProxyUnavailable(reason)) => Err(EgressUnavailable(reason)),
        Err(e) => {
            let message = tenant_facing_error(&e);
            if message != e.to_string() {
                log::warn!("Webhook delivery to {url} was refused by the egress guard: {e}");
            }
            Ok(Attempt::failed(started, message))
        }
    }
}

/// The attempt error is shown to the tenant. Resolution details would turn the attempt
/// log into an internal DNS/address oracle, so guard and proxy rejections collapse into
/// one message and the specifics stay in the server log.
fn tenant_facing_error(error: &EgressError) -> String {
    match error {
        EgressError::PrivateAddress(_)
        | EgressError::DnsFailure { .. }
        | EgressError::ProxyRefused => "destination is not an allowed address".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn guard_rejections_do_not_leak_the_resolved_address() {
        let private = EgressError::PrivateAddress(IpAddr::V4(Ipv4Addr::new(10, 4, 2, 17)));
        let unresolved = EgressError::DnsFailure {
            host: "consul.internal".to_string(),
            message: "no record found".to_string(),
        };

        assert_eq!(
            tenant_facing_error(&private),
            "destination is not an allowed address"
        );
        assert_eq!(
            tenant_facing_error(&unresolved),
            tenant_facing_error(&private),
            "a resolvable internal name must not be distinguishable from an unknown one"
        );
        assert_eq!(
            tenant_facing_error(&EgressError::ProxyRefused),
            tenant_facing_error(&private),
            "the proxy's own check reads the same as ours"
        );
    }

    #[test]
    fn other_errors_keep_their_text() {
        assert_eq!(
            tenant_facing_error(&EgressError::Timeout),
            "request timed out"
        );
    }
}
