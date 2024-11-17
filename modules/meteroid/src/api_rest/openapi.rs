use crate::api_rest::server::ApiDoc;
use utoipa::OpenApi;

pub fn generate_spec() {
    let path = "spec/api/v1/openapi.json";

    println!("Generating OpenAPI spec {:?}", path);

    let open_api = ApiDoc::openapi();

    std::fs::write(path, open_api.clone().to_pretty_json().unwrap())
        .expect("Unable to write openapi.json file");
}
