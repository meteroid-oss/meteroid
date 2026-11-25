//! `LIKE` / `ILIKE` patterns from what a caller searches for.

/// `search` matched literally: its `\`, `%` and `_` escaped with Postgres' default `LIKE` escape
/// character (`\`), so `50%` finds "50%" only and `a_b` never finds "axb".
pub fn escape_like(search: &str) -> String {
    let mut escaped = String::with_capacity(search.len());
    for c in search.chars() {
        if matches!(c, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// A pattern matching any text containing `search`, literally.
pub fn contains(search: &str) -> String {
    format!("%{}%", escape_like(search))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_are_matched_literally() {
        assert_eq!(contains("acme"), "%acme%");
        assert_eq!(contains("50%_off\\"), "%50\\%\\_off\\\\%");
        assert_eq!(escape_like(""), "");
    }
}
