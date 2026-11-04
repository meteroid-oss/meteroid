use crate::errors::ObjectStoreError;
use async_trait::async_trait;
use bytes::Bytes;
use common_domain::ids::{BaseId, StoredDocumentId, TenantId};
use error_stack::{Report, ResultExt};
use futures::stream::{StreamExt, TryStreamExt};
use http::Method;
use meteroid_store::Store;
use meteroid_store::domain::enums::StoredObjectKindEnum;
use meteroid_store::domain::stored_object::{StoredObject, StoredObjectNew, StoredObjectRef};
use meteroid_store::repositories::stored_object::StoredObjectInterface;
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::path::{Path, PathPart};
use object_store::signer::Signer;
use object_store::{ObjectStore, ObjectStoreExt, ObjectStoreScheme, PutPayload};
use std::sync::Arc;
use std::time::Duration;

/// The type of a stored object: the single source of truth for both its object-store path segment
/// and its registry discriminator, so a write names its kind exactly once.
#[derive(Clone)]
pub enum ObjectKind {
    InvoicePdf,
    CreditNotePdf,
    ReceiptPdf,
    InvoiceXml,
    ImageLogo,
    /// Keyed by connector id (by alias before connector-id webhook URLs).
    WebhookArchive {
        connector: String,
    },
    BatchJobInput,
    BatchJobErrorOutput,
    TaxReport,
}

impl ObjectKind {
    /// Tenant-free path segment(s) identifying the type; the key prepends `{tenant}/`.
    fn type_path(&self) -> String {
        match self {
            ObjectKind::InvoicePdf => "invoice_pdf".to_string(),
            ObjectKind::CreditNotePdf => "credit_note_pdf".to_string(),
            ObjectKind::ReceiptPdf => "receipt_pdf".to_string(),
            ObjectKind::InvoiceXml => "invoice_xml".to_string(),
            ObjectKind::ImageLogo => "image_logo".to_string(),
            ObjectKind::WebhookArchive { connector } => {
                format!("webhook_archive/{connector}")
            }
            ObjectKind::BatchJobInput => "batch_job".to_string(),
            ObjectKind::BatchJobErrorOutput => "batch_job_errors".to_string(),
            ObjectKind::TaxReport => "tax_report".to_string(),
        }
    }

    /// The registry discriminator this kind is recorded under.
    pub fn registry_kind(&self) -> StoredObjectKindEnum {
        match self {
            ObjectKind::InvoicePdf => StoredObjectKindEnum::InvoicePdf,
            ObjectKind::CreditNotePdf => StoredObjectKindEnum::CreditNotePdf,
            ObjectKind::ReceiptPdf => StoredObjectKindEnum::ReceiptPdf,
            ObjectKind::InvoiceXml => StoredObjectKindEnum::InvoiceXml,
            ObjectKind::ImageLogo => StoredObjectKindEnum::ImageLogo,
            ObjectKind::WebhookArchive { .. } => StoredObjectKindEnum::WebhookArchive,
            ObjectKind::BatchJobInput => StoredObjectKindEnum::BatchJobInput,
            ObjectKind::BatchJobErrorOutput => StoredObjectKindEnum::BatchJobErrorOutput,
            ObjectKind::TaxReport => StoredObjectKindEnum::TaxReport,
        }
    }

    /// Relative key for a freshly minted object: `{tenant}/{type}/{uuid}`.
    fn key(&self, tenant_id: TenantId, id: StoredDocumentId) -> String {
        format!("{tenant_id}/{}/{}", self.type_path(), id.as_uuid())
    }

    /// Relative key for a deterministic, named scratch artifact: `{tenant}/{type}/{name}`.
    fn named_key(&self, tenant_id: TenantId, name: &str) -> String {
        format!("{tenant_id}/{}/{name}", self.type_path())
    }
}

