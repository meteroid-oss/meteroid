//! Amounts too small to collect, carried from the invoice that billed them to the customer's
//! next invoice in the same currency and invoicing entity. Every write holds the customer lock.
use crate::errors::IntoDbResult;
use crate::lock::CustomerLock;
use crate::{DbResult, PgConn};
use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, InvoiceId, InvoicingEntityId, TenantId};
use diesel::{ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::invoice_carry_forward)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoiceCarryForwardRow {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub currency: String,
    pub amount: i64,
    pub source_invoice_id: InvoiceId,
    pub applied_invoice_id: Option<InvoiceId>,
    pub applied_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::invoice_carry_forward)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoiceCarryForwardRowNew {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub currency: String,
    pub amount: i64,
    pub source_invoice_id: InvoiceId,
}

/// Where carried balances may go: one customer's invoices in one currency, from one entity.
#[derive(Debug, Clone, Copy)]
pub struct CarryTarget<'a> {
    pub invoicing_entity_id: InvoicingEntityId,
    pub currency: &'a str,
}

impl InvoiceCarryForwardRowNew {
    pub async fn insert(self, conn: &mut PgConn, lock: &CustomerLock) -> DbResult<()> {
        lock.covers(conn, self.tenant_id, self.customer_id)?;
        use crate::schema::invoice_carry_forward::dsl as c;
        diesel::insert_into(c::invoice_carry_forward)
            .values(&self)
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while carrying an invoice balance forward")
            .into_db_result()
    }
}

impl InvoiceCarryForwardRow {
    /// Takes every balance still waiting for `target` onto `invoice_id`; returns their sum.
    pub async fn apply_pending(
        conn: &mut PgConn,
        lock: &CustomerLock,
        target: CarryTarget<'_>,
        invoice_id: InvoiceId,
        at: DateTime<Utc>,
    ) -> DbResult<i64> {
        lock.on(conn);
        use crate::schema::invoice_carry_forward::dsl as c;
        let amounts: Vec<i64> = diesel::update(c::invoice_carry_forward)
            .filter(c::tenant_id.eq(lock.tenant_id()))
            .filter(c::customer_id.eq(lock.customer_id()))
            .filter(c::invoicing_entity_id.eq(target.invoicing_entity_id))
            .filter(c::currency.eq(target.currency))
            .filter(c::applied_invoice_id.is_null())
            .filter(c::amount.gt(0))
            .filter(c::source_invoice_id.ne(invoice_id))
            .set((c::applied_invoice_id.eq(invoice_id), c::applied_at.eq(at)))
            .returning(c::amount)
            .get_results(conn)
            .await
            .attach("Error while applying carried balances")
            .into_db_result()?;
        Ok(amounts.iter().sum())
    }

    /// What is waiting for `target`, without taking it.
    pub async fn sum_pending(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        target: CarryTarget<'_>,
    ) -> DbResult<i64> {
        use crate::schema::invoice_carry_forward::dsl as c;
        let amounts: Vec<i64> = c::invoice_carry_forward
            .filter(c::tenant_id.eq(tenant_id))
            .filter(c::customer_id.eq(customer_id))
            .filter(c::invoicing_entity_id.eq(target.invoicing_entity_id))
            .filter(c::currency.eq(target.currency))
            .filter(c::applied_invoice_id.is_null())
            .select(c::amount)
            .load(conn)
            .await
            .attach("Error while summing carried balances")
            .into_db_result()?;
        Ok(amounts.iter().sum())
    }

    /// Hands the balances `invoice_id` took back to the next invoice, when it is voided.
    pub async fn release_applied(
        conn: &mut PgConn,
        lock: &CustomerLock,
        invoice_id: InvoiceId,
    ) -> DbResult<i64> {
        lock.on(conn);
        use crate::schema::invoice_carry_forward::dsl as c;
        let amounts: Vec<i64> = diesel::update(c::invoice_carry_forward)
            .filter(c::tenant_id.eq(lock.tenant_id()))
            .filter(c::customer_id.eq(lock.customer_id()))
            .filter(c::applied_invoice_id.eq(invoice_id))
            .set((
                c::applied_invoice_id.eq(None::<InvoiceId>),
                c::applied_at.eq(None::<DateTime<Utc>>),
            ))
            .returning(c::amount)
            .get_results(conn)
            .await
            .attach("Error while releasing carried balances")
            .into_db_result()?;
        Ok(amounts.iter().sum())
    }

