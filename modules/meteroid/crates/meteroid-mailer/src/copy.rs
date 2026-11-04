//! Operator-authored email copy (payment reminder steps): restricted Markdown plus a flat
//! `{{variable}}` whitelist. Values are substituted after the Markdown → HTML pass and
//! HTML-escaped, so neither the copy nor the data can inject markup.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::collections::BTreeMap;

pub const SUBJECT_MAX_CHARS: usize = 200;
pub const BODY_MAX_CHARS: usize = 5_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariableKind {
    Text,
    Date,
    Money,
    Int,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Variable {
    pub name: &'static str,
    pub kind: VariableKind,
    pub example: &'static str,
}

const fn var(name: &'static str, kind: VariableKind, example: &'static str) -> Variable {
    Variable {
        name,
        kind,
        example,
    }
}

pub const VARIABLES: &[Variable] = &[
    var("customer_name", VariableKind::Text, "Acme Corp"),
    var("company_name", VariableKind::Text, "Meteroid"),
    var("invoice_number", VariableKind::Text, "INV-2026-0042"),
    var("invoice_date", VariableKind::Date, "March 1, 2026"),
    var("due_date", VariableKind::Date, "March 31, 2026"),
    var("amount_due", VariableKind::Money, "€1,200.00"),
    var("currency", VariableKind::Text, "EUR"),
    var("days_overdue", VariableKind::Int, "7"),
    var("plan_name", VariableKind::Text, "Scale"),
];

pub fn variable(name: &str) -> Option<&'static Variable> {
    VARIABLES.iter().find(|v| v.name == name)
}

/// Variable values, already formatted for the email locale (dates spelled out, money with
/// its currency). Missing variables render as an empty string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vars(pub BTreeMap<&'static str, String>);

impl Vars {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every variable bound to its example value, for previews and the step editor.
    pub fn examples() -> Self {
        Self(
            VARIABLES
                .iter()
                .map(|v| (v.name, v.example.to_string()))
                .collect(),
        )
    }

    /// Ignores names outside [`VARIABLES`].
    pub fn set(&mut self, name: &str, value: impl Into<String>) -> &mut Self {
        if let Some(v) = variable(name) {
            self.0.insert(v.name, value.into());
        }
        self
    }

    pub fn with(mut self, name: &str, value: impl Into<String>) -> Self {
        self.set(name, value);
        self
    }

