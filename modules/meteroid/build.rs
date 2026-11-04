fn main() {
    println!("cargo:rerun-if-changed=migrations/diesel");
    println!("cargo:rerun-if-changed=build.rs");
    // The REST layer's timestamps are `ApiDateTime`, documented as the `date-time` they serialize to.
    utoipa_config::Config::new()
        .alias_for("ApiDateTime", "chrono::DateTime<chrono::Utc>")
        .write_to_file();
}
