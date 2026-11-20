use crate::errors::IntoDbResult;
use crate::quote_views::QuoteViewRow;
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, QuoteId, QuoteViewId, TenantId};
use diesel::sql_types;
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl QuoteViewRow {
    /// Extends the person's latest session when it was last seen at or after `new_session_before`,
    /// otherwise starts a new one.
    #[allow(clippy::too_many_arguments)]
    pub async fn record(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
        email: &str,
        revision: i32,
        seen_at: NaiveDateTime,
        new_session_before: NaiveDateTime,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
    ) -> DbResult<()> {
        diesel::sql_query(
            "WITH latest AS (
                 SELECT id, last_seen_at FROM quote_view
                 WHERE tenant_id = $1 AND quote_id = $2 AND lower(email) = lower($3)
                 ORDER BY viewed_at DESC
                 LIMIT 1
             ), extended AS (
                 UPDATE quote_view SET last_seen_at = $5
                 FROM latest
                 WHERE quote_view.id = latest.id AND latest.last_seen_at >= $6
                 RETURNING quote_view.id
             )
             INSERT INTO quote_view
                 (id, tenant_id, quote_id, email, revision, viewed_at, last_seen_at, ip_address,
                  user_agent)
             SELECT $7, $1, $2, $3, $4, $5, $5, $8, $9
             WHERE NOT EXISTS (SELECT 1 FROM extended)",
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Uuid, _>(quote_id)
        .bind::<sql_types::Text, _>(email)
        .bind::<sql_types::Integer, _>(revision)
        .bind::<sql_types::Timestamptz, _>(seen_at)
        .bind::<sql_types::Timestamptz, _>(new_session_before)
        .bind::<sql_types::Uuid, _>(QuoteViewId::new())
        .bind::<sql_types::Nullable<sql_types::Text>, _>(ip_address)
        .bind::<sql_types::Nullable<sql_types::Text>, _>(user_agent)
        .execute(conn)
        .await
        .map(|_| ())
        .attach("Error while recording a quote view session")
        .into_db_result()
    }

    /// The latest `per_person` sessions of each person, newest first.
    pub async fn list_latest_by_quote_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
        per_person: i64,
    ) -> DbResult<Vec<QuoteViewRow>> {
        diesel::sql_query(
            "SELECT id, tenant_id, quote_id, email, revision, viewed_at, last_seen_at, ip_address,
                    user_agent
             FROM (
                 SELECT v.*,
                        row_number() OVER (PARTITION BY lower(v.email) ORDER BY v.viewed_at DESC) AS rn
                 FROM quote_view v
                 WHERE v.tenant_id = $1 AND v.quote_id = $2
             ) s
             WHERE s.rn <= $3
             ORDER BY s.viewed_at DESC",
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Uuid, _>(quote_id)
        .bind::<sql_types::BigInt, _>(per_person)
        .get_results(conn)
        .await
        .attach("Error while listing quote view sessions")
        .into_db_result()
    }
}