    fn value(&self, name: &str) -> &str {
        self.0.get(name).map(String::as_str).unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CopyError {
    #[error("unknown variables: {}", .0.join(", "))]
    UnknownVariables(Vec<String>),
    #[error("subject exceeds {SUBJECT_MAX_CHARS} characters")]
    SubjectTooLong,
    #[error("body exceeds {BODY_MAX_CHARS} characters")]
    BodyTooLong,
    #[error("{0} is not allowed in email copy")]
    DisallowedMarkdown(&'static str),
}

/// A `{{ ident }}` token starting at `text[start..]`: the identifier and the byte length of the token.
fn token_at(text: &str, start: usize) -> Option<(&str, usize)> {
    let rest = text.get(start..)?.strip_prefix("{{")?;
    let rest = rest.trim_start();
    let ident_len = rest
        .bytes()
        .take_while(|b| b.is_ascii_lowercase() || *b == b'_')
        .count();
    if ident_len == 0 {
        return None;
    }
    let ident = &rest[..ident_len];
    let after = rest[ident_len..].trim_start();
    let close = after.strip_prefix("}}")?;
    Some((ident, text.len() - start - close.len()))
}

enum Piece<'a> {
    Text(&'a str),
    Var(&'a str),
}

/// Walks `text`, yielding every `{{ident}}` as a `Var` and everything else as `Text`.
fn scan<'a>(text: &'a str, mut on_piece: impl FnMut(Piece<'a>)) {
    let mut cursor = 0;
    let mut literal_from = 0;
    while let Some(offset) = text[cursor..].find("{{") {
        let start = cursor + offset;
        match token_at(text, start) {
            Some((ident, len)) => {
                on_piece(Piece::Text(&text[literal_from..start]));
                on_piece(Piece::Var(ident));
                cursor = start + len;
                literal_from = cursor;
            }
            None => cursor = start + 2,
        }
    }
    on_piece(Piece::Text(&text[literal_from..]));
}

/// `{{var}}` → `value(var)` for known variables; unknown ones are left as written so a stale
/// token is visible rather than silently blank.
fn replace_vars(text: &str, value: impl Fn(&'static str) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    scan(text, |piece| match piece {
        Piece::Text(t) => out.push_str(t),
        Piece::Var(ident) => match variable(ident) {
            Some(v) => out.push_str(&value(v.name)),
            None => {
                out.push_str("{{");
                out.push_str(ident);
                out.push_str("}}");
            }
        },
    });
    out
}

/// Variable names used in `text` that are not in [`VARIABLES`], in order of first appearance.
pub fn unknown_variables(text: &str) -> Vec<String> {
    let mut unknown: Vec<String> = Vec::new();
    scan(text, |piece| {
        if let Piece::Var(ident) = piece
            && variable(ident).is_none()
            && !unknown.iter().any(|u| u == ident)
        {
            unknown.push(ident.to_string());
        }
    });
    unknown
}

fn parser(body_md: &str) -> Parser<'_> {
    Parser::new_ext(body_md, Options::empty())
}

/// How an event outside the whitelist is handled at render time.
enum Drop {
    /// The event itself is not emitted.
    Event,
    /// The container tag is not emitted; its children are kept.
    Unwrap,
    /// The container and everything inside it is not emitted.
    Subtree,
}

/// `Ok` for a whitelisted event, otherwise the construct name and how to drop it.
fn classify(event: &Event<'_>) -> Result<(), (&'static str, Drop)> {
    match event {
        Event::Start(tag) => match tag {
            Tag::Paragraph
            | Tag::Emphasis
            | Tag::Strong
            | Tag::List(_)
            | Tag::Item
            | Tag::BlockQuote(_) => Ok(()),
            Tag::Heading { .. } => Err(("heading", Drop::Unwrap)),
            Tag::Link { .. } => Err(("link", Drop::Unwrap)),
            Tag::Image { .. } => Err(("image", Drop::Subtree)),
            Tag::CodeBlock(_) => Err(("code block", Drop::Subtree)),
            Tag::HtmlBlock => Err(("html", Drop::Subtree)),
            Tag::Table(_) | Tag::TableHead | Tag::TableRow | Tag::TableCell => {
                Err(("table", Drop::Subtree))
            }
            Tag::FootnoteDefinition(_) => Err(("footnote", Drop::Subtree)),
            Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {
                Err(("definition list", Drop::Subtree))
            }
            Tag::Strikethrough | Tag::Superscript | Tag::Subscript => {
                Err(("formatting", Drop::Unwrap))
            }
            Tag::MetadataBlock(_) => Err(("metadata", Drop::Subtree)),
        },
        Event::End(_) | Event::Text(_) | Event::SoftBreak | Event::HardBreak => Ok(()),
        Event::Code(_) => Err(("inline code", Drop::Event)),
        Event::Html(_) | Event::InlineHtml(_) => Err(("html", Drop::Event)),
        Event::InlineMath(_) | Event::DisplayMath(_) => Err(("math", Drop::Event)),
        Event::FootnoteReference(_) => Err(("footnote", Drop::Event)),
        Event::Rule => Err(("horizontal rule", Drop::Event)),
        Event::TaskListMarker(_) => Err(("task list", Drop::Event)),
    }
}

fn disallowed_markdown(body_md: &str) -> Option<&'static str> {
    parser(body_md)
        .find_map(|event| classify(&event).err())
        .map(|(name, _)| name)
}

/// Whitelisted events only: disallowed containers are unwrapped or skipped with their subtree.
fn whitelist<'a>(events: impl Iterator<Item = Event<'a>>) -> Vec<Event<'a>> {
    let mut kept = Vec::new();
    let mut unwrapped: Vec<TagEnd> = Vec::new();
    let mut skipping: Option<(TagEnd, usize)> = None;
    for event in events {
        if let Some((end, depth)) = skipping.as_mut() {
            match &event {
                Event::Start(tag) if tag.to_end() == *end => *depth += 1,
                Event::End(e) if e == end => {
                    if *depth == 0 {
                        skipping = None;
                    } else {
                        *depth -= 1;
                    }
                }
                _ => {}
            }
            continue;
        }
        match classify(&event) {
            Ok(()) => match &event {
                Event::End(end) if unwrapped.last() == Some(end) => {
                    unwrapped.pop();
                }
                _ => kept.push(event),
            },
            Err((_, Drop::Event)) => {}
            Err((_, Drop::Unwrap)) => {
                if let Event::Start(tag) = &event {
                    unwrapped.push(tag.to_end());
                }
            }
            Err((_, Drop::Subtree)) => {
                if let Event::Start(tag) = &event {
                    skipping = Some((tag.to_end(), 0));
                }
            }
        }
    }
    kept
}

