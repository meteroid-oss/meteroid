use crate::enums::{ReminderEnrolledViaEnum, ReminderStateEnum, ReminderStopReasonEnum};
use crate::errors::IntoDbResult;
use crate::lock::CustomerLock;
use crate::payment_reminders::{
    InvoicePaymentReminderRow, InvoicePaymentReminderRowNew, PaymentReminderSequenceRow,
    PaymentReminderSequenceRowNew, PaymentReminderStepRow, PaymentReminderStepRowNew,
};
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use common_domain::ids::{
    BaseId, CustomerId, InvoiceId, InvoicingEntityId, PaymentReminderSequenceId, TenantId,
};
use diesel::sql_types;
use diesel::{
    AsChangeset, ExpressionMethods, OptionalExtension, PgSortExpressionMethods, QueryDsl,
    QueryableByName, SelectableHelper,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

/// Mixed into the entity-derived advisory key so it can't collide with other xact-lock users.
const SEED_LOCK_SALT: i64 = 0x5052_5345_4544_0000; // "PRSEED"

/// The `WITH member, f` CTE behind audience evaluation; the caller appends its projection and
/// the compiled predicate (`$1` = tenant, `$2` = invoice ids, predicate binds from `$3`).
pub const AUDIENCE_FACTS_SQL: &str = include_str!("../../sql/payment_reminders/audience_facts.sql");

#[derive(Debug, QueryableByName)]
pub struct AudienceMatchRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub invoice_id: Uuid,
}

/// The customer-only `WITH f` CTE (`$1` = tenant, `$2` = customer ids): no invoice columns.
pub const CUSTOMER_AUDIENCE_FACTS_SQL: &str =
    include_str!("../../sql/payment_reminders/customer_audience_facts.sql");

#[derive(Debug, QueryableByName)]
pub struct AudienceCustomerMatchRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub customer_id: Uuid,
}

#[derive(Debug, QueryableByName)]
pub struct AudiencePreviewSampleRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub invoice_id: Uuid,
    #[diesel(sql_type = sql_types::Text)]
    pub invoice_number: String,
    #[diesel(sql_type = sql_types::Uuid)]
    pub customer_id: Uuid,
    #[diesel(sql_type = sql_types::Text)]
    pub customer_name: String,
    #[diesel(sql_type = sql_types::BigInt)]
    pub amount_due: i64,
    #[diesel(sql_type = sql_types::Text)]
    pub currency: String,
}

#[derive(Debug, QueryableByName)]
struct SequenceIdRow {
    #[diesel(sql_type = sql_types::Uuid)]
    id: Uuid,
}

#[derive(Debug, QueryableByName)]
pub struct SequenceInFlightRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub sequence_id: Uuid,
    #[diesel(sql_type = sql_types::BigInt)]
    pub in_flight: i64,
}

#[derive(Debug, QueryableByName)]
struct CustomerIdRow {
    #[diesel(sql_type = sql_types::Uuid)]
    id: Uuid,
}

fn locked_customers(conn: &PgConn, locked: &[CustomerLock]) -> Vec<Uuid> {
    locked
        .iter()
        .map(|lock| {
            lock.on(conn);
            lock.customer_id().as_uuid()
        })
        .collect()
}

#[derive(Debug, Default, AsChangeset)]
#[diesel(table_name = crate::schema::payment_reminder_sequence)]
pub struct PaymentReminderSequenceRowPatch {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    /// `Some(None)` clears the audience (the row becomes the default).
    pub audience: Option<Option<serde_json::Value>>,
    pub updated_at: Option<NaiveDateTime>,
}

impl PaymentReminderSequenceRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PaymentReminderSequenceRow> {
        use crate::schema::payment_reminder_sequence::dsl as sq_dsl;

        let query = diesel::insert_into(sq_dsl::payment_reminder_sequence).values(self);
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting payment_reminder_sequence")
            .into_db_result()
    }

    /// Serializes default seeding per entity for the rest of the transaction: two concurrent
    /// seeds would both take `MAX(position) + 1` and the second would hit the position index.
    pub async fn lock_entity_seed(
        conn: &mut PgConn,
        invoicing_entity_id: InvoicingEntityId,
    ) -> DbResult<()> {
        let bytes = invoicing_entity_id.as_uuid().into_bytes();
        let key = i64::from_be_bytes(bytes[..8].try_into().expect("8 bytes")) ^ SEED_LOCK_SALT;
        diesel::sql_query("SELECT pg_advisory_xact_lock($1)")
            .bind::<sql_types::BigInt, _>(key)
            .execute(conn)
            .await
            .attach("Error acquiring the payment_reminder_sequence seed lock")
            .into_db_result()?;
        Ok(())
    }

    /// Seeds the entity's default sequence at the highest live position. Nothing is returned when
    /// a live default already exists (`ON CONFLICT DO NOTHING` on the partial unique index) or
    /// when the entity does not belong to `tenant_id`.
    pub async fn insert_default_if_absent(
        conn: &mut PgConn,
        id: PaymentReminderSequenceId,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        name: &str,
    ) -> DbResult<Option<PaymentReminderSequenceId>> {
        let rows: Vec<SequenceIdRow> = diesel::sql_query(
            "INSERT INTO payment_reminder_sequence (id, tenant_id, invoicing_entity_id, name, position, enabled, audience) \
             SELECT $1, ie.tenant_id, ie.id, $4, COALESCE((SELECT MAX(position) + 1 FROM payment_reminder_sequence \
                     WHERE invoicing_entity_id = ie.id AND archived_at IS NULL), 0), false, NULL \
             FROM invoicing_entity ie WHERE ie.id = $3 AND ie.tenant_id = $2 \
             ON CONFLICT (invoicing_entity_id) WHERE audience IS NULL AND archived_at IS NULL DO NOTHING \
             RETURNING id",
        )
        .bind::<sql_types::Uuid, _>(id.as_uuid())
        .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
        .bind::<sql_types::Uuid, _>(invoicing_entity_id.as_uuid())
        .bind::<sql_types::Text, _>(name)
        .get_results(conn)
        .await
        .attach("Error while seeding the default payment_reminder_sequence")
        .into_db_result()?;

        Ok(rows
            .into_iter()
            .next()
            .map(|r| PaymentReminderSequenceId::from(r.id)))
    }
}

