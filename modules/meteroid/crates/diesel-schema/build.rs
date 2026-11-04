use std::fmt::Write;
use std::{env, fs, path::Path};

const SCHEMA: &str = "src/schema.rs";
const BASE: usize = 16;

fn table_names(schema: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut in_table = false;
    for line in schema.lines() {
        if line == "diesel::table! {" {
            in_table = true;
        } else if in_table
            && let Some((name, _)) = line.strip_prefix("    ").and_then(|l| l.split_once(" ("))
        {
            assert!(
                name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "unexpected table header in {SCHEMA}: {line}"
            );
            names.push(name);
            in_table = false;
        }
    }
    names
}

fn id_type(index: usize, digits: u32) -> String {
    (0..digits).fold("Nil".to_owned(), |tail, position| {
        format!("Digit<D{}, {tail}>", index / BASE.pow(position) % BASE)
    })
}

fn main() {
    println!("cargo::rerun-if-changed={SCHEMA}");
    let schema = fs::read_to_string(SCHEMA).expect("read schema.rs");
    let names = table_names(&schema);
    assert!(!names.is_empty(), "no table found in {SCHEMA}");
    let digits = names.len().max(2).ilog(BASE) + 1;

    let all: Vec<_> = (0..BASE).map(|d| format!("D{d}")).collect();
    let pairs: Vec<_> = (0..BASE)
        .flat_map(|l| {
            (0..BASE)
                .filter(move |&r| r != l)
                .map(move |r| format!("D{l} D{r}"))
        })
        .collect();
    let mut out = format!(
        "digits! {{ {} }}\ndigits_differ! {{ {} }}\n",
        all.join(" "),
        pairs.join(", ")
    );
    out.push_str("table_ids! {\n");
    for (index, name) in names.iter().enumerate() {
        writeln!(out, "    {name} => {};", id_type(index, digits)).unwrap();
    }
    out.push_str("}\n");
    let dest = Path::new(&env::var("OUT_DIR").unwrap()).join("table_ids.rs");
    fs::write(dest, out).expect("write table_ids.rs");
}
