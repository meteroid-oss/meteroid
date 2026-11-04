use crate::errors::IntoDbResult;
use crate::stored_object::{StoredObjectRow, StoredObjectRowNew};
use crate::{DbResult, PgConn};

use common_domain::ids::{StoredDocumentId, TenantId};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use error_stack::ResultExt;

impl StoredObjectRowNew {
    /// Insert the metadata rows for a batch of just-written documents. Called inside the same
    /// transaction as the owning-row update so document metadata and the reference commit together.
    pub async fn insert_batch(conn: &mut PgConn, rows: &[StoredObjectRowNew]) -> DbResult<()> {
        use crate::schema::stored_object::dsl::stored_object;
        use diesel_async::RunQueryDsl;

        if rows.is_empty() {
            return Ok(());
        }

        diesel::insert_into(stored_object)
            .values(rows)
            .execute(conn)
            .await
            .attach("Error while inserting object store documents")
            .into_db_result()?;

        Ok(())
    }
}

impl StoredObjectRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        id: StoredDocumentId,
    ) -> DbResult<Option<StoredObjectRow>> {
        use crate::schema::stored_object::dsl as d;
        use diesel_async::RunQueryDsl;

        let query = d::stored_object
            .filter(d::id.eq(id))
            .select(StoredObjectRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while fetching object store document")
            .into_db_result()
    }

    /// Every document belonging to a tenant — the enumerable inventory used to delete a tenant's
    /// objects exactly (legacy included).
    pub async fn list_by_tenant(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<StoredObjectRow>> {
        use crate::schema::stored_object::dsl as d;
        use diesel_async::RunQueryDsl;

        d::stored_object
            .filter(d::tenant_id.eq(tenant_id))
            .select(StoredObjectRow::as_select())
            .load(conn)
            .await
            .attach("Error while listing tenant object store documents")
            .into_db_result()
    }

    pub async fn delete_by_tenant(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<usize> {
        use crate::schema::stored_object::dsl as d;
        use diesel_async::RunQueryDsl;

        diesel::delete(d::stored_object.filter(d::tenant_id.eq(tenant_id)))
            .execute(conn)
            .await
            .attach("Error while deleting tenant object store documents")
            .into_db_result()
    }
}