/// Legacy operational prefixes that embedded the tenant as their *second* segment. A tenant wipe
/// sweeps these explicitly since they predate the unified `{tenant}/` layout.
const LEGACY_TENANT_SCOPED_PREFIXES: [&str; 4] = [
    "webhook_archive",
    "batch_job",
    "batch_job_errors",
    "tax_report",
];

/// Resolve a relative key (as stored in the registry) to a full backend [`Path`] under `base` — the
/// one place a key becomes a path. Splitting on `/` recovers separators; a `%2F` inside a segment is
/// a legacy operational key's encoded slash, decoded here so `PathPart` re-encodes it identically.
fn to_path(base: &Path, key: &str) -> Path {
    let mut parts: Vec<PathPart> = base.parts().collect();
    for segment in key.split('/') {
        parts.push(PathPart::from(segment.replace("%2F", "/")));
    }
    Path::from_iter(parts)
}

pub type Result<T> = core::result::Result<T, Report<ObjectStoreError>>;

#[async_trait]
pub trait ObjectStoreService: Send + Sync {
    /// Store `binary`, minting a random id, and return the id + the key it was written under. The
    /// caller records it in the registry; most writers use [`put`] to do both in one call.
    async fn store(
        &self,
        kind: ObjectKind,
        tenant_id: TenantId,
        binary: Bytes,
    ) -> Result<StoredObjectRef>;

    /// Read the object recorded by its registry [`StoredObject`].
    async fn retrieve(&self, document: &StoredObject) -> Result<Bytes>;

    /// Presigned URL for an object recorded by its registry [`StoredObject`], when the backend
    /// supports signing.
    async fn document_url(
        &self,
        document: &StoredObject,
        expires_in: Duration,
    ) -> Result<Option<String>>;

    /// Store an unregistered scratch artifact addressed by a deterministic `name` (e.g. a tax-report
    /// chunk segment). Not tracked in the registry: ephemeral, and already swept by the tenant
    /// prefix on [`delete_tenant`](Self::delete_tenant).
    async fn put_scratch(
        &self,
        kind: ObjectKind,
        tenant_id: TenantId,
        name: &str,
        binary: Bytes,
    ) -> Result<()>;
    /// Read a scratch artifact written with [`put_scratch`](Self::put_scratch).
    async fn read_scratch(
        &self,
        kind: ObjectKind,
        tenant_id: TenantId,
        name: &str,
    ) -> Result<Bytes>;
    /// Delete a scratch artifact written with [`put_scratch`](Self::put_scratch).
    async fn delete_scratch(&self, kind: ObjectKind, tenant_id: TenantId, name: &str)
    -> Result<()>;

    /// Delete every object belonging to `tenant_id`: the whole tenant-prefixed layout plus the
    /// legacy operational prefixes still keyed by tenant. Used when a tenant is deleted.
    async fn delete_tenant(&self, tenant_id: TenantId) -> Result<()>;
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
            Some(prefix) => path.join(prefix.as_str()),
            None => path,
        };

        Ok(S3Storage {
            object_store_client: Arc::new(client),
            path,
            signer,
        })
    }

    /// Key-addressed primitives every public method is expressed over. Private: callers work with
    /// `ObjectKind` + `StoredObject`, never raw keys.
    async fn put_key(&self, key: &str, binary: Bytes) -> Result<()> {
        self.object_store_client
            .put(&to_path(&self.path, key), PutPayload::from_bytes(binary))
            .await
            .change_context(ObjectStoreError::SaveError)?;
        Ok(())
    }

    async fn get_key(&self, key: &str) -> Result<Bytes> {
        self.object_store_client
            .get(&to_path(&self.path, key))
            .await
            .change_context(ObjectStoreError::LoadError)?
            .bytes()
            .await
            .change_context(ObjectStoreError::LoadError)
    }

    async fn delete_key(&self, key: &str) -> Result<()> {
        self.object_store_client
            .delete(&to_path(&self.path, key))
            .await
            .change_context(ObjectStoreError::SaveError)?;
        Ok(())
    }

    async fn signed_key(&self, key: &str, expires_in: Duration) -> Result<Option<String>> {
        // Only some backends support presigned URLs.
        match self.signer.clone() {
            Some(signer) => signer
                .signed_url(Method::GET, &to_path(&self.path, key), expires_in)
                .await
                .map(|url| Some(url.to_string()))
                .change_context(ObjectStoreError::SaveError),
            None => Ok(None),
        }
    }

    /// Delete everything under a relative prefix (`list` + `delete_stream`). The prefix is joined as
    /// a single path part, so a legacy `<type>/<tenant>` reproduces the `%2F`-joined segment.
    async fn delete_prefix(&self, relative: &str) -> Result<()> {
        let prefix = self.path.clone().join(relative);

        let locations = self
            .object_store_client
            .list(Some(&prefix))
            .map_ok(|meta| meta.location)
            .boxed();

        self.object_store_client
            .delete_stream(locations)
            .try_collect::<Vec<_>>()
            .await
            .change_context(ObjectStoreError::SaveError)?;

        Ok(())
    }
}

