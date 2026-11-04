use error_stack::Report;

pub mod error;
pub mod outbox;
pub mod processor;

pub type PgmqResult<T> = Result<T, Report<error::PgmqError>>;
