//! Fixed customer-email strings and the seeded default reminder copy, one Fluent
//! bundle per template locale (`common_domain::lang::SUPPORTED`).

use fluent_static::MessageBundle;

#[allow(clippy::all)]
mod bundle {
    use fluent_static::message_bundle;

    #[message_bundle(
        resources = [
            ("l10n/en-US/email.ftl", "en-US"),
            ("l10n/fr-FR/email.ftl", "fr-FR"),
            ("l10n/de-DE/email.ftl", "de-DE"),
            ("l10n/es-ES/email.ftl", "es-ES"),
            ("l10n/it-IT/email.ftl", "it-IT"),
            ("l10n/nl-NL/email.ftl", "nl-NL"),
            ("l10n/pt-PT/email.ftl", "pt-PT"),
            ("l10n/pt-BR/email.ftl", "pt-BR"),
        ],
        default_language = "en-US")]
    pub struct EmailL10n;
}

pub use bundle::EmailL10N;

/// The bundle for a template locale, `en-US` for anything else. Bidi isolation is off:
/// its U+2068/U+2069 marks would land in subjects and seeded copy as stray characters.
pub fn email_l10n(lang: &str) -> EmailL10N {
    let mut bundle = EmailL10N::get(lang).unwrap_or_default();
    bundle.set_use_isolating(false);
    bundle
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::lang::SUPPORTED;
    use fluent_static::LanguageAware;
    use fluent_syntax::ast::Entry;
    use std::collections::BTreeSet;

    const FTL: [(&str, &str); 8] = [
        ("en-US", include_str!("../l10n/en-US/email.ftl")),
        ("fr-FR", include_str!("../l10n/fr-FR/email.ftl")),
        ("de-DE", include_str!("../l10n/de-DE/email.ftl")),
        ("es-ES", include_str!("../l10n/es-ES/email.ftl")),
        ("it-IT", include_str!("../l10n/it-IT/email.ftl")),
        ("nl-NL", include_str!("../l10n/nl-NL/email.ftl")),
        ("pt-PT", include_str!("../l10n/pt-PT/email.ftl")),
        ("pt-BR", include_str!("../l10n/pt-BR/email.ftl")),
    ];

    /// The §2.3 operator-copy variables; the seeded reminders may use nothing else.
    const VARIABLES: [&str; 9] = [
        "customer_name",
        "company_name",
        "invoice_number",
        "invoice_date",
        "due_date",
        "amount_due",
        "currency",
        "days_overdue",
        "plan_name",
    ];

    fn message_ids(source: &str) -> BTreeSet<String> {
        let resource = fluent_syntax::parser::parse(source)
            .unwrap_or_else(|(_, errors)| panic!("invalid ftl: {errors:?}"));
        resource
            .body
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Message(message) => Some(message.id.name.to_string()),
                _ => None,
            })
            .collect()
    }

    fn variables_in(text: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find("{{") {
            let after = &rest[start + 2..];
            let end = after.find("}}").expect("unclosed variable");
            found.push(after[..end].trim().to_string());
            rest = &after[end + 2..];
        }
        found
    }

    fn reminder_pairs(bundle: &EmailL10N) -> [(&'static str, String, String); 4] {
        [
            (
                "before",
                bundle.reminder_default_before_subject().to_string(),
                bundle.reminder_default_before_body().to_string(),
            ),
            (
                "due",
                bundle.reminder_default_due_subject().to_string(),
                bundle.reminder_default_due_body().to_string(),
            ),
            (
                "overdue",
                bundle.reminder_default_overdue_subject().to_string(),
                bundle.reminder_default_overdue_body().to_string(),
            ),
            (
                "final",
                bundle.reminder_default_final_subject().to_string(),
                bundle.reminder_default_final_body().to_string(),
            ),
        ]
    }

    #[test]
    fn bundle_languages_are_the_supported_template_locales() {
        let bundle: BTreeSet<&str> = EmailL10N::supported_language_ids()
            .iter()
            .copied()
            .collect();
        let supported: BTreeSet<&str> = SUPPORTED.iter().copied().collect();
        assert_eq!(bundle, supported);
        assert_eq!(EmailL10N::default_language_id(), "en-US");
    }

    /// fluent-static falls back to en-US silently for a key a locale lacks, so the
    /// only guard against a half-translated file is this one.
    #[test]
    fn ftl_key_parity_across_locales() {
        let (_, en) = FTL[0];
        let reference = message_ids(en);
        assert!(!reference.is_empty());
        for (locale, source) in FTL {
            let ids = message_ids(source);
            let missing: Vec<_> = reference.difference(&ids).collect();
            let extra: Vec<_> = ids.difference(&reference).collect();
            assert!(
                missing.is_empty() && extra.is_empty(),
                "{locale}: missing {missing:?}, extra {extra:?}"
            );
        }
    }

    #[test]
    fn reminder_default_copy_is_valid() {
        for locale in SUPPORTED {
            let bundle = email_l10n(locale);
            assert_eq!(bundle.language_id(), locale);
            for (step, subject, body) in reminder_pairs(&bundle) {
                let ctx = format!("{locale}/{step}");
                for text in [&subject, &body] {
                    for var in variables_in(text) {
                        assert!(VARIABLES.contains(&var.as_str()), "{ctx}: variable {var}");
                    }
                    assert!(!text.contains(['<', '>']), "{ctx}: html: {text}");
                    assert!(
                        !text.contains(['\u{2068}', '\u{2069}']),
                        "{ctx}: bidi isolation marks: {text:?}"
                    );
                }
                assert!(!subject.trim().is_empty(), "{ctx}: blank subject");
                assert!(subject.chars().count() <= 200, "{ctx}: subject too long");
                assert!(!body.trim().is_empty(), "{ctx}: blank body");
                assert!(body.chars().count() <= 5000, "{ctx}: body too long");
                assert!(
                    body.contains("{{customer_name}}") && body.contains("{{invoice_number}}"),
                    "{ctx}: a reminder names the customer and the invoice"
                );
            }
        }
    }

    /// What a seeded default step −3 renders in en-US (Appendix A).
    #[test]
    fn en_us_seed_resolves_to_the_flat_variable_syntax() {
        let bundle = email_l10n("en-US");
        assert_eq!(
            bundle.reminder_default_before_subject(),
            "Invoice {{invoice_number}} is due on {{due_date}}"
        );
        assert_eq!(
            bundle.reminder_default_before_body(),
            "Hi {{customer_name}},\n\nA quick heads-up: invoice {{invoice_number}} for {{amount_due}} is due on {{due_date}}."
        );
    }

    #[test]
    fn unknown_locales_read_english() {
        assert_eq!(email_l10n("pl-PL").language_id(), "en-US");
        assert_eq!(email_l10n("fr-FR").language_id(), "fr-FR");
    }
}