#[async_trait]
impl ObjectStoreService for S3Storage {
    async fn store(
        &self,
        kind: ObjectKind,
        tenant_id: TenantId,
        binary: Bytes,
    ) -> Result<StoredObjectRef> {
        let id = StoredDocumentId::new();
        let key = kind.key(tenant_id, id);
        self.put_key(&key, binary).await?;
        Ok(StoredObjectRef { id, key })
    }

    async fn retrieve(&self, document: &StoredObject) -> Result<Bytes> {
        self.get_key(&document.key).await
    }

    async fn document_url(
        &self,
        document: &StoredObject,
        expires_in: Duration,
    ) -> Result<Option<String>> {
        self.signed_key(&document.key, expires_in).await
    }

    async fn put_scratch(
        &self,
        kind: ObjectKind,
        tenant_id: TenantId,
        name: &str,
        binary: Bytes,
    ) -> Result<()> {
        self.put_key(&kind.named_key(tenant_id, name), binary).await
    }

    async fn read_scratch(
        &self,
        kind: ObjectKind,
        tenant_id: TenantId,
        name: &str,
    ) -> Result<Bytes> {
        self.get_key(&kind.named_key(tenant_id, name)).await
    }

    async fn delete_scratch(
        &self,
        kind: ObjectKind,
        tenant_id: TenantId,
        name: &str,
    ) -> Result<()> {
        self.delete_key(&kind.named_key(tenant_id, name)).await
    }

    async fn delete_tenant(&self, tenant_id: TenantId) -> Result<()> {
        // Unified tenant-prefixed layout: one sweep wipes every new-format object.
        self.delete_prefix(tenant_id.to_string().as_str()).await?;

        // Legacy operational objects still live under `<type>/<tenant>/…`.
        for base in LEGACY_TENANT_SCOPED_PREFIXES {
            self.delete_prefix(format!("{base}/{tenant_id}").as_str())
                .await?;
        }

        Ok(())
    }
}

/// Store `binary` and record it in the registry — the one call non-transactional writers use, so a
/// write names its kind once and never threads keys or builds registry rows by hand. Transactional
/// writers (invoice/credit-note) instead call [`ObjectStoreService::store`] and insert the registry
/// row inside their own DB transaction, alongside the entity reference.
pub async fn put(
    object_store: &dyn ObjectStoreService,
    store: &Store,
    kind: ObjectKind,
    tenant_id: TenantId,
    binary: Bytes,
) -> Result<StoredObject> {
    let registry_kind = kind.registry_kind();
    let stored = object_store.store(kind, tenant_id, binary).await?;

    store
        .record_stored_object(StoredObjectNew {
            id: stored.id,
            tenant_id,
            kind: registry_kind,
            key: stored.key.clone(),
        })
        .await
        .change_context(ObjectStoreError::SaveError)?;

    Ok(StoredObject {
        id: stored.id,
        tenant_id,
        kind: registry_kind,
        key: stored.key,
    })
}

