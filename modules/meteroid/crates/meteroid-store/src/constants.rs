use meteroid_money::iso;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct Country {
    pub code: &'static str,
    pub name: &'static str,
    pub currency: &'static str,
    pub locale: &'static str,
}

#[derive(Debug, Clone)]
pub struct Currency {
    pub code: &'static str,
    pub name: &'static str,
    pub symbol: &'static str,
    pub precision: u8,
}

impl Currency {
    fn from_iso(c: &'static iso::Currency) -> Self {
        Currency {
            code: c.iso_alpha_code,
            name: c.name,
            symbol: c.symbol,
            precision: c.exponent as u8,
        }
    }
}

const COUNTRIES_JSON: &str = include_str!("../static/countries.json");

pub static COUNTRIES: std::sync::LazyLock<&'static [Country]> = std::sync::LazyLock::new(|| {
    Box::leak(
        serde_json::from_str::<Vec<Country>>(COUNTRIES_JSON)
            .unwrap()
            .into_boxed_slice(),
    )
});

pub struct Countries {}
impl Countries {
    pub fn resolve_country(country: &str) -> Option<Country> {
        COUNTRIES.iter().find(|c| c.code == country).cloned()
    }

    /// Countries whose law fixes the language of an invoice by reference to the
    /// *issuer's* establishment, so a customer's preference cannot override it.
    ///
    /// Belgium is the only one: an invoice is a document prescribed by law (VAT Code
    /// art. 53 §2), which brings it inside the language legislation. In Flanders the
    /// Decreet of 19 July 1973 art. 5 §1 requires Dutch and art. 10 makes a breaching
    /// document *nietig*, with the nullity raised by the court of its own motion and
    /// curable only prospectively; Wallonia's Décret of 30 June 1982 is the mirror
    /// image in French. *New Valmar* (C-15/15) struck the Dutch-only rule for
    /// cross-border supplies, but §57 leaves purely internal Belgian ones untouched.
    ///
    /// The Netherlands, Germany, Austria, Switzerland, Italy and Spain each permit any
    /// language, subject only to producing a translation on demand, so none belongs here.
    fn document_language_bound_by_issuer(entity_country_code: &str) -> bool {
        entity_country_code == "BE"
    }

    /// Countries whose single default language is a poor guess, so an operator should be
    /// asked rather than defaulted.
    ///
    /// Deliberately a *wider* and weaker notion than
    /// [`Self::document_language_bound_by_issuer`], which is about law: Belgium is both
    /// (a wrong language is void), Switzerland is only this. Swiss law imposes no
    /// language requirement on an invoice at all — neither MWSTG nor MWSTV says anything
    /// about it, and OR art. 957a al. 5 expressly allows the books to be kept in English
    /// — but the country has four official languages and no default serves them: German
    /// is 62% of the population, French 23%, Italian 8%, so `de-CH` is simply wrong for a
    /// Geneva or Lugano entity.
    ///
    /// Kept separate because the two drive different things. Binding changes *resolution*
    /// (the issuer's setting outranks the customer's, because law says so); ambiguity
    /// only changes *prompting* (ask at creation instead of guessing). Merging them would
    /// give Switzerland a precedence rule no Swiss statute supports.
    pub fn document_language_deserves_an_explicit_choice(entity_country_code: &str) -> bool {
        Self::document_language_bound_by_issuer(entity_country_code) || entity_country_code == "CH"
    }

    /// Presentation language for a rendered document, in priority order: an explicit
    /// customer override, then the invoicing entity's default, then the entity
    /// country's locale, then `en-US`. Empty overrides are treated as unset.
    ///
    /// Where the issuer's country binds the language by law, the entity's own setting
    /// wins over the customer's: setting a Walloon customer to French on a Flemish
    /// seller's account is the obvious, well-meant operator action that would otherwise
    /// produce a void invoice.
    pub fn resolve_document_language(
        customer_language: Option<&str>,
        entity_language: Option<&str>,
        entity_country_code: &str,
    ) -> String {
        fn non_empty(s: Option<&str>) -> Option<&str> {
            s.map(str::trim).filter(|s| !s.is_empty())
        }
        let entity_first = Self::document_language_bound_by_issuer(entity_country_code);
        let preferred = if entity_first {
            non_empty(entity_language).or_else(|| non_empty(customer_language))
        } else {
            non_empty(customer_language).or_else(|| non_empty(entity_language))
        };
        preferred
            .map(str::to_string)
            .or_else(|| Self::resolve_country(entity_country_code).map(|c| c.locale.to_string()))
            .unwrap_or_else(|| "en-US".to_string())
    }