/// Unknown variables, lengths and Markdown outside the whitelist. Blank copy is a
/// per-entry rule the store applies (only the entity-locale entry is required).
pub fn validate(subject: &str, body_md: &str) -> Result<(), CopyError> {
    let mut unknown = unknown_variables(subject);
    for name in unknown_variables(body_md) {
        if !unknown.contains(&name) {
            unknown.push(name);
        }
    }
    if !unknown.is_empty() {
        return Err(CopyError::UnknownVariables(unknown));
    }
    if subject.chars().count() > SUBJECT_MAX_CHARS {
        return Err(CopyError::SubjectTooLong);
    }
    if body_md.chars().count() > BODY_MAX_CHARS {
        return Err(CopyError::BodyTooLong);
    }
    if let Some(construct) = disallowed_markdown(body_md) {
        return Err(CopyError::DisallowedMarkdown(construct));
    }
    Ok(())
}

/// Restricted Markdown → HTML, then `{{var}}` → HTML-escaped value.
pub fn to_html(body_md: &str, vars: &Vars) -> String {
    let mut html = String::with_capacity(body_md.len() * 2);
    pulldown_cmark::html::push_html(&mut html, whitelist(parser(body_md)).into_iter());
    replace_vars(&html, |name| {
        html_escape::encode_text(vars.value(name)).into_owned()
    })
}

