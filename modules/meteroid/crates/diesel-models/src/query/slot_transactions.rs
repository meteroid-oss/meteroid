use crate::errors::IntoDbResult;

use crate::lock::CustomerLock;
use crate::slot_transactions::SlotTransactionRow;
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;

use common_domain::ids::{InvoiceId, ManagedQuantityId, SubscriptionId, TenantId};
use diesel::QueryableByName;
use diesel::{OptionalExtension, sql_types};
use error_stack::ResultExt;

/// The active rows of the seat ledger of managed quantity $3 effective by $4, of subscription $2
/// of tenant $1: seats are counted by their quantity, never by a unit's name (R-137).
pub(crate) const ACTIVE_SLOT_ROWS: &str = "SELECT st.delta, st.prev_active_slots, st.effective_at, st.transaction_at \
    FROM slot_transaction st INNER JOIN subscription s ON st.subscription_id = s.id \
    WHERE s.tenant_id = $1 AND st.subscription_id = $2 AND st.managed_quantity_id = $3 \
    AND st.status = 'ACTIVE' AND st.effective_at <= $4";

/// The seats `active` holds: the latest row of no seats sets the count, the rows changing seats
/// at or after its instant add theirs, whatever count they were written against; with no such
/// row, the first row's count and every change. No row, no count.
pub(crate) const SLOT_COUNT: &str = "SELECT b.prev_active_slots + COALESCE((\
    SELECT SUM(a.delta) FROM active a \
    WHERE a.delta <> 0 AND (NOT b.seed OR a.effective_at >= b.effective_at)), 0) AS slots \
    FROM ((SELECT prev_active_slots, effective_at, true AS seed FROM active \
    WHERE delta = 0 ORDER BY effective_at DESC, transaction_at DESC LIMIT 1) \
    UNION ALL (SELECT prev_active_slots, effective_at, false AS seed FROM active \
    ORDER BY effective_at, transaction_at LIMIT 1) ORDER BY seed DESC LIMIT 1) b";

impl SlotTransactionRow {
    /// Re-points pending slot transactions from a set of (consolidated) invoices to the
    /// merged parent. Tenant-scoped via an EXISTS guard on the source invoice.
    pub async fn repoint_invoice(
        conn: &mut PgConn,
        tenant_id: common_domain::ids::TenantId,
        from_invoice_ids: &[InvoiceId],
        to_invoice_id: InvoiceId,
    ) -> DbResult<usize> {
        use crate::schema::{invoice, slot_transaction};
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        if from_invoice_ids.is_empty() {
            return Ok(0);
        }

        let query = diesel::update(
            slot_transaction::table
                .filter(slot_transaction::invoice_id.eq_any(from_invoice_ids))
                .filter(diesel::dsl::exists(
                    invoice::table
                        .filter(invoice::id.nullable().eq(slot_transaction::invoice_id))
                        .filter(invoice::tenant_id.eq(tenant_id)),
                )),
        )
        .set(slot_transaction::invoice_id.eq(Some(to_invoice_id)));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while re-pointing slot transactions")
            .into_db_result()
    }

    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SlotTransactionRow> {
        use crate::schema::slot_transaction::dsl::slot_transaction;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(slot_transaction).values(self);

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while inserting slot transaction")
            .into_db_result()
            .map(|_| ())
    }

    /// Active seats of `quantity` at `at_ts` without locking the ledger (read paths, previews);
    /// `None` when the subscription never had a slot transaction for that quantity.
    pub async fn fetch_active_slots(
        conn: &mut PgConn,
        tenant_id: common_domain::ids::TenantId,
        subscription_uid: SubscriptionId,
        quantity: ManagedQuantityId,
        at_ts: NaiveDateTime,
    ) -> DbResult<Option<i32>> {
        use diesel_async::RunQueryDsl;

        // The count in force at $4 (`SLOT_COUNT`).
        let raw_sql = format!(
            "WITH active AS ({ACTIVE_SLOT_ROWS}) SELECT slots::integer AS current_active_slots FROM ({SLOT_COUNT}) c"
        );

        diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Uuid, _>(subscription_uid)
            .bind::<sql_types::Uuid, _>(quantity)
            .bind::<sql_types::Timestamp, _>(at_ts)
            .get_result::<FetchTransactionResult>(conn)
            .await
            .optional()
            .map(|r| r.map(|r| r.current_active_slots))
            .attach("Error while reading active slots")
            .into_db_result()
    }

    /// Withdraws the changes of `quantity`'s seats waiting to take effect after `after` (a
    /// renewal's removal or increase); seat counts set by a scheduled change stay.
    pub async fn delete_waiting_changes(
        conn: &mut PgConn,
        lock: &CustomerLock,
        subscription_uid: SubscriptionId,
        quantity: ManagedQuantityId,
        after: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::enums::SlotTransactionStatusEnum;
        use crate::schema::{slot_transaction, subscription};
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        lock.on(conn);
        let query = diesel::delete(
            slot_transaction::table
                .filter(slot_transaction::subscription_id.eq(subscription_uid))
                .filter(slot_transaction::managed_quantity_id.eq(quantity))
                .filter(slot_transaction::status.eq(SlotTransactionStatusEnum::Active))
                .filter(slot_transaction::delta.ne(0))
                .filter(slot_transaction::effective_at.gt(after))
                .filter(diesel::dsl::exists(
                    subscription::table
                        .filter(subscription::id.eq(slot_transaction::subscription_id))
                        .filter(subscription::tenant_id.eq(lock.tenant_id())),
                )),
        );
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while withdrawing waiting seat changes")
            .into_db_result()
    }

