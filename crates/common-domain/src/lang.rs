//! Document language support.
//!
//! The renderer only has message bundles for the languages listed here; anything
//! else still renders, in `en-US`. That fallback is deliberate — a document must
//! never fail to render over a language tag — but it is silent, so callers that
//! resolved an *explicit* operator choice should log it via
//! [`unsupported_override`].
//!
//! A bundle covers a language, not a country: regional tags collapse onto it
//! (`de-AT`, `de-CH` and `de-BE` all read `de-DE`). Country-specific *statutory*
//! wording is not a language concern — `meteroid_invoicing::legal` selects it from the seller's
//! jurisdiction, so a Swiss seller invoicing in German gets Swiss citations out of
//! the `de-DE` bundle.
//!
//! Portuguese is the one language shipped as two bundles. European and Brazilian
//! Portuguese diverge on the vocabulary a document is made of — *montante*/*valor*,
//! *taxa*/*alíquota*, *registo*/*registro*, *eletrónica*/*eletrônica* — so a Brazilian
//! seller handed the `pt-PT` bundle would issue a document that reads as foreign.
//! `pt-BR` keeps its own bundle; every other Portuguese tag reads `pt-PT`.

/// Template locales we ship message bundles for.
pub const SUPPORTED: [&str; 8] = [
    "en-US", "fr-FR", "de-DE", "es-ES", "it-IT", "nl-NL", "pt-PT", "pt-BR",
];

/// The language and region subtags of a BCP-47-ish tag, lowercased: `de-CH` ->
/// (`de`, Some(`ch`)). Tolerates the underscore form and stray casing, both of which
/// reach us from customer data.
fn subtags(requested: &str) -> (String, Option<String>) {
    let mut parts = requested.trim().split(['-', '_']);
    let primary = parts.next().unwrap_or_default().to_ascii_lowercase();
    let region = parts
        .next()
        .filter(|r| !r.is_empty())
        .map(str::to_ascii_lowercase);
    (primary, region)
}

/// Maps a requested language onto the template locale used for translations and
/// date formatting. Accepts the bare subtag (`fr`) as well as regional tags
/// (`fr-BE`). Anything unknown falls back to `en-US`.
pub fn template_lang(requested: &str) -> &'static str {
    let (primary, region) = subtags(requested);
    match (primary.as_str(), region.as_deref()) {
        ("fr", _) => "fr-FR",
        ("de", _) => "de-DE",
        ("es", _) => "es-ES",
        ("it", _) => "it-IT",
        ("nl", _) => "nl-NL",
        // Brazil has its own bundle; every other Portuguese-speaking region (Portugal,
        // Angola, Mozambique, Cabo Verde, …) writes European Portuguese.
        ("pt", Some("br")) => "pt-BR",
        ("pt", _) => "pt-PT",
        _ => "en-US",
    }
}

/// Whether a message bundle exists for `requested`.
pub fn is_supported(requested: &str) -> bool {
    matches!(
        subtags(requested).0.as_str(),
        "en" | "fr" | "de" | "es" | "it" | "nl" | "pt"
    )
}

/// Reject an operator-supplied language tag we ship no bundle for.
///
/// Rendering deliberately never fails over a language tag — see the module docs — but a
/// *write* is the moment to say so, while someone is there to read it. Without this the
/// value is stored, and the only signal is a log line at render time, long after the
/// caller has gone. Blank and absent are "unset", not invalid.
///
/// Returns the offending tag owned, so callers can name it in their own error type
/// without keeping a borrow of the input alive across an await.
pub fn reject_unsupported(requested: Option<&str>) -> Result<(), String> {
    match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some(tag) if !is_supported(tag) => Err(tag.to_string()),
        _ => Ok(()),
    }
}

