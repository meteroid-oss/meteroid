fn main() {
    let spec_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../spec");
    meteroid::api_rest::openapi::generate_spec(meteroid_edition_oss::modules().spec(), spec_dir);
}
