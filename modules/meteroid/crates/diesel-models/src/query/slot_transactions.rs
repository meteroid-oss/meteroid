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
        use crate::schema::slot_transaction::dsl::slot_transaction;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(slot_transaction).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting slot transaction")
            .into_db_result()
    }

    pub async fn insert_batch(conn: &mut PgConn, items: Vec<&SlotTransactionRow>) -> DbResult<()> {
        use crate::schema::slot_transaction::dsl::slot_transaction;
        use diesel_async::RunQueryDsl;

        if items.is_empty() {
            return Ok(());
        }

        let query = diesel::insert_into(slot_transaction).values(items);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while inserting slot transaction")
            .into_db_result()
            .map(|_| ())
    }

    pub async fn fetch_by_subscription_id_and_unit_locked(
        conn: &mut PgConn,
        tenant_id: common_domain::ids::TenantId,
        subscription_uid: SubscriptionId,
        unit: String,
        at_ts: Option<NaiveDateTime>,
    ) -> DbResult<FetchTransactionResult> {
        use diesel_async::RunQueryDsl;

        let at_ts = at_ts.unwrap_or_else(|| chrono::Utc::now().naive_utc());

        let raw_sql = r"
WITH LatestCheckpoint AS (
    SELECT st.prev_active_slots, st.transaction_at
    FROM slot_transaction st
    INNER JOIN subscription s ON st.subscription_id = s.id
    WHERE s.tenant_id = $1
      AND st.subscription_id = $2
      AND st.unit = $3
      AND st.transaction_at <= $4
      AND st.status = 'ACTIVE'
    ORDER BY transaction_at DESC
    LIMIT 1
    FOR UPDATE OF st
)
SELECT
    (COALESCE(lc.prev_active_slots, 0) + COALESCE(SUM(st.delta), 0))::integer AS current_active_slots
FROM LatestCheckpoint lc
LEFT JOIN slot_transaction st ON
    st.subscription_id = $2
    AND st.unit = $3
    AND st.effective_at >= lc.transaction_at
    AND st.effective_at <= $4
    AND st.status = 'ACTIVE'
GROUP BY lc.prev_active_slots;
";

        let query = diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Uuid, _>(subscription_uid)
            .bind::<sql_types::VarChar, _>(unit)
            .bind::<sql_types::Timestamp, _>(at_ts);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let result = query
            .get_result::<FetchTransactionResult>(conn)
            .await
            .optional()
            .map(|d| {
                d.unwrap_or(FetchTransactionResult {
                    current_active_slots: 0,
                })
            })
            .attach("Error while fetching slot transaction by id")
            .into_db_result()?;

        Ok(result)
    }
}

#[derive(QueryableByName)]
pub struct FetchTransactionResult {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub current_active_slots: i32,
}

impl SlotTransactionRow {
    pub async fn activate_pending_for_invoice(
        conn: &mut PgConn,
        tenant_id: common_domain::ids::TenantId,
        invoice_id: common_domain::ids::InvoiceId,
    ) -> DbResult<Vec<SlotTransactionRow>> {
        use crate::enums::SlotTransactionStatusEnum;
        use crate::schema::{invoice, slot_transaction};
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        let now = chrono::Utc::now().naive_utc();

        let query = diesel::update(
            slot_transaction::table
                .filter(slot_transaction::invoice_id.eq(invoice_id))
                .filter(slot_transaction::status.eq(SlotTransactionStatusEnum::Pending))
                .filter(diesel::dsl::exists(
                    invoice::table
                        .filter(invoice::id.eq(invoice_id))
                        .filter(invoice::tenant_id.eq(tenant_id)),
                )),
        )
        .set((
            slot_transaction::status.eq(SlotTransactionStatusEnum::Active),
            slot_transaction::effective_at.eq(now),
        ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while activating pending slot transactions")
            .into_db_result()
    }
}
