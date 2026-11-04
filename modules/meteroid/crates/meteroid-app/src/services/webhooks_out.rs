use std::collections::HashSet;

use common_domain::ids::TenantId;
use meteroid_store::domain::enums::TenantEnvironmentEnum;
use meteroid_store::domain::webhooks_out::WebhookHeaderInput;
use meteroid_store::repositories::tenants::TenantInterface;
use meteroid_store::{Store, StoreResult};

/// Per-endpoint in-flight cap of a new production endpoint. It bounds how much of the
/// delivery budget one endpoint can hold; higher means faster bursts, more reordering.
pub const MAX_IN_FLIGHT_PRODUCTION: i32 = 4;
pub const MAX_IN_FLIGHT_SANDBOX: i32 = 1;

/// The in-flight cap a new endpoint of this tenant starts on. Non-production tenants
/// stay serialised: their bursts are cheap to delay and their endpoints are the ones
/// most likely to be dead.
pub fn max_in_flight_for(environment: TenantEnvironmentEnum) -> i32 {
    match environment {
        TenantEnvironmentEnum::Production => MAX_IN_FLIGHT_PRODUCTION,
        _ => MAX_IN_FLIGHT_SANDBOX,
    }
}

/// In-flight cap for an endpoint about to be created for this tenant. Resolved once at
/// creation: later moves between environments do not exist, and updates leave it alone.
pub async fn tenant_max_in_flight(store: &Store, tenant_id: TenantId) -> StoreResult<i32> {
    let tenant = store.find_tenant_by_id(tenant_id).await?;

    Ok(max_in_flight_for(tenant.tenant.environment))
}

pub const MAX_URL_LEN: usize = 2048;
pub const MAX_DESCRIPTION_LEN: usize = 512;
pub const MAX_EVENT_TYPES: usize = 100;

/// Size caps shared by the gRPC and REST create/update paths. Event type names are
/// checked separately against the known set.
pub fn check_endpoint_input(
    url: Option<&str>,
    description: Option<&str>,
    event_types: Option<&[String]>,
) -> Result<(), String> {
    if let Some(url) = url
        && url.len() > MAX_URL_LEN
    {
        return Err(format!("url must be at most {MAX_URL_LEN} characters"));
    }
    if let Some(description) = description
        && description.chars().count() > MAX_DESCRIPTION_LEN
    {
        return Err(format!(
            "description must be at most {MAX_DESCRIPTION_LEN} characters"
        ));
    }
    if let Some(event_types) = event_types
        && event_types.len() > MAX_EVENT_TYPES
    {
        return Err(format!(
            "at most {MAX_EVENT_TYPES} event types can be subscribed to"
        ));
    }
    Ok(())
}

pub const MAX_RATE_LIMIT_PER_SEC: i32 = 1_000;

pub fn check_rate_limit(rate_limit_per_sec: Option<i32>) -> Result<(), String> {
    match rate_limit_per_sec {
        Some(rate) if !(1..=MAX_RATE_LIMIT_PER_SEC).contains(&rate) => Err(format!(
            "the rate limit must be between 1 and {MAX_RATE_LIMIT_PER_SEC} per second"
        )),
        _ => Ok(()),
    }
}

pub const MAX_HEADERS: usize = 20;
pub const MAX_HEADER_NAME_LEN: usize = 128;
pub const MAX_HEADER_VALUE_LEN: usize = 4096;

/// Headers the delivery itself sets, or that would change how the request travels.
const RESERVED_HEADERS: &[&str] = &[
    "connection",
    "content-length",
    "content-type",
    "expect",
    "host",
    "keep-alive",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "user-agent",
];
const RESERVED_HEADER_PREFIXES: &[&str] = &["webhook-", "svix-", "proxy-"];

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c)
}

