#[derive(Debug, thiserror::Error, PartialEq, Clone)]
pub enum DatabaseError {
    #[error("Json parsing error : {0}")]
    JsonParsingError(String),
}
