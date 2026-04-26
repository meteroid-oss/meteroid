use crate::entitlements::{
    EntitlementRow, EntitlementRowNew, EntitlementRowPatch, FeatureProductMeta, FeatureRow,
    FeatureRowNew, FeatureRowPatch, FeatureWithProductRow,
};
use crate::enums::{EntitlementEntityTypeEnum, FeatureStatusEnum};
use crate::errors::IntoDbResult;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::{DbResult, PgConn};
use common_domain::ids::{
    BaseId, EntitlementEntityId, EntitlementId, FeatureId, PlanVersionId, ProductId, TenantId,
};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, Insertable, IntoSql, NullableExpressionMethods,
    QueryDsl, SelectableHelper, debug_query,
};
use error_stack::ResultExt;
use uuid::Uuid;

impl FeatureRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<FeatureRow> {
        use crate::schema::feature::dsl::feature;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(feature).values(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting feature")
            .into_db_result()
    }
}

impl FeatureRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        param_id: FeatureId,
        param_tenant_id: TenantId,
    ) -> DbResult<FeatureWithProductRow> {
        use crate::schema::feature::dsl as f_dsl;
        use crate::schema::product;
        use diesel_async::RunQueryDsl;

        let query = f_dsl::feature
            .left_join(product::table)
            .filter(f_dsl::id.eq(param_id))
            .filter(f_dsl::tenant_id.eq(param_tenant_id))
            .select((
                FeatureRow::as_select(),
                (product::id, product::name).nullable(),
            ));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let (feature, product_opt): (FeatureRow, Option<(ProductId, String)>) = query
            .first(conn)
            .await
            .attach("Error while finding feature by id")
            .into_db_result()?;
        Ok(FeatureWithProductRow {
            feature,
            product: product_opt.map(|(id, name)| FeatureProductMeta { id, name }),
        })
    }

    pub async fn list(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        pagination: PaginationRequest,
        param_statuses: Option<Vec<FeatureStatusEnum>>,
        param_product_id: Option<ProductId>,
        search: Option<String>,
    ) -> DbResult<PaginatedVec<FeatureWithProductRow>> {
        use crate::schema::feature::dsl as f_dsl;
        use crate::schema::product;
        use diesel::PgTextExpressionMethods;
        use diesel_async::RunQueryDsl;

        // Phase 1: paginate on the feature table alone (no join — keeps the paginator simple).
        let mut query = f_dsl::feature
            .filter(f_dsl::tenant_id.eq(param_tenant_id))
            .into_boxed();

        if let Some(statuses) = param_statuses
            && !statuses.is_empty()
        {
            query = query.filter(f_dsl::status.eq_any(statuses));
        }

        if let Some(pid) = param_product_id {
            query = query.filter(f_dsl::product_id.eq(pid));
        }

        if let Some(search) = search
            && !search.trim().is_empty()
        {
            let pattern = format!("%{search}%");
            query = query.filter(f_dsl::name.ilike(pattern));
        }

        let query = query.order((f_dsl::name.asc(), f_dsl::id.asc()));
        let paginated = query.paginate(pagination);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&paginated));

        let raw: PaginatedVec<FeatureRow> = paginated
            .load_and_count_pages(conn)
            .await
            .attach("Error while listing features")
            .into_db_result()?;

        // Phase 2: batch-join product names for the page (at most one extra query).
        let product_ids: Vec<ProductId> = raw.items.iter().filter_map(|r| r.product_id).collect();

        let product_map: std::collections::HashMap<ProductId, String> = if product_ids.is_empty() {
            std::collections::HashMap::new()
        } else {
            product::table
                .filter(product::id.eq_any(&product_ids))
                .select((product::id, product::name))
                .get_results::<(ProductId, String)>(conn)
                .await
                .attach("Error while loading product names for feature list")
                .into_db_result()?
                .into_iter()
                .collect()
        };

        Ok(PaginatedVec {
            items: raw
                .items
                .into_iter()
                .map(|feature| {
                    let product = feature.product_id.and_then(|pid| {
                        product_map.get(&pid).map(|name| FeatureProductMeta {
                            id: pid,
                            name: name.clone(),
                        })
                    });
                    FeatureWithProductRow { feature, product }
                })
                .collect(),
            total_pages: raw.total_pages,
            total_results: raw.total_results,
        })
    }

    pub async fn update(
        conn: &mut PgConn,
        param_id: FeatureId,
        param_tenant_id: TenantId,
        patch: FeatureRowPatch,
    ) -> DbResult<FeatureWithProductRow> {
        use crate::schema::feature::dsl::{feature, id, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = diesel::update(feature)
            .filter(id.eq(param_id))
            .filter(tenant_id.eq(param_tenant_id))
            .set(&patch);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let updated_row: FeatureRow = query
            .get_result(conn)
            .await
            .attach("Error while updating feature")
            .into_db_result()?;

        // Re-fetch with the product join so we get the product name in one extra trip.
        // (The UPDATE ... RETURNING clause doesn't support JOINs in PostgreSQL.)
        FeatureRow::find_by_id(conn, updated_row.id, updated_row.tenant_id).await
    }

    pub async fn set_status(
        conn: &mut PgConn,
        param_id: FeatureId,
        param_tenant_id: TenantId,
        new_status: FeatureStatusEnum,
    ) -> DbResult<()> {
        use crate::schema::feature::dsl::{feature, id, status, tenant_id, updated_at};
        use chrono::Utc;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(feature)
            .filter(id.eq(param_id))
            .filter(tenant_id.eq(param_tenant_id))
            .set((status.eq(new_status), updated_at.eq(Utc::now())));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while updating feature status")
            .into_db_result()?;

        Ok(())
    }

    /// Find features by ids, excluding ARCHIVED rows (DISABLED features are still returned —
    /// the resolver treats disabled features as a global off-switch instead of hiding them).
    pub async fn find_by_ids(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        ids: &[FeatureId],
    ) -> DbResult<Vec<FeatureWithProductRow>> {
        use crate::schema::feature::dsl as f_dsl;
        use crate::schema::product;
        use diesel_async::RunQueryDsl;

        let query = f_dsl::feature
            .left_join(product::table)
            .filter(f_dsl::tenant_id.eq(param_tenant_id))
            .filter(f_dsl::id.eq_any(ids))
            .filter(f_dsl::status.ne(FeatureStatusEnum::Archived))
            .select((
                FeatureRow::as_select(),
                (product::id, product::name).nullable(),
            ));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let rows: Vec<(FeatureRow, Option<(ProductId, String)>)> = query
            .get_results(conn)
            .await
            .attach("Error while finding features by ids")
            .into_db_result()?;

        Ok(rows
            .into_iter()
            .map(|(feature, product_opt)| FeatureWithProductRow {
                feature,
                product: product_opt.map(|(id, name)| FeatureProductMeta { id, name }),
            })
            .collect())
    }

    /// Find non-archived features for a tenant, scoped to the given product_ids
    /// (or features with NULL product_id when `include_global` is true).
    pub async fn find_active_for_products(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        product_ids: &[ProductId],
        include_global: bool,
    ) -> DbResult<Vec<FeatureWithProductRow>> {
        use crate::schema::feature::dsl as f_dsl;
        use crate::schema::product;
        use diesel_async::RunQueryDsl;

        if product_ids.is_empty() && !include_global {
            // No product scope and no global — empty result without round-tripping the DB.
            return Ok(vec![]);
        }

        let mut query = f_dsl::feature
            .left_join(product::table)
            .filter(f_dsl::tenant_id.eq(param_tenant_id))
            .filter(f_dsl::status.ne(FeatureStatusEnum::Archived))
            .select((
                FeatureRow::as_select(),
                (product::id, product::name).nullable(),
            ))
            .into_boxed();

        if product_ids.is_empty() {
            query = query.filter(f_dsl::product_id.is_null());
        } else if include_global {
            query = query.filter(
                f_dsl::product_id
                    .is_null()
                    .or(f_dsl::product_id.eq_any(product_ids)),
            );
        } else {
            query = query.filter(f_dsl::product_id.eq_any(product_ids));
        }

        let rows: Vec<(FeatureRow, Option<(ProductId, String)>)> = query
            .get_results(conn)
            .await
            .attach("Error while finding features by product scope")
            .into_db_result()?;

        Ok(rows
            .into_iter()
            .map(|(feature, product_opt)| FeatureWithProductRow {
                feature,
                product: product_opt.map(|(id, name)| FeatureProductMeta { id, name }),
            })
            .collect())
    }
}

