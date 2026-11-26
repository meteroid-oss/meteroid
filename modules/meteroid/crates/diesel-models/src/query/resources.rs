use crate::errors::IntoDbResult;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::resources::{
    ResourceRow, ResourceRowNew, ResourceTypeKeyRow, ResourceTypeRow, ResourceTypeRowNew,
    ResourceTypeRowPatch,
};
use crate::{DbResult, PgConn};
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, CustomerId, ResourceId, ResourceTypeId, TenantId};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl ResourceTypeRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<ResourceTypeRow> {
        use crate::schema::resource_type::dsl;

        diesel::insert_into(dsl::resource_type)
            .values(self)
            .returning(ResourceTypeRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while inserting resource type")
            .into_db_result()
    }
}

impl ResourceTypeRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: ResourceTypeId,
    ) -> DbResult<ResourceTypeRow> {
        use crate::schema::resource_type::dsl;

        dsl::resource_type
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::id.eq(id))
            .select(Self::as_select())
            .first(conn)
            .await
            .attach("Error while finding resource type")
            .into_db_result()
    }

    /// Archived types still resolve in checks, so readers usually want them.
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        include_archived: bool,
    ) -> DbResult<Vec<ResourceTypeRow>> {
        use crate::schema::resource_type::dsl;

        let mut query = dsl::resource_type
            .filter(dsl::tenant_id.eq(tenant_id))
            .select(Self::as_select())
            .order(dsl::code.asc())
            .into_boxed();
        if !include_archived {
            query = query.filter(dsl::archived_at.is_null());
        }
        query
            .get_results(conn)
            .await
            .attach("Error while listing resource types")
            .into_db_result()
    }

    pub async fn update(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: ResourceTypeId,
        patch: &ResourceTypeRowPatch,
    ) -> DbResult<ResourceTypeRow> {
        use crate::schema::resource_type::dsl;

        diesel::update(dsl::resource_type)
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::id.eq(id))
            .set(patch)
            .returning(Self::as_returning())
            .get_result(conn)
            .await
            .attach("Error while updating resource type")
            .into_db_result()
    }

    /// Taken by a change of the type's keys, until it commits: a sale of one of its resources
    /// waits for it, and it waits for the sales in flight (`lock_keys_shared`).
    pub async fn lock_keys_exclusive(conn: &mut PgConn, id: ResourceTypeId) -> DbResult<()> {
        diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind::<diesel::sql_types::Text, _>(Self::keys_lock_name(id))
            .execute(conn)
            .await
            .attach("Error while locking the keys of a resource type")
            .into_db_result()?;
        Ok(())
    }

    /// Taken by a write selling one of the type's resources, until it commits.
    pub async fn lock_keys_shared(conn: &mut PgConn, id: ResourceTypeId) -> DbResult<()> {
        diesel::sql_query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
            .bind::<diesel::sql_types::Text, _>(Self::keys_lock_name(id))
            .execute(conn)
            .await
            .attach("Error while locking the keys of a resource type")
            .into_db_result()?;
        Ok(())
    }

    fn keys_lock_name(id: ResourceTypeId) -> String {
        format!("resource_type_keys:{}", id.as_uuid())
    }
}

impl ResourceTypeKeyRow {
    /// Every key of the tenant, archived types included, by type then position.
    pub async fn list_by_tenant(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<ResourceTypeKeyRow>> {
        use crate::schema::resource_type_key::dsl;

        dsl::resource_type_key
            .filter(dsl::tenant_id.eq(tenant_id))
            .order((dsl::resource_type_id.asc(), dsl::position.asc()))
            .select(Self::as_select())
            .get_results(conn)
            .await
            .attach("Error while listing resource type keys")
            .into_db_result()
    }

    /// Sets a type's keys, canonical first. A key held by another type fails on
    /// `resource_type_key_tenant_id_key_key`.
    pub async fn replace_for_type(
        conn: &mut PgConn,
        tenant_id: TenantId,
        resource_type_id: ResourceTypeId,
        keys: &[String],
    ) -> DbResult<Vec<ResourceTypeKeyRow>> {
        use crate::schema::resource_type_key::dsl;

        diesel::delete(dsl::resource_type_key)
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::resource_type_id.eq(resource_type_id))
            .execute(conn)
            .await
            .attach("Error while clearing resource type keys")
            .into_db_result()?;
        let rows = keys
            .iter()
            .zip(0..)
            .map(|(key, position)| ResourceTypeKeyRow {
                tenant_id,
                resource_type_id,
                key: key.clone(),
                position,
            })
            .collect::<Vec<_>>();
        diesel::insert_into(dsl::resource_type_key)
            .values(&rows)
            .execute(conn)
            .await
            .attach("Error while inserting resource type keys")
            .into_db_result()?;
        Ok(rows)
    }
}

impl ResourceRowNew {
    /// Creates the resource, or updates the name, attributes and archive of the one with this
    /// `(customer, type, key value)`.
    pub async fn upsert(&self, conn: &mut PgConn) -> DbResult<ResourceRow> {
        use crate::schema::resource::dsl;
        use diesel::upsert::excluded;

        let query = diesel::insert_into(dsl::resource)
            .values(self)
            .on_conflict((
                dsl::tenant_id,
                dsl::customer_id,
                dsl::resource_type_id,
                dsl::key_value,
            ))
            .do_update()
            .set((
                dsl::name.eq(excluded(dsl::name)),
                dsl::attributes.eq(excluded(dsl::attributes)),
                dsl::archived_at.eq(excluded(dsl::archived_at)),
                dsl::updated_at.eq(common_clock::now()),
            ))
            .returning(ResourceRow::as_returning());
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while upserting resource")
            .into_db_result()
    }
}