impl PaymentReminderSequenceRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PaymentReminderSequenceId,
    ) -> DbResult<Option<PaymentReminderSequenceRow>> {
        use crate::schema::payment_reminder_sequence::dsl as sq_dsl;

        let query = sq_dsl::payment_reminder_sequence
            .filter(sq_dsl::tenant_id.eq(tenant_id))
            .filter(sq_dsl::id.eq(id))
            .select(PaymentReminderSequenceRow::as_select());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding payment_reminder_sequence")
            .into_db_result()
    }

    pub async fn find_by_id_for_update(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PaymentReminderSequenceId,
    ) -> DbResult<Option<PaymentReminderSequenceRow>> {
        use crate::schema::payment_reminder_sequence::dsl as sq_dsl;

        let query = sq_dsl::payment_reminder_sequence
            .for_update()
            .filter(sq_dsl::tenant_id.eq(tenant_id))
            .filter(sq_dsl::id.eq(id))
            .select(PaymentReminderSequenceRow::as_select());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while locking payment_reminder_sequence")
            .into_db_result()
    }

    /// A live sequence under a shared lock: the enrolment transaction holds it so an archive
    /// (`FOR UPDATE`) either waits for the enrolment or is seen by it.
    pub async fn find_live_for_share(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PaymentReminderSequenceId,
    ) -> DbResult<Option<PaymentReminderSequenceRow>> {
        use crate::schema::payment_reminder_sequence::dsl as sq_dsl;

        let query = sq_dsl::payment_reminder_sequence
            .for_share()
            .filter(sq_dsl::tenant_id.eq(tenant_id))
            .filter(sq_dsl::id.eq(id))
            .filter(sq_dsl::archived_at.is_null())
            .select(PaymentReminderSequenceRow::as_select());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while share-locking payment_reminder_sequence")
            .into_db_result()
    }

    /// The entity's live default (`audience IS NULL`), locked for the position shuffle.
    pub async fn find_default_for_update(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> DbResult<Option<PaymentReminderSequenceRow>> {
        use crate::schema::payment_reminder_sequence::dsl as sq_dsl;

        let query = sq_dsl::payment_reminder_sequence
            .for_update()
            .filter(sq_dsl::tenant_id.eq(tenant_id))
            .filter(sq_dsl::invoicing_entity_id.eq(invoicing_entity_id))
            .filter(sq_dsl::audience.is_null())
            .filter(sq_dsl::archived_at.is_null())
            .select(PaymentReminderSequenceRow::as_select());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while locking the default payment_reminder_sequence")
            .into_db_result()
    }

    /// Live sequences first in position order, archived ones (when asked for) after them.
    pub async fn list_by_entity(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        include_archived: bool,
    ) -> DbResult<Vec<PaymentReminderSequenceRow>> {
        use crate::schema::payment_reminder_sequence::dsl as sq_dsl;

        let mut query = sq_dsl::payment_reminder_sequence
            .filter(sq_dsl::tenant_id.eq(tenant_id))
            .filter(sq_dsl::invoicing_entity_id.eq(invoicing_entity_id))
            .order((
                sq_dsl::archived_at.asc().nulls_first(),
                sq_dsl::position.asc(),
            ))
            .select(PaymentReminderSequenceRow::as_select())
            .into_boxed();
        if !include_archived {
            query = query.filter(sq_dsl::archived_at.is_null());
        }
        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing payment_reminder_sequence rows")
            .into_db_result()
    }

    pub async fn set_position(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PaymentReminderSequenceId,
        position: i32,
    ) -> DbResult<usize> {
        use crate::schema::payment_reminder_sequence::dsl as sq_dsl;

        let query = diesel::update(sq_dsl::payment_reminder_sequence)
            .filter(sq_dsl::tenant_id.eq(tenant_id))
            .filter(sq_dsl::id.eq(id))
            .set(sq_dsl::position.eq(position));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while setting payment_reminder_sequence position")
            .into_db_result()
    }

    /// First pass of a reorder: every live position of the entity becomes `-position - 1`, so
    /// the final values can be written without tripping the partial unique index.
    pub async fn negate_live_positions(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> DbResult<usize> {
        diesel::sql_query(
            "UPDATE payment_reminder_sequence SET position = -position - 1 \
             WHERE tenant_id = $1 AND invoicing_entity_id = $2 AND archived_at IS NULL",
        )
        .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
        .bind::<sql_types::Uuid, _>(invoicing_entity_id.as_uuid())
        .execute(conn)
        .await
        .attach("Error while negating payment_reminder_sequence positions")
        .into_db_result()
    }

    pub async fn patch(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PaymentReminderSequenceId,
        patch: PaymentReminderSequenceRowPatch,
    ) -> DbResult<PaymentReminderSequenceRow> {
        use crate::schema::payment_reminder_sequence::dsl as sq_dsl;

        let query = diesel::update(sq_dsl::payment_reminder_sequence)
            .filter(sq_dsl::tenant_id.eq(tenant_id))
            .filter(sq_dsl::id.eq(id))
            .set(patch)
            .returning(PaymentReminderSequenceRow::as_returning());
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while updating payment_reminder_sequence")
            .into_db_result()
    }

    pub async fn archive(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PaymentReminderSequenceId,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::payment_reminder_sequence::dsl as sq_dsl;

        let query = diesel::update(sq_dsl::payment_reminder_sequence)
            .filter(sq_dsl::tenant_id.eq(tenant_id))
            .filter(sq_dsl::id.eq(id))
            .filter(sq_dsl::archived_at.is_null())
            .set((sq_dsl::archived_at.eq(now), sq_dsl::updated_at.eq(now)));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while archiving payment_reminder_sequence")
            .into_db_result()
    }

    /// Nulls every customer override pointing at the sequence; returns the customers touched.
    /// The customers an archive of the sequence writes, in id order: those overriding to it and
    /// those owning its live reminders. Stable once the sequence is locked (both take it first).
    pub async fn customers_of(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PaymentReminderSequenceId,
    ) -> DbResult<Vec<CustomerId>> {
        let rows: Vec<CustomerIdRow> = diesel::sql_query(
            "SELECT c.id FROM customer c WHERE c.tenant_id = $1 AND c.payment_reminder_sequence_id = $2 \
             UNION \
             SELECT i.customer_id FROM invoice_payment_reminder r JOIN invoice i ON i.id = r.invoice_id \
             WHERE r.tenant_id = $1 AND r.sequence_id = $2 AND r.state = 'ENROLLED' \
               AND r.stopped_at IS NULL AND r.next_reminder_at IS NOT NULL \
             ORDER BY id",
        )
        .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
        .bind::<sql_types::Uuid, _>(id.as_uuid())
        .get_results(conn)
        .await
        .attach("Error while listing the customers of a reminder sequence")
        .into_db_result()?;
        Ok(rows.into_iter().map(|r| CustomerId::from(r.id)).collect())
    }

    /// Among the `locked` customers.
    pub async fn clear_customer_overrides(
        conn: &mut PgConn,
        locked: &[CustomerLock],
        tenant_id: TenantId,
        id: PaymentReminderSequenceId,
    ) -> DbResult<Vec<CustomerId>> {
        let customers = locked_customers(conn, locked);
        let rows: Vec<CustomerIdRow> = diesel::sql_query(
            "UPDATE customer SET payment_reminder_sequence_id = NULL, updated_at = $4 \
             WHERE tenant_id = $1 AND payment_reminder_sequence_id = $2 AND id = ANY($3) RETURNING id",
        )
        .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
        .bind::<sql_types::Uuid, _>(id.as_uuid())
        .bind::<sql_types::Array<sql_types::Uuid>, _>(customers)
        .bind::<sql_types::Timestamp, _>(common_domain::clock::now().naive_utc())
        .get_results(conn)
        .await
        .attach("Error while clearing customer reminder overrides")
        .into_db_result()?;

        Ok(rows.into_iter().map(|r| CustomerId::from(r.id)).collect())
    }
}

impl PaymentReminderStepRowNew {
    pub async fn insert_batch(
        rows: &[Self],
        conn: &mut PgConn,
    ) -> DbResult<Vec<PaymentReminderStepRow>> {
        use crate::schema::payment_reminder_step::dsl as st_dsl;

        if rows.is_empty() {
            return Ok(vec![]);
        }

        let query = diesel::insert_into(st_dsl::payment_reminder_step).values(rows);
        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while inserting payment_reminder_step rows")
            .into_db_result()
    }
}

impl PaymentReminderStepRow {
    pub async fn list_by_sequence_ids(
        conn: &mut PgConn,
        tenant_id: TenantId,
        sequence_ids: &[PaymentReminderSequenceId],
    ) -> DbResult<Vec<PaymentReminderStepRow>> {
        use crate::schema::payment_reminder_step::dsl as st_dsl;

        let query = st_dsl::payment_reminder_step
            .filter(st_dsl::tenant_id.eq(tenant_id))
            .filter(st_dsl::sequence_id.eq_any(sequence_ids))
            .order((st_dsl::sequence_id, st_dsl::offset_days))
            .select(PaymentReminderStepRow::as_select());
        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing payment_reminder_step rows")
            .into_db_result()
    }

    pub async fn delete_by_sequence(
        conn: &mut PgConn,
        tenant_id: TenantId,
        sequence_id: PaymentReminderSequenceId,
    ) -> DbResult<usize> {
        use crate::schema::payment_reminder_step::dsl as st_dsl;

        let query = diesel::delete(st_dsl::payment_reminder_step)
            .filter(st_dsl::tenant_id.eq(tenant_id))
            .filter(st_dsl::sequence_id.eq(sequence_id));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting payment_reminder_step rows")
            .into_db_result()
    }
}

impl InvoicePaymentReminderRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<InvoicePaymentReminderRow> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let query = diesel::insert_into(r_dsl::invoice_payment_reminder).values(self);
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting invoice_payment_reminder")
            .into_db_result()
    }
}