/// The first entry of a customer's preference list we ship a bundle for.
///
/// A preference list is ordered and advisory: a customer asking for `["pl", "nl-BE"]`
/// gets a Dutch document, not an English one, because falling through to the next
/// choice is the point of the list. `None` means nothing in it can be served, and the
/// level below — the invoicing entity, then its country — decides.
pub fn first_supported(preferences: &[String]) -> Option<&str> {
    preferences
        .iter()
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty())
        .find(|tag| is_supported(tag))
}

/// Reject a preference list we can serve no entry of, for the reasons in
/// [`reject_unsupported`]. A list with one supported entry renders as asked, so the
/// unsupported ones alongside it are just preferences we could not honour, not errors.
/// An empty list is "unset".
pub fn reject_unsupported_preferences(preferences: &[String]) -> Result<(), String> {
    match unservable_choice(preferences) {
        Some(tag) => Err(tag.to_string()),
        None => Ok(()),
    }
}

/// The customer's first stated preference when we can serve none of them — the tag to
/// name when explaining that their choice did not take. `None` when they stated nothing,
/// or when something in the list is servable.
fn unservable_choice(preferences: &[String]) -> Option<&str> {
    preferences
        .iter()
        .map(|tag| tag.trim())
        .find(|tag| !tag.is_empty())
        .filter(|_| first_supported(preferences).is_none())
}

