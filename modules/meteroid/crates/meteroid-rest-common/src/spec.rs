use utoipa::openapi::{OpenApi, OpenApiBuilder};

/// What an edition's modules add to the REST specs.
pub struct EditionSpec {
    /// Operations of the public API, under the edition's license when it has one.
    pub api: OpenApi,
    /// Operations of the client API, under the same license.
    pub client: OpenApi,
    /// The webhook events, among those core publishes only when a module emits them, that the
    /// modules emit.
    pub webhook_events: Vec<&'static str>,
}

impl EditionSpec {
    /// No module.
    pub fn core() -> Self {
        Self {
            api: OpenApiBuilder::new().build(),
            client: OpenApiBuilder::new().build(),
            webhook_events: Vec::new(),
        }
    }
}