impl InvoicePaymentReminderRow {
    pub async fn find_by_invoice_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> DbResult<Option<InvoicePaymentReminderRow>> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let query = r_dsl::invoice_payment_reminder
            .filter(r_dsl::tenant_id.eq(tenant_id))
            .filter(r_dsl::invoice_id.eq(invoice_id))
            .select(InvoicePaymentReminderRow::as_select());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding invoice_payment_reminder")
            .into_db_result()
    }

    pub async fn find_by_invoice_id_for_update(
        conn: &mut PgConn,
        lock: &CustomerLock,
        invoice_id: InvoiceId,
    ) -> DbResult<Option<InvoicePaymentReminderRow>> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        lock.on(conn);
        let tenant_id = lock.tenant_id();

        let query = r_dsl::invoice_payment_reminder
            .for_update()
            .filter(r_dsl::tenant_id.eq(tenant_id))
            .filter(r_dsl::invoice_id.eq(invoice_id))
            .select(InvoicePaymentReminderRow::as_select());
        crate::debug_sql(&query);

        let row: Option<InvoicePaymentReminderRow> = query
            .first(conn)
            .await
            .optional()
            .attach("Error while locking invoice_payment_reminder")
            .into_db_result()?;
        if row.is_some() {
            use crate::schema::invoice::dsl as i_dsl;
            let (tenant, customer): (TenantId, CustomerId) = i_dsl::invoice
                .filter(i_dsl::id.eq(invoice_id))
                .select((i_dsl::tenant_id, i_dsl::customer_id))
                .first(conn)
                .await
                .attach("Error while reading the customer of a reminded invoice")
                .into_db_result()?;
            lock.covers(conn, tenant, customer)?;
        }
        Ok(row)
    }

    /// Invoices still moving through each sequence: enrolled, scheduled, not stopped.
    pub async fn count_in_flight_by_sequence(
        conn: &mut PgConn,
        tenant_id: TenantId,
        sequence_ids: &[PaymentReminderSequenceId],
    ) -> DbResult<Vec<SequenceInFlightRow>> {
        if sequence_ids.is_empty() {
            return Ok(vec![]);
        }
        let ids: Vec<Uuid> = sequence_ids.iter().map(|id| id.as_uuid()).collect();

        diesel::sql_query(
            "SELECT sequence_id, COUNT(*) AS in_flight FROM invoice_payment_reminder \
             WHERE tenant_id = $1 AND sequence_id = ANY($2::uuid[]) AND state = 'ENROLLED' \
               AND next_reminder_at IS NOT NULL AND stopped_at IS NULL \
             GROUP BY sequence_id",
        )
        .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
        .bind::<sql_types::Array<sql_types::Uuid>, _>(ids)
        .get_results(conn)
        .await
        .attach("Error while counting in-flight reminders")
        .into_db_result()
    }
}

/// The 10 sample rows an audience preview shows, newest first.
pub async fn audience_preview_sample(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_ids: &[InvoiceId],
    limit: i64,
) -> DbResult<Vec<AudiencePreviewSampleRow>> {
    if invoice_ids.is_empty() {
        return Ok(vec![]);
    }
    let ids: Vec<Uuid> = invoice_ids.iter().map(|id| id.as_uuid()).collect();

    diesel::sql_query(
        "SELECT i.id AS invoice_id, i.invoice_number, i.customer_id, c.name AS customer_name, \
                i.amount_due, i.currency \
         FROM invoice i JOIN customer c ON c.id = i.customer_id \
         WHERE i.tenant_id = $1 AND i.id = ANY($2::uuid[]) \
         ORDER BY i.due_at DESC NULLS LAST, i.id DESC LIMIT $3",
    )
    .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<sql_types::Array<sql_types::Uuid>, _>(ids)
    .bind::<sql_types::BigInt, _>(limit)
    .get_results(conn)
    .await
    .attach("Error while sampling audience preview rows")
    .into_db_result()
}

#[derive(Debug, QueryableByName)]
struct CountRow {
    #[diesel(sql_type = sql_types::BigInt)]
    count: i64,
}

pub async fn count_distinct_customers(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_ids: &[InvoiceId],
) -> DbResult<i64> {
    if invoice_ids.is_empty() {
        return Ok(0);
    }
    let ids: Vec<Uuid> = invoice_ids.iter().map(|id| id.as_uuid()).collect();

    let row: CountRow = diesel::sql_query(
        "SELECT COUNT(DISTINCT customer_id) AS count FROM invoice \
         WHERE tenant_id = $1 AND id = ANY($2::uuid[])",
    )
    .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<sql_types::Array<sql_types::Uuid>, _>(ids)
    .get_result(conn)
    .await
    .attach("Error while counting audience customers")
    .into_db_result()?;

    Ok(row.count)
}

