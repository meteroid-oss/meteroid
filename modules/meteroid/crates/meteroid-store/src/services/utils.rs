use chrono::Datelike;
use chrono::NaiveDate;

const DEFAULT_NUMBER_WIDTH: usize = 4;
const MAX_NUMBER_WIDTH: usize = 20;

/// Placeholders: `{number}` (4-digit zero padding), `{number:N}` (N-digit), `{YYYY}`, `{MM}`, `{DD}`.
pub fn format_document_number(number: i64, pattern: &str, date: NaiveDate) -> String {
    replace_number_placeholders(pattern, number)
        .replace("{YYYY}", &date.year().to_string())
        .replace("{MM}", &format!("{:02}", date.month()))
        .replace("{DD}", &format!("{:02}", date.day()))
}

/// A pattern without a sequence placeholder would issue the same number twice.
pub fn is_valid_number_pattern(pattern: &str) -> bool {
    let date = NaiveDate::default();
    format_document_number(1, pattern, date) != format_document_number(2, pattern, date)
}

fn replace_number_placeholders(pattern: &str, number: i64) -> String {
    const OPEN: &str = "{number";
    let mut out = String::with_capacity(pattern.len());
    let mut rest = pattern;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let tail = &rest[start + OPEN.len()..];
        match parse_number_width(tail) {
            Some((width, consumed)) => {
                out.push_str(&format!("{number:0width$}"));
                rest = &tail[consumed..];
            }
            None => {
                out.push_str(OPEN);
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Parses `}` or `:N}` right after `{number`, returning the width and bytes consumed.
fn parse_number_width(tail: &str) -> Option<(usize, usize)> {
    let end = tail.find('}')?;
    let width = match &tail[..end] {
        "" => DEFAULT_NUMBER_WIDTH,
        spec => spec
            .strip_prefix(':')?
            .parse::<usize>()
            .ok()
            .filter(|w| (1..=MAX_NUMBER_WIDTH).contains(w))?,
    };
    Some((width, end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, 7).unwrap()
    }

    #[test]
    fn default_width_and_date_parts() {
        assert_eq!(
            format_document_number(42, "INV-{YYYY}{MM}{DD}-{number}", date()),
            "INV-20260307-0042"
        );
    }

    #[test]
    fn explicit_width_matches_legacy_credit_note_format() {
        assert_eq!(
            format_document_number(7, "CN-{number:6}", date()),
            format!("CN-{:06}", 7)
        );
    }

    #[test]
    fn number_wider_than_padding_is_not_truncated() {
        assert_eq!(
            format_document_number(123456, "{number:2}", date()),
            "123456"
        );
    }

    #[test]
    fn malformed_placeholders_are_left_verbatim() {
        assert_eq!(
            format_document_number(1, "{number:x}-{number:0}-{number:99}-{number", date()),
            "{number:x}-{number:0}-{number:99}-{number"
        );
    }

    #[test]
    fn pattern_validity_requires_a_sequence_placeholder() {
        assert!(is_valid_number_pattern("CN-{number}"));
        assert!(is_valid_number_pattern("{YYYY}/{number:5}"));
        assert!(!is_valid_number_pattern("CN-{YYYY}"));
        assert!(!is_valid_number_pattern("CN-{number:abc}"));
    }
}
