use crate::errors::IntoDbResult;
use crate::quote_recipient_engagement::QuoteRecipientEngagementRow;
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, QuoteId, QuoteRecipientEngagementId, TenantId};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper, sql_types};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl QuoteRecipientEngagementRow {
    /// Counts a new view only when the previous one is older than `new_view_before`; any open
    /// moves the last view.
    pub async fn record_view(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
        email: &str,
        revision: i32,
        viewed_at: NaiveDateTime,
        new_view_before: NaiveDateTime,
    ) -> DbResult<()> {
        diesel::sql_query(
            "INSERT INTO quote_recipient_engagement
                 (id, tenant_id, quote_id, email, view_count, first_viewed_at, last_viewed_at,
                  last_viewed_revision)
             VALUES ($1, $2, $3, $4, 1, $5, $5, $6)
             ON CONFLICT (quote_id, lower(email)) DO UPDATE SET
                 view_count = quote_recipient_engagement.view_count
                     + CASE WHEN quote_recipient_engagement.last_viewed_at < $7 THEN 1 ELSE 0 END,
                 last_viewed_at = EXCLUDED.last_viewed_at,
                 last_viewed_revision = EXCLUDED.last_viewed_revision",
        )
        .bind::<sql_types::Uuid, _>(QuoteRecipientEngagementId::new())
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Uuid, _>(quote_id)
        .bind::<sql_types::Text, _>(email)
        .bind::<sql_types::Timestamptz, _>(viewed_at)
        .bind::<sql_types::Integer, _>(revision)
        .bind::<sql_types::Timestamptz, _>(new_view_before)
        .execute(conn)
        .await
        .map(|_| ())
        .attach("Error while recording a quote view")
        .into_db_result()
    }

    pub async fn list_by_quote_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> DbResult<Vec<QuoteRecipientEngagementRow>> {
        use crate::schema::quote_recipient_engagement::dsl as d;

        let query = d::quote_recipient_engagement
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .order(d::last_viewed_at.desc())
            .select(QuoteRecipientEngagementRow::as_select());
        crate::debug_sql(&query);
        query
            .get_results(conn)
            .await
            .attach("Error while listing quote recipient engagement")
            .into_db_result()
    }
}