    /// Whether this entity is issuing under a binding language law with nothing
    /// explicitly configured, so the country default is deciding a legal question by
    /// guess. True for a Belgian entity with no language set: the country default is
    /// `nl-BE`, which is right for Flanders and void in Wallonia.
    ///
    /// The `nl-BE` default stays despite that, deliberately: dropping it from
    /// `countries.json` would not make the guess loud, it would silently swap it for
    /// the `en-US` fallback below — void in *every* Belgian region within the law's
    /// reach, where Dutch is at least lawful in Flanders and Brussels — and would
    /// anglicize the entity's base-footer language too. The guess is instead surfaced
    /// where the operator can answer it: the invoicing-entity settings UI warns on
    /// this exact state via `invoiceLanguageIsUnsafeGuess` in the web app's
    /// `InvoiceLanguageSelect.tsx`, a mirror of this predicate — keep them in step.
    pub fn document_language_is_an_unsafe_guess(
        entity_language: Option<&str>,
        entity_country_code: &str,
    ) -> bool {
        Self::document_language_bound_by_issuer(entity_country_code)
            && entity_language.map(str::trim).is_none_or(str::is_empty)
    }

    /// PDF and email language for the same inputs (customer > entity > country; Belgium
    /// entity-first), collapsed to a template locale. Flags the configured choices no
    /// bundle can serve, naming `document` so the warning traces back to it.
    pub fn resolve_template_lang(
        customer_locales: &[String],
        entity_language: Option<&str>,
        country: &str,
        document: impl std::fmt::Display,
    ) -> &'static str {
        let customer = common_domain::lang::first_supported(customer_locales);
        let resolved = Self::resolve_document_language(customer, entity_language, country);
        if let Some(unsupported) =
            common_domain::lang::unsupported_override(customer_locales, entity_language)
        {
            tracing::warn!(
                document = %document,
                language = unsupported,
                resolved = resolved,
                supported = ?common_domain::lang::SUPPORTED,
                "no message bundle for the configured document language; \
                 the level below decided it instead"
            );
        }
        if Self::document_language_is_an_unsafe_guess(entity_language, country) {
            tracing::warn!(
                document = %document,
                entity_country = country,
                "invoicing entity in a jurisdiction that binds the document language to \
                 the issuer's establishment has no language configured; falling back to \
                 the country default, which may be the wrong language in law"
            );
        }
        common_domain::lang::template_lang(&resolved)
    }
}

#[cfg(test)]
mod issuer_bound_language_tests {
    use super::Countries;

    #[test]
    fn customer_preference_wins_where_no_law_binds_the_language() {
        assert_eq!(
            Countries::resolve_document_language(Some("de-DE"), Some("fr-FR"), "FR"),
            "de-DE"
        );
    }

    /// Belgium binds the language to the issuer's establishment, so a customer
    /// override must not decide it.
    #[test]
    fn belgian_issuers_keep_their_own_language() {
        assert_eq!(
            Countries::resolve_document_language(Some("fr-BE"), Some("nl-BE"), "BE"),
            "nl-BE"
        );
        // With nothing set on the entity, the customer's choice is still better than
        // guessing from the country.
        assert_eq!(
            Countries::resolve_document_language(Some("fr-BE"), None, "BE"),
            "fr-BE"
        );
    }

