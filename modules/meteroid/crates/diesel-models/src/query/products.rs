use crate::errors::IntoDbResult;
use crate::products::{ProductRow, ProductRowNew, ProductRowPatch};

use crate::{DbResult, PgConn};

use crate::extend::order::{OrderByParam, OrderDirection};
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use common_domain::ids::{PlanId, PlanVersionId, ProductFamilyId, ProductId, TenantId};
use diesel::{
    AggregateExpressionMethods, BoolExpressionMethods, ExpressionMethods, JoinOnDsl,
    NullableExpressionMethods, PgTextExpressionMethods, QueryDsl, SelectableHelper,
};
use error_stack::ResultExt;
use std::collections::HashMap;

/// The `type` tag of the fee structure JSON of each of `fee_types`.
fn structure_types(fee_types: &[crate::enums::FeeTypeEnum]) -> Vec<String> {
    use crate::enums::FeeTypeEnum;
    fee_types
        .iter()
        .map(|fee_type| {
            match fee_type {
                FeeTypeEnum::Rate => "Rate",
                FeeTypeEnum::Slot => "Slot",
                FeeTypeEnum::Capacity => "Capacity",
                FeeTypeEnum::Usage => "Usage",
                FeeTypeEnum::ExtraRecurring => "ExtraRecurring",
                FeeTypeEnum::OneTime => "OneTime",
                FeeTypeEnum::Credit => "Credit",
            }
            .to_owned()
        })
        .collect()
}

/// Products a plan charge or an add-on bills in one of the structure types bound after each
/// part: a charge owns how its product is billed.
const CHARGED_AS: [&str; 3] = [
    "\"product\".\"id\" IN (SELECT pc.product_id FROM price_component pc \
     WHERE pc.product_id IS NOT NULL AND pc.fee_structure ->> 'type' = ANY(",
    ") UNION SELECT a.product_id FROM add_on a WHERE a.fee_structure ->> 'type' = ANY(",
    "))",
];

impl ProductRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<ProductRow> {
        use crate::schema::product::dsl::product;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(product).values(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting product")
            .into_db_result()
    }
}

