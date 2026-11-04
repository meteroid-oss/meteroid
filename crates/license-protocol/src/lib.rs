//! The wire protocol between a Meteroid license issuer and the installations it licenses:
//! endpoint paths, request and response types, and the signatures that bind them. Core reads
//! it where it shows licenses issued to customers; the issuer and the instance client are EE.

mod error;
pub mod protocol;

pub use error::LicenseError;
