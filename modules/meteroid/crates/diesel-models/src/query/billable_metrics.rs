use crate::billable_metrics::{BillableMetricMetaRow, BillableMetricRow, BillableMetricRowNew};
use crate::errors::IntoDbResult;

use crate::{DbResult, PgConn};

use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use common_domain::ids::{BillableMetricId, ProductFamilyId, TenantId};
use diesel::{ExpressionMethods, QueryDsl};
use diesel::{JoinOnDsl, SelectableHelper, debug_query};
use error_stack::ResultExt;

impl BillableMetricRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<BillableMetricRow> {
        use crate::schema::billable_metric::dsl::billable_metric;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(billable_metric).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting billable metric")
            .into_db_result()
    }
}

impl BillableMetricRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        param_billable_metric_id: BillableMetricId,
        param_tenant_id: TenantId,
    ) -> DbResult<BillableMetricRow> {
        use crate::schema::billable_metric::dsl::{billable_metric, id, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = billable_metric
            .filter(id.eq(param_billable_metric_id))
            .filter(tenant_id.eq(param_tenant_id));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while finding billable metric by id")
            .into_db_result()
    }

    pub async fn get_by_ids(
        conn: &mut PgConn,
        metric_ids: &[BillableMetricId],
        tenant_id_param: &TenantId,
    ) -> DbResult<Vec<BillableMetricRow>> {
        use crate::schema::billable_metric::dsl::{billable_metric, id, tenant_id};
        use diesel_async::RunQueryDsl;

        billable_metric
            .filter(id.eq_any(metric_ids))
            .filter(tenant_id.eq(tenant_id_param))
            .get_results(conn)
            .await
            .attach("Error while fetching billable metrics")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        pagination: PaginationRequest,
        param_product_family_id: Option<ProductFamilyId>,
        param_archived: Option<bool>,
    ) -> DbResult<PaginatedVec<BillableMetricMetaRow>> {
        use crate::schema::billable_metric::dsl as bm_dsl;
        use crate::schema::product_family::dsl as pf_dsl;

        let mut query = bm_dsl::billable_metric
            .inner_join(pf_dsl::product_family.on(bm_dsl::product_family_id.eq(pf_dsl::id)))
            .filter(bm_dsl::tenant_id.eq(param_tenant_id))
            .into_boxed();

        if let Some(id) = param_product_family_id {
            query = query.filter(pf_dsl::id.eq(id));
        }

        query = match param_archived {
            Some(true) => query.filter(bm_dsl::archived_at.is_not_null()),
            _ => query.filter(bm_dsl::archived_at.is_null()),
        };

        let query = query
            .order(bm_dsl::created_at.asc())
            .select(BillableMetricMetaRow::as_select());

        let paginated_query = query.paginate(pagination);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&paginated_query));

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while fetching billable metrics")
            .into_db_result()
    }

    pub async fn list_by_code(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        code_param: &str,
    ) -> DbResult<Vec<BillableMetricRow>> {
        use crate::schema::billable_metric::dsl::{billable_metric, code, tenant_id};
        use diesel_async::RunQueryDsl;

        billable_metric
            .filter(tenant_id.eq(tenant_id_param))
            .filter(code.eq(code_param))
            .get_results(conn)
            .await
            .attach("Error while listing billable metrics by code")
            .into_db_result()
    }

    pub async fn archive(
        conn: &mut PgConn,
        param_billable_metric_id: BillableMetricId,
        param_tenant_id: TenantId,
    ) -> DbResult<()> {
        use crate::schema::billable_metric::dsl::{archived_at, billable_metric, id, tenant_id};
        use chrono::Utc;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(billable_metric)
            .filter(id.eq(param_billable_metric_id))
            .filter(tenant_id.eq(param_tenant_id))
            .set(archived_at.eq(Some(Utc::now().naive_utc())));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while archiving billable metric")
            .into_db_result()?;

        Ok(())
    }

    pub async fn unarchive(
        conn: &mut PgConn,
        param_billable_metric_id: BillableMetricId,
        param_tenant_id: TenantId,
    ) -> DbResult<()> {
        use crate::schema::billable_metric::dsl::{archived_at, billable_metric, id, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = diesel::update(billable_metric)
            .filter(id.eq(param_billable_metric_id))
            .filter(tenant_id.eq(param_tenant_id))
            .set(archived_at.eq::<Option<chrono::NaiveDateTime>>(None));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while unarchiving billable metric")
            .into_db_result()?;

        Ok(())
    }
}
