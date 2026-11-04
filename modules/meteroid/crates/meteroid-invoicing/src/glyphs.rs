//! Maps template inputs onto the glyphs the bundled fonts can actually draw.
//!
//! Typst aborts the whole compilation on the first character no font covers, so a single
//! exotic code point in a customer-supplied name, memo or line item would otherwise fail
//! the invoice. Uncovered characters are downgraded to a look-alike, or dropped.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt::Write;
use typst::foundations::{Dict, Value};
use typst::text::Font;

pub(crate) struct FontCoverage {
    fonts: Vec<Font>,
}

/// The characters a single document had to give up, so the substitutions the reader sees on
/// the page are traceable back to the data that produced them.
#[derive(Default)]
struct Rewrites {
    replaced: BTreeSet<char>,
    dropped: BTreeSet<char>,
}

impl Rewrites {
    fn is_empty(&self) -> bool {
        self.replaced.is_empty() && self.dropped.is_empty()
    }

    fn code_points(chars: &BTreeSet<char>) -> String {
        chars.iter().fold(String::new(), |mut out, c| {
            if !out.is_empty() {
                out.push_str(", ");
            }
            let _ = write!(out, "U+{:04X} ({c})", *c as u32);
            out
        })
    }
}

impl FontCoverage {
    pub(crate) fn new(fonts: &[Font]) -> Self {
        FontCoverage {
            fonts: fonts.to_vec(),
        }
    }

    /// Rewrites every string in a template input dictionary, recursing into nested
    /// dictionaries and arrays. Non-textual values are passed through untouched.
    pub(crate) fn sanitize_dict(&self, dict: Dict) -> Dict {
        let mut rewrites = Rewrites::default();
        let sanitized = self.dict(dict, &mut rewrites);

        if !rewrites.is_empty() {
            tracing::warn!(
                replaced = %Rewrites::code_points(&rewrites.replaced),
                dropped = %Rewrites::code_points(&rewrites.dropped),
                "document text contains characters no bundled font covers; they were substituted to keep the render from failing"
            );
        }

        sanitized
    }

    /// Whether a string reaches the page unchanged. Callers use it to pick a renderable
    /// alternative — a currency code for an uncovered symbol — before dropping to nothing.
    pub(crate) fn renders(&self, text: &str) -> bool {
        text.chars().all(|c| self.covers(c))
    }

    fn dict(&self, dict: Dict, rewrites: &mut Rewrites) -> Dict {
        dict.into_iter()
            .map(|(key, value)| (key, self.value(value, rewrites)))
            .collect()
    }

    fn value(&self, value: Value, rewrites: &mut Rewrites) -> Value {
        match value {
            Value::Str(s) => Value::Str(self.sanitize_str(s.as_str(), rewrites).into()),
            Value::Dict(d) => Value::Dict(self.dict(d, rewrites)),
            Value::Array(a) => {
                Value::Array(a.into_iter().map(|v| self.value(v, rewrites)).collect())
            }
            other => other,
        }
    }

    fn sanitize_str<'a>(&self, text: &'a str, rewrites: &mut Rewrites) -> Cow<'a, str> {
        if self.renders(text) {
            return Cow::Borrowed(text);
        }

        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            if self.covers(c) {
                out.push(c);
                continue;
            }

            let stand_in: String = look_alike(c).chars().filter(|c| self.covers(*c)).collect();
            if stand_in.is_empty() {
                rewrites.dropped.insert(c);
            } else {
                rewrites.replaced.insert(c);
                out.push_str(&stand_in);
            }
        }
        Cow::Owned(out)
    }

    fn covers(&self, c: char) -> bool {
        // Line breaks and tabs are resolved into layout before shaping, so they never
        // need a glyph.
        c.is_ascii_whitespace()
            || is_bidi_control(c)
            || self
                .fonts
                .iter()
                .any(|font| font.ttf().glyph_index(c).is_some())
    }
}