impl EntitlementRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<EntitlementRow> {
        use crate::schema::entitlement::dsl::entitlement;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(entitlement).values(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting entitlement")
            .into_db_result()
    }

    pub async fn insert_batch(rows: &[Self], conn: &mut PgConn) -> DbResult<Vec<EntitlementRow>> {
        use crate::schema::entitlement::dsl::entitlement;
        use diesel_async::RunQueryDsl;

        diesel::insert_into(entitlement)
            .values(rows)
            .get_results(conn)
            .await
            .attach("Error while batch inserting entitlements")
            .into_db_result()
    }

    /// Like `insert_batch`, but skips rows whose (feature_id, entity_id, entity_type)
    /// tuple already exists (matching the entitlement table's UNIQUE constraint).
    /// Returns only the rows actually inserted.
    pub async fn insert_batch_skip_conflicts(
        rows: &[Self],
        conn: &mut PgConn,
    ) -> DbResult<Vec<EntitlementRow>> {
        use crate::schema::entitlement::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        if rows.is_empty() {
            return Ok(vec![]);
        }
        diesel::insert_into(e_dsl::entitlement)
            .values(rows)
            .on_conflict((e_dsl::feature_id, e_dsl::entity_id, e_dsl::entity_type))
            .do_nothing()
            .get_results(conn)
            .await
            .attach("Error while batch inserting entitlements (skip conflicts)")
            .into_db_result()
    }
}

