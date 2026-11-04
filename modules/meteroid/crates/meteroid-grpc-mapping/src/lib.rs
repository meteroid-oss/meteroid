//! The proto↔domain mappings and gRPC error types of the core services, and the gRPC helpers
//! they use, shared by the gRPC API and the modules.

pub mod activity;
pub mod connectors;
pub mod customers;
pub mod customproperties;
pub mod errors;
pub mod filters;
pub mod invoices;
pub mod invoicingentities;
pub mod organizations;
pub mod rbac;
pub mod shared;
pub mod tenants;
pub mod utils;
pub mod wallets;
