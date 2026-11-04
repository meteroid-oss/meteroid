//! Locale-aware formatting shared by the document and email renderers.

use chrono::{Locale, NaiveDate};

/// chrono locale backing a template locale. `en-US` has none: its format strings
/// are written for chrono's default (C) locale.
pub fn chrono_locale(lang: &str) -> Option<Locale> {
    match lang {
        "fr-FR" => Some(Locale::fr_FR),
        "de-DE" => Some(Locale::de_DE),
        "es-ES" => Some(Locale::es_ES),
        "it-IT" => Some(Locale::it_IT),
        "nl-NL" => Some(Locale::nl_NL),
        "pt-PT" => Some(Locale::pt_PT),
        "pt-BR" => Some(Locale::pt_BR),
        _ => None,
    }
}

/// The long date form a language spells out on documents ("4 janvier 2024").
pub fn long_date(lang: &str, date: &NaiveDate) -> String {
    match (chrono_locale(lang), lang) {
        // `%-e` rather than `%e`: the padded form leaves a leading space on
        // single-digit days (" 4 janvier 2024").
        (Some(locale), "de-DE") => date.format_localized("%-e. %B %Y", locale).to_string(),
        (Some(locale), "es-ES" | "pt-PT" | "pt-BR") => {
            date.format_localized("%-e de %B de %Y", locale).to_string()
        }
        (Some(locale), _) => date.format_localized("%-e %B %Y", locale).to_string(),
        (None, _) => date.format("%B %-e, %Y").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_locale_but_english_has_a_chrono_locale() {
        for lang in crate::lang::SUPPORTED {
            assert_eq!(
                chrono_locale(lang).is_some(),
                lang != "en-US",
                "lang: {lang}"
            );
        }
        assert!(chrono_locale("pl-PL").is_none());
    }

    #[test]
    fn long_date_spells_the_month_out_in_the_document_language() {
        let date = NaiveDate::from_ymd_opt(2024, 1, 4).unwrap();
        assert_eq!(long_date("en-US", &date), "January 4, 2024");
        assert_eq!(long_date("fr-FR", &date), "4 janvier 2024");
        assert_eq!(long_date("de-DE", &date), "4. Januar 2024");
        assert_eq!(long_date("es-ES", &date), "4 de enero de 2024");
        assert_eq!(long_date("it-IT", &date), "4 gennaio 2024");
        assert_eq!(long_date("nl-NL", &date), "4 januari 2024");
        assert_eq!(long_date("pt-PT", &date), "4 de janeiro de 2024");
        assert_eq!(long_date("pt-BR", &date), "4 de janeiro de 2024");
        assert_eq!(long_date("zz", &date), "January 4, 2024");
    }
}
