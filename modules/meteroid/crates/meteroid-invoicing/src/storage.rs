use crate::errors::{InvoicingError, InvoicingResult};
use async_trait::async_trait;
use bytes::Bytes;
use object_store::path::Path;
use object_store::{ObjectStore, PutPayload};
use reqwest::Client;
use std::sync::Arc;

#[async_trait]
pub trait Storage: Send + Sync {
    async fn store_pdf(&self, binary: Vec<u8>, prefix: Option<String>) -> InvoicingResult<String>;
}

pub struct S3Storage {
    object_store_client: Arc<dyn ObjectStore>,
    path: Path,
}

impl S3Storage {
    pub fn create(url: String, path_prefix: Option<String>) -> InvoicingResult<Self> {
        let url = url::Url::parse(&url)
            .map_err(|_| InvoicingError::StorageError("Failed to parse storage URL".to_string()))?;

        let (client, path) = object_store::parse_url(&url).map_err(|_| {
            InvoicingError::StorageError("Failed to parse storage URL to object store".to_string())
        })?;

        let path = match path_prefix {
            Some(prefix) => path.child(prefix.as_str()),
            None => path,
        };

        Ok(S3Storage {
            object_store_client: Arc::new(client),
            path,
        })
    }
}

#[async_trait]
impl Storage for S3Storage {
    async fn store_pdf(
        &self,
        binary: Vec<u8>,
        file_prefix: Option<String>,
    ) -> InvoicingResult<String> {
        let payload = PutPayload::from_bytes(Bytes::from(binary));

        let unique_id = uuid::Uuid::new_v4();
        let path = match file_prefix {
            Some(prefix) => self.path.child(format!("{}-{}.pdf", prefix, unique_id)),
            None => self.path.child(format!("{}.pdf", unique_id)),
        };

        // multipart is efficient from parts >5MB, let's stay on single part for now
        let result = self
            .object_store_client
            .put(&path, payload)
            .await
            .map_err(|_| InvoicingError::StorageError("Failed to store PDF".to_string()))?;

        // result.e_tag ?
        println!("Stored PDF at {}", result.e_tag.unwrap_or_default());

        Ok(path.to_string())
    }
}
