//! Customer-facing language of an invoicing entity (emails now, the portal later), decoupled
//! from the invoice language. The invoice PDF keeps its own legal rule (`Countries`).

use crate::constants::Countries;
use crate::domain::InvoicingEntity;
use common_domain::lang;

/// What the invoice language resolves to for the entity alone (no customer preference).
pub fn entity_locale(entity: &InvoicingEntity) -> &'static str {
    Countries::resolve_template_lang(
        &[],
        entity.invoicing_language.as_deref(),
        &entity.country.code,
        format_args!("invoicing entity {}", entity.id),
    )
}

/// The entity's customer-facing default: `customer_locale` when set and supported, else the
/// invoice language.
pub fn customer_default_locale(entity: &InvoicingEntity) -> &'static str {
    supported_template_lang(entity.customer_locale.as_deref())
        .unwrap_or_else(|| entity_locale(entity))
}

/// Locales a customer-facing surface may use: the default first, then the supported extras,
/// normalized and deduped.
pub fn customer_enabled_locales(entity: &InvoicingEntity) -> Vec<&'static str> {
    enabled_locales(
        customer_default_locale(entity),
        &entity.customer_extra_locales,
    )
}

/// With an explicit customer-facing default, the customer's first preference among the enabled
/// locales. Without one, the customer's first supported preference, as the invoice would read
/// it minus the country's legal language rule, which is PDF law only.
pub fn customer_locale_for(entity: &InvoicingEntity, customer_locales: &[String]) -> &'static str {
    if supported_template_lang(entity.customer_locale.as_deref()).is_some() {
        pick_locale(&customer_enabled_locales(entity), customer_locales)
    } else {
        first_supported(customer_locales).unwrap_or_else(|| entity_locale(entity))
    }
}

fn first_supported(customer_locales: &[String]) -> Option<&'static str> {
    customer_locales
        .iter()
        .find_map(|tag| supported_template_lang(Some(tag)))
}

fn supported_template_lang(tag: Option<&str>) -> Option<&'static str> {
    tag.map(str::trim)
        .filter(|t| !t.is_empty() && lang::is_supported(t))
        .map(lang::template_lang)
}

fn enabled_locales(default: &'static str, extras: &[String]) -> Vec<&'static str> {
    let mut locales = vec![default];
    for locale in extras
        .iter()
        .filter_map(|e| supported_template_lang(Some(e)))
    {
        if !locales.contains(&locale) {
            locales.push(locale);
        }
    }
    locales
}

/// `enabled` is never empty: its first entry is the default.
fn pick_locale(enabled: &[&'static str], customer_locales: &[String]) -> &'static str {
    customer_locales
        .iter()
        .filter_map(|tag| supported_template_lang(Some(tag)))
        .find(|locale| enabled.contains(locale))
        .unwrap_or(enabled[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|t| t.to_string()).collect()
    }

    fn pick(default: &'static str, extras: &[&str], prefs: &[&str]) -> &'static str {
        pick_locale(&enabled_locales(default, &strings(extras)), &strings(prefs))
    }

    #[test]
    fn enabled_locales_put_the_default_first_and_dedupe_supported_extras() {
        assert_eq!(enabled_locales("fr-FR", &[]), vec!["fr-FR"]);
        let extras = strings(&["de-DE", "fr-FR", "de-AT", "pl-PL", "es-ES"]);
        assert_eq!(
            enabled_locales("fr-FR", &extras),
            vec!["fr-FR", "de-DE", "es-ES"]
        );
    }

    #[test]
    fn customer_locale_tags_are_normalized_or_ignored() {
        assert_eq!(supported_template_lang(Some(" en-GB ")), Some("en-US"));
        assert_eq!(supported_template_lang(Some("pl-PL")), None);
        assert_eq!(supported_template_lang(Some("  ")), None);
        assert_eq!(supported_template_lang(None), None);
    }

    #[test]
    fn an_enabled_customer_locale_is_used() {
        assert_eq!(pick("fr-FR", &["de-DE"], &["de-CH"]), "de-DE");
        assert_eq!(pick("fr-FR", &["de-DE"], &["fr-BE"]), "fr-FR");
    }

    #[test]
    fn a_customer_locale_not_enabled_gets_the_default() {
        assert_eq!(pick("fr-FR", &[], &["de-DE"]), "fr-FR");
        assert_eq!(pick("fr-FR", &[], &[]), "fr-FR");
    }

    #[test]
    fn the_first_enabled_preference_wins() {
        assert_eq!(
            pick("fr-FR", &["es-ES", "it-IT"], &["de-DE", "it-IT", "es-ES"]),
            "it-IT"
        );
    }

    #[test]
    fn unsupported_preferences_are_skipped() {
        assert_eq!(pick("fr-FR", &["es-ES"], &["pl-PL", " ", "es-MX"]), "es-ES");
        // `pl-PL` would read `en-US` through `template_lang`; it must not match an enabled en-US.
        assert_eq!(pick("fr-FR", &["en-US"], &["pl-PL"]), "fr-FR");
    }
}