/// Open invoices an audience preview evaluates: finalized, not paid, not a consolidated child.
pub async fn list_open_invoice_ids_for_entity(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoicing_entity_id: InvoicingEntityId,
    limit: i64,
) -> DbResult<Vec<InvoiceId>> {
    let rows: Vec<AudienceMatchRow> = diesel::sql_query(
        "SELECT id AS invoice_id FROM invoice \
         WHERE tenant_id = $1 AND invoicing_entity_id = $2 AND status = 'FINALIZED' \
           AND payment_status <> 'PAID' AND consolidated_into_invoice_id IS NULL \
         ORDER BY due_at DESC NULLS LAST, id DESC LIMIT $3",
    )
    .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<sql_types::Uuid, _>(invoicing_entity_id.as_uuid())
    .bind::<sql_types::BigInt, _>(limit)
    .get_results(conn)
    .await
    .attach("Error while listing open invoices for an audience preview")
    .into_db_result()?;

    Ok(rows
        .into_iter()
        .map(|r| InvoiceId::from(r.invoice_id))
        .collect())
}

/// Finalized invoices of the entity, newest first, consolidated children excluded.
pub async fn list_recent_invoice_ids_for_entity(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoicing_entity_id: InvoicingEntityId,
    limit: i64,
) -> DbResult<Vec<InvoiceId>> {
    let rows: Vec<AudienceMatchRow> = diesel::sql_query(
        "SELECT id AS invoice_id FROM invoice \
         WHERE tenant_id = $1 AND invoicing_entity_id = $2 AND status = 'FINALIZED' \
           AND consolidated_into_invoice_id IS NULL \
         ORDER BY invoice_date DESC, id DESC LIMIT $3",
    )
    .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<sql_types::Uuid, _>(invoicing_entity_id.as_uuid())
    .bind::<sql_types::BigInt, _>(limit)
    .get_results(conn)
    .await
    .attach("Error while listing recent invoices for an audience preview")
    .into_db_result()?;

    Ok(rows
        .into_iter()
        .map(|r| InvoiceId::from(r.invoice_id))
        .collect())
}

/// Live customers of the entity, newest first.
pub async fn list_customer_ids_for_entity(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoicing_entity_id: InvoicingEntityId,
    limit: i64,
) -> DbResult<Vec<CustomerId>> {
    let rows: Vec<AudienceCustomerMatchRow> = diesel::sql_query(
        "SELECT id AS customer_id FROM customer \
         WHERE tenant_id = $1 AND invoicing_entity_id = $2 AND archived_at IS NULL \
         ORDER BY created_at DESC, id DESC LIMIT $3",
    )
    .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<sql_types::Uuid, _>(invoicing_entity_id.as_uuid())
    .bind::<sql_types::BigInt, _>(limit)
    .get_results(conn)
    .await
    .attach("Error while listing customers for an audience preview")
    .into_db_result()?;

    Ok(rows
        .into_iter()
        .map(|r| CustomerId::from(r.customer_id))
        .collect())
}

/// Guarded writes on `invoice_payment_reminder`. Per-invoice writers run under the caller's
/// row lock inside its transaction; the `*_for_*` writers are set-based.
pub struct InvoiceReminderUpdates;

