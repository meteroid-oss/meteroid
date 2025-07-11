use crate::errors::IntoDbResult;

use crate::slot_transactions::SlotTransactionRow;
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;

use common_domain::ids::SubscriptionId;
use diesel::{OptionalExtension, sql_types};
use diesel::{QueryableByName, debug_query};
use error_stack::ResultExt;

impl SlotTransactionRow {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SlotTransactionRow> {
        use crate::schema::slot_transaction::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(slot_transaction).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting slot transaction")
            .into_db_result()
    }

    pub async fn insert_batch(conn: &mut PgConn, items: Vec<&SlotTransactionRow>) -> DbResult<()> {
        use crate::schema::slot_transaction::dsl::*;
        use diesel_async::RunQueryDsl;

        if items.is_empty() {
            return Ok(());
        }

        let query = diesel::insert_into(slot_transaction).values(items);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while inserting slot transaction")
            .into_db_result()
            .map(|_| ())
    }

    pub async fn fetch_by_subscription_id_and_unit(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        // TODO product instead ?
        unit: String,
        at_ts: Option<NaiveDateTime>,
    ) -> DbResult<FetchTransactionResult> {
        use diesel_async::RunQueryDsl;

        let ts = at_ts.unwrap_or_else(|| chrono::Utc::now().naive_utc());

        let raw_sql = r#"
WITH RankedSlotTransactions AS (
    SELECT
        st.*,
        ROW_NUMBER() OVER (PARTITION BY st.subscription_id, st.unit ORDER BY st.transaction_at DESC) AS row_num
    FROM
        slot_transaction st
    WHERE
        st.subscription_id = $1
        AND st.unit = $2
        AND st.transaction_at <= $3
)
SELECT
    (X.prev_active_slots + COALESCE(SUM(Y.delta), 0))::integer AS current_active_slots
FROM
    RankedSlotTransactions X
    LEFT JOIN
    RankedSlotTransactions Y ON Y.effective_at BETWEEN X.transaction_at AND $4
WHERE
    X.row_num = 1
GROUP BY
    X.prev_active_slots;
"#;

        let query = diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(subscription_uid)
            .bind::<sql_types::VarChar, _>(unit)
            .bind::<sql_types::Timestamp, _>(ts)
            .bind::<sql_types::Timestamp, _>(ts);

        log::info!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result::<FetchTransactionResult>(conn)
            .await
            .optional()
            .map(|d| {
                d.unwrap_or(FetchTransactionResult {
                    current_active_slots: 0,
                })
            })
            .attach_printable("Error while fetching slot transaction by id")
            .into_db_result()
    }
}

#[derive(QueryableByName)]
pub struct FetchTransactionResult {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub current_active_slots: i32,
}
