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
        use crate::schema::billable_metric::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(billable_metric).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting billable metric")
            .into_db_result()
    }
}

impl BillableMetricRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        param_billable_metric_id: BillableMetricId,
        param_tenant_id: TenantId,
    ) -> DbResult<BillableMetricRow> {
        use crate::schema::billable_metric::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = billable_metric
            .filter(id.eq(param_billable_metric_id))
            .filter(tenant_id.eq(param_tenant_id));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding billable metric by id")
            .into_db_result()
    }

    pub async fn get_by_ids(
        conn: &mut PgConn,
        metric_ids: &[BillableMetricId],
        tenant_id_param: &TenantId,
    ) -> DbResult<Vec<BillableMetricRow>> {
        use crate::schema::billable_metric::dsl::*;
        use diesel_async::RunQueryDsl;

        billable_metric
            .filter(id.eq_any(metric_ids))
            .filter(tenant_id.eq(tenant_id_param))
            .get_results(conn)
            .await
            .attach_printable("Error while fetching billable metrics")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        pagination: PaginationRequest,
        param_product_family_id: Option<ProductFamilyId>,
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

        let query = query
            .order(bm_dsl::created_at.asc())
            .select(BillableMetricMetaRow::as_select());

        let paginated_query = query.paginate(pagination);

        log::debug!(
            "{}",
            debug_query::<diesel::pg::Pg, _>(&paginated_query).to_string()
        );

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while fetching billable metrics")
            .into_db_result()
    }
}
