use utoipa::openapi::path::Operation;
use utoipa::openapi::{OpenApi, OpenApiBuilder};

/// What an edition's modules add to the REST specs.
pub struct EditionSpec {
    /// Operations of the public API, under the edition's license when it has one.
    pub api: OpenApi,
    /// Operations of the client API, under the same license.
    pub client: OpenApi,
    /// Operations of the public API v2, under the same license.
    pub api_v2: OpenApi,
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
            api_v2: OpenApiBuilder::new().build(),
            webhook_events: Vec::new(),
        }
    }
}

/// The edition that serves an operation, published on it as `x-meteroid-edition` so one
/// documentation can show every edition's operations, each marked. Core's operations are served
/// by every edition; a module's by the editions shipping it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationEdition {
    /// Core: the open-source edition and every edition built on it.
    Oss,
    /// An Enterprise module's.
    Enterprise,
    /// Served by Meteroid Cloud only (the `cloud` module, acting on a cloud license).
    Cloud,
}

impl OperationEdition {
    /// The OpenAPI extension carrying it.
    pub const EXTENSION: &'static str = "x-meteroid-edition";

    pub fn as_str(self) -> &'static str {
        match self {
            OperationEdition::Oss => "oss",
            OperationEdition::Enterprise => "enterprise",
            OperationEdition::Cloud => "cloud",
        }
    }

    /// The first line of the description of an operation the edition alone serves, since
    /// documentation renderers ignore extensions. None for core's, served everywhere.
    pub fn notice(self) -> Option<&'static str> {
        match self {
            OperationEdition::Oss => None,
            OperationEdition::Enterprise => Some("Enterprise edition."),
            OperationEdition::Cloud => Some("Meteroid Cloud only."),
        }
    }
}

fn operations_mut(openapi: &mut OpenApi) -> impl Iterator<Item = &mut Operation> {
    openapi.paths.paths.values_mut().flat_map(|item| {
        [
            item.get.as_mut(),
            item.put.as_mut(),
            item.post.as_mut(),
            item.delete.as_mut(),
            item.options.as_mut(),
            item.head.as_mut(),
            item.patch.as_mut(),
            item.trace.as_mut(),
        ]
        .into_iter()
        .flatten()
    })
}

/// The edition an operation is marked with, if any.
pub fn operation_edition(operation: &Operation) -> Option<&str> {
    operation
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get(OperationEdition::EXTENSION))
        .and_then(serde_json::Value::as_str)
}

/// Marks every operation of `openapi` not marked yet as served by `edition`: the
/// `x-meteroid-edition` extension, and the edition's notice as the first line of its
/// description. A module's document is marked before it is merged into core's
/// (`Modules::spec`), then core's operations, the ones left unmarked, as `oss`: the order of the
/// merge never matters and marking twice changes nothing.
pub fn mark_operations(openapi: &mut OpenApi, edition: OperationEdition) {
    for operation in operations_mut(openapi) {
        if operation_edition(operation).is_some() {
            continue;
        }
        operation
            .extensions
            .get_or_insert_with(Default::default)
            .insert(
                OperationEdition::EXTENSION.to_owned(),
                serde_json::Value::String(edition.as_str().to_owned()),
            );
        if let Some(notice) = edition.notice() {
            operation.description = Some(match operation.description.take() {
                Some(description) if !description.trim().is_empty() => {
                    format!("{notice}\n\n{description}")
                }
                _ => notice.to_owned(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use utoipa::openapi::path::{HttpMethod, OperationBuilder, PathItem, PathsBuilder};

    fn doc(path: &str, description: Option<&str>) -> OpenApi {
        let operation = OperationBuilder::new().description(description).build();
        OpenApiBuilder::new()
            .paths(PathsBuilder::new().path(path, PathItem::new(HttpMethod::Get, operation)))
            .build()
    }

    fn marks<'a>(openapi: &'a OpenApi, path: &str) -> (Option<&'a str>, Option<&'a str>) {
        let operation = openapi.paths.paths[path].get.as_ref().unwrap();
        (
            operation_edition(operation),
            operation.description.as_deref(),
        )
    }

    #[test]
    fn a_module_operation_keeps_its_edition_once_merged_into_core() {
        let mut module = doc("/budgets", Some("Spending limits."));
        mark_operations(&mut module, OperationEdition::Enterprise);
        let mut core = doc("/customers", Some("Who is billed."));
        core.merge(module);
        mark_operations(&mut core, OperationEdition::Oss);
        mark_operations(&mut core, OperationEdition::Oss);

        assert_eq!(
            marks(&core, "/budgets"),
            (
                Some("enterprise"),
                Some("Enterprise edition.\n\nSpending limits.")
            )
        );
        assert_eq!(
            marks(&core, "/customers"),
            (Some("oss"), Some("Who is billed."))
        );
    }

    #[test]
    fn an_operation_without_description_gets_the_notice_alone() {
        let mut cloud = doc("/admin", None);
        mark_operations(&mut cloud, OperationEdition::Cloud);
        assert_eq!(
            marks(&cloud, "/admin"),
            (Some("cloud"), Some("Meteroid Cloud only."))
        );
    }
}
