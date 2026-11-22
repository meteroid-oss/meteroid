/// Canonical form of a login email: what accounts are stored and matched under.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::normalize_email;

    #[test]
    fn trims_and_lowercases() {
        assert_eq!(normalize_email("  Alice@Acme.COM "), "alice@acme.com");
        assert_eq!(normalize_email("bob@acme.com"), "bob@acme.com");
    }
}