/// The explicitly configured language that will silently render in something else, if
/// any.
///
/// Only an operator's own choice (the customer's preference list, then the invoicing
/// entity default) is worth flagging — a locale derived from the entity's country is
/// not a misconfiguration, and warning on it would fire for every seller we ship no
/// bundle for. One servable entry in the list is enough for the document to be worded
/// as asked, so only a list we can serve nothing of is flagged.
pub fn unsupported_override<'a>(
    customer_preferences: &'a [String],
    entity_language: Option<&'a str>,
) -> Option<&'a str> {
    let customer_stated_something = customer_preferences
        .iter()
        .any(|tag| !tag.trim().is_empty());
    if customer_stated_something {
        return unservable_choice(customer_preferences);
    }
    entity_language
        .map(str::trim)
        .filter(|tag| !tag.is_empty() && !is_supported(tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_tags_map_to_their_bundle() {
        assert_eq!(template_lang("fr"), "fr-FR");
        assert_eq!(template_lang("fr-FR"), "fr-FR");
        assert_eq!(template_lang(" fr-FR "), "fr-FR");
        assert_eq!(template_lang("en-US"), "en-US");
        assert_eq!(template_lang("de-DE"), "de-DE");
        assert_eq!(template_lang("es-ES"), "es-ES");
        assert_eq!(template_lang("it-IT"), "it-IT");
        assert_eq!(template_lang("nl-NL"), "nl-NL");
        assert_eq!(template_lang("pt-PT"), "pt-PT");
        assert_eq!(template_lang("pt-BR"), "pt-BR");
    }

    /// Brazil is the one region with a bundle of its own; the rest of the Portuguese-
    /// speaking world, and the bare tag, read European Portuguese.
    #[test]
    fn portuguese_splits_on_region() {
        assert_eq!(template_lang("pt"), "pt-PT");
        for tag in [
            "pt-AO", "pt-MZ", "pt-CV", "pt-GW", "pt-ST", "pt-TL", "pt-MO",
        ] {
            assert_eq!(template_lang(tag), "pt-PT", "tag: {tag}");
        }
        for tag in ["pt-BR", "pt_BR", "PT-br", " pt-BR "] {
            assert_eq!(template_lang(tag), "pt-BR", "tag: {tag}");
            assert!(is_supported(tag), "tag: {tag}");
        }
    }

    /// Belgium and Switzerland issue in the neighbouring language, so their tags must
    /// land on that language's bundle rather than silently on English.
    #[test]
    fn regional_tags_collapse_onto_their_language() {
        for (tag, bundle) in [
            ("nl-BE", "nl-NL"),
            ("fr-BE", "fr-FR"),
            ("de-BE", "de-DE"),
            ("de-CH", "de-DE"),
            ("fr-CH", "fr-FR"),
            ("it-CH", "it-IT"),
            ("de-AT", "de-DE"),
            ("fr-LU", "fr-FR"),
            ("es-MX", "es-ES"),
            ("en-GB", "en-US"),
        ] {
            assert_eq!(template_lang(tag), bundle, "tag: {tag}");
            assert!(is_supported(tag), "tag: {tag}");
        }
    }

    /// Customer-supplied tags arrive in whatever shape the API caller typed.
    #[test]
    fn casing_and_underscore_forms_are_tolerated() {
        assert_eq!(template_lang("FR"), "fr-FR");
        assert_eq!(template_lang("fr_FR"), "fr-FR");
        assert_eq!(template_lang("DE_de"), "de-DE");
    }

    #[test]
    fn unknown_tags_fall_back_to_english() {
        for tag in ["pl-PL", "zz", "", "nonsense"] {
            assert_eq!(template_lang(tag), "en-US", "tag: {tag}");
            assert!(!is_supported(tag), "tag: {tag}");
        }
    }

    #[test]
    fn every_supported_locale_maps_to_itself() {
        for tag in SUPPORTED {
            assert_eq!(template_lang(tag), tag);
            assert!(is_supported(tag));
        }
    }

    #[test]
    fn unsupported_tags_are_rejected_on_write_but_unset_is_fine() {
        assert!(reject_unsupported(None).is_ok());
        assert!(reject_unsupported(Some("   ")).is_ok());
        assert!(reject_unsupported(Some("fr-BE")).is_ok());
        assert!(reject_unsupported(Some("DE_de")).is_ok());
        assert_eq!(reject_unsupported(Some("pl-PL")), Err("pl-PL".to_string()));
        assert_eq!(
            reject_unsupported(Some(" klingon ")),
            Err("klingon".to_string())
        );
    }

    #[test]
    fn override_warning_targets_the_winning_explicit_choice() {
        // The customer's list wins over the entity, even when the entity's is fine.
        assert_eq!(
            unsupported_override(&prefs(&["pl-PL"]), Some("fr-FR")),
            Some("pl-PL")
        );
        // An empty list is "unset", so the entity's choice is what applies.
        assert_eq!(
            unsupported_override(&prefs(&["  "]), Some("pl-PL")),
            Some("pl-PL")
        );
        // Nothing to flag when the winning choice is supported, or nothing is set.
        assert_eq!(
            unsupported_override(&prefs(&["fr-FR"]), Some("pl-PL")),
            None
        );
        assert_eq!(
            unsupported_override(&prefs(&["pl-PL", "nl-BE"]), None),
            None
        );
        assert_eq!(unsupported_override(&prefs(&["de-DE"]), None), None);
        assert_eq!(unsupported_override(&[], None), None);
    }

    fn prefs(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|t| t.to_string()).collect()
    }

    /// Falling through to the next choice is the point of an ordered list: a language we
    /// ship no bundle for must not cost the customer the one they also asked for.
    #[test]
    fn the_first_servable_preference_wins() {
        assert_eq!(
            first_supported(&prefs(&["pl", "nl-BE", "fr"])),
            Some("nl-BE")
        );
        assert_eq!(first_supported(&prefs(&["  ", "de-CH"])), Some("de-CH"));
        assert_eq!(first_supported(&prefs(&["pl-PL", "zz"])), None);
        assert_eq!(first_supported(&[]), None);
    }

    #[test]
    fn a_list_is_rejected_on_write_only_when_nothing_in_it_can_be_served() {
        assert!(reject_unsupported_preferences(&[]).is_ok());
        assert!(reject_unsupported_preferences(&prefs(&["  "])).is_ok());
        assert!(reject_unsupported_preferences(&prefs(&["fr-BE"])).is_ok());
        assert!(reject_unsupported_preferences(&prefs(&["pl-PL", "de-DE"])).is_ok());
        assert_eq!(
            reject_unsupported_preferences(&prefs(&["pl-PL", "zz"])),
            Err("pl-PL".to_string())
        );
    }
}
