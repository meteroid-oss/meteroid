//! Mollie webhooks: unsigned `id=tr_…` form posts. The caller authenticates them (URL token) and
//! re-reads the payment.

/// Returns the payment id from an `id=tr_…` form body.
pub fn parse_classic_ping(payload: &[u8]) -> Option<String> {
    let body = std::str::from_utf8(payload).ok()?.trim();
    body.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == "id"
            && !v.is_empty()
            && v.len() <= 64
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
        .then(|| v.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_ping_parsing() {
        assert_eq!(
            parse_classic_ping(b"id=tr_d0b0E3EA3v"),
            Some("tr_d0b0E3EA3v".into())
        );
        assert_eq!(parse_classic_ping(b"id=tr_x\n"), Some("tr_x".into()));
        assert_eq!(parse_classic_ping(b"foo=bar&id=tr_y"), Some("tr_y".into()));
        assert_eq!(parse_classic_ping(b"id="), None);
        assert_eq!(parse_classic_ping(b"id=tr_<script>"), None);
        assert_eq!(parse_classic_ping(b"{\"id\":\"tr_x\"}"), None);
    }
}
