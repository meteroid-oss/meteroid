use crate::customer_credit::CreditRecomputeMarkRow;
use crate::entitlements::{
    EntitlementRow, EntitlementRowNew, FeatureListOrder, FeatureProductMeta, FeatureRow,
    FeatureRowNew, FeatureRowPatch, FeatureWithProductRow,
};
use crate::enums::{EntitlementEntityTypeEnum, FeatureStatusEnum};
use crate::errors::IntoDbResult;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::{DbResult, PgConn};
use common_domain::ids::{
    AliasOr, BaseId, EntitlementEntityId, EntitlementId, FeatureId, PlanVersionId, ProductId,
    ResourceId, ResourceTypeId, SubscriptionId, TenantId,
};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, JoinOnDsl, NullableExpressionMethods, QueryDsl,
    SelectableHelper,
};
use error_stack::ResultExt;
use uuid::Uuid;

/// Marks the customers each distinct owner of the entitlements reaches, so their usage monitors
/// follow the change.
async fn mark_owners(conn: &mut PgConn, rows: &[EntitlementRow]) -> DbResult<()> {
    let owners = rows
        .iter()
        .map(|row| (row.tenant_id, row.entity_type, row.entity_id))
        .collect::<std::collections::HashSet<_>>();
    for (tenant_id, entity_type, entity_id) in owners {
        CreditRecomputeMarkRow::mark_for_entitlement_owner(
            conn,
            tenant_id,
            &entity_type,
            entity_id,
        )
        .await?;
    }
    Ok(())
}

impl FeatureRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<FeatureRow> {
        use crate::schema::feature::dsl::feature;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(feature)
            .values(self)
            .returning(FeatureRow::as_returning());
        crate::debug_sql(&query);

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
            .left_join(
                product::table.on(f_dsl::product_id
                    .eq(product::id.nullable())
                    .and(product::tenant_id.eq(f_dsl::tenant_id))),
            )
            .filter(f_dsl::id.eq(param_id))
            .filter(f_dsl::tenant_id.eq(param_tenant_id))
            .select((
                FeatureRow::as_select(),
                (product::id, product::name).nullable(),
            ));
        crate::debug_sql(&query);

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