    /// Belgium is asked because the law binds it; Switzerland because no single default
    /// serves four official languages. The two predicates must not collapse into one:
    /// Swiss law imposes no language on an invoice, so Switzerland must never gain the
    /// precedence rule that Belgium has.
    #[test]
    fn ambiguous_and_binding_are_different_questions() {
        for country in ["BE", "CH"] {
            assert!(
                Countries::document_language_deserves_an_explicit_choice(country),
                "{country} should be asked at creation"
            );
        }
        assert!(Countries::document_language_bound_by_issuer("BE"));
        assert!(!Countries::document_language_bound_by_issuer("CH"));

        for country in ["FR", "DE", "NL", "US"] {
            assert!(
                !Countries::document_language_deserves_an_explicit_choice(country),
                "{country} has a serviceable default and should not be asked"
            );
        }

        // Switzerland gains no precedence rule: a customer override still wins there,
        // exactly as it does in France.
        assert_eq!(
            Countries::resolve_document_language(Some("fr-FR"), Some("de-DE"), "CH"),
            "fr-FR"
        );
    }

    #[test]
    fn an_unconfigured_belgian_entity_is_flagged_as_a_guess() {
        assert!(Countries::document_language_is_an_unsafe_guess(None, "BE"));
        assert!(Countries::document_language_is_an_unsafe_guess(
            Some("  "),
            "BE"
        ));
        assert!(!Countries::document_language_is_an_unsafe_guess(
            Some("fr-BE"),
            "BE"
        ));
        assert!(!Countries::document_language_is_an_unsafe_guess(None, "FR"));
    }
}

pub struct Currencies {}

impl Currencies {
    /// Resolve a currency to its data from the owned `meteroid_money::iso` set — the single source
    /// of truth for which currencies are supported and their minor-unit precision. Returns
    /// `None` for a code we don't support.
    pub fn resolve_currency(currency: &str) -> Option<Currency> {
        iso::find(currency).map(Currency::from_iso)
    }

    pub fn resolve_currency_precision(currency: &str) -> Option<u8> {
        Self::resolve_currency(currency).map(|c| c.precision)
    }

    /// Every supported currency, for listings such as the currency picker.
    pub fn all() -> impl Iterator<Item = Currency> {
        iso::ALL_CURRENCIES.iter().copied().map(Currency::from_iso)
    }
}

#[cfg(test)]
mod tests {
    use super::Currencies;

    #[test]
    fn precision_matches_owned_iso_table() {
        // Our owned overrides (see crates/meteroid-money iso set).
        assert_eq!(Currencies::resolve_currency_precision("HUF"), Some(2));
        assert_eq!(Currencies::resolve_currency_precision("MRU"), Some(2));
        // IQD is ISO-3 but kept at 2 to match existing invoices stored under the old precision;
        // switching to 3 requires a ×10 data migration first.
        assert_eq!(Currencies::resolve_currency_precision("IQD"), Some(2));
        // ISO/reality for these (no prod data was in them, so ISO values are safe to adopt).
        assert_eq!(Currencies::resolve_currency_precision("ISK"), Some(0));
        assert_eq!(Currencies::resolve_currency_precision("CLP"), Some(0));
        assert_eq!(Currencies::resolve_currency_precision("IDR"), Some(2));
        assert_eq!(Currencies::resolve_currency_precision("LYD"), Some(3));
        // Zero- and three-decimal currencies everyone agrees on.
        assert_eq!(Currencies::resolve_currency_precision("EUR"), Some(2));
        assert_eq!(Currencies::resolve_currency_precision("JPY"), Some(0));
        assert_eq!(Currencies::resolve_currency_precision("BHD"), Some(3));
    }

    #[test]
    fn supported_set_covers_the_full_iso_range() {
        let all: Vec<_> = Currencies::all().collect();
        // Full ISO set (+ CNH/VEF) — deliberately not narrowed, so no active currency or
        // country default resolves to `None`.
        assert!(all.len() > 150, "unexpectedly small set: {}", all.len());
        for c in &all {
            assert!(Currencies::resolve_currency(c.code).is_some());
        }
        // Hand-added, plus currencies an Adyen-only set would have dropped.
        for code in ["CNH", "VEF", "USD", "EUR", "BGN", "MGA", "HRK"] {
            assert!(
                Currencies::resolve_currency(code).is_some(),
                "{code} should resolve"
            );
        }
    }

