//! Finds raw-event reads that bypass `EventsSource` (ADR-0012 §6). Shared by the test of each
//! edition, which names its own source roots.
use std::fs;
use std::path::{Path, PathBuf};

const TOKENS: &[&str] = &[
    "raw_events_v2",
    "raw_events_local",
    "raw_events_table",
    "RAW_EVENTS_LOCAL_TABLE",
    ".unfiltered()",
];

/// Table settings, first-ingestion reads, tenant purges, the consumer's insert and change lookup,
/// and the copy of pre-1.0 events.
const ALLOWED: &[(&str, &str)] = &[
    ("metric-core/src/config.rs", "ClickhouseConfig"),
    ("metering/src/config.rs", "EventTablesConfig"),
    ("metering/src/connectors/clickhouse/mod.rs", "purge_tenants"),
    (
        "metering/src/connectors/clickhouse/mod.rs",
        "list_tenant_first_events",
    ),
    (
        "metering/src/connectors/clickhouse/mod.rs",
        "list_customer_first_events",
    ),
    ("metering/src/connectors/clickhouse/event_rows.rs", "lookup"),
    (
        "metering/src/connectors/clickhouse/event_rows.rs",
        "check_access",
    ),
    ("metering/src/ingest/consumer.rs", "insert_quorum"),
    ("metering/src/ingest/consumer.rs", "insert_rows"),
    ("metering/src/migrations/legacy.rs", "copy_base62_events"),
    (
        "meteroid/crates/meteroid-testkit/src/metering_it/config.rs",
        "mocked_config",
    ),
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.map(Result::unwrap) {
        let path = entry.path();
        let skipped = ["tests", "target"].iter().any(|d| path.ends_with(d));
        if path.is_dir() && !skipped {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The item a line belongs to: the last `fn`, or top-level `const` or `struct`, declared above it.
fn item_name(line: &str) -> Option<&str> {
    let top_level = !line.starts_with(char::is_whitespace);
    let words: Vec<&str> = line.split_whitespace().collect();
    let at = words
        .iter()
        .position(|w| *w == "fn" || (top_level && matches!(*w, "const" | "struct")))?;
    let name = words.get(at + 1)?;
    let end = name
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(name.len());
    Some(&name[..end])
}

#[derive(Clone, Copy)]
enum Lexed {
    Code,
    Str,
    RawStr(usize),
    Block(u32),
    LineComment,
}

/// The source's lines without comments. String literals stay (they hold table names), and a `//`
/// inside one is not a comment.
fn code_lines(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let (mut lines, mut line, mut state, mut i) = (Vec::new(), String::new(), Lexed::Code, 0);
    while i < chars.len() {
        let (c, next) = (chars[i], at(i + 1));
        let mut step = 1;
        match state {
            _ if c == '\n' => {
                lines.push(std::mem::take(&mut line));
                if let Lexed::LineComment = state {
                    state = Lexed::Code;
                }
            }
            Lexed::Code if c == '/' && next == '/' => state = Lexed::LineComment,
            Lexed::Code if c == '/' && next == '*' => (state, step) = (Lexed::Block(1), 2),
            Lexed::Code => {
                let hashes = chars[i + 1..].iter().take_while(|h| **h == '#').count();
                let raw_start = c == 'r'
                    && at(i + 1 + hashes) == '"'
                    && (i == 0
                        || !ident(chars[i - 1])
                        || (chars[i - 1] == 'b' && (i < 2 || !ident(chars[i - 2]))));
                if raw_start {
                    (state, step) = (Lexed::RawStr(hashes), hashes + 2);
                } else if c == '"' {
                    state = Lexed::Str;
                } else if c == '\'' && next == '\\' {
                    let rest = chars.get(i + 3..).unwrap_or_default();
                    step = 4 + rest.iter().take_while(|q| **q != '\'').count();
                } else if c == '\'' && at(i + 2) == '\'' {
                    step = 3;
                }
                line.extend(&chars[i..(i + step).min(chars.len())]);
            }
            Lexed::Str => {
                if c == '\\' && next != '\n' {
                    step = 2;
                } else if c == '"' {
                    state = Lexed::Code;
                }
                line.extend(&chars[i..(i + step).min(chars.len())]);
            }
            Lexed::RawStr(hashes) => {
                if c == '"'
                    && chars[i + 1..]
                        .iter()
                        .take(hashes)
                        .filter(|h| **h == '#')
                        .count()
                        == hashes
                {
                    (state, step) = (Lexed::Code, hashes + 1);
                }
                line.extend(&chars[i..(i + step).min(chars.len())]);
            }
            Lexed::Block(depth) if c == '/' && next == '*' => {
                (state, step) = (Lexed::Block(depth + 1), 2)
            }
            Lexed::Block(depth) if c == '*' && next == '/' => {
                state = if depth == 1 {
                    Lexed::Code
                } else {
                    Lexed::Block(depth - 1)
                };
                step = 2;
            }
            Lexed::Block(_) | Lexed::LineComment => {}
        }
        i += step;
    }
    lines.push(line);
    lines
}

/// Lines outside `#[cfg(test)]` items that name the table, as `file:line: item`.
pub fn violations(modules: &Path, file: &Path) -> Vec<String> {
    let rel = file
        .strip_prefix(modules)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    let source = fs::read_to_string(file).unwrap();
    let (mut item, mut skip_depth, mut in_test) = (String::new(), 0i32, false);
    let mut found = Vec::new();
    for (n, (line, code)) in source.lines().zip(code_lines(&source)).enumerate() {
        let code = code.as_str();
        if code.trim_start().starts_with("#[cfg(test)]") {
            in_test = true;
            skip_depth = 0;
            continue;
        }
        if in_test {
            skip_depth += code.matches('{').count() as i32 - code.matches('}').count() as i32;
            in_test = skip_depth > 0 || !(code.contains('}') || code.trim_end().ends_with(';'));
            continue;
        }
        if let Some(name) = item_name(code) {
            item = name.to_string();
        }
        let names_table = TOKENS.iter().any(|t| code.contains(t)) || is_events_table(code);
        let constructs = code.contains("EventsTable::new(") && !code.contains(".unfiltered()");
        let allowed = constructs || ALLOWED.contains(&(rel.as_str(), item.as_str()));
        if names_table && !allowed {
            found.push(format!("{rel}:{}: {item}: {}", n + 1, line.trim()));
        }
    }
    found
}

/// `events_table` as a whole identifier: a table name held as a plain string.
fn is_events_table(code: &str) -> bool {
    code.match_indices("events_table").any(|(at, _)| {
        let before = code[..at].chars().next_back();
        let after = code[at + "events_table".len()..].chars().next();
        let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        !ident(before) && !ident(after)
    })
}

/// Every violation under `roots` (relative to `modules`), and how many files were scanned.
pub fn scan(modules: &Path, roots: &[&str]) -> (usize, Vec<String>) {
    let mut files = Vec::new();
    for root in roots {
        rust_files(&modules.join(root), &mut files);
    }
    let found = files.iter().flat_map(|f| violations(modules, f)).collect();
    (files.len(), found)
}