    pub async fn find_by_source(
        conn: &mut PgConn,
        tenant_id: TenantId,
        source_invoice_id: InvoiceId,
    ) -> DbResult<Option<Self>> {
        use crate::schema::invoice_carry_forward::dsl as c;
        use diesel::OptionalExtension;
        c::invoice_carry_forward
            .filter(c::tenant_id.eq(tenant_id))
            .filter(c::source_invoice_id.eq(source_invoice_id))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a carried balance")
            .into_db_result()
    }

    /// Drops the balance `source_invoice_id` carried while no invoice has taken it yet.
    pub async fn delete_pending_by_source(
        conn: &mut PgConn,
        lock: &CustomerLock,
        source_invoice_id: InvoiceId,
    ) -> DbResult<usize> {
        lock.on(conn);
        use crate::schema::invoice_carry_forward::dsl as c;
        diesel::delete(c::invoice_carry_forward)
            .filter(c::tenant_id.eq(lock.tenant_id()))
            .filter(c::customer_id.eq(lock.customer_id()))
            .filter(c::source_invoice_id.eq(source_invoice_id))
            .filter(c::applied_invoice_id.is_null())
            .execute(conn)
            .await
            .attach("Error while dropping a carried balance")
            .into_db_result()
    }

    /// Takes back every debt still waiting in `currency`, when no invoice will take it; returns
    /// each source invoice with the debt it gets back.
    pub async fn take_back_pending(
        conn: &mut PgConn,
        lock: &CustomerLock,
        currency: &str,
    ) -> DbResult<Vec<(InvoiceId, i64)>> {
        lock.on(conn);
        use crate::schema::invoice_carry_forward::dsl as c;
        diesel::delete(c::invoice_carry_forward)
            .filter(c::tenant_id.eq(lock.tenant_id()))
            .filter(c::customer_id.eq(lock.customer_id()))
            .filter(c::currency.eq(currency))
            .filter(c::applied_invoice_id.is_null())
            .filter(c::amount.gt(0))
            .returning((c::source_invoice_id, c::amount))
            .get_results(conn)
            .await
            .attach("Error while taking back carried debts")
            .into_db_result()
    }

    /// Whether a draft that bills a subscription will still take what is carried in `currency`.
    pub async fn customer_has_draft_in(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        currency: &str,
    ) -> DbResult<bool> {
        use crate::enums::{InvoiceStatusEnum, InvoiceType};
        use crate::schema::invoice::dsl as i;
        diesel::select(diesel::dsl::exists(
            i::invoice
                .filter(i::tenant_id.eq(tenant_id))
                .filter(i::customer_id.eq(customer_id))
                .filter(i::currency.eq(currency))
                .filter(i::status.eq(InvoiceStatusEnum::Draft))
                .filter(i::invoice_type.eq_any([InvoiceType::Recurring, InvoiceType::Final])),
        ))
        .get_result(conn)
        .await
        .attach("Error while looking for a draft invoice")
        .into_db_result()
    }

    /// The balances carried into `invoice_id`, oldest first, for its document.
    pub async fn list_applied_to(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::invoice_carry_forward::dsl as c;
        c::invoice_carry_forward
            .filter(c::tenant_id.eq(tenant_id))
            .filter(c::applied_invoice_id.eq(invoice_id))
            .order(c::created_at.asc())
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while listing carried balances")
            .into_db_result()
    }

    /// Lowers the debt `source_invoice_id` carried by `amount`, a credit note cancelling it.
    pub async fn cancel_debt(
        conn: &mut PgConn,
        lock: &CustomerLock,
        source_invoice_id: InvoiceId,
        amount: i64,
    ) -> DbResult<()> {
        lock.on(conn);
        use crate::schema::invoice_carry_forward::dsl as c;
        diesel::update(c::invoice_carry_forward)
            .filter(c::tenant_id.eq(lock.tenant_id()))
            .filter(c::customer_id.eq(lock.customer_id()))
            .filter(c::source_invoice_id.eq(source_invoice_id))
            .set(c::amount.eq(c::amount - amount))
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while cancelling a carried debt")
            .into_db_result()
    }

