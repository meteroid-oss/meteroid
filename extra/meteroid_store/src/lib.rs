mod domain;
mod errors;
mod repositories;
mod store;

pub type StoreResult<T> = error_stack::Result<T, errors::StoreError>;
