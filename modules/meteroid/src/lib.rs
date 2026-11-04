//! The composition of an edition: runs the API, the workers or both with the edition's modules.

pub mod run;
pub mod svix_import;

pub mod api_rest {
    pub use meteroid_rest::*;

    pub mod server;
}

pub use meteroid_api as api;
pub use meteroid_app::services;
pub use meteroid_app::{
    bootstrap, clients, config, constants, encoding, errors, mapping, migrations,
};
pub use meteroid_grpc_mapping::parse_uuid;
pub use meteroid_kit as module;
pub use meteroid_kit::{eventbus, process};
pub use meteroid_workers as workers;
