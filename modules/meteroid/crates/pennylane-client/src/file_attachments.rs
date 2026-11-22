use crate::client::{PennylaneClient, Retry};
use crate::error::PennylaneError;
use bytes::Bytes;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct FileAttachment {
    pub id: i64,
}

impl PennylaneClient {
    /// <https://pennylane.readme.io/reference/postfileattachments>. Sending it twice only leaves
    /// an unused attachment behind, so it is retried like a read.
    pub async fn upload_pdf(
        &self,
        filename: &str,
        file: Bytes,
        access_token: &SecretString,
    ) -> Result<FileAttachment, PennylaneError> {
        self.post_multipart(
            "/api/external/v2/file_attachments",
            access_token,
            Retry::Transient,
            || {
                let part = reqwest::multipart::Part::stream(reqwest::Body::from(file.clone()))
                    .file_name(filename.to_owned())
                    .mime_str("application/pdf")
                    .expect("static mime type");
                reqwest::multipart::Form::new().part("file", part)
            },
        )
        .await
    }
}
