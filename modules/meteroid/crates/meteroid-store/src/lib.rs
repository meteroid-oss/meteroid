pub mod compute;
pub mod crypt;
pub mod domain;
pub mod errors;
pub mod repositories;
pub mod store;
pub mod utils;

pub use store::Store;

pub type StoreResult<T> = error_stack::Result<T, errors::StoreError>;