/// Unicode bidirectional formatting controls. The bidi algorithm consumes these during
/// layout, so like whitespace they never reach glyph shaping and need no font coverage.
/// Fluent wraps every interpolated placeable in FSI (U+2068) … PDI (U+2069) to keep an
/// embedded RTL value (say an Arabic customer name) from reordering the surrounding text,
/// so they must pass through untouched — dropping them would break bidi rendering.
fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// A stand-in for a character the fonts do not cover, itself subject to a coverage check.
/// The empty string drops the character, which is the right call for invisible formatting
/// marks and the safe default for anything unrecognized.
fn look_alike(c: char) -> &'static str {
    match c {
        // Separator and operator dots, e.g. the word separator middle dot.
        '\u{2027}' | '\u{2219}' | '\u{22C5}' | '\u{2E31}' | '\u{30FB}' | '\u{FF65}' => "\u{00B7}",
        // Dashes, minus signs and the longer horizontal bars.
        '\u{2010}'..='\u{2015}' | '\u{2212}' | '\u{2E3A}' | '\u{2E3B}' | '\u{FE58}' => "-",
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' => "'",
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' | '\u{2033}' => "\"",
        '\u{2026}' => "...",
        _ if c.is_whitespace() => " ",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use typst::foundations::{Array, IntoValue};

    fn coverage() -> FontCoverage {
        FontCoverage::new(&crate::typst_render::load_fonts().unwrap())
    }

    fn sanitize(coverage: &FontCoverage, text: &str) -> String {
        coverage
            .sanitize_str(text, &mut Rewrites::default())
            .into_owned()
    }

    #[test]
    fn uncovered_separators_become_a_middle_dot() {
        let coverage = coverage();
        assert!(!coverage.covers('\u{2E31}'));
        assert_eq!(sanitize(&coverage, "a\u{2E31}b"), "a\u{00B7}b");
    }

    #[test]
    fn covered_text_is_left_alone() {
        let coverage = coverage();
        for text in [
            "Acme SAS — Facture n°1",
            "請求書 2024",
            "الفاتورة",
            "a\tb\nc",
        ] {
            assert_eq!(sanitize(&coverage, text), text);
        }
    }

    #[test]
    fn bidi_controls_pass_through_without_warning() {
        let coverage = coverage();
        // Fluent wraps placeables in FSI … PDI; these are consumed by the bidi algorithm,
        // not shaped, so they survive untouched and record no rewrite.
        let mut rewrites = Rewrites::default();
        let text = "Exchange rate on \u{2068}Jan 1\u{2069}";
        assert_eq!(
            coverage.sanitize_str(text, &mut rewrites),
            Cow::Borrowed(text)
        );
        assert!(rewrites.is_empty());
    }

    #[test]
    fn unmappable_characters_are_dropped() {
        let coverage = coverage();
        // Egyptian hieroglyph: no font covers it and no look-alike exists.
        assert_eq!(sanitize(&coverage, "a\u{13000}b"), "ab");
    }

    #[test]
    fn rewrites_are_recorded_for_logging() {
        let coverage = coverage();
        let mut rewrites = Rewrites::default();
        coverage.sanitize_str("a\u{2E31}b\u{13000}c\u{2E31}", &mut rewrites);

        assert_eq!(
            Rewrites::code_points(&rewrites.replaced),
            "U+2E31 (\u{2E31})"
        );
        assert_eq!(
            Rewrites::code_points(&rewrites.dropped),
            "U+13000 (\u{13000})"
        );
    }

    #[test]
    fn sanitizing_recurses_into_nested_inputs() {
        let coverage = coverage();
        let nested = typst::foundations::dict! {
            "name" => "a\u{2E31}b".into_value(),
            "lines" => Array::from_iter([typst::foundations::dict! {
                "label" => "c\u{2E31}d".into_value(),
            }
            .into_value()])
            .into_value(),
            "count" => 3.into_value(),
        };

        let sanitized = coverage.sanitize_dict(nested);
        assert_eq!(
            sanitized.at("name".into(), None).unwrap(),
            "a\u{00B7}b".into_value()
        );
        assert_eq!(sanitized.at("count".into(), None).unwrap(), 3.into_value());

        let Value::Array(lines) = sanitized.at("lines".into(), None).unwrap() else {
            panic!("lines should still be an array");
        };
        let Value::Dict(line) = lines.at(0, None).unwrap() else {
            panic!("line should still be a dict");
        };
        assert_eq!(
            line.at("label".into(), None).unwrap(),
            "c\u{00B7}d".into_value()
        );
    }
}