/// Resolve a stored object's registry record by id, erroring if it isn't recorded.
async fn lookup(store: &Store, id: StoredDocumentId) -> Result<StoredObject> {
    store
        .get_stored_object(id)
        .await
        .change_context(ObjectStoreError::LoadError)?
        .ok_or_else(|| {
            Report::new(ObjectStoreError::LoadError).attach(format!("unknown stored object {id}"))
        })
}

/// Read a stored object's bytes by id — the read counterpart of [`put`]. Resolves the registry
/// record then reads it; errors if the id is not recorded.
pub async fn read(
    object_store: &dyn ObjectStoreService,
    store: &Store,
    id: StoredDocumentId,
) -> Result<Bytes> {
    object_store.retrieve(&lookup(store, id).await?).await
}

/// Like [`read`], but first asserts the object is of `kind` — for public, unauthenticated endpoints
/// that must not serve an id of the wrong kind.
pub async fn read_of_kind(
    object_store: &dyn ObjectStoreService,
    store: &Store,
    id: StoredDocumentId,
    kind: StoredObjectKindEnum,
) -> Result<Bytes> {
    let document = lookup(store, id).await?;
    if document.kind != kind {
        return Err(
            Report::new(ObjectStoreError::LoadError).attach("stored object is of the wrong kind")
        );
    }
    object_store.retrieve(&document).await
}

/// Presigned URL for a stored object by id, or `None` if the id isn't recorded or the backend can't
/// sign. The link counterpart of [`read`].
pub async fn signed_url(
    object_store: &dyn ObjectStoreService,
    store: &Store,
    id: StoredDocumentId,
    expires_in: Duration,
) -> Result<Option<String>> {
    match store
        .get_stored_object(id)
        .await
        .change_context(ObjectStoreError::LoadError)?
    {
        Some(document) => object_store.document_url(&document, expires_in).await,
        None => Ok(None),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_kind_key_is_tenant_type_leaf() {
        let tenant = TenantId::new();
        let id = StoredDocumentId::new();
        let uuid = id.as_uuid().to_string();

        assert_eq!(
            ObjectKind::InvoicePdf.key(tenant, id),
            format!("{tenant}/invoice_pdf/{uuid}")
        );
        assert_eq!(
            ObjectKind::WebhookArchive {
                connector: "stripe".to_string(),
            }
            .named_key(tenant, "seg-1"),
            format!("{tenant}/webhook_archive/stripe/seg-1")
        );
    }

    /// The migration backfills these exact relative keys for pre-tenant-prefix objects, and reads
    /// resolve them via `to_path`. Legacy operational keys embed the tenant (and, for webhooks, the
    /// alias) in a single `%2F`-joined path part — the encoding `object_store` produced when the old
    /// code joined a slash-containing string. With an empty base, resolving the key must reproduce it
    /// byte-for-byte, so the SQL backfill and the reader stay in lockstep.
    #[test]
    fn stored_keys_round_trip_through_to_path() {
        let base = Path::from("");
        let uuid = "0193a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";
        for key in [
            // business legacy — tenant-less
            format!("invoice_pdf/{uuid}"),
            format!("credit_note_pdf/{uuid}"),
            format!("receipt_pdf/{uuid}"),
            format!("image_logo/{uuid}"),
            // operational legacy — tenant embedded as a %2F-joined second segment
            format!("batch_job%2Ften_2Abc/{uuid}"),
            format!("batch_job_errors%2Ften_2Abc/{uuid}"),
            format!("tax_report%2Ften_2Abc/{uuid}"),
            format!("webhook_archive%2Ften_2Abc%2Fstripe/{uuid}"),
            // new tenant-prefixed
            format!("ten_2Abc/invoice_pdf/{uuid}"),
            format!("ten_2Abc/webhook_archive/stripe/{uuid}"),
        ] {
            assert_eq!(
                to_path(&base, &key).to_string(),
                key,
                "key must resolve back to itself"
            );
        }
    }
}
