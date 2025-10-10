pub fn slugify(s: &str) -> String {
    s.to_lowercase()
        .replace(' ', "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || c == &'-')
        .collect()
}