/// Plain-text substitution for subjects: no Markdown, no escaping.
pub fn substitute(subject: &str, vars: &Vars) -> String {
    replace_vars(subject, |name| vars.value(name).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vars {
        Vars::new()
            .with("customer_name", "Acme")
            .with("invoice_number", "INV-1")
            .with("amount_due", "€1,200.00")
            .with("due_date", "March 31, 2026")
            .with("days_overdue", "7")
    }

    #[test]
    fn nine_variables_with_examples() {
        assert_eq!(VARIABLES.len(), 9);
        for v in VARIABLES {
            assert!(!v.example.is_empty(), "{}", v.name);
        }
        assert_eq!(Vars::examples().0.len(), 9);
        assert_eq!(variable("plan_name").unwrap().kind, VariableKind::Text);
        assert_eq!(variable("days_overdue").unwrap().kind, VariableKind::Int);
    }

    #[test]
    fn rejects_unknown_variables() {
        assert_eq!(
            unknown_variables("Hi {{customer_name}}, {{ nope }} and {{other_one}} {{nope}}"),
            vec!["nope", "other_one"]
        );
        assert!(
            unknown_variables("{{Bad}} {{ not-a-var }} {{}} {{ 42 }} {{invoice_number").is_empty()
        );
        assert_eq!(
            validate(
                "Invoice {{invoice_nr}}",
                "Hi {{customer_name}} {{planname}}"
            ),
            Err(CopyError::UnknownVariables(vec![
                "invoice_nr".into(),
                "planname".into()
            ]))
        );
        assert_eq!(
            validate("Invoice {{invoice_number}}", "Hi {{customer_name}}"),
            Ok(())
        );
    }

    #[test]
    fn enforces_lengths() {
        assert_eq!(
            validate(&"é".repeat(201), "body"),
            Err(CopyError::SubjectTooLong)
        );
        assert_eq!(validate(&"é".repeat(200), "body"), Ok(()));
        assert_eq!(
            validate("s", &"é".repeat(5_001)),
            Err(CopyError::BodyTooLong)
        );
    }

    #[test]
    fn validate_rejects_markdown_outside_the_whitelist() {
        for (body, construct) in [
            ("# Title", "heading"),
            ("see [here](https://x.example)", "link"),
            ("![alt](https://x.example/i.png)", "image"),
            ("<script>alert(1)</script>", "html"),
            ("hi <b>there</b>", "html"),
            ("```\ncode\n```", "code block"),
            ("    indented code", "code block"),
            ("use `x`", "inline code"),
            ("a\n\n---\n\nb", "horizontal rule"),
        ] {
            assert_eq!(
                validate("s", body),
                Err(CopyError::DisallowedMarkdown(construct)),
                "{body}"
            );
        }
        assert_eq!(
            validate(
                "s",
                "Hi **{{customer_name}}**,\n\n- one\n- *two*\n\n1. first\n2. second\n\n> quoted\n\nline  \nbreak"
            ),
            Ok(())
        );
    }

    #[test]
    fn markdown_whitelist_drops_html_images_headings_links() {
        let html = to_html(
            "# Heading text\n\n<script>alert(1)</script>\n\nSee [the portal](https://x.example) and ![logo](https://x.example/l.png) <b>bold</b>\n\n- one\n- two\n\n1. first\n\n> quote\n\n---\n\nend `code`",
            &vars(),
        );
        assert!(!html.contains("<h1"), "{html}");
        assert!(html.contains("Heading text"), "{html}");
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("alert(1)"), "{html}");
        assert!(!html.contains("<a "), "{html}");
        assert!(!html.contains("x.example"), "{html}");
        assert!(html.contains("the portal"), "{html}");
        assert!(!html.contains("<img"), "{html}");
        assert!(!html.contains("logo"), "{html}");
        assert!(!html.contains("<b>"), "{html}");
        assert!(html.contains("bold"), "{html}");
        assert!(
            html.contains("<ul>\n<li>one</li>\n<li>two</li>\n</ul>"),
            "{html}"
        );
        assert!(html.contains("<ol>\n<li>first</li>\n</ol>"), "{html}");
        assert!(html.contains("<blockquote>"), "{html}");
        assert!(!html.contains("<hr"), "{html}");
        assert!(!html.contains("<code"), "{html}");
        assert!(html.contains("<p>end </p>"), "{html}");
    }

    #[test]
    fn values_are_escaped_after_markdown() {
        let v = vars().with("customer_name", "<b>");
        let html = to_html(
            "Hi **{{customer_name}}**, invoice {{ invoice_number }} of {{amount_due}}",
            &v,
        );
        assert_eq!(
            html,
            "<p>Hi <strong>&lt;b&gt;</strong>, invoice INV-1 of €1,200.00</p>\n"
        );
        let v = vars().with("invoice_number", "a & b <c>");
        assert_eq!(
            to_html("{{invoice_number}}", &v),
            "<p>a &amp; b &lt;c&gt;</p>\n"
        );
    }

    #[test]
    fn markdown_special_characters_in_values_are_inert() {
        let v = vars().with("customer_name", "*Acme* [x](https://evil.example) # <i>");
        let html = to_html("Hi {{customer_name}}", &v);
        assert!(
            !html.contains("<em>") && !html.contains("<a ") && !html.contains("<i>"),
            "{html}"
        );
        assert!(
            html.contains("*Acme* [x](https://evil.example) # &lt;i&gt;"),
            "{html}"
        );
    }

    #[test]
    fn unknown_and_missing_variables_at_render() {
        assert_eq!(
            to_html("{{nope}} {{plan_name}}!", &vars()),
            "<p>{{nope}} !</p>\n"
        );
        assert_eq!(substitute("{{nope}} {{plan_name}}!", &vars()), "{{nope}} !");
    }

    #[test]
    fn substitute_is_plain_text() {
        let v = vars().with("customer_name", "<b>");
        assert_eq!(
            substitute(
                "Invoice {{invoice_number}} for {{customer_name}} is {{days_overdue}} days overdue",
                &v
            ),
            "Invoice INV-1 for <b> is 7 days overdue"
        );
    }

    #[test]
    fn seeded_default_copy_renders_paragraphs() {
        let html = to_html(
            "Hi {{customer_name}},\n\nA quick heads-up: invoice {{invoice_number}} for {{amount_due}} is due on {{due_date}}.",
            &vars(),
        );
        assert_eq!(
            html,
            "<p>Hi Acme,</p>\n<p>A quick heads-up: invoice INV-1 for €1,200.00 is due on March 31, 2026.</p>\n"
        );
    }
}
