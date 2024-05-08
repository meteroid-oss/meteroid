use crate::DbResult;
use diesel::result::Error as DieselError;
use error_stack::Report;

pub struct DatabaseErrorContainer {
    pub error: error_stack::Report<DatabaseError>,
}

impl From<Report<DatabaseError>> for DatabaseErrorContainer {
    fn from(error: Report<DatabaseError>) -> Self {
        Self { error }
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("An error occurred when obtaining database connection")]
    DatabaseConnectionError,
    #[error("The requested resource was not found in the database")]
    NotFound,
    #[error("A unique constraint violation occurred")]
    UniqueViolation,
    #[error("No fields were provided to be updated")]
    NoFieldsToUpdate,
    #[error("An error occurred when generating typed SQL query")]
    QueryGenerationFailed,
    // InsertFailed,
    #[error("An unknown error occurred: {0}")]
    Others(String),
}

impl From<&diesel::result::Error> for DatabaseError {
    fn from(error: &diesel::result::Error) -> Self {
        match error {
            diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                _,
            ) => Self::UniqueViolation,
            diesel::result::Error::NotFound => Self::NotFound,
            diesel::result::Error::QueryBuilderError(_) => Self::QueryGenerationFailed,
            err => Self::Others(err.to_string()),
        }
    }
}

impl From<diesel::result::Error> for DatabaseErrorContainer {
    fn from(error: diesel::result::Error) -> Self {
        let db_error = DatabaseError::from(&error);
        DatabaseErrorContainer {
            error: Report::from(error).change_context(db_error),
        }
    }
}

pub trait IntoDbResult: Sized {
    type Ok;
    fn into_db_result(self) -> DbResult<Self::Ok>;
}

impl<T> IntoDbResult for Result<T, DieselError> {
    type Ok = T;
    fn into_db_result(self) -> DbResult<T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => {
                let db_err = DatabaseError::from(&err);
                Err(DatabaseErrorContainer::from(
                    Report::from(err).change_context(db_err),
                ))
            }
        }
    }
}

impl<T> IntoDbResult for error_stack::Result<T, DieselError> {
    type Ok = T;
    fn into_db_result(self) -> DbResult<T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => {
                let db_err = DatabaseError::from(err.current_context());
                Err(DatabaseErrorContainer::from(err.change_context(db_err)))
            }
        }
    }
}

impl<E> From<DatabaseErrorContainer> for error_stack::Report<E>
where
    E: Send + Sync + std::error::Error + 'static,
    E: From<DatabaseError>,
{
    fn from(container: DatabaseErrorContainer) -> Self {
        let new_error: E = (container.error.current_context().clone()).into();
        container.error.change_context(new_error)
    }
}
