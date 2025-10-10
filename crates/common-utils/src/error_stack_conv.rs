use error_stack::Report;
use thiserror::Error as ThisError;

#[derive(ThisError, Debug)]
pub struct AnyhowError {
    #[source]
    source: anyhow::Error,
}

impl std::fmt::Display for AnyhowError {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.write_str(format!("{:?}", self.source).as_str())
    }
}

pub trait AnyhowIntoReport: Sized {
    type Ok;
    type Err;
    fn into_report(self) -> Result<Self::Ok, Report<Self::Err>>;
}

impl<T> AnyhowIntoReport for anyhow::Result<T> {
    type Ok = T;
    type Err = AnyhowError;

    #[track_caller]
    fn into_report(self) -> Result<T, Report<AnyhowError>> {
        match self {
            Ok(value) => Ok(value),
            Err(error) => Err(Report::from(AnyhowError { source: error })),
        }
    }
}
