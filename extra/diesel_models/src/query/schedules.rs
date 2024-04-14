use crate::errors::IntoDbResult;
use crate::schedules::{Schedule, ScheduleNew};
use crate::schema::schedule;
use crate::{DbResult, PgConn};

use error_stack::ResultExt;

use diesel::{debug_query, ExpressionMethods, JoinOnDsl, QueryDsl, SelectableHelper};

impl ScheduleNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<Schedule> {
        use crate::schema::schedule::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(schedule).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting schedule")
            .into_db_result()
    }
}

impl Schedule {
    pub async fn insert_schedule_batch(
        conn: &mut PgConn,
        batch: Vec<ScheduleNew>,
    ) -> DbResult<Vec<Schedule>> {
        use crate::schema::schedule::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(schedule).values(&batch);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting schedule batch")
            .into_db_result()
    }

    pub async fn list_schedules_by_subscription(
        conn: &mut PgConn,
        tenant_id_params: &uuid::Uuid,
        subscription_id: &uuid::Uuid,
    ) -> DbResult<Vec<Schedule>> {
        use crate::schema::schedule::dsl as schedule_dsl;
        use crate::schema::subscription;

        use diesel_async::RunQueryDsl;

        let query = schedule_dsl::schedule
            .inner_join(
                subscription::table.on(schedule::plan_version_id.eq(subscription::plan_version_id)),
            )
            .filter(subscription::id.eq(subscription_id))
            .filter(subscription::tenant_id.eq(tenant_id_params))
            .select(Schedule::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching schedules by subscription")
            .into_db_result()
    }

    pub async fn list_schedules_by_plans(
        conn: &mut PgConn,
        tenant_id_params: &uuid::Uuid,
        subscription_ids: &[uuid::Uuid],
    ) -> DbResult<Vec<Schedule>> {
        use crate::schema::schedule::dsl as schedule_dsl;
        use crate::schema::subscription;

        use diesel_async::RunQueryDsl;

        let query = schedule_dsl::schedule
            .inner_join(
                subscription::table.on(schedule::plan_version_id.eq(subscription::plan_version_id)),
            )
            .filter(subscription::id.eq_any(subscription_ids))
            .filter(subscription::tenant_id.eq(tenant_id_params))
            .select(Schedule::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching schedules by subscriptions")
            .into_db_result()
    }
}