impl InvoiceReminderUpdates {
    #[allow(clippy::too_many_arguments)]
    pub async fn enroll(
        conn: &mut PgConn,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        sequence_id: PaymentReminderSequenceId,
        enrolled_via: ReminderEnrolledViaEnum,
        cursor: Option<i32>,
        next_reminder_at: Option<NaiveDateTime>,
        now: NaiveDateTime,
    ) -> DbResult<InvoicePaymentReminderRow> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let row = InvoicePaymentReminderRowNew {
            invoice_id,
            tenant_id,
            state: ReminderStateEnum::Enrolled,
            not_enrolled_reason: None,
            resolved_at: now,
            generation: 1,
            sequence_id: Some(sequence_id),
            enrolled_via: Some(enrolled_via),
            enrolled_at: Some(now),
            last_offset_days: cursor,
            next_reminder_at,
            completed_at: next_reminder_at.is_none().then_some(now),
        };
        let query = diesel::insert_into(r_dsl::invoice_payment_reminder)
            .values(&row)
            .on_conflict(r_dsl::invoice_id)
            .do_update()
            .set((
                r_dsl::state.eq(ReminderStateEnum::Enrolled),
                r_dsl::not_enrolled_reason.eq(None::<crate::enums::ReminderNotEnrolledReasonEnum>),
                r_dsl::resolved_at.eq(now),
                r_dsl::generation.eq(r_dsl::generation + 1),
                r_dsl::sequence_id.eq(Some(sequence_id)),
                r_dsl::enrolled_via.eq(Some(enrolled_via)),
                r_dsl::enrolled_at.eq(Some(now)),
                r_dsl::last_offset_days.eq(cursor),
                r_dsl::next_reminder_at.eq(next_reminder_at),
                r_dsl::paused_until.eq(None::<NaiveDateTime>),
                r_dsl::stopped_at.eq(None::<NaiveDateTime>),
                r_dsl::stop_reason.eq(None::<ReminderStopReasonEnum>),
                r_dsl::completed_at.eq(next_reminder_at.is_none().then_some(now)),
                r_dsl::ended_at.eq(None::<NaiveDateTime>),
            ))
            .returning(InvoicePaymentReminderRow::as_returning());
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while enrolling invoice in payment reminders")
            .into_db_result()
    }

    /// The permanent `LADDER_STALE` stamp; an existing row is left as is.
    pub async fn stamp_not_enrolled(
        conn: &mut PgConn,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        reason: crate::enums::ReminderNotEnrolledReasonEnum,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let row = InvoicePaymentReminderRowNew {
            invoice_id,
            tenant_id,
            state: ReminderStateEnum::NotEnrolled,
            not_enrolled_reason: Some(reason),
            resolved_at: now,
            generation: 0,
            sequence_id: None,
            enrolled_via: None,
            enrolled_at: None,
            last_offset_days: None,
            next_reminder_at: None,
            completed_at: None,
        };
        let query = diesel::insert_into(r_dsl::invoice_payment_reminder)
            .values(&row)
            .on_conflict_do_nothing();
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while stamping invoice as not enrolled")
            .into_db_result()
    }

    /// After a step executed: cursor moves to it, the next fire is stamped (or completion).
    pub async fn advance(
        conn: &mut PgConn,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        cursor: i32,
        next_reminder_at: Option<NaiveDateTime>,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let query = diesel::update(r_dsl::invoice_payment_reminder)
            .filter(r_dsl::tenant_id.eq(tenant_id))
            .filter(r_dsl::invoice_id.eq(invoice_id))
            .set((
                r_dsl::last_offset_days.eq(Some(cursor)),
                r_dsl::next_reminder_at.eq(next_reminder_at),
                r_dsl::completed_at.eq(next_reminder_at.is_none().then_some(now)),
            ));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while advancing payment reminder cursor")
            .into_db_result()
    }

    /// Pushes the claim out without touching the cursor (quarantine, pending payment, window).
    pub async fn defer(
        conn: &mut PgConn,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        next_reminder_at: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let query = diesel::update(r_dsl::invoice_payment_reminder)
            .filter(r_dsl::tenant_id.eq(tenant_id))
            .filter(r_dsl::invoice_id.eq(invoice_id))
            .set(r_dsl::next_reminder_at.eq(Some(next_reminder_at)));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deferring payment reminder")
            .into_db_result()
    }

    pub async fn complete(
        conn: &mut PgConn,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let query = diesel::update(r_dsl::invoice_payment_reminder)
            .filter(r_dsl::tenant_id.eq(tenant_id))
            .filter(r_dsl::invoice_id.eq(invoice_id))
            .set((
                r_dsl::next_reminder_at.eq(None::<NaiveDateTime>),
                r_dsl::completed_at.eq(Some(now)),
            ));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while completing payment reminders")
            .into_db_result()
    }

    /// Paid / void / uncollectible: the schedule is cleared, sequence and cursor stay for the UI.
    pub async fn clear_schedule(
        conn: &mut PgConn,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let query = diesel::update(r_dsl::invoice_payment_reminder)
            .filter(r_dsl::tenant_id.eq(tenant_id))
            .filter(r_dsl::invoice_id.eq(invoice_id))
            .filter(r_dsl::state.eq(ReminderStateEnum::Enrolled))
            .set((
                r_dsl::state.eq(ReminderStateEnum::Ended),
                r_dsl::next_reminder_at.eq(None::<NaiveDateTime>),
                r_dsl::paused_until.eq(None::<NaiveDateTime>),
                r_dsl::ended_at.eq(Some(now)),
            ));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while clearing payment reminder schedule")
            .into_db_result()
    }

    /// A reopened invoice whose ladder an operator had stopped: back to `ENROLLED`, the stop kept.
    pub async fn reopen_stopped(
        conn: &mut PgConn,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let query = diesel::update(r_dsl::invoice_payment_reminder)
            .filter(r_dsl::tenant_id.eq(tenant_id))
            .filter(r_dsl::invoice_id.eq(invoice_id))
            .filter(r_dsl::state.eq(ReminderStateEnum::Ended))
            .filter(r_dsl::stopped_at.is_not_null())
            .set((
                r_dsl::state.eq(ReminderStateEnum::Enrolled),
                r_dsl::ended_at.eq(None::<NaiveDateTime>),
            ));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while reopening stopped payment reminders")
            .into_db_result()
    }

    /// A reopen that waits for an operator: the ladder is stopped as `ENROLLED`. A missing row is
    /// created bare (no sequence) so the enrolment scan skips the invoice; an earlier stop and a
    /// `NOT_ENROLLED` stamp are kept. Returns the rows stopped by this call.
    pub async fn hold(
        conn: &mut PgConn,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        reason: ReminderStopReasonEnum,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let bare = InvoicePaymentReminderRowNew {
            invoice_id,
            tenant_id,
            state: ReminderStateEnum::Enrolled,
            not_enrolled_reason: None,
            resolved_at: now,
            generation: 0,
            sequence_id: None,
            enrolled_via: None,
            enrolled_at: None,
            last_offset_days: None,
            next_reminder_at: None,
            completed_at: None,
        };
        let insert = diesel::insert_into(r_dsl::invoice_payment_reminder)
            .values(&bare)
            .on_conflict_do_nothing();
        crate::debug_sql(&insert);
        insert
            .execute(conn)
            .await
            .attach("Error while creating held payment reminders")
            .into_db_result()?;

        let query = diesel::update(r_dsl::invoice_payment_reminder)
            .filter(r_dsl::tenant_id.eq(tenant_id))
            .filter(r_dsl::invoice_id.eq(invoice_id))
            .filter(r_dsl::state.ne(ReminderStateEnum::NotEnrolled))
            .filter(r_dsl::stopped_at.is_null())
            .set((
                r_dsl::state.eq(ReminderStateEnum::Enrolled),
                r_dsl::ended_at.eq(None::<NaiveDateTime>),
                r_dsl::stopped_at.eq(Some(now)),
                r_dsl::stop_reason.eq(Some(reason)),
                r_dsl::next_reminder_at.eq(None::<NaiveDateTime>),
                r_dsl::paused_until.eq(None::<NaiveDateTime>),
            ));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while holding payment reminders")
            .into_db_result()
    }

    pub async fn pause(
        conn: &mut PgConn,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        until: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let query = diesel::update(r_dsl::invoice_payment_reminder)
            .filter(r_dsl::tenant_id.eq(tenant_id))
            .filter(r_dsl::invoice_id.eq(invoice_id))
            .filter(r_dsl::state.eq(ReminderStateEnum::Enrolled))
            .filter(r_dsl::stopped_at.is_null())
            .set(r_dsl::paused_until.eq(Some(until)));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while pausing payment reminders")
            .into_db_result()
    }

    pub async fn resume(
        conn: &mut PgConn,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let query = diesel::update(r_dsl::invoice_payment_reminder)
            .filter(r_dsl::tenant_id.eq(tenant_id))
            .filter(r_dsl::invoice_id.eq(invoice_id))
            .filter(r_dsl::paused_until.is_not_null())
            .set(r_dsl::paused_until.eq(None::<NaiveDateTime>));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while resuming payment reminders")
            .into_db_result()
    }

    pub async fn stop(
        conn: &mut PgConn,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        reason: ReminderStopReasonEnum,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let query = diesel::update(r_dsl::invoice_payment_reminder)
            .filter(r_dsl::tenant_id.eq(tenant_id))
            .filter(r_dsl::invoice_id.eq(invoice_id))
            .filter(r_dsl::state.eq(ReminderStateEnum::Enrolled))
            .filter(r_dsl::stopped_at.is_null())
            .set((
                r_dsl::stopped_at.eq(Some(now)),
                r_dsl::stop_reason.eq(Some(reason)),
                r_dsl::next_reminder_at.eq(None::<NaiveDateTime>),
                r_dsl::paused_until.eq(None::<NaiveDateTime>),
            ));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while stopping payment reminders")
            .into_db_result()
    }

    /// Restart under the row lock: generation bumped, stop/pause/completion/end cleared, the
    /// sequence re-resolved by the caller and the cursor placed by the planner.
    #[allow(clippy::too_many_arguments)]
    pub async fn restart(
        conn: &mut PgConn,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        sequence_id: PaymentReminderSequenceId,
        enrolled_via: ReminderEnrolledViaEnum,
        cursor: Option<i32>,
        next_reminder_at: Option<NaiveDateTime>,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let query = diesel::update(r_dsl::invoice_payment_reminder)
            .filter(r_dsl::tenant_id.eq(tenant_id))
            .filter(r_dsl::invoice_id.eq(invoice_id))
            .set((
                r_dsl::state.eq(ReminderStateEnum::Enrolled),
                r_dsl::not_enrolled_reason.eq(None::<crate::enums::ReminderNotEnrolledReasonEnum>),
                r_dsl::resolved_at.eq(now),
                r_dsl::generation.eq(r_dsl::generation + 1),
                r_dsl::sequence_id.eq(Some(sequence_id)),
                r_dsl::enrolled_via.eq(Some(enrolled_via)),
                r_dsl::enrolled_at.eq(Some(now)),
                r_dsl::last_offset_days.eq(cursor),
                r_dsl::next_reminder_at.eq(next_reminder_at),
                r_dsl::paused_until.eq(None::<NaiveDateTime>),
                r_dsl::stopped_at.eq(None::<NaiveDateTime>),
                r_dsl::stop_reason.eq(None::<ReminderStopReasonEnum>),
                r_dsl::completed_at.eq(next_reminder_at.is_none().then_some(now)),
                r_dsl::ended_at.eq(None::<NaiveDateTime>),
            ));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while restarting payment reminders")
            .into_db_result()
    }

    /// Of the invoices of the `locked` customers.
    pub async fn stop_all_for_sequence(
        conn: &mut PgConn,
        locked: &[CustomerLock],
        tenant_id: TenantId,
        sequence_id: PaymentReminderSequenceId,
        reason: ReminderStopReasonEnum,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::invoice::dsl as i_dsl;
        use crate::schema::invoice_payment_reminder::dsl as r_dsl;

        let customers: Vec<CustomerId> = locked_customers(conn, locked)
            .into_iter()
            .map(CustomerId::from)
            .collect();
        let query = diesel::update(r_dsl::invoice_payment_reminder)
            .filter(r_dsl::tenant_id.eq(tenant_id))
            .filter(
                r_dsl::invoice_id.eq_any(
                    i_dsl::invoice
                        .filter(i_dsl::tenant_id.eq(tenant_id))
                        .filter(i_dsl::customer_id.eq_any(customers))
                        .select(i_dsl::id),
                ),
            )
            .filter(r_dsl::sequence_id.eq(sequence_id))
            .filter(r_dsl::state.eq(ReminderStateEnum::Enrolled))
            .filter(r_dsl::stopped_at.is_null())
            .filter(r_dsl::next_reminder_at.is_not_null())
            .set((
                r_dsl::stopped_at.eq(Some(now)),
                r_dsl::stop_reason.eq(Some(reason)),
                r_dsl::next_reminder_at.eq(None::<NaiveDateTime>),
                r_dsl::paused_until.eq(None::<NaiveDateTime>),
            ));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while stopping payment reminders for a sequence")
            .into_db_result()
    }

    pub async fn stop_all_for_customer(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        reason: ReminderStopReasonEnum,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        diesel::sql_query(
            "UPDATE invoice_payment_reminder r SET stopped_at = $3, stop_reason = $4, \
                    next_reminder_at = NULL, paused_until = NULL \
             FROM invoice i WHERE i.id = r.invoice_id AND r.tenant_id = $1 AND i.customer_id = $2 \
               AND r.state = 'ENROLLED' AND r.stopped_at IS NULL AND r.next_reminder_at IS NOT NULL",
        )
        .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
        .bind::<sql_types::Uuid, _>(customer_id.as_uuid())
        .bind::<sql_types::Timestamp, _>(now)
        .bind::<crate::schema::sql_types::ReminderStopReasonEnum, _>(reason)
        .execute(conn)
        .await
        .attach("Error while stopping payment reminders for a customer")
        .into_db_result()
    }

    /// Step edit: in-flight invoices are pulled to `now` so the claim-time planner
    /// re-derives; completed ones are re-armed only when the new last step is still reachable
    /// (anchor + max_offset inside the 24 h enrolment window).
    pub async fn rearm_for_sequence(
        conn: &mut PgConn,
        tenant_id: TenantId,
        sequence_id: PaymentReminderSequenceId,
        now: NaiveDateTime,
        max_offset_days: Option<i32>,
    ) -> DbResult<usize> {
        let in_flight = diesel::sql_query(
            "UPDATE invoice_payment_reminder SET next_reminder_at = LEAST(next_reminder_at, $3) \
             WHERE tenant_id = $1 AND sequence_id = $2 AND state = 'ENROLLED' \
               AND stopped_at IS NULL AND completed_at IS NULL AND next_reminder_at IS NOT NULL",
        )
        .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
        .bind::<sql_types::Uuid, _>(sequence_id.as_uuid())
        .bind::<sql_types::Timestamp, _>(now)
        .execute(conn)
        .await
        .attach("Error while re-arming in-flight payment reminders")
        .into_db_result()?;

        let Some(max_offset_days) = max_offset_days else {
            return Ok(in_flight);
        };

        let completed = diesel::sql_query(
            "UPDATE invoice_payment_reminder r SET next_reminder_at = $3, completed_at = NULL \
             FROM invoice i WHERE i.id = r.invoice_id AND r.tenant_id = $1 AND r.sequence_id = $2 \
               AND r.state = 'ENROLLED' AND r.stopped_at IS NULL AND r.completed_at IS NOT NULL \
               AND i.status = 'FINALIZED' AND i.payment_status IN ('UNPAID','PARTIALLY_PAID','ERRORED') \
               AND i.due_at IS NOT NULL AND i.finalized_at IS NOT NULL \
               AND GREATEST(i.due_at, i.finalized_at) + make_interval(days => $4) >= $3 - interval '24 hours'",
        )
        .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
        .bind::<sql_types::Uuid, _>(sequence_id.as_uuid())
        .bind::<sql_types::Timestamp, _>(now)
        .bind::<sql_types::Integer, _>(max_offset_days)
        .execute(conn)
        .await
        .attach("Error while re-arming completed payment reminders")
        .into_db_result()?;

        Ok(in_flight + completed)
    }
}

