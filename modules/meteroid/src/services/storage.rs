use crate::errors::ObjectStoreError;
use async_trait::async_trait;
use bytes::Bytes;
use common_domain::ids::TenantId;
use error_stack::{Report, ResultExt};
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreScheme, PutPayload};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub enum Prefix {
    InvoicePdf,
    InvoiceXml,
    ImageLogo,
    WebhookArchive {
        provider_uid: String,
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
                provider_uid,
                tenant_id,
            } => format!("webhook_archive/{}/{}", provider_uid, tenant_id),
        }
    }
}

pub type Result<T> = error_stack::Result<T, ObjectStoreError>;

#[async_trait]
pub trait ObjectStoreService: Send + Sync {
    async fn store(&self, binary: Bytes, prefix: Prefix) -> Result<Uuid>;
    async fn retrieve(&self, uid: Uuid, prefix: Prefix) -> Result<Bytes>;
}

pub struct S3Storage {
    object_store_client: Arc<dyn ObjectStore>,
    path: Path,
}

impl S3Storage {
    pub fn try_new(url: &str, path_prefix: &Option<String>) -> Result<Self> {
        let url = url::Url::parse(url).change_context(ObjectStoreError::InvalidUrl)?;

        let (scheme, path) =
            ObjectStoreScheme::parse(&url).change_context(ObjectStoreError::InvalidUrl)?;

        let client: Box<dyn ObjectStore> = match scheme {
            ObjectStoreScheme::Local => Box::new(LocalFileSystem::new()),
            ObjectStoreScheme::Memory => Box::new(InMemory::new()),
            ObjectStoreScheme::AmazonS3 => Box::new(
                AmazonS3Builder::from_env()
                    .with_url(url.to_string())
                    .build()
                    .change_context(ObjectStoreError::InvalidUrl)?,
            ),
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
}

pub fn in_memory_object_store() -> Arc<dyn ObjectStoreService> {
    let in_mem_client = Arc::new(object_store::memory::InMemory::new());

    Arc::new(S3Storage {
        object_store_client: in_mem_client,
        path: Path::from(""),
    })
}