    #[test]
    fn unsupported_code_does_not_resolve() {
        assert!(Currencies::resolve_currency("XZZ").is_none());
    }
}

#[cfg(test)]
mod document_language_tests {
    use super::Countries;

    fn resolve(customer: Option<&str>, entity: Option<&str>, country: &str) -> String {
        Countries::resolve_document_language(customer, entity, country)
    }

    #[test]
    fn customer_override_wins_over_everything() {
        assert_eq!(resolve(Some("fr-FR"), Some("en-US"), "DE"), "fr-FR");
    }

    #[test]
    fn entity_default_applies_without_a_customer_override() {
        assert_eq!(resolve(None, Some("fr-FR"), "DE"), "fr-FR");
    }

    #[test]
    fn country_locale_is_the_fallback_when_nothing_is_configured() {
        assert_eq!(resolve(None, None, "FR"), "fr-FR");
        assert_eq!(resolve(None, None, "DE"), "de-DE");
    }

    #[test]
    fn unknown_country_lands_on_english() {
        assert_eq!(resolve(None, None, "ZZ"), "en-US");
    }

    #[test]
    fn blank_overrides_are_unset_and_fall_through() {
        // Legacy rows may hold "" rather than NULL; it must not shadow the level below.
        assert_eq!(resolve(Some(""), Some("fr-FR"), "DE"), "fr-FR");
        assert_eq!(resolve(Some("   "), None, "DE"), "de-DE");
        assert_eq!(resolve(Some(""), Some("  "), "FR"), "fr-FR");
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_off_the_resolved_tag() {
        assert_eq!(resolve(Some(" fr-FR "), None, "DE"), "fr-FR");
    }
}

#[cfg(test)]
mod template_lang_tests {
    use super::Countries;
    use common_domain::lang;

    fn prefs(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|t| t.to_string()).collect()
    }

    /// The resolver must give exactly what the two rendering call sites computed inline
    /// before it existed, so a PDF and the emails about it are worded alike.
    #[test]
    fn matches_the_inline_resolution_of_the_rendering_call_sites() {
        let customers = [
            vec![],
            prefs(&["  "]),
            prefs(&["fr-BE"]),
            prefs(&["pl-PL"]),
            prefs(&["pl", "nl-BE", "fr"]),
            prefs(&["PT-br"]),
        ];
        let entities = [
            None,
            Some(""),
            Some("de-CH"),
            Some("pl-PL"),
            Some(" nl-BE "),
        ];
        let countries = ["FR", "BE", "CH", "BR", "ZZ"];
        for customer_locales in &customers {
            for entity_language in entities {
                for country in countries {
                    let inline = lang::template_lang(&Countries::resolve_document_language(
                        lang::first_supported(customer_locales),
                        entity_language,
                        country,
                    ));
                    assert_eq!(
                        Countries::resolve_template_lang(
                            customer_locales,
                            entity_language,
                            country,
                            "test"
                        ),
                        inline,
                        "customer: {customer_locales:?}, entity: {entity_language:?}, country: {country}"
                    );
                }
            }
        }
    }

    #[test]
    fn always_lands_on_a_shipped_template_locale() {
        assert_eq!(
            Countries::resolve_template_lang(&prefs(&["fr-BE"]), None, "DE", "test"),
            "fr-FR"
        );
        assert_eq!(
            Countries::resolve_template_lang(&[], Some("pt-BR"), "FR", "test"),
            "pt-BR"
        );
        assert_eq!(
            Countries::resolve_template_lang(&[], None, "FR", "test"),
            "fr-FR"
        );
        assert_eq!(
            Countries::resolve_template_lang(&prefs(&["pl-PL"]), Some("pl-PL"), "ZZ", "test"),
            "en-US"
        );
        assert_eq!(
            Countries::resolve_template_lang(&prefs(&["fr-BE"]), Some("nl-BE"), "BE", "test"),
            "nl-NL"
        );
    }
}
