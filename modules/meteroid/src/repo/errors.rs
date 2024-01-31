#[derive(Debug, thiserror::Error, PartialEq, Clone)]
pub enum RepoError {
    #[error("Database error")]
    DatabaseError,
    #[error("Json field decoding error")]
    JsonFieldDecodingError,
    #[error("Json field encoding error")]
    JsonFieldEncodingError,
}
