use url::Url;

use crate::error::EgressError;

pub(crate) fn validate_url(url: &Url, allow_http: bool) -> Result<(), EgressError> {
    match url.scheme() {
        "https" => {}
        "http" if allow_http => {}
        other => return Err(EgressError::DisallowedScheme(other.to_string())),
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err(EgressError::CredentialsInUrl);
    }

    url.host()
        .ok_or_else(|| EgressError::DisallowedHost("missing host".to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(raw: &str, allow_http: bool) -> Result<(), EgressError> {
        validate_url(&Url::parse(raw).unwrap(), allow_http)
    }

    #[test]
    fn http_and_https_pass() {
        assert_eq!(check("https://example.com/hooks", false), Ok(()));
        assert_eq!(check("http://127.0.0.1:8080/hooks", true), Ok(()));
        assert_eq!(check("https://[::1]:8080/hooks", true), Ok(()));
    }

    #[test]
    fn plain_http_needs_allowing() {
        assert_eq!(
            check("http://example.com/hooks", false),
            Err(EgressError::DisallowedScheme("http".to_string()))
        );
    }

    #[test]
    fn other_schemes_are_rejected() {
        for allow_http in [false, true] {
            assert_eq!(
                check("file:///etc/passwd", allow_http),
                Err(EgressError::DisallowedScheme("file".to_string()))
            );
            assert_eq!(
                check("ftp://example.com/x", allow_http),
                Err(EgressError::DisallowedScheme("ftp".to_string()))
            );
        }
    }

    #[test]
    fn credentials_are_rejected() {
        for raw in [
            "https://user@example.com/hooks",
            "https://user:pass@example.com/hooks",
            "https://:pass@example.com/hooks",
            "https://user:pass@127.0.0.1/hooks",
        ] {
            assert_eq!(
                check(raw, true),
                Err(EgressError::CredentialsInUrl),
                "{raw}"
            );
        }
    }
}
