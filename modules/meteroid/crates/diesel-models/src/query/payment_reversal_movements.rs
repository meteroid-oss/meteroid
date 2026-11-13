use crate::errors::IntoDbResult;
use crate::lock::CustomerLock;
use crate::payment_reversal_movements::{
    PaymentReversalMovementRow, PaymentReversalMovementRowNew,
};
use crate::{DbResult, PgConn};
use common_domain::ids::PaymentTransactionId;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl PaymentReversalMovementRowNew {
    /// The caller holds the payment's customer. `None` when the provider reference was already
    /// applied in this direction (a redelivery).
    pub async fn insert(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
    ) -> DbResult<Option<PaymentReversalMovementRow>> {
        use crate::schema::payment_reversal_movement::dsl as m;
        lock.covers(conn, self.tenant_id, self.customer_id)?;
        diesel::insert_into(m::payment_reversal_movement)
            .values((
                self,
                crate::schema::payment_reversal_movement::created_at
                    .eq(common_domain::clock::now().naive_utc()),
            ))
            .on_conflict_do_nothing()
            .returning(PaymentReversalMovementRow::as_returning())
            .get_result(conn)
            .await
            .optional()
            .attach("Error while recording a payment reversal movement")
            .into_db_result()
    }
}

impl PaymentReversalMovementRow {
    /// Whether this provider reference already moved the payment in this direction (its
    /// withdrawal may have been met by a reinstatement held for it, see `held_reinstatement`).
    pub async fn applied(
        conn: &mut PgConn,
        payment: PaymentTransactionId,
        provider_ref: &str,
        reversal: bool,
    ) -> DbResult<bool> {
        use crate::schema::payment_reversal_movement::dsl as m;
        let moves: Vec<(i64, i64)> = m::payment_reversal_movement
            .filter(m::payment_transaction_id.eq(payment))
            .filter(m::provider_ref.eq_any([provider_ref.to_owned(), held_ref(provider_ref)]))
            .select((m::amount, m::excess))
            .load(conn)
            .await
            .attach("Error while reading payment reversal movements")
            .into_db_result()?;
        Ok(moves
            .iter()
            .any(|(amount, excess)| (amount + excess > 0) == reversal))
    }

    pub async fn list_by_payment(
        conn: &mut PgConn,
        tenant_id: common_domain::ids::TenantId,
        payment: PaymentTransactionId,
    ) -> DbResult<Vec<PaymentReversalMovementRow>> {
        use crate::schema::payment_reversal_movement::dsl as m;
        m::payment_reversal_movement
            .filter(m::tenant_id.eq(tenant_id))
            .filter(m::payment_transaction_id.eq(payment))
            .order((m::occurred_at.asc(), m::created_at.asc()))
            .select(PaymentReversalMovementRow::as_select())
            .load(conn)
            .await
            .attach("Error while listing payment reversal movements")
            .into_db_result()
    }

    /// How many movements the payment has and the net amount they applied.
    pub async fn applied_total(
        conn: &mut PgConn,
        payment: PaymentTransactionId,
    ) -> DbResult<(i64, i64)> {
        use crate::schema::payment_reversal_movement::dsl as m;
        let amounts: Vec<i64> = m::payment_reversal_movement
            .filter(m::payment_transaction_id.eq(payment))
            .select(m::amount)
            .load(conn)
            .await
            .attach("Error while summing payment reversal movements")
            .into_db_result()?;
        Ok((amounts.len() as i64, amounts.iter().sum()))
    }

    /// The latest withdrawal by a dispute on the invoice's payments, or on `payment` alone.
    pub async fn latest_disputed_withdrawal(
        conn: &mut PgConn,
        tenant_id: common_domain::ids::TenantId,
        invoice: Option<common_domain::ids::InvoiceId>,
        payment: Option<PaymentTransactionId>,
    ) -> DbResult<Option<PaymentReversalMovementRow>> {
        use crate::schema::payment_reversal_movement::dsl as m;
        let mut query = m::payment_reversal_movement
            .filter(m::tenant_id.eq(tenant_id))
            .filter(m::disputed.eq(true))
            .filter(m::amount.gt(0))
            .into_boxed();
        query = match (invoice, payment) {
            (Some(invoice), _) => query.filter(m::invoice_id.eq(invoice)),
            (None, Some(payment)) => query.filter(m::payment_transaction_id.eq(payment)),
            (None, None) => return Ok(None),
        };
        query
            .order((m::occurred_at.desc(), m::created_at.desc()))
            .select(PaymentReversalMovementRow::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading the latest disputed withdrawal")
            .into_db_result()
    }

    /// What a reinstatement that arrived before its withdrawal still holds for it: recorded as a
    /// marker (`amount` 0, `excess` −held), consumed by markers under `held_ref` as withdrawals
    /// arrive.
    pub async fn held_reinstatement(
        conn: &mut PgConn,
        payment: PaymentTransactionId,
        provider_ref: &str,
    ) -> DbResult<i64> {
        use crate::schema::payment_reversal_movement::dsl as m;
        let excess: Vec<i64> = m::payment_reversal_movement
            .filter(m::payment_transaction_id.eq(payment))
            .filter(m::provider_ref.eq_any([provider_ref.to_owned(), held_ref(provider_ref)]))
            .filter(m::amount.eq(0))
            .select(m::excess)
            .load(conn)
            .await
            .attach("Error while reading held reinstatements")
            .into_db_result()?;
        Ok(-excess.iter().sum::<i64>())
    }
}

/// The reference a withdrawal met by a held reinstatement is recorded under.
pub fn held_ref(provider_ref: &str) -> String {
    format!("{provider_ref}:held")
}
