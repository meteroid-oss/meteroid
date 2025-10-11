use crate::api_rest::server::ApiDoc;
use crate::api_rest::{AppState, api_routes};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

pub fn generate_spec() {
    let path = "spec/api/v1/openapi.json";

    println!("Generating OpenAPI spec {path:?}");

    let (_router, open_api) = OpenApiRouter::<AppState>::with_openapi(ApiDoc::openapi())
        .merge(api_routes())
        .split_for_parts();

    std::fs::write(path, open_api.clone().to_pretty_json().unwrap())
        .expect("Unable to write openapi.json file");
}