    /// The invoices whose debt `invoice_id` took.
    pub async fn sources_of(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> DbResult<Vec<InvoiceId>> {
        use crate::schema::invoice_carry_forward::dsl as c;
        c::invoice_carry_forward
            .filter(c::tenant_id.eq(tenant_id))
            .filter(c::applied_invoice_id.eq(invoice_id))
            .order(c::created_at.asc())
            .select(c::source_invoice_id)
            .load(conn)
            .await
            .attach("Error while listing carried sources")
            .into_db_result()
    }

    /// Whether the customer keeps a subscription in `currency` running past `after`, so that a
    /// later invoice can take what is carried: neither its end date nor a cancellation or term
    /// end (scheduled or not) ends it by then.
    pub async fn customer_has_later_invoice(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        currency: &str,
        after: chrono::NaiveDate,
    ) -> DbResult<bool> {
        use crate::enums::SubscriptionStatusEnum;
        use crate::schema::subscription::dsl as s;
        use crate::schema::subscription_ending::dsl as e;
        use diesel::BoolExpressionMethods;
        let ends_by = after.and_time(chrono::NaiveTime::MIN);
        diesel::select(diesel::dsl::exists(
            s::subscription
                .filter(s::tenant_id.eq(tenant_id))
                .filter(s::customer_id.eq(customer_id))
                .filter(s::currency.eq(currency))
                .filter(s::status.eq_any(SubscriptionStatusEnum::not_terminal()))
                .filter(s::end_date.is_null().or(s::end_date.gt(after)))
                .filter(diesel::dsl::not(diesel::dsl::exists(
                    e::subscription_ending
                        .filter(e::subscription_id.eq(s::id))
                        .filter(e::tenant_id.eq(tenant_id))
                        .filter(e::at.le(ends_by)),
                ))),
        ))
        .get_result(conn)
        .await
        .attach("Error while looking for a later invoice")
        .into_db_result()
    }

    /// The balances `invoice_id` carried out or took in, with the other invoice's number.
    pub async fn list_links(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> DbResult<Vec<CarryLink>> {
        use crate::schema::invoice::dsl as i;
        use crate::schema::invoice_carry_forward::dsl as c;
        use diesel::BoolExpressionMethods;
        let rows: Vec<Self> = c::invoice_carry_forward
            .filter(c::tenant_id.eq(tenant_id))
            .filter(
                c::source_invoice_id
                    .eq(invoice_id)
                    .or(c::applied_invoice_id.eq(invoice_id)),
            )
            .order(c::created_at.asc())
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while listing carried balances")
            .into_db_result()?;
        let others: Vec<InvoiceId> = rows
            .iter()
            .filter_map(|row| {
                if row.source_invoice_id == invoice_id {
                    row.applied_invoice_id
                } else {
                    Some(row.source_invoice_id)
                }
            })
            .collect();
        let numbers: std::collections::HashMap<InvoiceId, String> = i::invoice
            .filter(i::tenant_id.eq(tenant_id))
            .filter(i::id.eq_any(&others))
            .select((i::id, i::invoice_number))
            .load::<(InvoiceId, String)>(conn)
            .await
            .attach("Error while reading carried invoice numbers")
            .into_db_result()?
            .into_iter()
            .collect();
        Ok(rows
            .into_iter()
            .map(|row| {
                let outgoing = row.source_invoice_id == invoice_id;
                let other = if outgoing {
                    row.applied_invoice_id
                } else {
                    Some(row.source_invoice_id)
                };
                CarryLink {
                    outgoing,
                    amount: row.amount,
                    invoice_id: other,
                    invoice_number: other.and_then(|id| numbers.get(&id).cloned()),
                }
            })
            .collect())
    }
}

/// A balance seen from one invoice: carried out to `invoice_id` (none yet while pending), or
/// taken in from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarryLink {
    pub outgoing: bool,
    pub amount: i64,
    pub invoice_id: Option<InvoiceId>,
    pub invoice_number: Option<String>,
}
