use crate::errors::IntoDbResult;

use crate::slot_transactions::SlotTransactionRow;
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;

use common_domain::ids::{PriceComponentId, SubscriptionId};
use diesel::sql_types;
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

    pub async fn fetch_by_subscription_id_and_price_component_id(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        // TODO unit instead ?
        price_component_uid: PriceComponentId,
        at_ts: Option<NaiveDateTime>,
    ) -> DbResult<FetchTransactionResult> {
        use diesel_async::RunQueryDsl;

        let ts = at_ts.unwrap_or_else(|| chrono::Utc::now().naive_utc());

        let raw_sql = r#"
WITH RankedSlotTransactions AS (
    SELECT
        st.*,
        ROW_NUMBER() OVER (PARTITION BY st.subscription_id, st.price_component_id ORDER BY st.transaction_at DESC) AS row_num
    FROM
        slot_transaction st
    WHERE
        st.subscription_id = $1
        AND st.price_component_id = $2
        AND st.transaction_at <= $3
)
SELECT
    X.prev_active_slots + COALESCE(SUM(Y.delta), 0) AS current_active_slots
FROM
    RankedSlotTransactions X
    LEFT JOIN
    RankedSlotTransactions Y ON Y.effective_at BETWEEN X.transaction_at AND $4
WHERE
    X.row_num = 1
GROUP BY
    X.prev_active_slots;
"#;

        diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(subscription_uid)
            .bind::<sql_types::Uuid, _>(price_component_uid)
            .bind::<sql_types::Timestamp, _>(ts)
            .bind::<sql_types::Timestamp, _>(ts)
            .get_result::<FetchTransactionResult>(conn)
            .await
            .attach_printable("Error while fetching slot transaction by id")
            .into_db_result()
    }
}

#[derive(QueryableByName)]
pub struct FetchTransactionResult {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub current_active_slots: i32,
}
