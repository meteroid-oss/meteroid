#[derive(Debug, thiserror::Error)]
#[allow(clippy::enum_variant_names)]
pub enum PgmqError {
    #[error("HandleMessagesError")]
    HandleMessages,
    #[error("ReadMessagesError")]
    ReadMessages,
    #[error("DeleteMessagesError")]
    DeleteMessages,
    #[error("ArchiveMessagesError")]
    ArchiveMessages,
    #[error("SerdeError")]
    Serde(#[from] serde_json::Error),
    #[error("EmptyMessage")]
    EmptyMessage,
    #[error("EmptyHeaders")]
    EmptyHeaders,
    #[error("ListArchived")]
    ListArchived,
}
