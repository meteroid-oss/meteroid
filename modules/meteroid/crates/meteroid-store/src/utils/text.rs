//! Text normalization applied on the write paths.

/// Collapses blank optional text to `None`, so "unset" has a single representation
/// in the database. Forms submit `""` for a cleared field, and readers (SQL
/// `IS NULL`, the Typst `!= none` guards) only understand `NULL` — without this,
/// clearing a field leaves a value that renders as an empty section.
///
/// Patch semantics are unaffected: the outer `Option` still means "touch this
/// column", only the inner value becomes `NULL`.
pub fn blank_to_none(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::blank_to_none;

    #[test]
    fn blank_variants_become_none() {
        assert_eq!(blank_to_none(None), None);
        assert_eq!(blank_to_none(Some(String::new())), None);
        assert_eq!(blank_to_none(Some("   \n\t".into())), None);
    }

    #[test]
    fn content_is_preserved_verbatim() {
        assert_eq!(blank_to_none(Some("fr-FR".into())), Some("fr-FR".into()));
        // Not a trim helper: surrounding whitespace of real content is left alone.
        assert_eq!(blank_to_none(Some(" a ".into())), Some(" a ".into()));
    }
}