impl EntitlementRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        param_id: EntitlementId,
        param_tenant_id: TenantId,
    ) -> DbResult<EntitlementRow> {
        use crate::schema::entitlement::dsl::{entitlement, id, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = entitlement
            .filter(id.eq(param_id))
            .filter(tenant_id.eq(param_tenant_id));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while finding entitlement by id")
            .into_db_result()
    }

    pub async fn list_by_feature(
        conn: &mut PgConn,
        param_feature_id: FeatureId,
        param_tenant_id: TenantId,
    ) -> DbResult<Vec<EntitlementRow>> {
        use crate::schema::entitlement::dsl::{entitlement, feature_id, tenant_id};
        use diesel_async::RunQueryDsl;

        entitlement
            .filter(feature_id.eq(param_feature_id))
            .filter(tenant_id.eq(param_tenant_id))
            .get_results(conn)
            .await
            .attach("Error while listing entitlements by feature")
            .into_db_result()
    }

    pub async fn list_by_entity_ids(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        entities: &[EntitlementEntityId],
        filter_feature_id: Option<FeatureId>,
    ) -> DbResult<Vec<EntitlementRow>> {
        use crate::enums::EntitlementEntityTypeEnum;
        use crate::schema::entitlement::dsl::{
            entitlement, entity_id, entity_type, feature_id, tenant_id,
        };
        use diesel::BoolExpressionMethods;
        use diesel_async::RunQueryDsl;
        use std::collections::HashMap;

        if entities.is_empty() {
            return Ok(vec![]);
        }

        // Group requested entities by type, then push a single
        //   WHERE (entity_type = T1 AND entity_id IN (...)) OR (entity_type = T2 AND ...)
        // so the DB filters on the full pair rather than us discarding rows in Rust.
        let mut by_type: HashMap<EntitlementEntityTypeEnum, Vec<Uuid>> = HashMap::new();
        for e in entities {
            by_type
                .entry(EntitlementEntityTypeEnum::from(e))
                .or_default()
                .push(e.as_uuid());
        }

        // Stable iteration order so the generated SQL is reproducible across runs (helps the
        // query planner cache).
        let mut groups: Vec<(EntitlementEntityTypeEnum, Vec<Uuid>)> = by_type.into_iter().collect();
        groups.sort_by_key(|(t, _)| *t);

        let mut groups_iter = groups.into_iter();
        let (first_type, first_uuids) = groups_iter.next().expect("entities non-empty");
        let mut predicate: Box<
            dyn diesel::BoxableExpression<
                    crate::schema::entitlement::table,
                    diesel::pg::Pg,
                    SqlType = diesel::sql_types::Bool,
                >,
        > = Box::new(
            entity_type
                .eq(first_type)
                .and(entity_id.eq_any(first_uuids)),
        );
        for (t, uuids) in groups_iter {
            predicate = Box::new(predicate.or(entity_type.eq(t).and(entity_id.eq_any(uuids))));
        }

        let mut query = entitlement
            .filter(tenant_id.eq(param_tenant_id))
            .filter(predicate)
            .into_boxed();

        if let Some(fid) = filter_feature_id {
            query = query.filter(feature_id.eq(fid));
        }

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while fetching entitlements by entity ids")
            .into_db_result()
    }

    pub async fn list_feature_level_entitlements(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        feature_ids: Option<&Vec<FeatureId>>,
    ) -> DbResult<Vec<EntitlementRow>> {
        use crate::enums::EntitlementEntityTypeEnum;
        use crate::schema::entitlement::dsl::{entitlement, entity_id, entity_type, tenant_id};
        use diesel_async::RunQueryDsl;

        if matches!(feature_ids, Some(ids) if ids.is_empty()) {
            return Ok(vec![]);
        }

        let mut query = entitlement
            .filter(tenant_id.eq(param_tenant_id))
            .filter(entity_type.eq(EntitlementEntityTypeEnum::Feature))
            .into_boxed();

        if let Some(ids) = feature_ids {
            let uuids: Vec<Uuid> = ids.iter().map(|id| id.as_uuid()).collect();
            query = query.filter(entity_id.eq_any(uuids));
        }

        query
            .get_results::<EntitlementRow>(conn)
            .await
            .attach("Error while listing feature-level entitlements")
            .into_db_result()
    }

    pub async fn list_by_entity(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_entity_id: EntitlementEntityId,
    ) -> DbResult<Vec<EntitlementRow>> {
        use crate::schema::entitlement::dsl::{entitlement, entity_id, entity_type, tenant_id};
        use diesel_async::RunQueryDsl;

        let entity_type_param: EntitlementEntityTypeEnum = (&param_entity_id).into();

        entitlement
            .filter(tenant_id.eq(param_tenant_id))
            .filter(entity_id.eq(param_entity_id.as_uuid()))
            .filter(entity_type.eq(entity_type_param))
            .get_results(conn)
            .await
            .attach("Error while listing entitlements by entity")
            .into_db_result()
    }

    pub async fn update(
        conn: &mut PgConn,
        param_id: EntitlementId,
        param_tenant_id: TenantId,
        patch: EntitlementRowPatch,
    ) -> DbResult<EntitlementRow> {
        use crate::schema::entitlement::dsl::{entitlement, id, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = diesel::update(entitlement)
            .filter(id.eq(param_id))
            .filter(tenant_id.eq(param_tenant_id))
            .set(&patch);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while updating entitlement")
            .into_db_result()
    }

    pub async fn delete(
        conn: &mut PgConn,
        param_id: EntitlementId,
        param_tenant_id: TenantId,
    ) -> DbResult<()> {
        use crate::schema::entitlement::dsl::{entitlement, id, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(entitlement)
            .filter(id.eq(param_id))
            .filter(tenant_id.eq(param_tenant_id));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while deleting entitlement")
            .into_db_result()?;

        Ok(())
    }

    pub async fn clone_all_for_plan_version(
        conn: &mut PgConn,
        src_plan_version_id: PlanVersionId,
        dst_plan_version_id: PlanVersionId,
        param_tenant_id: TenantId,
        new_created_by: Uuid,
    ) -> DbResult<usize> {
        use crate::enums::EntitlementEntityTypeEnum;
        use crate::schema::entitlement::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        diesel::define_sql_function! {
            fn gen_random_uuid() -> diesel::sql_types::Uuid;
        }

        let src_uuid: Uuid = *src_plan_version_id;
        let dst_uuid: Uuid = *dst_plan_version_id;

        let query = e_dsl::entitlement
            .filter(e_dsl::entity_id.eq(src_uuid))
            .filter(e_dsl::entity_type.eq(EntitlementEntityTypeEnum::PlanVersion))
            .filter(e_dsl::tenant_id.eq(param_tenant_id))
            .select((
                gen_random_uuid(),
                e_dsl::tenant_id,
                e_dsl::feature_id,
                dst_uuid.into_sql::<diesel::sql_types::Uuid>(),
                EntitlementEntityTypeEnum::PlanVersion
                    .into_sql::<crate::schema::sql_types::EntitlementEntityTypeEnum>(),
                e_dsl::mode,
                e_dsl::value,
                new_created_by.into_sql::<diesel::sql_types::Uuid>(),
            ))
            .insert_into(e_dsl::entitlement)
            .into_columns((
                e_dsl::id,
                e_dsl::tenant_id,
                e_dsl::feature_id,
                e_dsl::entity_id,
                e_dsl::entity_type,
                e_dsl::mode,
                e_dsl::value,
                e_dsl::created_by,
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while cloning entitlements for plan version")
            .into_db_result()
    }
}
