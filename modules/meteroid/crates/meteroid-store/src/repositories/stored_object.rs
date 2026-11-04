use crate::domain::stored_object::{StoredObject, StoredObjectNew};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::{StoredDocumentId, TenantId};
use diesel_models::stored_object::{StoredObjectRow, StoredObjectRowNew};
use error_stack::Report;

#[async_trait::async_trait]
pub trait StoredObjectInterface {
    /// Record a stored object's metadata. Used by write paths that aren't wrapped in a document
    /// save transaction (e.g. logo upload); transactional callers insert the row inline instead.
    async fn record_stored_object(&self, document: StoredObjectNew) -> StoreResult<()>;

    /// The full registry entry (tenant, path layout, kind) for a stored object, or `None` if the
    /// id is unknown. Used by the public logo endpoint to resolve the object *and* assert its kind.
    async fn get_stored_object(&self, id: StoredDocumentId) -> StoreResult<Option<StoredObject>>;

    /// Every stored object owned by a tenant — the enumerable inventory used to delete them.
    async fn list_stored_objects_by_tenant(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<StoredObject>>;

    async fn delete_stored_objects_by_tenant(&self, tenant_id: TenantId) -> StoreResult<usize>;
}

#[async_trait::async_trait]
impl StoredObjectInterface for Store {
    async fn record_stored_object(&self, document: StoredObjectNew) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        StoredObjectRowNew::insert_batch(&mut conn, &[document.into()])
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(())
    }

    async fn get_stored_object(&self, id: StoredDocumentId) -> StoreResult<Option<StoredObject>> {
        let mut conn = self.get_conn().await?;

        let row = StoredObjectRow::get_by_id(&mut conn, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(row.map(Into::into))
    }

    async fn list_stored_objects_by_tenant(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<StoredObject>> {
        let mut conn = self.get_conn().await?;

        let rows = StoredObjectRow::list_by_tenant(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn delete_stored_objects_by_tenant(&self, tenant_id: TenantId) -> StoreResult<usize> {
        let mut conn = self.get_conn().await?;

        StoredObjectRow::delete_by_tenant(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }
}