// ---- sweeper scans --------------------------------------------------------------------------
// `$now` is always bound from Rust: none of these statements may read the session clock (I10).

/// Enrolment candidates of one entity: open pay-link (or errored) invoices with a PDF, not yet
/// resolved (`invoice_payment_reminder` anti-join), keyset on `(anchor, id)` bounded by the
/// entity's ladder reach. Served by `idx_invoice_reminder_candidates`.
pub const LIST_REMINDER_CANDIDATES_SQL: &str = "\
SELECT i.id AS invoice_id, i.customer_id, i.due_at, i.finalized_at, \
       GREATEST(i.due_at, i.finalized_at) AS anchor, \
       c.payment_reminder_sequence_id AS customer_override_id \
FROM invoice i \
JOIN tenant t ON t.id = i.tenant_id AND t.status = 'ACTIVE' AND t.email_delivery_mode <> 'OFF' \
JOIN customer c ON c.id = i.customer_id \
LEFT JOIN invoice_payment_reminder r ON r.invoice_id = i.id \
WHERE i.invoicing_entity_id = $1 AND i.tenant_id = $2 AND i.delivery_held_at IS NULL \
  AND i.status = 'FINALIZED' AND i.payment_status IN ('UNPAID', 'PARTIALLY_PAID', 'ERRORED') \
  AND i.amount_due > 0 \
  AND i.consolidated_into_invoice_id IS NULL AND i.due_at IS NOT NULL AND i.finalized_at IS NOT NULL \
  AND i.pdf_document_id IS NOT NULL \
  AND (i.payment_status = 'ERRORED' OR i.auto_charge_requested_at IS NULL) \
  AND c.payment_reminders_opt_out = false \
  AND GREATEST(i.due_at, i.finalized_at) >= $3::timestamp \
  AND r.invoice_id IS NULL \
  AND ($4::timestamp IS NULL OR (GREATEST(i.due_at, i.finalized_at), i.id) > ($4::timestamp, $5::uuid)) \
ORDER BY GREATEST(i.due_at, i.finalized_at), i.id \
LIMIT $6";

