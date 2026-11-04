//! BCP-47 language tags, in the shape the API exposes them (`["en", "fr-FR"]`).
//!
//! Canonicalization only. Which tags a document can actually be *rendered* in is the
//! renderer's business ([`crate::lang`]), and that is also where a tag we
//! ship no bundle for gets rejected.

use unic_langid::LanguageIdentifier;

/// Canonical form of a language tag: `fr_fr`, `FR-fr` and ` fr-FR ` all give `fr-FR`.
/// `None` when the tag is blank or not well-formed.
pub fn normalize_locale(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed
        .replace('_', "-")
        .parse::<LanguageIdentifier>()
        .ok()
        .map(|tag| tag.to_string())
}

/// Canonicalizes a preference list, preserving order and dropping blanks, malformed
/// tags and duplicates, so `["fr_fr", "", "FR-fr"]` stores as `["fr-FR"]`.
pub fn normalize_locales<I, S>(raw: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out: Vec<String> = Vec::new();
    for tag in raw {
        if let Some(normalized) = normalize_locale(tag.as_ref())
            && !out.contains(&normalized)
        {
            out.push(normalized);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn casing_and_separator_are_canonicalized() {
        assert_eq!(normalize_locale("fr_fr").as_deref(), Some("fr-FR"));
        assert_eq!(normalize_locale(" EN-us ").as_deref(), Some("en-US"));
        assert_eq!(normalize_locale("PT-br").as_deref(), Some("pt-BR"));
        assert_eq!(
            normalize_locale("zh-hant-tw").as_deref(),
            Some("zh-Hant-TW")
        );
        assert_eq!(normalize_locale("fr").as_deref(), Some("fr"));
    }

    #[test]
    fn blank_and_malformed_tags_are_rejected() {
        for raw in ["", "   ", "123", "e", "en-", "fr FR"] {
            assert_eq!(normalize_locale(raw), None, "tag: {raw}");
        }
    }

    #[test]
    fn list_normalization_keeps_order_and_drops_repeats() {
        assert_eq!(
            normalize_locales(["fr_FR", "", "en", "FR-fr", "!!"]),
            vec!["fr-FR".to_string(), "en".to_string()]
        );
    }
}
