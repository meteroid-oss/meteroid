use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

pub fn main() {
    // let's generate statics for the countries.json
    generate_country_translation();
}

fn generate_country_translation() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("i18n_data.rs");
    let mut file = File::create(&dest_path).unwrap();

    let i18n_dir = Path::new("l10n");
    let mut locales = Vec::new();

    // Traverse the i18n folder and find all country.json files
    for entry in fs::read_dir(i18n_dir).unwrap() {
        let locale_dir = entry.unwrap().path();
        let locale = locale_dir
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        if locale_dir.is_dir() {
            let country_json_path = locale_dir.join("countries.json");

            if country_json_path.exists() {
                let country_data = fs::read_to_string(&country_json_path).unwrap();
                locales.push((locale, country_data));
            }
        }
    }

    // Generate Rust code to embed these files
    writeln!(file, "use once_cell::sync::Lazy;").unwrap();
    writeln!(file, "use std::collections::HashMap;").unwrap();

    // No need for the Country struct, we directly use HashMap<String, String>
    writeln!(file, "pub static LOCALES: Lazy<HashMap<&'static str, &mut HashMap<String, String>>> = Lazy::new(|| {{").unwrap();
    writeln!(file, "let mut map = HashMap::new();").unwrap();

    for (locale, data) in locales {
        writeln!(file, "map.insert(\"{locale}\", Box::leak(Box::new(serde_json::from_str::<HashMap<String, String>>(r#\"{data}\"#).unwrap())));").unwrap();
    }

    writeln!(file, "map").unwrap(); // End the HashMap definition
    writeln!(file, "}});").unwrap();
}