/// Entities with at least one enabled live sequence, and how far their ladders reach. Zero-step
/// sequences count with a reach of 0; disabled sequences never extend it.
pub const LIST_ENTITY_REACH_SQL: &str = "\
SELECT sq.tenant_id, sq.invoicing_entity_id, COALESCE(MAX(st.offset_days), 0) AS max_offset \
FROM payment_reminder_sequence sq \
JOIN tenant t ON t.id = sq.tenant_id AND t.status = 'ACTIVE' AND t.email_delivery_mode <> 'OFF' \
LEFT JOIN payment_reminder_step st ON st.sequence_id = sq.id \
WHERE sq.archived_at IS NULL AND sq.enabled \
GROUP BY sq.tenant_id, sq.invoicing_entity_id \
ORDER BY sq.tenant_id, sq.invoicing_entity_id";

/// Due rows with every kill switch joined as a hold: a disabled sequence, an opted-out
/// customer, a tenant with emails off or an entity whose `payment_reminder` kind is off (an
/// entity without a settings row has the kind off) simply does not match.
pub const LIST_REMINDER_DUE_SQL: &str = "\
SELECT r.invoice_id, r.tenant_id, r.next_reminder_at \
FROM invoice_payment_reminder r \
JOIN invoice i ON i.id = r.invoice_id \
JOIN payment_reminder_sequence sq ON sq.id = r.sequence_id AND sq.enabled AND sq.archived_at IS NULL \
JOIN tenant t ON t.id = i.tenant_id AND t.status = 'ACTIVE' AND t.email_delivery_mode <> 'OFF' \
JOIN customer c ON c.id = i.customer_id AND NOT c.payment_reminders_opt_out \
LEFT JOIN invoicing_entity_email_settings es ON es.invoicing_entity_id = i.invoicing_entity_id \
WHERE r.state = 'ENROLLED' AND r.next_reminder_at <= $1 AND r.stopped_at IS NULL \
  AND i.delivery_held_at IS NULL \
  AND (r.paused_until IS NULL OR r.paused_until <= $1) \
  AND NOT ('payment_reminder' = ANY(COALESCE(es.disabled_kinds, '{payment_reminder}'))) \
  AND i.status = 'FINALIZED' AND i.payment_status IN ('UNPAID', 'PARTIALLY_PAID', 'ERRORED') \
  AND i.amount_due > 0 \
  AND ($3::timestamp IS NULL OR (r.next_reminder_at, r.invoice_id) > ($3::timestamp, $4::uuid)) \
ORDER BY r.next_reminder_at, r.invoice_id \
LIMIT $2";

/// Scheduled rows whose invoice can no longer be collected end in one statement; `PROCESSING`
/// is held. Invoices are locked first and re-read, so a row a concurrent reopen just
/// restarted is never ended (READ COMMITTED); a locked invoice waits for the next run.
pub const END_UNPAYABLE_SCHEDULED_SQL: &str = "\
WITH unpayable AS ( \
  SELECT i.id FROM invoice i \
  JOIN invoice_payment_reminder r ON r.invoice_id = i.id \
  WHERE r.state = 'ENROLLED' AND r.next_reminder_at IS NOT NULL \
    AND (i.status <> 'FINALIZED' OR i.payment_status = 'PAID' OR i.amount_due <= 0) \
  FOR UPDATE OF i SKIP LOCKED \
) \
UPDATE invoice_payment_reminder r \
SET state = 'ENDED', next_reminder_at = NULL, paused_until = NULL, ended_at = $1 \
FROM unpayable u \
WHERE r.invoice_id = u.id AND r.state = 'ENROLLED' AND r.next_reminder_at IS NOT NULL";

#[derive(Debug, Clone, QueryableByName)]
pub struct ReminderCandidateRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub invoice_id: Uuid,
    #[diesel(sql_type = sql_types::Uuid)]
    pub customer_id: Uuid,
    #[diesel(sql_type = sql_types::Timestamp)]
    pub due_at: NaiveDateTime,
    #[diesel(sql_type = sql_types::Timestamp)]
    pub finalized_at: NaiveDateTime,
    #[diesel(sql_type = sql_types::Timestamp)]
    pub anchor: NaiveDateTime,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Uuid>)]
    pub customer_override_id: Option<Uuid>,
}

#[derive(Debug, Clone, QueryableByName)]
pub struct EntityReachRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub tenant_id: Uuid,
    #[diesel(sql_type = sql_types::Uuid)]
    pub invoicing_entity_id: Uuid,
    #[diesel(sql_type = sql_types::Integer)]
    pub max_offset: i32,
}

#[derive(Debug, Clone, QueryableByName)]
pub struct ReminderDueRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub invoice_id: Uuid,
    #[diesel(sql_type = sql_types::Uuid)]
    pub tenant_id: Uuid,
    #[diesel(sql_type = sql_types::Timestamp)]
    pub next_reminder_at: NaiveDateTime,
}

/// An enrolled, scheduled invoice of an entity, with what the planner needs to re-derive it.
#[derive(Debug, Clone, QueryableByName)]
pub struct ReminderInFlightRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub invoice_id: Uuid,
    #[diesel(sql_type = sql_types::Uuid)]
    pub sequence_id: Uuid,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Integer>)]
    pub last_offset_days: Option<i32>,
    #[diesel(sql_type = sql_types::Timestamp)]
    pub due_at: NaiveDateTime,
    #[diesel(sql_type = sql_types::Timestamp)]
    pub finalized_at: NaiveDateTime,
}

#[derive(Debug, Clone, QueryableByName)]
pub struct SentReminderRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub sent_email_id: Uuid,
    #[diesel(sql_type = sql_types::Integer)]
    pub offset_days: i32,
}

#[derive(Debug, QueryableByName)]
struct LastSentRow {
    #[diesel(sql_type = sql_types::Nullable<sql_types::Timestamp>)]
    last_at: Option<NaiveDateTime>,
}

/// Keyset position `(anchor or next_reminder_at, invoice_id)` of the last row of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanCursor {
    pub at: NaiveDateTime,
    pub invoice_id: InvoiceId,
}