    /// Seats of `quantity` bought but not active yet (their payment is in flight): the maximum
    /// counts them, or concurrent purchases could each pass it.
    pub async fn pending_upgrades(
        conn: &mut PgConn,
        tenant_id: common_domain::ids::TenantId,
        subscription_uid: SubscriptionId,
        quantity: ManagedQuantityId,
    ) -> DbResult<i32> {
        use crate::enums::SlotTransactionStatusEnum;
        use crate::schema::{slot_transaction, subscription};
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        slot_transaction::table
            .inner_join(subscription::table)
            .filter(subscription::tenant_id.eq(tenant_id))
            .filter(slot_transaction::subscription_id.eq(subscription_uid))
            .filter(slot_transaction::managed_quantity_id.eq(quantity))
            .filter(slot_transaction::status.eq(SlotTransactionStatusEnum::Pending))
            .filter(slot_transaction::delta.gt(0))
            .select(diesel::dsl::sum(slot_transaction::delta))
            .first::<Option<i64>>(conn)
            .await
            .map(|sum| sum.unwrap_or(0) as i32)
            .attach("Error while reading pending slot upgrades")
            .into_db_result()
    }

    /// The seats of `quantity` at `at_ts` (now when None), its ledger rows locked.
    pub async fn fetch_by_subscription_id_and_quantity_locked(
        conn: &mut PgConn,
        lock: &CustomerLock,
        subscription_uid: SubscriptionId,
        quantity: ManagedQuantityId,
        at_ts: Option<NaiveDateTime>,
    ) -> DbResult<FetchTransactionResult> {
        use diesel_async::RunQueryDsl;

        {
            use crate::schema::subscription::dsl as s_dsl;
            use diesel::{ExpressionMethods, QueryDsl};
            let (tenant, customer): (TenantId, common_domain::ids::CustomerId) =
                s_dsl::subscription
                    .filter(s_dsl::id.eq(subscription_uid))
                    .select((s_dsl::tenant_id, s_dsl::customer_id))
                    .first(conn)
                    .await
                    .attach("Error while reading the customer of a subscription")
                    .into_db_result()?;
            lock.covers(conn, tenant, customer)?;
        }
        let tenant_id = lock.tenant_id();
        let at_ts = at_ts.unwrap_or_else(|| common_clock::now().naive_utc());

        // The quantity's rows are locked, then counted as `fetch_active_slots` does.
        let raw_sql = format!(
            "WITH locked AS (SELECT st.id FROM slot_transaction st WHERE st.subscription_id = $2 AND st.managed_quantity_id = $3 FOR UPDATE), \
             active AS ({ACTIVE_SLOT_ROWS}) \
             SELECT slots::integer AS current_active_slots FROM ({SLOT_COUNT}) c \
             WHERE (SELECT count(*) FROM locked) >= 0"
        );

        let query = diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Uuid, _>(subscription_uid)
            .bind::<sql_types::Uuid, _>(quantity)
            .bind::<sql_types::Timestamp, _>(at_ts);

        crate::debug_sql(&query);

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
    /// Whether `invoice_id` bills seats that wait for its payment: deleted, they never start
    /// (R-124).
    pub async fn holds_pending_seats(conn: &mut PgConn, invoice_id: InvoiceId) -> DbResult<bool> {
        use crate::schema::slot_transaction::dsl;
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;

        diesel::select(diesel::dsl::exists(
            dsl::slot_transaction
                .filter(dsl::invoice_id.eq(invoice_id))
                .filter(dsl::status.eq(crate::enums::SlotTransactionStatusEnum::Pending)),
        ))
        .get_result(conn)
        .await
        .attach("Error while checking whether an invoice holds pending seats")
        .into_db_result()
    }

    pub async fn activate_pending_for_invoice(
        conn: &mut PgConn,
        lock: &CustomerLock,
        invoice_id: common_domain::ids::InvoiceId,
        effective_at: NaiveDateTime,
    ) -> DbResult<Vec<SlotTransactionRow>> {
        use crate::enums::SlotTransactionStatusEnum;
        use crate::schema::{invoice, slot_transaction};
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        lock.on(conn);
        let query = diesel::update(
            slot_transaction::table
                .filter(slot_transaction::invoice_id.eq(invoice_id))
                .filter(slot_transaction::status.eq(SlotTransactionStatusEnum::Pending))
                .filter(diesel::dsl::exists(
                    invoice::table
                        .filter(invoice::id.eq(invoice_id))
                        .filter(invoice::tenant_id.eq(lock.tenant_id()))
                        .filter(invoice::customer_id.eq(lock.customer_id())),
                )),
        )
        .set((
            slot_transaction::status.eq(SlotTransactionStatusEnum::Active),
            slot_transaction::effective_at.eq(effective_at),
        ));

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while activating pending slot transactions")
            .into_db_result()
    }

    /// Drops the pending slot changes an invoice would have activated (its payment was refused).
    pub async fn delete_pending_for_invoice(
        conn: &mut PgConn,
        lock: &CustomerLock,
        invoice_id: common_domain::ids::InvoiceId,
    ) -> DbResult<usize> {
        use crate::enums::SlotTransactionStatusEnum;
        use crate::schema::{invoice, slot_transaction};
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        lock.on(conn);
        let query = diesel::delete(
            slot_transaction::table
                .filter(slot_transaction::invoice_id.eq(invoice_id))
                .filter(slot_transaction::status.eq(SlotTransactionStatusEnum::Pending))
                .filter(diesel::dsl::exists(
                    invoice::table
                        .filter(invoice::id.eq(invoice_id))
                        .filter(invoice::tenant_id.eq(lock.tenant_id()))
                        .filter(invoice::customer_id.eq(lock.customer_id())),
                )),
        );

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting pending slot transactions")
            .into_db_result()
    }

    /// Adds `delta` to the seat counts `seeds` set from after `after` for `quantity`: seats
    /// changed before a change setting its seats takes effect carry into its count.
    pub async fn carry_into_seeds(
        conn: &mut PgConn,
        seeds: &[common_domain::ids::SlotTransactionId],
        quantity: ManagedQuantityId,
        after: NaiveDateTime,
        delta: i32,
    ) -> DbResult<usize> {
        use crate::schema::slot_transaction;
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        if seeds.is_empty() || delta == 0 {
            return Ok(0);
        }
        let query = diesel::update(
            slot_transaction::table
                .filter(slot_transaction::id.eq_any(seeds))
                .filter(slot_transaction::managed_quantity_id.eq(quantity))
                .filter(slot_transaction::delta.eq(0))
                .filter(slot_transaction::effective_at.gt(after)),
        )
        .set(slot_transaction::prev_active_slots.eq(slot_transaction::prev_active_slots + delta));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while carrying seats into a change's count")
            .into_db_result()
    }

    pub async fn list_by_subscription_id(
        conn: &mut PgConn,
        tenant_id: common_domain::ids::TenantId,
        subscription_id: SubscriptionId,
        quantity: Option<ManagedQuantityId>,
        status: Option<crate::enums::SlotTransactionStatusEnum>,
    ) -> DbResult<Vec<SlotTransactionRow>> {
        use crate::schema::{slot_transaction, subscription};
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        let mut query = slot_transaction::table
            .inner_join(
                subscription::table.on(slot_transaction::subscription_id.eq(subscription::id)),
            )
            .filter(subscription::tenant_id.eq(tenant_id))
            .filter(slot_transaction::subscription_id.eq(subscription_id))
            .select(slot_transaction::all_columns)
            .order(slot_transaction::transaction_at.desc())
            .into_boxed();

        if let Some(quantity) = quantity {
            query = query.filter(slot_transaction::managed_quantity_id.eq(quantity));
        }

        if let Some(status) = status {
            query = query.filter(slot_transaction::status.eq(status));
        }

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing slot transactions")
            .into_db_result()
    }

    pub async fn subscription_of(
        conn: &mut PgConn,
        tenant_id: common_domain::ids::TenantId,
        transaction_id: common_domain::ids::SlotTransactionId,
    ) -> DbResult<Option<common_domain::ids::SubscriptionId>> {
        use crate::schema::{slot_transaction, subscription};
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        slot_transaction::table
            .inner_join(subscription::table)
            .filter(slot_transaction::id.eq(transaction_id))
            .filter(subscription::tenant_id.eq(tenant_id))
            .select(slot_transaction::subscription_id)
            .first(conn)
            .await
            .optional()
            .attach("Error while reading the subscription of a slot transaction")
            .into_db_result()
    }

    pub async fn delete_transaction(
        conn: &mut PgConn,
        tenant_id: common_domain::ids::TenantId,
        transaction_id: common_domain::ids::SlotTransactionId,
    ) -> DbResult<()> {
        use crate::enums::SlotTransactionStatusEnum;
        use crate::schema::{slot_transaction, subscription};
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        let now = common_clock::now().naive_utc();

        let query = diesel::delete(
            slot_transaction::table
                .filter(slot_transaction::id.eq(transaction_id))
                .filter(slot_transaction::status.eq(SlotTransactionStatusEnum::Active))
                .filter(slot_transaction::effective_at.gt(now))
                .filter(diesel::dsl::exists(
                    subscription::table
                        .filter(subscription::id.eq(slot_transaction::subscription_id))
                        .filter(subscription::tenant_id.eq(tenant_id)),
                )),
        );

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while deleting slot transaction")
            .into_db_result()
    }
}