impl ProductRow {
    pub async fn find_by_id_and_tenant_id(
        conn: &mut PgConn,
        id: ProductId,
        tenant_id: TenantId,
    ) -> DbResult<ProductRow> {
        use crate::schema::product::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::product
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id));

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding product by id and tenant id")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        catalog_only: bool,
        fee_types: &[crate::enums::FeeTypeEnum],
        pagination: PaginationRequest,
        order_by: Option<&str>,
    ) -> DbResult<PaginatedVec<ProductRow>> {
        use crate::schema::product::dsl as p_dsl;
        use crate::schema::product_family::dsl as pf_dsl;

        let mut query = p_dsl::product
            .inner_join(pf_dsl::product_family.on(p_dsl::product_family_id.eq(pf_dsl::id)))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(p_dsl::archived_at.is_null())
            .into_boxed();

        if catalog_only {
            query = query.filter(p_dsl::catalog.eq(true));
        }

        if let Some(family_id) = family_id {
            query = query.filter(pf_dsl::id.eq(family_id));
        }

        if !fee_types.is_empty() {
            let kinds = structure_types(fee_types);
            query = query.filter(
                diesel::dsl::sql::<diesel::sql_types::Bool>(CHARGED_AS[0])
                    .bind::<diesel::sql_types::Array<diesel::sql_types::Text>, _>(kinds.clone())
                    .sql(CHARGED_AS[1])
                    .bind::<diesel::sql_types::Array<diesel::sql_types::Text>, _>(kinds)
                    .sql(CHARGED_AS[2]),
            );
        }

        let mut query = query.select(ProductRow::as_select());

        let order = OrderByParam::parse(order_by, "name.asc");

        match (order.column.as_str(), order.direction) {
            ("name", OrderDirection::Asc) => {
                query = query.order((p_dsl::name.asc(), p_dsl::id.asc()))
            }
            ("name", OrderDirection::Desc) => {
                query = query.order((p_dsl::name.desc(), p_dsl::id.desc()))
            }
            ("created_at", OrderDirection::Asc) => {
                query = query.order((p_dsl::created_at.asc(), p_dsl::id.asc()))
            }
            ("created_at", OrderDirection::Desc) => {
                query = query.order((p_dsl::created_at.desc(), p_dsl::id.desc()))
            }
            _ => query = query.order((p_dsl::name.asc(), p_dsl::id.asc())),
        }

        let paginated_query = query.paginate(pagination);

        crate::debug_sql(&paginated_query);

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while fetching products")
            .into_db_result()
    }

    /// The tenant's products after `after` in id order, at most `limit` (API v2 keyset pages).
    pub async fn list_page(
        conn: &mut PgConn,
        tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        include_archived: bool,
        after: Option<ProductId>,
        limit: i64,
    ) -> DbResult<Vec<ProductRow>> {
        use crate::schema::product::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = p_dsl::product
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .into_boxed();
        if !include_archived {
            query = query.filter(p_dsl::archived_at.is_null());
        }
        if let Some(family_id) = family_id {
            query = query.filter(p_dsl::product_family_id.eq(family_id));
        }
        if let Some(after) = after {
            query = query.filter(p_dsl::id.gt(after));
        }
        let query = query
            .order(p_dsl::id.asc())
            .limit(limit)
            .select(ProductRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while fetching a page of products")
            .into_db_result()
    }

    pub async fn search(
        conn: &mut PgConn,
        tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        query: &str,
        catalog_only: bool,
        archived: bool,
        fee_types: &[crate::enums::FeeTypeEnum],
        pagination: PaginationRequest,
        order_by: Option<&str>,
    ) -> DbResult<PaginatedVec<ProductRow>> {
        use crate::schema::product::dsl as p_dsl;
        use crate::schema::product_family::dsl as pf_dsl;

        let mut query = p_dsl::product
            .inner_join(pf_dsl::product_family.on(p_dsl::product_family_id.eq(pf_dsl::id)))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(p_dsl::name.ilike(format!("%{query}%")))
            .into_boxed();

        query = if archived {
            query.filter(p_dsl::archived_at.is_not_null())
        } else {
            query.filter(p_dsl::archived_at.is_null())
        };

        if catalog_only {
            query = query.filter(p_dsl::catalog.eq(true));
        }

        if let Some(family_id) = family_id {
            query = query.filter(pf_dsl::id.eq(family_id));
        }

        if !fee_types.is_empty() {
            let kinds = structure_types(fee_types);
            query = query.filter(
                diesel::dsl::sql::<diesel::sql_types::Bool>(CHARGED_AS[0])
                    .bind::<diesel::sql_types::Array<diesel::sql_types::Text>, _>(kinds.clone())
                    .sql(CHARGED_AS[1])
                    .bind::<diesel::sql_types::Array<diesel::sql_types::Text>, _>(kinds)
                    .sql(CHARGED_AS[2]),
            );
        }

        let mut query = query.select(ProductRow::as_select());

        let order = OrderByParam::parse(order_by, "name.asc");

        match (order.column.as_str(), order.direction) {
            ("name", OrderDirection::Asc) => {
                query = query.order((p_dsl::name.asc(), p_dsl::id.asc()))
            }
            ("name", OrderDirection::Desc) => {
                query = query.order((p_dsl::name.desc(), p_dsl::id.desc()))
            }
            ("created_at", OrderDirection::Asc) => {
                query = query.order((p_dsl::created_at.asc(), p_dsl::id.asc()))
            }
            ("created_at", OrderDirection::Desc) => {
                query = query.order((p_dsl::created_at.desc(), p_dsl::id.desc()))
            }
            _ => query = query.order((p_dsl::name.asc(), p_dsl::id.asc())),
        }

        let paginated_query = query.paginate(pagination);

        crate::debug_sql(&paginated_query);

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while fetching products")
            .into_db_result()
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        ids: &[ProductId],
        tenant_id: TenantId,
    ) -> DbResult<Vec<ProductRow>> {
        use crate::schema::product::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        if ids.is_empty() {
            return Ok(vec![]);
        }

        let query = p_dsl::product
            .filter(p_dsl::id.eq_any(ids))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .select(ProductRow::as_select());

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing products by ids")
            .into_db_result()
    }

    pub async fn archive(
        conn: &mut PgConn,
        product_id: ProductId,
        param_tenant_id: TenantId,
    ) -> DbResult<ProductRow> {
        use crate::schema::product::dsl as p_dsl;

        use diesel_async::RunQueryDsl;

        let query = diesel::update(p_dsl::product)
            .filter(p_dsl::id.eq(product_id))
            .filter(p_dsl::tenant_id.eq(param_tenant_id))
            .set((
                p_dsl::archived_at.eq(Some(common_clock::now().naive_utc())),
                p_dsl::updated_at.eq(common_clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while archiving product")
            .into_db_result()
    }

    pub async fn unarchive(
        conn: &mut PgConn,
        product_id: ProductId,
        param_tenant_id: TenantId,
    ) -> DbResult<ProductRow> {
        use crate::schema::product::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(p_dsl::product)
            .filter(p_dsl::id.eq(product_id))
            .filter(p_dsl::tenant_id.eq(param_tenant_id))
            .set((
                p_dsl::archived_at.eq::<Option<chrono::NaiveDateTime>>(None),
                p_dsl::updated_at.eq(common_clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while unarchiving product")
            .into_db_result()
    }

    pub async fn list_all_by_tenant(
        conn: &mut PgConn,
        tenant_id: TenantId,
        catalog_only: bool,
    ) -> DbResult<Vec<ProductRow>> {
        use crate::schema::product::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = p_dsl::product
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(p_dsl::archived_at.is_null())
            .into_boxed();

        if catalog_only {
            query = query.filter(p_dsl::catalog.eq(true));
        }

        let query = query.select(ProductRow::as_select());

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing all products by tenant")
            .into_db_result()
    }

    /// How many live plans sell each product: its charges in a plan's active or draft version.
    pub async fn plan_counts(
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_ids: &[ProductId],
    ) -> DbResult<HashMap<ProductId, i64>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::price_component::dsl as pc_dsl;
        use diesel_async::RunQueryDsl;

        if product_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let query = pc_dsl::price_component
            .inner_join(pv_dsl::plan_version.on(pc_dsl::plan_version_id.eq(pv_dsl::id)))
            .inner_join(p_dsl::plan.on(pv_dsl::plan_id.eq(p_dsl::id)))
            .filter(pc_dsl::product_id.eq_any(product_ids))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(p_dsl::archived_at.is_null())
            .filter(
                p_dsl::active_version_id
                    .eq(pv_dsl::id.nullable())
                    .or(p_dsl::draft_version_id.eq(pv_dsl::id.nullable())),
            )
            .group_by(pc_dsl::product_id)
            .select((
                pc_dsl::product_id,
                diesel::dsl::count(p_dsl::id).aggregate_distinct(),
            ));

        crate::debug_sql(&query);

        query
            .load::<(Option<ProductId>, i64)>(conn)
            .await
            .attach("Error while counting plans by product")
            .into_db_result()
            .map(|rows| {
                rows.into_iter()
                    .filter_map(|(id, count)| id.map(|id| (id, count)))
                    .collect()
            })
    }

    /// The versions selling the product: a charge of it in a live plan's active or draft
    /// version. By plan name, the active version before the draft.
    pub async fn live_plan_versions(
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_id: ProductId,
    ) -> DbResult<Vec<ProductPlanVersionRow>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::price_component::dsl as pc_dsl;
        use diesel_async::RunQueryDsl;

        let query = pc_dsl::price_component
            .inner_join(pv_dsl::plan_version.on(pc_dsl::plan_version_id.eq(pv_dsl::id)))
            .inner_join(p_dsl::plan.on(pv_dsl::plan_id.eq(p_dsl::id)))
            .filter(pc_dsl::product_id.eq(product_id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(p_dsl::archived_at.is_null())
            .filter(
                p_dsl::active_version_id
                    .eq(pv_dsl::id.nullable())
                    .or(p_dsl::draft_version_id.eq(pv_dsl::id.nullable())),
            )
            .select((
                p_dsl::id,
                p_dsl::name,
                pv_dsl::id,
                pv_dsl::version,
                pv_dsl::is_draft_version,
            ))
            .distinct()
            .order((
                p_dsl::name.asc(),
                p_dsl::id.asc(),
                pv_dsl::is_draft_version.asc(),
            ));

        crate::debug_sql(&query);

        query
            .load::<(PlanId, String, PlanVersionId, i32, bool)>(conn)
            .await
            .attach("Error while listing the plans selling a product")
            .into_db_result()
            .map(|rows| {
                rows.into_iter()
                    .map(|(plan_id, plan_name, plan_version_id, version, is_draft)| {
                        ProductPlanVersionRow {
                            plan_id,
                            plan_name,
                            plan_version_id,
                            version,
                            is_draft,
                        }
                    })
                    .collect()
            })
    }
}

/// A live plan version with a charge of a product.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductPlanVersionRow {
    pub plan_id: PlanId,
    pub plan_name: String,
    pub plan_version_id: PlanVersionId,
    pub version: i32,
    pub is_draft: bool,
}

impl ProductRowPatch {
    pub async fn patch(&self, conn: &mut PgConn) -> DbResult<ProductRow> {
        use crate::schema::product::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(p_dsl::product)
            .filter(p_dsl::id.eq(self.id))
            .filter(p_dsl::tenant_id.eq(self.tenant_id))
            .set((self, p_dsl::updated_at.eq(common_clock::now().naive_utc())));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while updating product")
            .into_db_result()
    }
}
