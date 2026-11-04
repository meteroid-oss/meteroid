use std::collections::BTreeMap;

use crate::domain::webhooks_out::{WebhookEndpointHeader, WebhookHeaderInput};

/// Sensitive header values by header name, as kept encrypted next to the endpoint.
pub type SensitiveHeaderValues = BTreeMap<String, String>;

/// Applies a full header list onto what an endpoint had. A sensitive header given
/// without a value keeps its previous value (matched by name, case-insensitively), so a
/// client can resubmit the list without ever reading a secret back. Returns the stored
/// header list and the sensitive values to encrypt.
pub fn merge_headers(
    input: Vec<WebhookHeaderInput>,
    previous: &SensitiveHeaderValues,
) -> (Vec<WebhookEndpointHeader>, SensitiveHeaderValues) {
    let mut sensitive_values = SensitiveHeaderValues::new();

    let headers = input
        .into_iter()
        .map(|header| {
            if !header.sensitive {
                return WebhookEndpointHeader {
                    set: header.value.is_some(),
                    name: header.name,
                    value: header.value,
                    sensitive: false,
                };
            }

            let value = header.value.or_else(|| {
                previous
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(&header.name))
                    .map(|(_, value)| value.clone())
            });

            let set = value.is_some();
            if let Some(value) = value {
                sensitive_values.insert(header.name.clone(), value);
            }

            WebhookEndpointHeader {
                name: header.name,
                value: None,
                sensitive: true,
                set,
            }
        })
        .collect();

    (headers, sensitive_values)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, value: Option<&str>, sensitive: bool) -> WebhookHeaderInput {
        WebhookHeaderInput {
            name: name.to_string(),
            value: value.map(str::to_string),
            sensitive,
        }
    }

    #[test]
    fn plain_headers_keep_their_value_in_the_list() {
        let (headers, sensitive) = merge_headers(
            vec![input("X-Team", Some("billing"), false)],
            &BTreeMap::new(),
        );

        assert_eq!(headers[0].value.as_deref(), Some("billing"));
        assert!(headers[0].set);
        assert!(sensitive.is_empty());
    }

    #[test]
    fn sensitive_values_never_land_in_the_list() {
        let (headers, sensitive) = merge_headers(
            vec![input("Authorization", Some("Bearer x"), true)],
            &BTreeMap::new(),
        );

        assert_eq!(headers[0].value, None);
        assert!(headers[0].set);
        assert_eq!(sensitive["Authorization"], "Bearer x");
    }

    #[test]
    fn a_sensitive_header_without_value_keeps_the_previous_one() {
        let previous = BTreeMap::from([("authorization".to_string(), "Bearer old".to_string())]);

        let (headers, sensitive) =
            merge_headers(vec![input("Authorization", None, true)], &previous);

        assert!(headers[0].set);
        assert_eq!(sensitive["Authorization"], "Bearer old");
    }

    #[test]
    fn a_sensitive_header_without_any_value_is_unset() {
        let (headers, sensitive) =
            merge_headers(vec![input("api-key", None, true)], &BTreeMap::new());

        assert!(!headers[0].set);
        assert!(sensitive.is_empty());
    }

    #[test]
    fn dropped_headers_lose_their_sensitive_value() {
        let previous = BTreeMap::from([("api-key".to_string(), "k".to_string())]);

        let (headers, sensitive) = merge_headers(vec![], &previous);

        assert!(headers.is_empty());
        assert!(sensitive.is_empty());
    }
}