/// Custom header checks shared by the gRPC and REST create/update paths. On create a
/// sensitive header needs its value too; on update an absent one keeps the old value.
pub fn check_headers(headers: &[WebhookHeaderInput], creating: bool) -> Result<(), String> {
    if headers.len() > MAX_HEADERS {
        return Err(format!("at most {MAX_HEADERS} custom headers are allowed"));
    }

    let mut seen = HashSet::new();
    for header in headers {
        let name = header.name.trim();
        if name.is_empty() || name.len() > MAX_HEADER_NAME_LEN || !name.chars().all(is_token_char) {
            return Err(format!("`{}` is not a valid header name", header.name));
        }

        let lower = name.to_ascii_lowercase();
        if RESERVED_HEADERS.contains(&lower.as_str())
            || RESERVED_HEADER_PREFIXES
                .iter()
                .any(|p| lower.starts_with(p))
        {
            return Err(format!(
                "the `{name}` header is set by Meteroid and cannot be overridden"
            ));
        }
        if !seen.insert(lower) {
            return Err(format!("the `{name}` header is listed twice"));
        }

        match &header.value {
            Some(value) => {
                if value.len() > MAX_HEADER_VALUE_LEN {
                    return Err(format!(
                        "the value of `{name}` must be at most {MAX_HEADER_VALUE_LEN} bytes"
                    ));
                }
                if value.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
                    return Err(format!("the value of `{name}` contains a line break"));
                }
            }
            None if !header.sensitive || creating => {
                return Err(format!("the `{name}` header needs a value"));
            }
            None => {}
        }
    }

    Ok(())
}

/// Absent leaves the description alone; an empty (or blank) one clears it.
pub fn description_patch(description: Option<String>) -> Option<Option<String>> {
    description.map(|d| Some(d).filter(|d| !d.trim().is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_production_endpoints_get_the_production_cap() {
        assert_eq!(
            max_in_flight_for(TenantEnvironmentEnum::Production),
            MAX_IN_FLIGHT_PRODUCTION
        );
        for environment in [
            TenantEnvironmentEnum::Staging,
            TenantEnvironmentEnum::Qa,
            TenantEnvironmentEnum::Development,
            TenantEnvironmentEnum::Sandbox,
            TenantEnvironmentEnum::Demo,
        ] {
            assert_eq!(max_in_flight_for(environment), MAX_IN_FLIGHT_SANDBOX);
        }
    }

    #[test]
    fn endpoint_inputs_are_capped() {
        let long_url = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        assert!(check_endpoint_input(Some(&long_url), None, None).is_err());
        assert!(check_endpoint_input(Some("https://example.com/hook"), None, None).is_ok());

        let long_description = "d".repeat(MAX_DESCRIPTION_LEN + 1);
        assert!(check_endpoint_input(None, Some(&long_description), None).is_err());
        assert!(check_endpoint_input(None, Some(&"d".repeat(MAX_DESCRIPTION_LEN)), None).is_ok());

        let many: Vec<String> = (0..=MAX_EVENT_TYPES).map(|i| format!("t{i}")).collect();
        assert!(check_endpoint_input(None, None, Some(&many)).is_err());
        assert!(check_endpoint_input(None, None, Some(&many[..MAX_EVENT_TYPES])).is_ok());
    }

    fn header(name: &str, value: Option<&str>, sensitive: bool) -> WebhookHeaderInput {
        WebhookHeaderInput {
            name: name.to_string(),
            value: value.map(str::to_string),
            sensitive,
        }
    }

    #[test]
    fn custom_headers_are_validated() {
        assert!(check_headers(&[header("Authorization", Some("Bearer x"), true)], true).is_ok());
        assert!(check_headers(&[header("X-Team", Some("billing"), false)], true).is_ok());

        assert!(check_headers(&[header("bad name", Some("x"), false)], true).is_err());
        assert!(check_headers(&[header("Content-Type", Some("x"), false)], true).is_err());
        assert!(check_headers(&[header("webhook-signature", Some("x"), false)], true).is_err());
        assert!(check_headers(&[header("Svix-Id", Some("x"), false)], true).is_err());
        assert!(check_headers(&[header("X-A", Some("a\r\nb"), false)], true).is_err());
        assert!(
            check_headers(
                &[
                    header("X-A", Some("1"), false),
                    header("x-a", Some("2"), false)
                ],
                true
            )
            .is_err()
        );
    }

    #[test]
    fn a_sensitive_value_may_be_omitted_on_update_only() {
        let keep = [header("api-key", None, true)];
        assert!(check_headers(&keep, false).is_ok());
        assert!(check_headers(&keep, true).is_err());
        assert!(check_headers(&[header("X-Team", None, false)], false).is_err());
    }

    #[test]
    fn an_empty_description_clears_it() {
        assert_eq!(description_patch(None), None);
        assert_eq!(description_patch(Some(String::new())), Some(None));
        assert_eq!(description_patch(Some("  ".to_string())), Some(None));
        assert_eq!(
            description_patch(Some("prod".to_string())),
            Some(Some("prod".to_string()))
        );
    }
}
