/// Enumerates regions which have unique formatting standards for Currencies.
///
/// Each Locale maps 1:1 to a LocalFormat, which contains the characteristics for formatting.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Locale {
    EnUs,
    EnIn,
    EnEu,
    EnBy,
}

/// Stores currency formatting metadata for a specific region (e.g. EN-US).
#[derive(Debug, PartialEq, Eq)]
pub struct LocalFormat {
    pub name: &'static str,
    pub digit_separator: char,
    pub digit_separator_pattern: &'static [usize],
    pub exponent_separator: char,
}

// Pre-defined patterns to avoid allocation
const PATTERN_3_3_3: &[usize] = &[3, 3, 3];
const PATTERN_3_2_2: &[usize] = &[3, 2, 2];

impl LocalFormat {
    /// Returns the associated LocalFormat given a Locale.
    pub fn from_locale(locale: Locale) -> LocalFormat {
        use Locale::*;

        match locale {
            EnUs => LocalFormat {
                name: "en-us",
                digit_separator: ',',
                digit_separator_pattern: PATTERN_3_3_3,
                exponent_separator: '.',
            },
            EnIn => LocalFormat {
                name: "en-in",
                digit_separator: ',',
                digit_separator_pattern: PATTERN_3_2_2,
                exponent_separator: '.',
            },
            EnEu => LocalFormat {
                name: "en-eu",
                digit_separator: '.',
                digit_separator_pattern: PATTERN_3_3_3,
                exponent_separator: ',',
            },
            EnBy => LocalFormat {
                name: "en-by",
                digit_separator: ' ',
                digit_separator_pattern: PATTERN_3_3_3,
                exponent_separator: ',',
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn en_us_locale_format() {
        let format = LocalFormat::from_locale(Locale::EnUs);
        assert_eq!(format.name, "en-us");
        assert_eq!(format.digit_separator, ',');
        assert_eq!(format.exponent_separator, '.');
        assert_eq!(format.digit_separator_pattern, &[3, 3, 3]);
    }

    #[test]
    fn en_in_locale_format() {
        // Indian numbering: lakhs and crores (2,2,3 pattern from right)
        let format = LocalFormat::from_locale(Locale::EnIn);
        assert_eq!(format.name, "en-in");
        assert_eq!(format.digit_separator, ',');
        assert_eq!(format.exponent_separator, '.');
        assert_eq!(format.digit_separator_pattern, &[3, 2, 2]);
    }

    #[test]
    fn en_eu_locale_format() {
        // European: swap comma and period
        let format = LocalFormat::from_locale(Locale::EnEu);
        assert_eq!(format.name, "en-eu");
        assert_eq!(format.digit_separator, '.');
        assert_eq!(format.exponent_separator, ',');
        assert_eq!(format.digit_separator_pattern, &[3, 3, 3]);
    }

    #[test]
    fn en_by_locale_format() {
        // Belarusian: space as digit separator
        let format = LocalFormat::from_locale(Locale::EnBy);
        assert_eq!(format.name, "en-by");
        assert_eq!(format.digit_separator, ' ');
        assert_eq!(format.exponent_separator, ',');
        assert_eq!(format.digit_separator_pattern, &[3, 3, 3]);
    }

    #[test]
    fn all_locales_have_distinct_names() {
        let locales = [Locale::EnUs, Locale::EnIn, Locale::EnEu, Locale::EnBy];
        let names: Vec<_> = locales
            .iter()
            .map(|l| LocalFormat::from_locale(*l).name)
            .collect();

        // Verify all names are unique
        for (i, name) in names.iter().enumerate() {
            for (j, other) in names.iter().enumerate() {
                if i != j {
                    assert_ne!(name, other, "Locale names must be unique");
                }
            }
        }
    }

    #[test]
    fn digit_and_exponent_separators_differ() {
        // Critical invariant: digit and exponent separators must differ
        // to avoid ambiguous parsing (e.g., "1,000" vs "1,00")
        let locales = [Locale::EnUs, Locale::EnIn, Locale::EnEu, Locale::EnBy];
        for locale in locales {
            let format = LocalFormat::from_locale(locale);
            assert_ne!(
                format.digit_separator, format.exponent_separator,
                "Locale {:?} has same digit and exponent separator",
                locale
            );
        }
    }
}
