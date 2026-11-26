use crate::dunning_policy::DunningPolicyRow;
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::TenantId;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl DunningPolicyRow {
    pub async fn find(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<Option<Self>> {
        use crate::schema::dunning_policy::dsl as d;
        d::dunning_policy
            .filter(d::tenant_id.eq(tenant_id))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while getting dunning_policy")
            .into_db_result()
    }

    pub async fn upsert(&self, conn: &mut PgConn) -> DbResult<Self> {
        use crate::schema::dunning_policy::dsl as d;
        diesel::insert_into(d::dunning_policy)
            .values(self)
            .on_conflict(d::tenant_id)
            .do_update()
            .set((
                d::retry_days.eq(&self.retry_days),
                d::exhausted_action.eq(self.exhausted_action),
                d::updated_at.eq(self.updated_at),
            ))
            .returning(Self::as_returning())
            .get_result(conn)
            .await
            .attach("Error while upserting dunning_policy")
            .into_db_result()
    }
}
