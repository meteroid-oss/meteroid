//! What importing one Svix application does, decided without any I/O.

use std::collections::{HashMap, HashSet};

use meteroid_store::domain::webhooks_out::WebhookHeaderInput;

use super::client::{SvixEndpoint, SvixEndpointHeaders};
use crate::services::webhooks_out::{MAX_RATE_LIMIT_PER_SEC, check_endpoint_input, check_headers};

/// Why an endpoint is imported disabled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisabledBecause {
    DisabledInSvix,
    /// Svix never reveals these values: they have to be entered in Meteroid.
    SensitiveHeaders(Vec<String>),
    /// The delivery worker would refuse the url (egress settings).
    UrlRefused(String),
    /// Meteroid never set channels on its messages, so Svix sent this endpoint nothing.
    Channels(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewEndpoint {
    pub description: Option<String>,
    /// Empty: every event type.
    pub event_types: Vec<String>,
    /// Sensitive headers come without a value.
    pub headers: Vec<WebhookHeaderInput>,
    pub rate_limit_per_sec: Option<i32>,
    pub disabled_because: Vec<DisabledBecause>,
    /// What was not carried over as is.
    pub notes: Vec<String>,
}

impl NewEndpoint {
    pub fn disabled(&self) -> bool {
        !self.disabled_because.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EndpointPlan {
    Create(NewEndpoint),
    /// The tenant already has a non-archived endpoint with this url.
    AlreadyPresent,
    Skip(String),
}

#[derive(Clone, Debug)]
pub struct PlannedEndpoint {
    pub svix_endpoint_id: String,
    pub url: String,
    pub plan: EndpointPlan,
}

/// Plans every endpoint of one Svix application, in Svix's order. Several Svix endpoints
/// with one url cannot all be imported (the url is what makes a re-run idempotent): the
/// first enabled one is, else the first one.
pub fn plan_application(
    endpoints: &[(SvixEndpoint, SvixEndpointHeaders)],
    existing_urls: &HashSet<String>,
    is_known_event_type: &dyn Fn(&str) -> bool,
    check_url: &dyn Fn(&str) -> Result<(), String>,
) -> Vec<PlannedEndpoint> {
    let mut chosen: HashMap<&str, &SvixEndpoint> = HashMap::new();
    for (endpoint, _) in endpoints {
        chosen
            .entry(endpoint.url.as_str())
            .and_modify(|current| {
                if is_disabled(current) && !is_disabled(endpoint) {
                    *current = endpoint;
                }
            })
            .or_insert(endpoint);
    }

    endpoints
        .iter()
        .map(|(endpoint, headers)| {
            let plan = if existing_urls.contains(&endpoint.url) {
                EndpointPlan::AlreadyPresent
            } else {
                match chosen.get(endpoint.url.as_str()) {
                    Some(winner) if winner.id != endpoint.id => EndpointPlan::Skip(format!(
                        "Svix endpoint {} has the same url and is imported instead",
                        winner.id
                    )),
                    _ => plan_endpoint(endpoint, headers, is_known_event_type, check_url),
                }
            };

            PlannedEndpoint {
                svix_endpoint_id: endpoint.id.clone(),
                url: endpoint.url.clone(),
                plan,
            }
        })
        .collect()
}

fn is_disabled(endpoint: &SvixEndpoint) -> bool {
    endpoint.disabled.unwrap_or(false)
}

pub fn plan_endpoint(
    endpoint: &SvixEndpoint,
    headers: &SvixEndpointHeaders,
    is_known_event_type: &dyn Fn(&str) -> bool,
    check_url: &dyn Fn(&str) -> Result<(), String>,
) -> EndpointPlan {
    let mut notes = Vec::new();
    let mut disabled_because = Vec::new();

    let (event_types, unknown) =
        split_event_types(endpoint.filter_types.as_deref(), is_known_event_type);
    if !unknown.is_empty() {
        if event_types.is_empty() {
            // Importing an empty filter would subscribe the endpoint to every event type.
            return EndpointPlan::Skip(format!(
                "subscribed only to event types Meteroid does not send: {}",
                unknown.join(", ")
            ));
        }
        notes.push(format!(
            "event types Meteroid does not send were dropped: {}",
            unknown.join(", ")
        ));
    }

    let description = endpoint
        .description
        .clone()
        .filter(|d| !d.trim().is_empty());

    if let Err(reason) = check_endpoint_input(
        Some(&endpoint.url),
        description.as_deref(),
        Some(&event_types),
    ) {
        return EndpointPlan::Skip(reason);
    }

    let (headers, sensitive) = split_headers(headers, &mut notes);
    if let Err(reason) = check_headers(&headers, false) {
        return EndpointPlan::Skip(format!("its headers cannot be imported: {reason}"));
    }

    if is_disabled(endpoint) {
        disabled_because.push(DisabledBecause::DisabledInSvix);
    }
    if !sensitive.is_empty() {
        disabled_because.push(DisabledBecause::SensitiveHeaders(sensitive));
    }
    if let Err(detail) = check_url(&endpoint.url) {
        disabled_because.push(DisabledBecause::UrlRefused(detail));
    }
    if let Some(channels) = endpoint.channels.as_ref().filter(|c| !c.is_empty()) {
        disabled_because.push(DisabledBecause::Channels(channels.clone()));
    }

    let rate_limit_per_sec = rate_limit(endpoint, &mut notes);

    EndpointPlan::Create(NewEndpoint {
        description,
        event_types,
        headers,
        rate_limit_per_sec,
        disabled_because,
        notes,
    })
}

/// The known event types, deduplicated in Svix's order, and the unknown ones.
fn split_event_types(
    filter_types: Option<&[String]>,
    is_known_event_type: &dyn Fn(&str) -> bool,
) -> (Vec<String>, Vec<String>) {
    let mut seen = HashSet::new();
    filter_types
        .unwrap_or_default()
        .iter()
        .filter(|t| seen.insert(t.as_str()))
        .cloned()
        .partition(|t| is_known_event_type(t.as_str()))
}

/// Plain headers keep their value; a sensitive one comes without. A header the native
/// API refuses (reserved name, invalid value) is dropped with a note.
fn split_headers(
    svix: &SvixEndpointHeaders,
    notes: &mut Vec<String>,
) -> (Vec<WebhookHeaderInput>, Vec<String>) {
    let sensitive_names = svix.sensitive.clone().unwrap_or_default();
    let is_sensitive = |name: &str| sensitive_names.iter().any(|s| s.eq_ignore_ascii_case(name));

    let plain = svix
        .headers
        .iter()
        .flatten()
        .filter(|(name, _)| !is_sensitive(name.as_str()))
        .map(|(name, value)| WebhookHeaderInput {
            name: name.clone(),
            value: Some(value.clone()),
            sensitive: false,
        });
    let sensitive = sensitive_names.iter().map(|name| WebhookHeaderInput {
        name: name.clone(),
        value: None,
        sensitive: true,
    });

    let mut headers = Vec::new();
    let mut dropped = Vec::new();
    for header in plain.chain(sensitive) {
        match check_headers(std::slice::from_ref(&header), false) {
            Ok(()) => headers.push(header),
            Err(reason) => dropped.push(reason),
        }
    }
    if !dropped.is_empty() {
        notes.push(format!("headers dropped: {}", dropped.join("; ")));
    }

    let sensitive = headers
        .iter()
        .filter(|h| h.sensitive)
        .map(|h| h.name.clone())
        .collect();
    (headers, sensitive)
}

fn rate_limit(endpoint: &SvixEndpoint, notes: &mut Vec<String>) -> Option<i32> {
    let rate = endpoint.throttle_rate.or(endpoint.rate_limit)?;
    if rate == 0 {
        return None;
    }
    let max = MAX_RATE_LIMIT_PER_SEC as u32;
    if rate > max {
        notes.push(format!(
            "rate limit lowered from {rate} to {max} per second, the most Meteroid allows"
        ));
    }
    Some(rate.min(max) as i32)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn endpoint(id: &str, url: &str) -> SvixEndpoint {
        SvixEndpoint {
            id: id.to_string(),
            url: url.to_string(),
            ..Default::default()
        }
    }

    fn headers(plain: &[(&str, &str)], sensitive: &[&str]) -> SvixEndpointHeaders {
        SvixEndpointHeaders {
            headers: Some(
                plain
                    .iter()
                    .map(|(n, v)| (n.to_string(), v.to_string()))
                    .collect::<BTreeMap<_, _>>(),
            ),
            sensitive: Some(sensitive.iter().map(|s| s.to_string()).collect()),
        }
    }

    fn known(event_type: &str) -> bool {
        matches!(event_type, "invoice.paid" | "customer.created")
    }

    fn accept(_: &str) -> Result<(), String> {
        Ok(())
    }

    fn create(plan: EndpointPlan) -> NewEndpoint {
        match plan {
            EndpointPlan::Create(new) => new,
            other => panic!("expected a creation, got {other:?}"),
        }
    }

    #[test]
    fn a_plain_endpoint_is_imported_enabled_as_is() {
        let mut svix = endpoint("ep_1", "https://a.example/hook");
        svix.description = Some("Billing".to_string());
        svix.filter_types = Some(vec!["invoice.paid".to_string()]);

        let new = create(plan_endpoint(
            &svix,
            &headers(&[("X-Team", "billing")], &[]),
            &known,
            &accept,
        ));

        assert!(!new.disabled());
        assert_eq!(new.description.as_deref(), Some("Billing"));
        assert_eq!(new.event_types, vec!["invoice.paid".to_string()]);
        assert_eq!(
            new.headers,
            vec![WebhookHeaderInput {
                name: "X-Team".to_string(),
                value: Some("billing".to_string()),
                sensitive: false,
            }]
        );
        assert!(new.notes.is_empty());
    }

    #[test]
    fn no_filter_means_every_event_type() {
        let new = create(plan_endpoint(
            &endpoint("ep_1", "https://a.example"),
            &SvixEndpointHeaders::default(),
            &known,
            &accept,
        ));

        assert!(new.event_types.is_empty());
        assert_eq!(new.description, None);
    }

    #[test]
    fn unknown_event_types_are_dropped_and_reported() {
        let mut svix = endpoint("ep_1", "https://a.example");
        svix.filter_types = Some(vec![
            "invoice.paid".to_string(),
            "legacy.thing".to_string(),
            "invoice.paid".to_string(),
        ]);

        let new = create(plan_endpoint(&svix, &headers(&[], &[]), &known, &accept));

        assert_eq!(new.event_types, vec!["invoice.paid".to_string()]);
        assert_eq!(new.notes.len(), 1);
        assert!(new.notes[0].contains("legacy.thing"));
    }

    #[test]
    fn an_endpoint_filtered_on_unknown_types_only_is_skipped() {
        let mut svix = endpoint("ep_1", "https://a.example");
        svix.filter_types = Some(vec!["legacy.thing".to_string()]);

        let plan = plan_endpoint(&svix, &headers(&[], &[]), &known, &accept);

        assert!(matches!(plan, EndpointPlan::Skip(reason) if reason.contains("legacy.thing")));
    }

    #[test]
    fn sensitive_headers_are_imported_unset_and_disable_the_endpoint() {
        let new = create(plan_endpoint(
            &endpoint("ep_1", "https://a.example"),
            &headers(&[("X-Team", "billing")], &["Authorization"]),
            &known,
            &accept,
        ));

        assert_eq!(
            new.disabled_because,
            vec![DisabledBecause::SensitiveHeaders(vec![
                "Authorization".to_string()
            ])]
        );
        assert!(new.headers.contains(&WebhookHeaderInput {
            name: "Authorization".to_string(),
            value: None,
            sensitive: true,
        }));
    }

    #[test]
    fn a_header_listed_as_sensitive_never_keeps_a_value() {
        let new = create(plan_endpoint(
            &endpoint("ep_1", "https://a.example"),
            &headers(&[("authorization", "Bearer x")], &["Authorization"]),
            &known,
            &accept,
        ));

        assert_eq!(new.headers.len(), 1);
        assert!(new.headers[0].sensitive);
        assert_eq!(new.headers[0].value, None);
    }

    #[test]
    fn reserved_headers_are_dropped_and_reported() {
        let new = create(plan_endpoint(
            &endpoint("ep_1", "https://a.example"),
            &headers(&[("User-Agent", "custom"), ("X-Team", "billing")], &[]),
            &known,
            &accept,
        ));

        assert_eq!(new.headers.len(), 1);
        assert_eq!(new.headers[0].name, "X-Team");
        assert!(new.notes[0].contains("User-Agent"));
        assert!(!new.disabled());
    }

    #[test]
    fn a_disabled_endpoint_stays_disabled() {
        let mut svix = endpoint("ep_1", "https://a.example");
        svix.disabled = Some(true);

        let new = create(plan_endpoint(&svix, &headers(&[], &[]), &known, &accept));

        assert_eq!(new.disabled_because, vec![DisabledBecause::DisabledInSvix]);
    }

    #[test]
    fn a_refused_url_disables_the_endpoint() {
        let new = create(plan_endpoint(
            &endpoint("ep_1", "http://10.0.0.1/hook"),
            &headers(&[], &[]),
            &known,
            &|_| Err("private address".to_string()),
        ));

        assert_eq!(
            new.disabled_because,
            vec![DisabledBecause::UrlRefused("private address".to_string())]
        );
    }

    #[test]
    fn an_endpoint_filtered_on_channels_is_imported_disabled() {
        let mut svix = endpoint("ep_1", "https://a.example");
        svix.channels = Some(vec!["eu".to_string()]);

        let new = create(plan_endpoint(&svix, &headers(&[], &[]), &known, &accept));

        assert_eq!(
            new.disabled_because,
            vec![DisabledBecause::Channels(vec!["eu".to_string()])]
        );
    }

    #[test]
    fn the_rate_limit_is_carried_over_within_bounds() {
        let mut svix = endpoint("ep_1", "https://a.example");
        svix.rate_limit = Some(20);
        assert_eq!(
            create(plan_endpoint(&svix, &headers(&[], &[]), &known, &accept)).rate_limit_per_sec,
            Some(20)
        );

        svix.throttle_rate = Some(5_000);
        let new = create(plan_endpoint(&svix, &headers(&[], &[]), &known, &accept));
        assert_eq!(new.rate_limit_per_sec, Some(MAX_RATE_LIMIT_PER_SEC));
        assert_eq!(new.notes.len(), 1);

        svix.throttle_rate = Some(0);
        svix.rate_limit = None;
        assert_eq!(
            create(plan_endpoint(&svix, &headers(&[], &[]), &known, &accept)).rate_limit_per_sec,
            None
        );
    }

    #[test]
    fn an_endpoint_whose_url_exists_is_already_present() {
        let planned = plan_application(
            &[(endpoint("ep_1", "https://a.example"), headers(&[], &[]))],
            &HashSet::from(["https://a.example".to_string()]),
            &known,
            &accept,
        );

        assert_eq!(planned[0].plan, EndpointPlan::AlreadyPresent);
    }

    #[test]
    fn of_two_svix_endpoints_with_one_url_the_enabled_one_is_imported() {
        let mut disabled = endpoint("ep_1", "https://a.example");
        disabled.disabled = Some(true);
        let enabled = endpoint("ep_2", "https://a.example");
        let other = endpoint("ep_3", "https://b.example");

        let planned = plan_application(
            &[
                (disabled, headers(&[], &[])),
                (enabled, headers(&[], &[])),
                (other, headers(&[], &[])),
            ],
            &HashSet::new(),
            &known,
            &accept,
        );

        assert!(matches!(&planned[0].plan, EndpointPlan::Skip(reason) if reason.contains("ep_2")));
        assert!(matches!(&planned[1].plan, EndpointPlan::Create(new) if !new.disabled()));
        assert!(matches!(&planned[2].plan, EndpointPlan::Create(_)));
    }
}
