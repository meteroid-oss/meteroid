use crate::email_send_claim::{EmailSendClaimRow, EmailSendClaimRowNew};
use crate::enums::EmailSendClaimStateEnum;
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use common_domain::ids::{EntityActivityId, TenantId};
use diesel::upsert::excluded;
use diesel::{
    BoolExpressionMethods, ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl EmailSendClaimRow {
    /// Wins a fresh key or a released `FAILED` claim of the same tenant; `None` when the key is
    /// `SENT`, `UNKNOWN`, still `SENDING` or foreign (the caller reads the row to tell).
    pub async fn claim(
        conn: &mut PgConn,
        tenant_id: TenantId,
        dedupe_key: &str,
        now: NaiveDateTime,
    ) -> DbResult<Option<EmailSendClaimRow>> {
        use crate::schema::email_send_claim::dsl as c_dsl;

        let row = EmailSendClaimRowNew {
            dedupe_key: dedupe_key.to_string(),
            tenant_id,
            state: EmailSendClaimStateEnum::Sending,
            claimed_at: now,
        };
        let upsert = diesel::insert_into(c_dsl::email_send_claim)
            .values(&row)
            .on_conflict(c_dsl::dedupe_key)
            .do_update()
            .set((
                c_dsl::state.eq(EmailSendClaimStateEnum::Sending),
                c_dsl::claimed_at.eq(excluded(c_dsl::claimed_at)),
                c_dsl::finished_at.eq(None::<NaiveDateTime>),
            ));
        let query = diesel::query_dsl::methods::FilterDsl::filter(
            upsert,
            c_dsl::state
                .eq(EmailSendClaimStateEnum::Failed)
                .and(c_dsl::tenant_id.eq(tenant_id)),
        )
        .returning(EmailSendClaimRow::as_returning());
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while claiming email_send_claim")
            .into_db_result()
    }

    pub async fn find_by_dedupe_key(
        conn: &mut PgConn,
        dedupe_key: &str,
    ) -> DbResult<Option<EmailSendClaimRow>> {
        use crate::schema::email_send_claim::dsl as c_dsl;

        let query = c_dsl::email_send_claim
            .filter(c_dsl::dedupe_key.eq(dedupe_key))
            .select(EmailSendClaimRow::as_select());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding email_send_claim")
            .into_db_result()
    }

    /// `SENT` with the recorded email, or `FAILED` (released, re-claimable) with none. Only a
    /// `SENDING` claim finishes: a terminal `UNKNOWN` is never overwritten.
    pub async fn finish(
        conn: &mut PgConn,
        tenant_id: TenantId,
        dedupe_key: &str,
        state: EmailSendClaimStateEnum,
        sent_email_id: Option<EntityActivityId>,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::email_send_claim::dsl as c_dsl;

        let query = diesel::update(c_dsl::email_send_claim)
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(c_dsl::dedupe_key.eq(dedupe_key))
            .filter(c_dsl::state.eq(EmailSendClaimStateEnum::Sending))
            .set((
                c_dsl::state.eq(state),
                c_dsl::finished_at.eq(now),
                c_dsl::sent_email_id.eq(sent_email_id),
            ));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while finishing email_send_claim")
            .into_db_result()
    }

    /// The only writer of `UNKNOWN`: a `SENDING` claim won before `stale_before` is never
    /// resent. A fresh one (re-won meanwhile) is left to its sender.
    pub async fn mark_unknown(
        conn: &mut PgConn,
        tenant_id: TenantId,
        dedupe_key: &str,
        stale_before: NaiveDateTime,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::email_send_claim::dsl as c_dsl;

        let query = diesel::update(c_dsl::email_send_claim)
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(c_dsl::dedupe_key.eq(dedupe_key))
            .filter(c_dsl::state.eq(EmailSendClaimStateEnum::Sending))
            .filter(c_dsl::claimed_at.le(stale_before))
            .set((
                c_dsl::state.eq(EmailSendClaimStateEnum::Unknown),
                c_dsl::finished_at.eq(now),
            ));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while marking email_send_claim unknown")
            .into_db_result()
    }
}
