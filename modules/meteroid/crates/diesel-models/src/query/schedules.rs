use crate::errors::IntoDbResult;
use crate::schedules::{SchedulePatchRow, ScheduleRow, ScheduleRowNew};
use crate::schema::schedule;
use crate::{DbResult, PgConn};

use error_stack::ResultExt;

use diesel::{debug_query, ExpressionMethods, Insertable, JoinOnDsl, QueryDsl, SelectableHelper};

impl ScheduleRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<ScheduleRow> {
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

impl ScheduleRow {
    pub async fn delete(
        conn: &mut PgConn,
        id: uuid::Uuid,
        tenant_id: uuid::Uuid,
    ) -> DbResult<usize> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::schedule::dsl as s_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(s_dsl::schedule)
            .filter(s_dsl::id.eq(id))
            .filter(
                s_dsl::plan_version_id.eq_any(
                    pv_dsl::plan_version
                        .select(pv_dsl::id)
                        .filter(pv_dsl::tenant_id.eq(tenant_id))
                        .filter(pv_dsl::is_draft_version.eq(true)),
                ),
            );

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while deleting schedule")
            .into_db_result()
    }

    pub async fn insert_schedule_batch(
        conn: &mut PgConn,
        batch: Vec<ScheduleRowNew>,
    ) -> DbResult<Vec<ScheduleRow>> {
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
    ) -> DbResult<Vec<ScheduleRow>> {
        use crate::schema::schedule::dsl as schedule_dsl;
        use crate::schema::subscription;

        use diesel_async::RunQueryDsl;

        let query = schedule_dsl::schedule
            .inner_join(
                subscription::table.on(schedule::plan_version_id.eq(subscription::plan_version_id)),
            )
            .filter(subscription::id.eq(subscription_id))
            .filter(subscription::tenant_id.eq(tenant_id_params))
            .select(ScheduleRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching schedules by subscription")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        plan_version_id: uuid::Uuid,
        tenant_id: uuid::Uuid,
    ) -> DbResult<Vec<ScheduleRow>> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::schedule::dsl as s_dsl;

        use diesel_async::RunQueryDsl;

        let query = s_dsl::schedule
            .inner_join(pv_dsl::plan_version.on(s_dsl::plan_version_id.eq(pv_dsl::id)))
            .filter(pv_dsl::id.eq(plan_version_id))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .select(ScheduleRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching schedules")
            .into_db_result()
    }

    pub async fn clone_all(
        conn: &mut PgConn,
        src_plan_version_id: uuid::Uuid,
        dst_plan_version_id: uuid::Uuid,
    ) -> DbResult<usize> {
        use crate::schema::schedule::dsl as s_dsl;
        use diesel_async::RunQueryDsl;

        diesel::sql_function! {
            fn gen_random_uuid() -> Uuid;
        }

        let query = s_dsl::schedule
            .filter(s_dsl::plan_version_id.eq(src_plan_version_id))
            .select((
                gen_random_uuid(),
                s_dsl::billing_period,
                diesel::dsl::sql::<diesel::sql_types::Uuid>(
                    format!("'{}'", dst_plan_version_id).as_str(),
                ),
                s_dsl::ramps,
            ))
            .insert_into(s_dsl::schedule)
            .into_columns((
                s_dsl::id,
                s_dsl::billing_period,
                s_dsl::plan_version_id,
                s_dsl::ramps,
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while cloning schedules")
            .into_db_result()
    }
}

impl SchedulePatchRow {
    pub async fn update(&self, conn: &mut PgConn, tenant_id: uuid::Uuid) -> DbResult<ScheduleRow> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::schedule::dsl as s_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(s_dsl::schedule)
            .filter(s_dsl::id.eq(self.id))
            .filter(
                s_dsl::plan_version_id.eq_any(
                    pv_dsl::plan_version
                        .select(pv_dsl::id)
                        .filter(pv_dsl::tenant_id.eq(tenant_id))
                        .filter(pv_dsl::is_draft_version.eq(true)),
                ),
            )
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .map_err(|e| {
                log::error!("Error while updating schedule: {:?}", e);
                e
            })
            .attach_printable("Error while updating schedule")
            .into_db_result()
    }
}
