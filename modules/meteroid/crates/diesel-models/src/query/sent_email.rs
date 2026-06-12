use crate::errors::IntoDbResult;
use crate::sent_email::{SentEmailRow, SentEmailRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{EntityActivityId, TenantId};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper, debug_query};
use error_stack::ResultExt;

impl SentEmailRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SentEmailRow> {
        use crate::schema::sent_email::dsl::sent_email;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(sent_email).values(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting sent_email")
            .into_db_result()
    }
}

impl SentEmailRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_id: EntityActivityId,
    ) -> DbResult<SentEmailRow> {
        use crate::schema::sent_email::dsl as se;
        use diesel_async::RunQueryDsl;

        se::sent_email
            .filter(se::id.eq(param_id))
            .filter(se::tenant_id.eq(param_tenant_id))
            .select(SentEmailRow::as_select())
            .get_result(conn)
            .await
            .attach("Error while finding sent_email by id")
            .into_db_result()
    }
}
