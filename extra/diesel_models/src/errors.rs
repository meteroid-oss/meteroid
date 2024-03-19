use diesel::result::Error as DieselError;
use error_stack::{IntoReport, Report, ResultExt};

pub type DatabaseResult<T> = error_stack::Result<T, DatabaseError>;

#[derive(Copy, Clone, Debug, thiserror::Error)]
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
    #[error("An unknown error occurred")]
    Others,
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
            _ => Self::Others,
        }
    }
}

pub trait IntoDbResult: Sized {
    type Ok;
    fn into_db_result(self) -> DatabaseResult<Self::Ok>;
}

impl<T> IntoDbResult for Result<T, DieselError> {
    type Ok = T;

    fn into_db_result(self) -> DatabaseResult<T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => {
                let db_err = DatabaseError::from(&err);
                Err(Report::from(err)).change_context(db_err)
            }
        }
    }
}
