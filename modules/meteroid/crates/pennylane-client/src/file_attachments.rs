use crate::client::PennylaneClient;
use crate::error::PennylaneError;
use governor::Jitter;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::time::Duration;

#[async_trait::async_trait]
pub trait FileAttachmentsApi {
    async fn create_attachment(
        &self,
        attachment: NewAttachment,
        access_token: &SecretString,
    ) -> Result<Attachment, PennylaneError>;
}

#[async_trait::async_trait]
impl FileAttachmentsApi for PennylaneClient {
    /// <https://pennylane.readme.io/v2.0/reference/postfileattachments>
    async fn create_attachment(
        &self,
        attachment: NewAttachment,
        access_token: &SecretString,
    ) -> Result<Attachment, PennylaneError> {
        let stream = reqwest::Body::from(attachment.file);

        let part = reqwest::multipart::Part::stream(stream)
            .file_name(attachment.filename)
            .mime_str(attachment.media_type.as_str())?;

        let form = reqwest::multipart::Form::new().part("file", part);

        let url = self
            .api_base
            .join("/api/external/v2/file_attachments")
            .expect("invalid path");

        self.rate_limiter
            .until_key_ready_with_jitter(
                &access_token.expose_secret().to_string(),
                Jitter::up_to(Duration::from_secs(1)),
            )
            .await;

        let request = self
            // multipart does not work with middleware
            .raw_client
            .post(url)
            .bearer_auth(access_token.expose_secret())
            .multipart(form);

        let response = request.send().await.map_err(PennylaneError::from)?;
        let status_code = &response.status();

        if !status_code.is_success() {
            return Err(PennylaneError::ClientError {
                error: response.text().await.unwrap_or_default(),
                status_code: Some(status_code.as_u16()),
            });
        }

        response.json().await.map_err(PennylaneError::from)
    }
}

#[derive(Debug)]
pub struct NewAttachment {
    pub filename: String,
    pub file: bytes::Bytes,
    pub media_type: MediaType,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Attachment {
    pub id: i64,
}

#[derive(Debug)]
pub enum MediaType {
    ApplicationPdf,
}

impl MediaType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MediaType::ApplicationPdf => "application/pdf",
        }
    }
}
