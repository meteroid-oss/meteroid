use thiserror::Error;

#[derive(Debug, Error)]
pub enum FangExtError {
    #[error("Database connection error")]
    DatabaseConnection,
    #[error("Database query error")]
    DatabaseQuery,
}
