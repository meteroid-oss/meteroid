use std::net::IpAddr;

use egress_http::EgressError;
use url::{Host, Url};

use crate::ip_rules::is_disallowed_ip;

pub(crate) fn check_destination(url: &Url, allow_private: bool) -> Result<(), EgressError> {
    let host = url
        .host()
        .ok_or_else(|| EgressError::DisallowedHost("missing host".to_string()))?;

    if allow_private {
        return Ok(());
    }

    let port = url
        .port_or_known_default()
        .ok_or_else(|| EgressError::DisallowedHost("missing port".to_string()))?;
    if !matches!(port, 80 | 443) {
        return Err(EgressError::DisallowedPort(port));
    }

    match host {
        Host::Ipv4(v4) => reject_private(IpAddr::V4(v4)),
        Host::Ipv6(v6) => reject_private(IpAddr::V6(v6)),
        Host::Domain("") => Err(EgressError::DisallowedHost("empty host".to_string())),
        Host::Domain(_) => Ok(()),
    }
}

fn reject_private(ip: IpAddr) -> Result<(), EgressError> {
    if is_disallowed_ip(ip) {
        Err(EgressError::PrivateAddress(ip))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use egress_http::{EgressError, Url};

    use crate::tests::guarded_client;

    fn check(raw: &str) -> Result<(), EgressError> {
        guarded_client(false).validate_url(&Url::parse(raw).unwrap())
    }

    fn check_private(raw: &str) -> Result<(), EgressError> {
        guarded_client(true).validate_url(&Url::parse(raw).unwrap())
    }

    #[test]
    fn https_public_hosts_pass() {
        assert_eq!(check("https://example.com/hooks"), Ok(()));
        assert_eq!(check("https://example.com:443/hooks"), Ok(()));
        assert_eq!(check("https://1.1.1.1/hooks"), Ok(()));
        assert_eq!(check("https://[2606:4700:4700::1111]/hooks"), Ok(()));
    }

    #[test]
    fn non_https_schemes_are_rejected() {
        assert_eq!(
            check("http://example.com/hooks"),
            Err(EgressError::DisallowedScheme("http".to_string()))
        );
        assert_eq!(
            check("file:///etc/passwd"),
            Err(EgressError::DisallowedScheme("file".to_string()))
        );
        assert_eq!(
            check("ftp://example.com/x"),
            Err(EgressError::DisallowedScheme("ftp".to_string()))
        );
    }

    #[test]
    fn non_standard_ports_are_rejected() {
        assert_eq!(
            check("https://example.com:8443/hooks"),
            Err(EgressError::DisallowedPort(8443))
        );
        assert_eq!(check("https://example.com:80/hooks"), Ok(()));
    }

    #[test]
    fn credentials_are_rejected() {
        assert_eq!(
            check("https://user@example.com/hooks"),
            Err(EgressError::CredentialsInUrl)
        );
        assert_eq!(
            check("https://user:pass@example.com/hooks"),
            Err(EgressError::CredentialsInUrl)
        );
    }

    #[test]
    fn literal_private_ips_are_rejected() {
        for raw in [
            "https://127.0.0.1/hooks",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/hooks",
            "https://[::ffff:10.0.0.1]/hooks",
            // Decimal / hex literal forms normalise to the same v4 address.
            "https://2130706433/hooks",
            "https://0x7f000001/hooks",
        ] {
            assert!(
                matches!(check(raw), Err(EgressError::PrivateAddress(_))),
                "{raw}"
            );
        }
    }

    #[test]
    fn allow_private_destinations_bypasses_ip_scheme_and_port_rules() {
        assert_eq!(check_private("http://127.0.0.1:8080/hooks"), Ok(()));
        assert_eq!(check_private("http://localhost:3000/hooks"), Ok(()));
        assert_eq!(check_private("https://10.0.0.5:9443/hooks"), Ok(()));
        assert_eq!(check_private("https://[::1]:8080/hooks"), Ok(()));
    }

    #[test]
    fn allow_private_destinations_still_rejects_other_schemes_and_credentials() {
        assert_eq!(
            check_private("file:///etc/passwd"),
            Err(EgressError::DisallowedScheme("file".to_string()))
        );
        assert_eq!(
            check_private("https://user:pass@127.0.0.1/hooks"),
            Err(EgressError::CredentialsInUrl)
        );
    }
}