pub async fn list_reminder_candidates(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoicing_entity_id: InvoicingEntityId,
    anchor_bound: NaiveDateTime,
    after: Option<ScanCursor>,
    limit: i64,
) -> DbResult<Vec<ReminderCandidateRow>> {
    diesel::sql_query(LIST_REMINDER_CANDIDATES_SQL)
        .bind::<sql_types::Uuid, _>(invoicing_entity_id.as_uuid())
        .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
        .bind::<sql_types::Timestamp, _>(anchor_bound)
        .bind::<sql_types::Nullable<sql_types::Timestamp>, _>(after.map(|c| c.at))
        .bind::<sql_types::Nullable<sql_types::Uuid>, _>(after.map(|c| c.invoice_id.as_uuid()))
        .bind::<sql_types::BigInt, _>(limit)
        .get_results(conn)
        .await
        .attach("Error while listing payment reminder candidates")
        .into_db_result()
}

pub async fn list_entity_reach(conn: &mut PgConn) -> DbResult<Vec<EntityReachRow>> {
    diesel::sql_query(LIST_ENTITY_REACH_SQL)
        .get_results(conn)
        .await
        .attach("Error while listing payment reminder entity reach")
        .into_db_result()
}

pub async fn list_reminder_due(
    conn: &mut PgConn,
    now: NaiveDateTime,
    limit: i64,
    after: Option<ScanCursor>,
) -> DbResult<Vec<ReminderDueRow>> {
    diesel::sql_query(LIST_REMINDER_DUE_SQL)
        .bind::<sql_types::Timestamp, _>(now)
        .bind::<sql_types::BigInt, _>(limit)
        .bind::<sql_types::Nullable<sql_types::Timestamp>, _>(after.map(|c| c.at))
        .bind::<sql_types::Nullable<sql_types::Uuid>, _>(after.map(|c| c.invoice_id.as_uuid()))
        .get_results(conn)
        .await
        .attach("Error while listing due payment reminders")
        .into_db_result()
}

pub async fn end_unpayable_scheduled(conn: &mut PgConn, now: NaiveDateTime) -> DbResult<usize> {
    diesel::sql_query(END_UNPAYABLE_SCHEDULED_SQL)
        .bind::<sql_types::Timestamp, _>(now)
        .execute(conn)
        .await
        .attach("Error while ending unpayable payment reminders")
        .into_db_result()
}

/// When the customer last received any email about the invoice (cross-kind cooldown).
pub async fn last_customer_email_at(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
) -> DbResult<Option<NaiveDateTime>> {
    let row: LastSentRow = diesel::sql_query(
        "SELECT MAX(s.sent_at) AS last_at FROM sent_email s \
         JOIN entity_activity a ON a.id = s.id \
         WHERE s.tenant_id = $1 AND a.entity_type = 'invoice' AND a.entity_id = $2",
    )
    .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<sql_types::Uuid, _>(invoice_id.as_uuid())
    .get_result(conn)
    .await
    .attach("Error while reading the last customer email for an invoice")
    .into_db_result()?;
    Ok(row.last_at)
}

/// Reminder emails sent for the invoice, newest first, with the step they carried.
pub async fn list_sent_reminders(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
) -> DbResult<Vec<SentReminderRow>> {
    diesel::sql_query(
        "SELECT s.id AS sent_email_id, (s.context ->> 'offset_days')::int AS offset_days \
         FROM sent_email s JOIN entity_activity a ON a.id = s.id \
         WHERE s.tenant_id = $1 AND a.entity_type = 'invoice' AND a.entity_id = $2 \
           AND s.kind = 'payment_reminder' AND (s.context ->> 'offset_days') IS NOT NULL \
         ORDER BY s.sent_at DESC",
    )
    .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<sql_types::Uuid, _>(invoice_id.as_uuid())
    .get_results(conn)
    .await
    .attach("Error while listing sent payment reminders")
    .into_db_result()
}

/// Whether a manual send with this `request_id` was already recorded for the invoice.
pub async fn manual_reminder_recorded(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
    request_id: &str,
) -> DbResult<bool> {
    #[derive(QueryableByName)]
    struct Found {
        #[diesel(sql_type = sql_types::Bool)]
        found: bool,
    }
    diesel::sql_query(
        "SELECT EXISTS (SELECT 1 FROM entity_activity a \
         WHERE a.tenant_id = $1 AND a.entity_type = 'invoice' AND a.entity_id = $2 \
           AND a.activity_type = 'payment_reminder.step_executed' \
           AND a.metadata ->> 'request_id' = $3) AS found",
    )
    .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<sql_types::Uuid, _>(invoice_id.as_uuid())
    .bind::<sql_types::Text, _>(request_id)
    .get_result::<Found>(conn)
    .await
    .map(|r| r.found)
    .attach("Error while checking for a recorded manual reminder")
    .into_db_result()
}

impl InvoicePaymentReminderRow {
    /// Enrolled, scheduled, not stopped rows of an entity (the ones a send-window change moves).
    pub async fn list_in_flight_for_entity(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> DbResult<Vec<ReminderInFlightRow>> {
        diesel::sql_query(
            "SELECT r.invoice_id, r.sequence_id, r.last_offset_days, i.due_at, i.finalized_at \
             FROM invoice_payment_reminder r JOIN invoice i ON i.id = r.invoice_id \
             WHERE r.tenant_id = $1 AND i.invoicing_entity_id = $2 AND r.state = 'ENROLLED' \
               AND r.stopped_at IS NULL AND r.next_reminder_at IS NOT NULL \
               AND r.sequence_id IS NOT NULL AND i.due_at IS NOT NULL AND i.finalized_at IS NOT NULL",
        )
        .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
        .bind::<sql_types::Uuid, _>(invoicing_entity_id.as_uuid())
        .get_results(conn)
        .await
        .attach("Error while listing in-flight payment reminders")
        .into_db_result()
    }
}

#[cfg(test)]
mod scan_tests {
    use super::*;

    /// The sweep takes `now` from Rust so runs are replayable; a session clock in the scan
    /// would silently drift from the bound value.
    #[test]
    fn scan_sql_has_no_session_clock() {
        for (name, sql) in [
            ("candidates", LIST_REMINDER_CANDIDATES_SQL),
            ("reach", LIST_ENTITY_REACH_SQL),
            ("due", LIST_REMINDER_DUE_SQL),
            ("end_unpayable", END_UNPAYABLE_SCHEDULED_SQL),
        ] {
            let lower = sql.to_ascii_lowercase();
            for clock in [
                "now()",
                "current_timestamp",
                "current_date",
                "localtimestamp",
                "clock_timestamp",
            ] {
                assert!(
                    !lower.contains(clock),
                    "{name} scan reads the session clock via {clock}"
                );
            }
        }
    }

    #[test]
    fn scans_are_tenant_scoped_and_hold_every_kill_switch() {
        assert!(LIST_REMINDER_CANDIDATES_SQL.contains("i.tenant_id = $2"));
        assert!(LIST_REMINDER_CANDIDATES_SQL.contains("r.invoice_id IS NULL"));
        for gate in [
            "sq.enabled",
            "t.email_delivery_mode <> 'OFF'",
            "NOT c.payment_reminders_opt_out",
            "'{payment_reminder}'",
            "r.stopped_at IS NULL",
        ] {
            assert!(
                LIST_REMINDER_DUE_SQL.contains(gate),
                "due scan misses {gate}"
            );
        }
    }
}