    pub async fn find_by_id_or_code(
        conn: &mut PgConn,
        id_or_code: AliasOr<FeatureId>,
        param_tenant_id: TenantId,
    ) -> DbResult<FeatureWithProductRow> {
        use crate::schema::feature::dsl as f_dsl;
        use crate::schema::product;
        use diesel_async::RunQueryDsl;

        let mut query = f_dsl::feature
            .left_join(
                product::table.on(f_dsl::product_id
                    .eq(product::id.nullable())
                    .and(product::tenant_id.eq(f_dsl::tenant_id))),
            )
            .filter(f_dsl::tenant_id.eq(param_tenant_id))
            .select((
                FeatureRow::as_select(),
                (product::id, product::name).nullable(),
            ))
            .into_boxed();

        // Archived features release their code, so a code names the one non-archived feature.
        match id_or_code {
            AliasOr::Id(id) => query = query.filter(f_dsl::id.eq(id)),
            AliasOr::Alias(code) => {
                query = query
                    .filter(f_dsl::code.eq(code))
                    .filter(f_dsl::status.ne(FeatureStatusEnum::Archived))
            }
        }
        crate::debug_sql(&query);

        let (feature, product_opt): (FeatureRow, Option<(ProductId, String)>) = query
            .first(conn)
            .await
            .attach("Error while finding feature by id or code")
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
        level: Option<Option<ResourceTypeId>>,
        order: FeatureListOrder,
    ) -> DbResult<PaginatedVec<FeatureWithProductRow>> {
        use crate::schema::feature::dsl as f_dsl;
        use crate::schema::product;
        use diesel::PgTextExpressionMethods;
        use diesel::sql_types::{Nullable, Text};
        use diesel_async::RunQueryDsl;

        diesel::define_sql_function! {
            fn lower(x: Nullable<Text>) -> Nullable<Text>;
        }

        // Shared by both orders: only the product order joins, so the name order keeps its plan.
        macro_rules! apply_filters {
            ($query:expr) => {{
                let mut query = $query.filter(f_dsl::tenant_id.eq(param_tenant_id));

                if let Some(statuses) = param_statuses
                    && !statuses.is_empty()
                {
                    query = query.filter(f_dsl::status.eq_any(statuses));
                }

                if let Some(pid) = param_product_id {
                    query = query.filter(f_dsl::product_id.eq(pid));
                }

                match level {
                    Some(Some(type_id)) => {
                        query = query.filter(f_dsl::level_resource_type_id.eq(type_id))
                    }
                    Some(None) => query = query.filter(f_dsl::level_resource_type_id.is_null()),
                    None => {}
                }

                if let Some(search) = search
                    && !search.trim().is_empty()
                {
                    let pattern = format!("%{search}%");
                    query = query.filter(f_dsl::name.ilike(pattern));
                }
                query
            }};
        }

        // Phase 1: paginate the feature rows; product names are batch-loaded in phase 2.
        let raw: PaginatedVec<FeatureRow> = match order {
            FeatureListOrder::Name => {
                let query = apply_filters!(f_dsl::feature.into_boxed())
                    .order((f_dsl::name.asc(), f_dsl::id.asc()))
                    .select(FeatureRow::as_select());
                let paginated = query.paginate(pagination);
                crate::debug_sql(&paginated);
                paginated
                    .load_and_count_pages(conn)
                    .await
                    .attach("Error while listing features")
                    .into_db_result()?
            }
            FeatureListOrder::Product => {
                let joined = f_dsl::feature
                    .left_join(
                        product::table.on(f_dsl::product_id
                            .eq(product::id.nullable())
                            .and(product::tenant_id.eq(f_dsl::tenant_id))),
                    )
                    .into_boxed();
                let query = apply_filters!(joined)
                    .order((
                        f_dsl::product_id.is_null().asc(),
                        lower(product::name.nullable()).asc(),
                        f_dsl::product_id.asc(),
                        lower(f_dsl::name.nullable()).asc(),
                        f_dsl::id.asc(),
                    ))
                    .select(FeatureRow::as_select());
                let paginated = query.paginate(pagination);
                crate::debug_sql(&paginated);
                paginated
                    .load_and_count_pages(conn)
                    .await
                    .attach("Error while listing features")
                    .into_db_result()?
            }
        };

        // Phase 2: batch-join product names for the page (at most one extra query).
        let product_ids: Vec<ProductId> = raw.items.iter().filter_map(|r| r.product_id).collect();

        let product_map: std::collections::HashMap<ProductId, String> = if product_ids.is_empty() {
            std::collections::HashMap::new()
        } else {
            product::table
                .filter(product::id.eq_any(&product_ids))
                .filter(product::tenant_id.eq(param_tenant_id))
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

        let previous_product = if patch.product_id.is_some() {
            use crate::schema::feature::dsl::product_id;
            Some(
                feature
                    .filter(id.eq(param_id))
                    .filter(tenant_id.eq(param_tenant_id))
                    .select(product_id)
                    .first::<Option<ProductId>>(conn)
                    .await
                    .attach("Error while reading the product of a feature")
                    .into_db_result()?,
            )
        } else {
            None
        };
        let query = diesel::update(feature)
            .filter(id.eq(param_id))
            .filter(tenant_id.eq(param_tenant_id))
            .set(&patch)
            .returning(FeatureRow::as_returning());
        crate::debug_sql(&query);

        let updated_row: FeatureRow = query
            .get_result(conn)
            .await
            .attach("Error while updating feature")
            .into_db_result()?;
        if let Some(previous) =
            previous_product.filter(|previous| *previous != updated_row.product_id)
        {
            CreditRecomputeMarkRow::mark_for_feature_product(conn, updated_row.tenant_id, previous)
                .await?;
        }
        CreditRecomputeMarkRow::mark_for_entitlement_owner(
            conn,
            updated_row.tenant_id,
            &EntitlementEntityTypeEnum::Feature,
            updated_row.id.as_uuid(),
        )
        .await?;

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

        use diesel_async::RunQueryDsl;

        let query = diesel::update(feature)
            .filter(id.eq(param_id))
            .filter(tenant_id.eq(param_tenant_id))
            .set((status.eq(new_status), updated_at.eq(common_clock::now())));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while updating feature status")
            .into_db_result()?;
        CreditRecomputeMarkRow::mark_for_entitlement_owner(
            conn,
            param_tenant_id,
            &EntitlementEntityTypeEnum::Feature,
            param_id.as_uuid(),
        )
        .await?;

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
            .left_join(
                product::table.on(f_dsl::product_id
                    .eq(product::id.nullable())
                    .and(product::tenant_id.eq(f_dsl::tenant_id))),
            )
            .filter(f_dsl::tenant_id.eq(param_tenant_id))
            .filter(f_dsl::id.eq_any(ids))
            .filter(f_dsl::status.ne(FeatureStatusEnum::Archived))
            .select((
                FeatureRow::as_select(),
                (product::id, product::name).nullable(),
            ));
        crate::debug_sql(&query);

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

    /// The subset of `ids` that are archived features of this tenant.
    pub async fn find_archived_ids(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        ids: &[FeatureId],
    ) -> DbResult<Vec<FeatureId>> {
        use crate::schema::feature::dsl as f_dsl;
        use diesel_async::RunQueryDsl;

        let query = archived_feature_ids(param_tenant_id).filter(f_dsl::id.eq_any(ids));
        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while finding archived features by ids")
            .into_db_result()
    }

    /// Every feature of this tenant with `code`, archived ones included.
    pub async fn list_by_code(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_code: &str,
    ) -> DbResult<Vec<FeatureRow>> {
        use crate::schema::feature::dsl as f_dsl;
        use diesel_async::RunQueryDsl;

        let query = f_dsl::feature
            .filter(f_dsl::tenant_id.eq(param_tenant_id))
            .filter(f_dsl::code.eq(param_code))
            .select(FeatureRow::as_select());
        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing features by code")
            .into_db_result()
    }

    /// The feature other than `exclude` that holds `code`, archived ones aside.
    pub async fn find_unarchived_by_code(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_code: &str,
        exclude: FeatureId,
    ) -> DbResult<Option<FeatureRow>> {
        use crate::schema::feature::dsl as f_dsl;
        use diesel::OptionalExtension;
        use diesel_async::RunQueryDsl;

        let query = f_dsl::feature
            .filter(f_dsl::tenant_id.eq(param_tenant_id))
            .filter(f_dsl::code.eq(param_code))
            .filter(f_dsl::id.ne(exclude))
            .filter(f_dsl::status.ne(FeatureStatusEnum::Archived))
            .select(FeatureRow::as_select());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding feature by code")
            .into_db_result()
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
            .left_join(
                product::table.on(f_dsl::product_id
                    .eq(product::id.nullable())
                    .and(product::tenant_id.eq(f_dsl::tenant_id))),
            )
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

impl FeatureRow {
    /// The tenant's features after `after`, at most `limit`, each with its product: a keyset
    /// page. `order` `None` pages by id; a named order pages by its keys, the id last.
    #[allow(clippy::too_many_arguments)]
    pub async fn list_after(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_statuses: Option<Vec<FeatureStatusEnum>>,
        param_product_id: Option<ProductId>,
        level: Option<Option<ResourceTypeId>>,
        order: Option<FeatureListOrder>,
        after: Option<FeatureId>,
        limit: i64,
    ) -> DbResult<Vec<FeatureWithProductRow>> {
        use crate::schema::feature::dsl as f_dsl;
        use crate::schema::product;
        use diesel::sql_types::{Bool, Uuid as SqlUuid};
        use diesel_async::RunQueryDsl;

        // The keys of each order, the id last; a global feature sorts last, by its name.
        const BY_ID: &[&str] = &["feature.id"];
        const BY_NAME: &[&str] = &["feature.name", "feature.id"];
        const BY_PRODUCT: &[&str] = &[
            "feature.product_id IS NULL",
            "COALESCE(lower(product.name), '')",
            "COALESCE(feature.product_id, '00000000-0000-0000-0000-000000000000'::uuid)",
            "lower(feature.name)",
            "feature.id",
        ];
        const CURSOR_FROM: &str = "FROM feature LEFT JOIN product ON product.id = feature.product_id \
             AND product.tenant_id = feature.tenant_id WHERE feature.id = ";

        let mut query = f_dsl::feature
            .left_join(
                product::table.on(f_dsl::product_id
                    .eq(product::id.nullable())
                    .and(product::tenant_id.eq(f_dsl::tenant_id))),
            )
            .filter(f_dsl::tenant_id.eq(param_tenant_id))
            .select((
                FeatureRow::as_select(),
                (product::id, product::name).nullable(),
            ))
            .into_boxed();
        if let Some(statuses) = param_statuses
            && !statuses.is_empty()
        {
            query = query.filter(f_dsl::status.eq_any(statuses));
        }
        if let Some(pid) = param_product_id {
            query = query.filter(f_dsl::product_id.eq(pid));
        }
        match level {
            Some(Some(type_id)) => query = query.filter(f_dsl::level_resource_type_id.eq(type_id)),
            Some(None) => query = query.filter(f_dsl::level_resource_type_id.is_null()),
            None => {}
        }
        let keys = match order {
            None => BY_ID,
            Some(FeatureListOrder::Name) => BY_NAME,
            Some(FeatureListOrder::Product) => BY_PRODUCT,
        };
        let row = keys.join(", ");
        if let Some(after) = after {
            // The cursor's keys, read in a subquery whose tables shadow the outer ones.
            query = query.filter(
                diesel::dsl::sql::<Bool>(&format!("({row}) > (SELECT {row} {CURSOR_FROM}"))
                    .bind::<SqlUuid, _>(after)
                    .sql(" AND feature.tenant_id = ")
                    .bind::<SqlUuid, _>(param_tenant_id)
                    .sql(")"),
            );
        }
        let ordered = keys
            .iter()
            .map(|key| format!("{key} ASC"))
            .collect::<Vec<_>>()
            .join(", ");
        let query = query.order(diesel::dsl::sql::<Bool>(&ordered)).limit(limit);
        crate::debug_sql(&query);

        let rows: Vec<(FeatureRow, Option<(ProductId, String)>)> = query
            .get_results(conn)
            .await
            .attach("Error while fetching a page of features")
            .into_db_result()?;

        Ok(rows
            .into_iter()
            .map(|(feature, product_opt)| FeatureWithProductRow {
                feature,
                product: product_opt.map(|(id, name)| FeatureProductMeta { id, name }),
            })
            .collect())
    }

    /// The live (non-archived) features of a product.
    pub async fn list_live_by_product(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_product_id: ProductId,
    ) -> DbResult<Vec<FeatureRow>> {
        use crate::schema::feature::dsl as f_dsl;
        use diesel_async::RunQueryDsl;

        let query = f_dsl::feature
            .filter(f_dsl::tenant_id.eq(param_tenant_id))
            .filter(f_dsl::product_id.eq(param_product_id))
            .filter(f_dsl::status.ne(FeatureStatusEnum::Archived))
            .order(f_dsl::id.asc())
            .select(FeatureRow::as_select());
        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing the features of a product")
            .into_db_result()
    }

    /// How many features each product grants, archived ones left out.
    pub async fn count_by_products(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        product_ids: &[ProductId],
    ) -> DbResult<std::collections::HashMap<ProductId, i64>> {
        use crate::schema::feature::dsl as f_dsl;
        use diesel_async::RunQueryDsl;

        if product_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let query = f_dsl::feature
            .filter(f_dsl::tenant_id.eq(param_tenant_id))
            .filter(f_dsl::product_id.eq_any(product_ids))
            .filter(f_dsl::status.ne(FeatureStatusEnum::Archived))
            .group_by(f_dsl::product_id)
            .select((f_dsl::product_id, diesel::dsl::count(f_dsl::id)));

        crate::debug_sql(&query);

        query
            .load::<(Option<ProductId>, i64)>(conn)
            .await
            .attach("Error while counting features by product")
            .into_db_result()
            .map(|rows| {
                rows.into_iter()
                    .filter_map(|(id, count)| id.map(|id| (id, count)))
                    .collect()
            })
    }
}

/// Entitlements of an archived feature are kept but hidden until it is restored.
#[diesel::dsl::auto_type(no_type_alias)]
fn archived_feature_ids(param_tenant_id: TenantId) -> _ {
    let archived: FeatureStatusEnum = FeatureStatusEnum::Archived;
    crate::schema::feature::table
        .filter(crate::schema::feature::tenant_id.eq(param_tenant_id))
        .filter(crate::schema::feature::status.eq(archived))
        .select(crate::schema::feature::id)
}

type OwnerPredicate = Box<
    dyn diesel::BoxableExpression<
            crate::schema::entitlement::table,
            diesel::pg::Pg,
            SqlType = diesel::sql_types::Bool,
        >,
>;

/// A row as written: with its lineage's anchor.
#[derive(diesel::Insertable)]
#[diesel(table_name = crate::schema::entitlement)]
struct AnchoredRow {
    #[diesel(embed)]
    row: EntitlementRowNew,
    lineage_anchor: chrono::DateTime<chrono::Utc>,
}

/// Where each row's windows start without an activation: its lineage's first anchor, or for a new
/// lineage the earlier of its `valid_from` and its creation. Never moved by edits.
async fn anchored(conn: &mut PgConn, rows: &[EntitlementRowNew]) -> DbResult<Vec<AnchoredRow>> {
    use crate::schema::entitlement::dsl as e;
    use diesel_async::RunQueryDsl;

    let lineages = rows.iter().map(|r| r.lineage_id).collect::<Vec<_>>();
    let mut anchors: std::collections::HashMap<EntitlementId, chrono::DateTime<chrono::Utc>> =
        e::entitlement
            .filter(e::lineage_id.eq_any(&lineages))
            .order_by((e::lineage_id, e::created_at, e::id))
            .distinct_on(e::lineage_id)
            .select((e::lineage_id, e::lineage_anchor))
            .load(conn)
            .await
            .attach("Error while reading entitlement lineage anchors")
            .into_db_result()?
            .into_iter()
            .collect();
    Ok(rows
        .iter()
        .map(|row| {
            let created = row.lineage_created_at;
            let anchor = *anchors
                .entry(row.lineage_id)
                .or_insert_with(|| row.valid_from.unwrap_or(created).min(created));
            AnchoredRow {
                row: row.clone(),
                lineage_anchor: anchor,
            }
        })
        .collect())
}

impl EntitlementRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<EntitlementRow> {
        Ok(Self::insert_batch(std::slice::from_ref(self), conn)
            .await?
            .remove(0))
    }

    pub async fn insert_batch(rows: &[Self], conn: &mut PgConn) -> DbResult<Vec<EntitlementRow>> {
        use crate::schema::entitlement::dsl::entitlement;
        use diesel_async::RunQueryDsl;

        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let anchored = anchored(conn, rows).await?;
        let query = diesel::insert_into(entitlement).values(&anchored);
        crate::debug_sql(&query);
        let inserted: Vec<EntitlementRow> = query
            .get_results(conn)
            .await
            .attach("Error while inserting entitlements")
            .into_db_result()?;
        mark_owners(conn, &inserted).await?;
        Ok(inserted)
    }

    /// An edit: revokes the open row of `self.lineage_id` (locked first) and inserts `self` as its
    /// successor, keeping the lineage's creation time. Runs in the caller's transaction.
    pub async fn insert_successor(
        mut self,
        conn: &mut PgConn,
        revoked_by: Option<Uuid>,
    ) -> DbResult<EntitlementRow> {
        let previous =
            EntitlementRow::revoke_open(conn, self.tenant_id, self.lineage_id, revoked_by).await?;
        self.lineage_created_at = previous.lineage_created_at;
        self.insert(conn).await
    }

    /// The key `overlaps` locks. Batches lock in its order, so two batches never wait on each other.
    pub fn lock_key(&self) -> String {
        format!(
            "entitlement:{}:{}:{:?}:{:?}:{:?}:{}",
            self.feature_id,
            self.entity_id,
            self.level,
            self.level_type_id,
            self.resource_id,
            self.condition_hash
        )
    }

    /// Whether an open row of the same feature, owner, level, resource and condition overlaps
    /// this one's validity, which starts at its lineage's creation when undated, or one of
    /// `earlier` does (ADR 0011 §3.2). Locks the key until the transaction ends.
    pub async fn overlaps(&self, conn: &mut PgConn, earlier: &[Self]) -> DbResult<bool> {
        use crate::schema::entitlement::dsl as e;
        use diesel::PgExpressionMethods;
        use diesel::sql_types::{Bool, Nullable, Text, Timestamptz};
        use diesel_async::RunQueryDsl;

        let same_key = |o: &Self| {
            (
                o.feature_id,
                o.entity_type,
                o.entity_id,
                o.level,
                o.level_type_id,
                o.resource_id,
            ) == (
                self.feature_id,
                self.entity_type,
                self.entity_id,
                self.level,
                self.level_type_id,
                self.resource_id,
            ) && o.condition_hash == self.condition_hash
        };
        let from = |r: &Self| {
            let from = r.valid_from.unwrap_or(r.lineage_created_at);
            r.valid_until.map_or(from, |until| from.min(until))
        };
        let meets = |o: &Self| {
            o.valid_until.is_none_or(|until| from(self) < until)
                && self.valid_until.is_none_or(|until| from(o) < until)
        };
        if earlier.iter().any(|o| same_key(o) && meets(o)) {
            return Ok(true);
        }
        diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind::<Text, _>(self.lock_key())
            .execute(conn)
            .await
            .attach("Error while locking an entitlement key")
            .into_db_result()?;
        let within = diesel::dsl::sql::<Bool>(
            "tstzrange(LEAST(COALESCE(valid_from, lineage_created_at), \
             COALESCE(valid_until, 'infinity')), valid_until) && tstzrange(",
        )
        .bind::<Timestamptz, _>(from(self))
        .sql(", ")
        .bind::<Nullable<Timestamptz>, _>(self.valid_until)
        .sql(")");
        diesel::select(diesel::dsl::exists(
            e::entitlement
                .filter(e::revoked_at.is_null())
                .filter(e::feature_id.eq(self.feature_id))
                .filter(e::entity_type.eq(self.entity_type))
                .filter(e::entity_id.eq(self.entity_id))
                .filter(e::level.eq(self.level))
                .filter(e::level_type_id.is_not_distinct_from(self.level_type_id))
                .filter(e::resource_id.is_not_distinct_from(self.resource_id))
                .filter(e::condition_hash.eq(&self.condition_hash))
                .filter(e::lineage_id.ne(self.lineage_id))
                .filter(within),
        ))
        .get_result(conn)
        .await
        .attach("Error while looking for an overlapping entitlement")
        .into_db_result()
    }

    /// Inserts the rows nothing overlaps, skipping the others; returns those inserted.
    pub async fn insert_batch_skip_conflicts(
        mut rows: Vec<Self>,
        conn: &mut PgConn,
    ) -> DbResult<Vec<EntitlementRow>> {
        rows.sort_by_cached_key(Self::lock_key);
        let mut kept: Vec<Self> = Vec::with_capacity(rows.len());
        for row in rows {
            if !row.overlaps(conn, &kept).await? {
                kept.push(row);
            }
        }
        if kept.is_empty() {
            return Ok(vec![]);
        }
        Self::insert_batch(&kept, conn).await
    }
}

impl EntitlementRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        param_id: EntitlementId,
        param_tenant_id: TenantId,
    ) -> DbResult<EntitlementRow> {
        use crate::schema::entitlement::dsl::{entitlement, feature_id, id, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = entitlement
            .filter(id.eq(param_id))
            .filter(tenant_id.eq(param_tenant_id))
            .filter(feature_id.ne_all(archived_feature_ids(param_tenant_id)));
        crate::debug_sql(&query);

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
        use crate::schema::entitlement::dsl::{entitlement, feature_id, revoked_at, tenant_id};
        use diesel_async::RunQueryDsl;

        entitlement
            .filter(feature_id.eq(param_feature_id))
            .filter(tenant_id.eq(param_tenant_id))
            .filter(revoked_at.is_null())
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
        Self::list_open_for_owners(
            conn,
            param_tenant_id,
            entities,
            filter_feature_id.as_ref().map(std::slice::from_ref),
        )
        .await
    }

    /// The open rows of `owners`, optionally only for `feature_ids`; archived features' rows
    /// stay hidden.
    pub async fn list_open_for_owners(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        owners: &[EntitlementEntityId],
        feature_ids: Option<&[FeatureId]>,
    ) -> DbResult<Vec<EntitlementRow>> {
        use crate::schema::entitlement::dsl::{
            entitlement, entity_id, entity_type, feature_id, revoked_at, tenant_id,
        };
        use diesel_async::RunQueryDsl;
        use std::collections::BTreeMap;

        if owners.is_empty() || feature_ids.is_some_and(<[FeatureId]>::is_empty) {
            return Ok(vec![]);
        }

        // One `(entity_type = T AND entity_id IN (...))` per type, in a stable order.
        let mut by_type: BTreeMap<EntitlementEntityTypeEnum, Vec<Uuid>> = BTreeMap::new();
        for owner in owners {
            by_type
                .entry(EntitlementEntityTypeEnum::from(owner))
                .or_default()
                .push(owner.as_uuid());
        }
        let mut groups = by_type.into_iter();
        let (first_type, first_uuids) = groups.next().expect("owners non-empty");
        let mut predicate: OwnerPredicate = Box::new(
            entity_type
                .eq(first_type)
                .and(entity_id.eq_any(first_uuids)),
        );
        for (t, uuids) in groups {
            predicate = Box::new(predicate.or(entity_type.eq(t).and(entity_id.eq_any(uuids))));
        }

        let mut query = entitlement
            .filter(tenant_id.eq(param_tenant_id))
            .filter(predicate)
            .filter(revoked_at.is_null())
            .filter(feature_id.ne_all(archived_feature_ids(param_tenant_id)))
            .into_boxed();
        if let Some(ids) = feature_ids {
            query = query.filter(feature_id.eq_any(ids));
        }
        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while fetching entitlements by owners")
            .into_db_result()
    }

    pub async fn list_feature_level_entitlements(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        feature_ids: Option<&Vec<FeatureId>>,
    ) -> DbResult<Vec<EntitlementRow>> {
        use crate::enums::EntitlementEntityTypeEnum;
        use crate::schema::entitlement::dsl::{
            entitlement, entity_id, entity_type, revoked_at, tenant_id,
        };
        use diesel_async::RunQueryDsl;

        if matches!(feature_ids, Some(ids) if ids.is_empty()) {
            return Ok(vec![]);
        }

        let mut query = entitlement
            .filter(tenant_id.eq(param_tenant_id))
            .filter(entity_type.eq(EntitlementEntityTypeEnum::Feature))
            .filter(revoked_at.is_null())
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
        use crate::schema::entitlement::dsl::{
            entitlement, entity_id, entity_type, feature_id, revoked_at, tenant_id,
        };
        use diesel_async::RunQueryDsl;

        let entity_type_param: EntitlementEntityTypeEnum = (&param_entity_id).into();

        entitlement
            .filter(tenant_id.eq(param_tenant_id))
            .filter(entity_id.eq(param_entity_id.as_uuid()))
            .filter(entity_type.eq(entity_type_param))
            .filter(revoked_at.is_null())
            .filter(feature_id.ne_all(archived_feature_ids(param_tenant_id)))
            .get_results(conn)
            .await
            .attach("Error while listing entitlements by entity")
            .into_db_result()
    }

    /// Whether any row, open or revoked, overrides the resource.
    pub async fn exists_for_resource(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_resource_id: ResourceId,
    ) -> DbResult<bool> {
        use crate::schema::entitlement::dsl::{entitlement, resource_id, tenant_id};
        use diesel_async::RunQueryDsl;

        diesel::select(diesel::dsl::exists(
            entitlement
                .filter(tenant_id.eq(param_tenant_id))
                .filter(resource_id.eq(param_resource_id)),
        ))
        .get_result(conn)
        .await
        .attach("Error while looking for a resource's overrides")
        .into_db_result()
    }

    /// A version is a snapshot of what its author could see, so archived features' hidden
    /// entitlements stay on the source and do not come back on the copy. Every column is copied
    /// with a new id and lineage, whichever columns the schema has (ADR 0011 §7).
    pub async fn clone_all_for_plan_version(
        conn: &mut PgConn,
        src_plan_version_id: PlanVersionId,
        dst_plan_version_id: PlanVersionId,
        param_tenant_id: TenantId,
    ) -> DbResult<usize> {
        use diesel_async::RunQueryDsl;

        diesel::sql_query(
            "INSERT INTO entitlement
             SELECT (jsonb_populate_record(NULL::entitlement, to_jsonb(e) || jsonb_build_object(
                 'id', n.id, 'lineage_id', n.id, 'entity_id', $2::uuid, 'lineage_anchor',
                 LEAST(COALESCE(e.valid_from, e.lineage_created_at), e.lineage_created_at)))).*
             FROM entitlement e, LATERAL (SELECT gen_random_uuid() AS id) n
             WHERE e.tenant_id = $3
               AND e.entity_type = 'PLAN_VERSION'
               AND e.entity_id = $1
               AND to_jsonb(e) ->> 'revoked_at' IS NULL
               AND e.feature_id NOT IN (SELECT id FROM feature WHERE tenant_id = $3 AND status = 'ARCHIVED')",
        )
        .bind::<diesel::sql_types::Uuid, _>(src_plan_version_id.as_uuid())
        .bind::<diesel::sql_types::Uuid, _>(dst_plan_version_id.as_uuid())
        .bind::<diesel::sql_types::Uuid, _>(param_tenant_id.as_uuid())
        .execute(conn)
        .await
        .attach("Error while cloning entitlements for plan version")
        .into_db_result()
    }

    /// Locks the lineage's open row until the transaction ends.
    pub async fn lock_open(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_lineage_id: EntitlementId,
    ) -> DbResult<EntitlementRow> {
        use crate::schema::entitlement::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        e_dsl::entitlement
            .filter(e_dsl::tenant_id.eq(param_tenant_id))
            .filter(e_dsl::lineage_id.eq(param_lineage_id))
            .filter(e_dsl::feature_id.ne_all(archived_feature_ids(param_tenant_id)))
            .filter(e_dsl::revoked_at.is_null())
            .select(EntitlementRow::as_select())
            .for_update()
            .first(conn)
            .await
            .attach("Error while locking an entitlement lineage")
            .into_db_result()
    }

    /// Whether the lineage has an open row, as committed when the statement starts.
    pub async fn has_open(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_lineage_id: EntitlementId,
    ) -> DbResult<bool> {
        use crate::schema::entitlement::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        diesel::select(diesel::dsl::exists(
            e_dsl::entitlement
                .filter(e_dsl::tenant_id.eq(param_tenant_id))
                .filter(e_dsl::lineage_id.eq(param_lineage_id))
                .filter(e_dsl::feature_id.ne_all(archived_feature_ids(param_tenant_id)))
                .filter(e_dsl::revoked_at.is_null()),
        ))
        .get_result(conn)
        .await
        .attach("Error while reading an entitlement lineage")
        .into_db_result()
    }

    /// Locks the lineage's open row and revokes it at the transaction's time.
    pub async fn revoke_open(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_lineage_id: EntitlementId,
        param_revoked_by: Option<Uuid>,
    ) -> DbResult<EntitlementRow> {
        use crate::schema::entitlement::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        let open = Self::lock_open(conn, param_tenant_id, param_lineage_id).await?;
        let revoked: EntitlementRow = diesel::update(e_dsl::entitlement)
            .filter(e_dsl::id.eq(open.id))
            .set((
                e_dsl::revoked_at.eq(common_clock::now()),
                e_dsl::revoked_by.eq(param_revoked_by),
            ))
            .returning(EntitlementRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while revoking an entitlement")
            .into_db_result()?;
        mark_owners(conn, std::slice::from_ref(&revoked)).await?;
        Ok(revoked)
    }

    /// Every row of the lineage, oldest first; the last one is open unless it was revoked.
    pub async fn list_history(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_lineage_id: EntitlementId,
    ) -> DbResult<Vec<EntitlementRow>> {
        use crate::schema::entitlement::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        e_dsl::entitlement
            .filter(e_dsl::tenant_id.eq(param_tenant_id))
            .filter(e_dsl::lineage_id.eq(param_lineage_id))
            .filter(e_dsl::feature_id.ne_all(archived_feature_ids(param_tenant_id)))
            .order((e_dsl::created_at.asc(), e_dsl::revoked_at.asc()))
            .select(EntitlementRow::as_select())
            .get_results(conn)
            .await
            .attach("Error while listing an entitlement's history")
            .into_db_result()
    }
}

impl EntitlementRow {
    /// Deletes every version of `lineages` the owner holds: rows of a draft plan version, never
    /// published, keep no history.
    pub async fn delete_lineages(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        owner: EntitlementEntityId,
        lineages: &[EntitlementId],
    ) -> DbResult<Vec<EntitlementRow>> {
        use crate::schema::entitlement::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        if lineages.is_empty() {
            return Ok(vec![]);
        }
        let query = diesel::delete(e_dsl::entitlement)
            .filter(e_dsl::tenant_id.eq(param_tenant_id))
            .filter(e_dsl::entity_type.eq(EntitlementEntityTypeEnum::from(&owner)))
            .filter(e_dsl::entity_id.eq(owner.as_uuid()))
            .filter(e_dsl::lineage_id.eq_any(lineages))
            .returning(EntitlementRow::as_returning());
        crate::debug_sql(&query);
        let deleted: Vec<EntitlementRow> = query
            .get_results(conn)
            .await
            .attach("Error while deleting entitlement lineages")
            .into_db_result()?;
        mark_owners(conn, &deleted).await?;
        Ok(deleted)
    }

    /// Deletes every row of the owner, revoked ones included, for an owner being deleted.
    pub async fn delete_all_of_owner(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        owner: EntitlementEntityId,
    ) -> DbResult<usize> {
        use crate::schema::entitlement::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(e_dsl::entitlement)
            .filter(e_dsl::tenant_id.eq(param_tenant_id))
            .filter(e_dsl::entity_type.eq(EntitlementEntityTypeEnum::from(&owner)))
            .filter(e_dsl::entity_id.eq(owner.as_uuid()));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while deleting an owner's entitlements")
            .into_db_result()
    }
}

/// The products each subscription's plan version includes without a price, as
/// `(subscription, product)`: a price-less component the subscription did not exclude brings its
/// product's features, never a fee.
pub async fn included_products_of_subscriptions(
    conn: &mut PgConn,
    param_tenant_id: TenantId,
    subscription_ids: &[SubscriptionId],
) -> DbResult<Vec<(SubscriptionId, ProductId)>> {
    use diesel::sql_types;
    use diesel_async::RunQueryDsl;

    #[derive(diesel::QueryableByName)]
    struct Included {
        #[diesel(sql_type = sql_types::Uuid)]
        subscription_id: SubscriptionId,
        #[diesel(sql_type = sql_types::Uuid)]
        product_id: ProductId,
    }

    if subscription_ids.is_empty() {
        return Ok(vec![]);
    }
    let ids: Vec<Uuid> = subscription_ids.iter().map(|id| id.as_uuid()).collect();
    let query = diesel::sql_query(
        "SELECT DISTINCT s.id AS subscription_id, pc.product_id \
         FROM subscription s \
         JOIN price_component pc ON pc.plan_version_id = s.plan_version_id \
         WHERE s.id = ANY($1) AND s.tenant_id = $2 \
           AND pc.product_id IS NOT NULL AND pc.legacy_fee IS NULL \
           AND NOT EXISTS (SELECT 1 FROM plan_component_price pcp \
                           WHERE pcp.plan_component_id = pc.id) \
           AND NOT EXISTS (SELECT 1 FROM subscription_excluded_component x \
                           WHERE x.subscription_id = s.id AND x.price_component_id = pc.id)",
    )
    .bind::<sql_types::Array<sql_types::Uuid>, _>(ids)
    .bind::<sql_types::Uuid, _>(param_tenant_id.as_uuid());
    crate::debug_sql(&query);
    query
        .get_results::<Included>(conn)
        .await
        .attach("Error while listing the products subscriptions include with no price")
        .into_db_result()
        .map(|rows| {
            rows.into_iter()
                .map(|row| (row.subscription_id, row.product_id))
                .collect()
        })
}

/// How each of `add_on_ids` bills, archived ones included: a subscription keeps an add-on
/// archived since, and a seat add-on's rows count its seats.
pub async fn add_on_fee_structures(
    conn: &mut PgConn,
    param_tenant_id: TenantId,
    add_on_ids: &[common_domain::ids::AddOnId],
) -> DbResult<Vec<(common_domain::ids::AddOnId, serde_json::Value)>> {
    use crate::schema::add_on::dsl as ao_dsl;
    use diesel_async::RunQueryDsl;

    if add_on_ids.is_empty() {
        return Ok(vec![]);
    }
    let query = ao_dsl::add_on
        .filter(ao_dsl::tenant_id.eq(param_tenant_id))
        .filter(ao_dsl::id.eq_any(add_on_ids))
        .select((ao_dsl::id, ao_dsl::fee_structure));
    crate::debug_sql(&query);
    query
        .load(conn)
        .await
        .attach("Error while reading how add-ons bill")
        .into_db_result()
}
