//! ADR-0012 §6: raw-event readers reach the table only through `EventsSource`, which drops
//! tombstones and voided copies. Naming the table anywhere else fails here.
use std::fs;
use std::path::Path;

mod events_source;

const ROOTS: &[&str] = &[
    "metering/src",
    "metering/crates",
    "metric-core/src",
    "meteroid/src",
    "meteroid/crates",
];

#[test]
fn only_events_source_names_the_raw_events_table() {
    let modules = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let (files, found) = events_source::scan(&modules, ROOTS);
    assert!(files > 20, "scanned {files} files");
    assert!(
        found.is_empty(),
        "read raw events through EventsTable::source (ADR-0012 §6):\n{}",
        found.join("\n")
    );
}

#[test]
fn the_guard_sees_a_direct_read() {
    let dir = std::env::temp_dir().join(format!("events_source_only_{}", std::process::id()));
    fs::create_dir_all(dir.join("analytics/src")).unwrap();
    let file = dir.join("analytics/src/config.rs");
    fs::write(
        &file,
        r##"fn reader(events_table: &str) -> String {
    format!("FROM {events_table}")
}
#[cfg(test)]
mod tests {
    const T: &str = "raw_events_v2";
}
fn purge() -> &'static str { "raw_events_local_v2" }
fn one_liner() -> String { EventsTable::new("t").unfiltered().to_string() }
fn built() -> EventsTable { EventsTable::new("raw_events_v2") }
fn url() -> &'static str { "http://raw_events_v2" }
fn raw() -> &'static str { r#"// " raw_events_v2"# }
fn quote() -> char { '"' } // raw_events_v2
/* raw_events_v2 */ fn commented() {}
pub struct Settings { raw_events_table: String }
"##,
    )
    .unwrap();
    let found = events_source::violations(&dir, &file);
    fs::remove_dir_all(&dir).unwrap();
    let items: Vec<&str> = found
        .iter()
        .map(|f| f.split(": ").nth(1).unwrap())
        .collect();
    assert_eq!(
        items,
        [
            "reader",
            "reader",
            "purge",
            "one_liner",
            "url",
            "raw",
            "Settings"
        ],
        "{found:?}"
    );
}