impl ResourceRow {
    /// FOR UPDATE: it is deleted. An override created meanwhile (KEY SHARE) waits for it.
    pub async fn lock_by_id(
        conn: &mut PgConn,
        lock: &crate::lock::CustomerLock,
        id: ResourceId,
    ) -> DbResult<Option<ResourceRow>> {
        use crate::schema::resource::dsl;

        let resource = dsl::resource
            .filter(dsl::tenant_id.eq(lock.tenant_id()))
            .filter(dsl::id.eq(id))
            .select(Self::as_select())
            .for_update()
            .first(conn)
            .await
            .optional()
            .attach("Error while locking resource")
            .into_db_result()?;
        if let Some(resource) = &resource {
            lock.covers(conn, resource.tenant_id, resource.customer_id)?;
        }
        Ok(resource)
    }

    pub async fn find_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: ResourceId,
    ) -> DbResult<Option<ResourceRow>> {
        use crate::schema::resource::dsl;

        dsl::resource
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::id.eq(id))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while finding resource")
            .into_db_result()
    }

    pub async fn find_by_ids(
        conn: &mut PgConn,
        tenant_id: TenantId,
        ids: &[ResourceId],
    ) -> DbResult<Vec<ResourceRow>> {
        use crate::schema::resource::dsl;

        dsl::resource
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::id.eq_any(ids))
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while finding resources")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        resource_type_id: Option<ResourceTypeId>,
        include_archived: bool,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<ResourceRow>> {
        use crate::schema::resource::dsl;

        let mut query = dsl::resource
            .filter(dsl::tenant_id.eq(tenant_id))
            .into_boxed();
        if let Some(customer_id) = customer_id {
            query = query.filter(dsl::customer_id.eq(customer_id));
        }
        if let Some(resource_type_id) = resource_type_id {
            query = query.filter(dsl::resource_type_id.eq(resource_type_id));
        }
        if !include_archived {
            query = query.filter(dsl::archived_at.is_null());
        }
        query
            .order((dsl::key_value.asc(), dsl::id.asc()))
            .select(Self::as_select())
            .paginate(pagination)
            .load_and_count_pages(conn)
            .await
            .attach("Error while listing resources")
            .into_db_result()
    }

    /// A page of the customer's resources after `after`, by id.
    pub async fn list_after(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        resource_type_id: Option<ResourceTypeId>,
        include_archived: bool,
        after: Option<ResourceId>,
        limit: i64,
    ) -> DbResult<Vec<ResourceRow>> {
        use crate::schema::resource::dsl;

        let mut query = dsl::resource
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::customer_id.eq(customer_id))
            .into_boxed();
        if let Some(resource_type_id) = resource_type_id {
            query = query.filter(dsl::resource_type_id.eq(resource_type_id));
        }
        if !include_archived {
            query = query.filter(dsl::archived_at.is_null());
        }
        if let Some(after) = after {
            query = query.filter(dsl::id.gt(after));
        }
        query
            .order(dsl::id.asc())
            .limit(limit)
            .select(Self::as_select())
            .get_results(conn)
            .await
            .attach("Error while paging resources")
            .into_db_result()
    }

    /// The customer's stored resources among `(type, key value)` pairs, archived ones included.
    pub async fn load_by_keys(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        keys: &[(ResourceTypeId, String)],
    ) -> DbResult<Vec<ResourceRow>> {
        use crate::schema::resource::dsl;

        if keys.is_empty() {
            return Ok(vec![]);
        }
        let rows: Vec<ResourceRow> = dsl::resource
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::customer_id.eq(customer_id))
            .filter(dsl::resource_type_id.eq_any(keys.iter().map(|(t, _)| *t).collect::<Vec<_>>()))
            .filter(dsl::key_value.eq_any(keys.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>()))
            .select(Self::as_select())
            .get_results(conn)
            .await
            .attach("Error while loading resources by key")
            .into_db_result()?;
        Ok(rows
            .into_iter()
            .filter(|r| {
                keys.iter()
                    .any(|(t, v)| *t == r.resource_type_id && *v == r.key_value)
            })
            .collect())
    }

    pub async fn set_archived(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: ResourceId,
        archived_at: Option<DateTime<Utc>>,
    ) -> DbResult<ResourceRow> {
        use crate::schema::resource::dsl;

        diesel::update(dsl::resource)
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::id.eq(id))
            .set((
                dsl::archived_at.eq(archived_at),
                dsl::updated_at.eq(common_clock::now()),
            ))
            .returning(Self::as_returning())
            .get_result(conn)
            .await
            .attach("Error while archiving resource")
            .into_db_result()
    }

    pub async fn delete(conn: &mut PgConn, tenant_id: TenantId, id: ResourceId) -> DbResult<usize> {
        use crate::schema::resource::dsl;

        diesel::delete(dsl::resource)
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::id.eq(id))
            .execute(conn)
            .await
            .attach("Error while deleting resource")
            .into_db_result()
    }

    /// The customer's stored resources of a type, counted under a transaction advisory lock on
    /// `(customer, type)`, so concurrent upserts checking the cap serialise until commit.
    pub async fn count_locked(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        resource_type_id: ResourceTypeId,
    ) -> DbResult<i64> {
        use crate::schema::resource::dsl;

        diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind::<diesel::sql_types::Text, _>(format!(
                "resource_cap:{}:{}",
                customer_id.as_uuid(),
                resource_type_id.as_uuid()
            ))
            .execute(conn)
            .await
            .attach("Error while locking the resources of a customer and type")
            .into_db_result()?;
        dsl::resource
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::customer_id.eq(customer_id))
            .filter(dsl::resource_type_id.eq(resource_type_id))
            .count()
            .get_result(conn)
            .await
            .attach("Error while counting resources")
            .into_db_result()
    }
}
