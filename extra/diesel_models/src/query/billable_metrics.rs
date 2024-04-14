use crate::billable_metrics::{BillableMetric, BillableMetricNew};
use crate::errors::IntoDbResult;

use crate::{DbResult, PgConn};

use diesel::debug_query;
use diesel::{ExpressionMethods, QueryDsl};
use error_stack::ResultExt;

impl BillableMetricNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<BillableMetric> {
        use crate::schema::billable_metric::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(billable_metric).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting billable metric")
            .into_db_result()
    }
}

impl BillableMetric {
    pub async fn get_by_ids(
        conn: &mut PgConn,
        metric_ids: &[uuid::Uuid],
        tenant_id_param: &uuid::Uuid,
    ) -> DbResult<Vec<BillableMetric>> {
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
}
