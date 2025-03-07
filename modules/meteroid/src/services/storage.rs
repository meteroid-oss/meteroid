use crate::errors::ObjectStoreError;
use async_trait::async_trait;
use bytes::Bytes;
use common_domain::ids::TenantId;
use error_stack::{Report, ResultExt};
use http::Method;
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::signer::Signer;
use object_store::{ObjectStore, ObjectStoreScheme, PutPayload};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

#[derive(Clone)]
pub enum Prefix {
    InvoicePdf,
    InvoiceXml,
    ImageLogo,
    WebhookArchive {
        connection_alias: String,
        tenant_id: TenantId,
    },
}

impl Prefix {
    pub fn to_path_string(&self) -> String {
        match self {
            Prefix::InvoicePdf => "invoice_pdf".to_string(),
            Prefix::InvoiceXml => "invoice_xml".to_string(),
            Prefix::ImageLogo => "image_logo".to_string(),
            Prefix::WebhookArchive {
                connection_alias,
                tenant_id,
            } => format!("webhook_archive/{}/{}", tenant_id, connection_alias),
        }
    }
}

pub type Result<T> = error_stack::Result<T, ObjectStoreError>;

#[async_trait]
pub trait ObjectStoreService: Send + Sync {
    async fn store(&self, binary: Bytes, prefix: Prefix) -> Result<Uuid>;
    async fn retrieve(&self, uid: Uuid, prefix: Prefix) -> Result<Bytes>;
    async fn get_url(
        &self,
        uid: Uuid,
        prefix: Prefix,
        expires_in: Duration,
    ) -> Result<Option<String>>;
}

pub struct S3Storage {
    object_store_client: Arc<dyn ObjectStore>,
    path: Path,
    signer: Option<Arc<dyn Signer>>,
}

impl S3Storage {
    pub fn try_new(url: &str, path_prefix: &Option<String>) -> Result<Self> {
        let url = url::Url::parse(url).change_context(ObjectStoreError::InvalidUrl)?;

        let (scheme, path) =
            ObjectStoreScheme::parse(&url).change_context(ObjectStoreError::InvalidUrl)?;

        let (client, signer): (Arc<dyn ObjectStore>, Option<Arc<dyn Signer>>) = match scheme {
            ObjectStoreScheme::Local => (Arc::new(LocalFileSystem::new()), None),
            ObjectStoreScheme::Memory => (Arc::new(InMemory::new()), None),
            ObjectStoreScheme::AmazonS3 => {
                let service = Arc::new(
                    AmazonS3Builder::from_env()
                        .with_url(url.to_string())
                        .build()
                        .change_context(ObjectStoreError::InvalidUrl)?,
                );

                (service.clone(), Some(service))
            }
            _ => {
                return Err(Report::new(ObjectStoreError::UnsupportedStore(
                    "Please request support for this object store protocol.".to_string(),
                )));
            }
        };

        let path = match path_prefix {
            Some(prefix) => path.child(prefix.as_str()),
            None => path,
        };

        Ok(S3Storage {
            object_store_client: Arc::new(client),
            path,
            signer,
        })
    }
}

#[async_trait]
impl ObjectStoreService for S3Storage {
    async fn store(&self, binary: Bytes, document_type: Prefix) -> Result<Uuid> {
        let payload = PutPayload::from_bytes(binary);

        let uid = Uuid::now_v7();

        let path = self
            .path
            .child(document_type.to_path_string().as_str())
            .child(uid.to_string().as_str());

        self.object_store_client
            .put(&path, payload)
            .await
            .change_context(ObjectStoreError::SaveError)?;

        Ok(uid)
    }

    async fn retrieve(&self, uid: Uuid, document_type: Prefix) -> Result<Bytes> {
        let path = self
            .path
            .child(document_type.to_path_string().as_str())
            .child(uid.to_string().as_str());

        let data = self
            .object_store_client
            .get(&path)
            .await
            .change_context(ObjectStoreError::LoadError)?
            .bytes()
            .await
            .change_context(ObjectStoreError::LoadError)?;

        Ok(data)
    }
    async fn get_url(
        &self,
        uid: Uuid,
        prefix: Prefix,
        expires_in: Duration,
    ) -> Result<Option<String>> {
        let path = self
            .path
            .child(prefix.to_path_string().as_str())
            .child(uid.to_string().as_str());

        // Only some backends supports presigned URLs
        if let Some(s3_client) = self.signer.clone() {
            let url = s3_client
                .signed_url(Method::GET, &path, expires_in)
                .await
                .change_context(ObjectStoreError::SaveError)?;

            Ok(Some(url.to_string()))
        } else {
            Ok(None)
        }
    }
}

pub fn in_memory_object_store() -> Arc<dyn ObjectStoreService> {
    let in_mem_client = Arc::new(object_store::memory::InMemory::new());

    Arc::new(S3Storage {
        object_store_client: in_mem_client,
        path: Path::from(""),
        signer: None,
    })
}
