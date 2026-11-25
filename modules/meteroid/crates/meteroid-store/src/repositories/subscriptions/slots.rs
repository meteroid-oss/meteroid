#![allow(clippy::too_many_arguments)]

use crate::StoreResult;
use crate::domain::{SlotForTransaction, SlotTransactionStatusEnum};
use crate::errors::StoreError;
use crate::store::{PgConn, Store};
use chrono::NaiveTime;
use common_domain::ids::SlotTransactionId;
use common_domain::ids::{BaseId, InvoiceId, SubscriptionId, TenantId};
use diesel_models::lock::CustomerLock;
use diesel_models::managed_quantities::ManagedQuantityRow;
use diesel_models::slot_transactions::SlotTransactionRow;
use diesel_models::subscriptions::SubscriptionRow;
use meteroid_store_macros::with_conn_delegate;
use scoped_futures::ScopedFutureExt;

#[with_conn_delegate]
pub trait SubscriptionSlotsInterface {
    #[delegated]
    async fn get_active_slots_value(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        unit: String,
        // default to now
        at_ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<u32>;

    #[delegated]
    async fn list_slot_transactions(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        unit: Option<String>,
        status: Option<SlotTransactionStatusEnum>,
    ) -> StoreResult<Vec<SlotTransactionRow>>;

    #[delegated]
    async fn cancel_slot_transaction(
        &self,
        tenant_id: TenantId,
        transaction_id: SlotTransactionId,
    ) -> StoreResult<()>;
}

impl Store {
    /// A change of `delta` seats: in force at `ts` (at most the period's end), or from the
    /// period's end when `deferred`. Only a deferred removal waits for the renewal (R-108).
    #[allow(clippy::too_many_arguments)]
    pub async fn add_slot_transaction_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        subscription_id: SubscriptionId,
        current_period_end: chrono::NaiveDate,
        delta: i32,
        slot: &SlotForTransaction,
        ts: Option<chrono::NaiveDateTime>,
        deferred: bool,
    ) -> StoreResult<SlotTransactionRow> {
        self.add_slot_transaction_tx_internal(
            conn,
            lock,
            subscription_id,
            None,
            current_period_end,
            delta,
            slot,
            ts,
            SlotTransactionStatusEnum::Active,
            deferred,
        )
        .await
    }

    pub(crate) async fn add_pending_slot_transaction_with_conn(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        subscription_id: SubscriptionId,
        current_period_end: chrono::NaiveDate,
        delta: i32,
        slot: &SlotForTransaction,
        invoice_id: InvoiceId,
        at_ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<SlotTransactionRow> {
        self.add_slot_transaction_tx_internal(
            conn,
            lock,
            subscription_id,
            Some(invoice_id),
            current_period_end,
            delta,
            slot,
            at_ts,
            SlotTransactionStatusEnum::Pending,
            false,
        )
        .await
    }

    async fn add_slot_transaction_tx_internal(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        subscription_id: SubscriptionId,
        invoice_id: Option<InvoiceId>,
        current_period_end: chrono::NaiveDate,
        delta: i32,
        slot: &SlotForTransaction,
        ts: Option<chrono::NaiveDateTime>,
        status: SlotTransactionStatusEnum,
        deferred: bool,
    ) -> StoreResult<SlotTransactionRow> {
        let now = ts.unwrap_or(common_clock::now().naive_utc());
        let period_end = current_period_end.and_time(NaiveTime::MIN); // TODO MAX => change after the inclusive period update

        let effective_at = if deferred {
            period_end
        } else {
            now.min(period_end)
        };

        let current_slots = SlotTransactionRow::fetch_by_subscription_id_and_unit_locked(
            conn,
            lock,
            subscription_id,
            slot.unit.clone(),
            Some(effective_at),
        )
        .await?
        .current_active_slots;

        // Seats bought and still paying count against the maximum, as they will be active.
        let pending = if delta > 0 {
            SlotTransactionRow::pending_upgrades(
                conn,
                lock.tenant_id(),
                subscription_id,
                slot.unit.clone(),
            )
            .await?
        } else {
            0
        };
        validate_slot_limits(slot, delta, current_slots + pending)?;
        // A change waiting for the renewal moves the count from there: the bounds hold there too.
        if effective_at < period_end {
            let at_renewal = SlotTransactionRow::fetch_by_subscription_id_and_unit_locked(
                conn,
                lock,
                subscription_id,
                slot.unit.clone(),
                Some(period_end),
            )
            .await?
            .current_active_slots;
            validate_slot_limits(slot, delta, at_renewal + pending)?;
        }

        let managed_quantity_id =
            ManagedQuantityRow::ensure_codes(conn, lock.tenant_id(), &[slot.unit.as_str()])
                .await?
                .remove(&slot.unit)
                .ok_or_else(|| StoreError::ValueNotFound(format!("quantity {}", slot.unit)))?;
        let transaction = SlotTransactionRow {
            id: SlotTransactionId::new(),
            subscription_id,
            delta,
            prev_active_slots: current_slots,
            effective_at,
            transaction_at: now,
            unit: slot.unit.clone(),
            status: status.into(),
            invoice_id,
            managed_quantity_id,
            removal_waits_for_renewal: delta < 0 && deferred,
        };

        let transaction = transaction.insert(conn).await?;
        crate::services::facts::pending::carry_into_set_seats_tx(
            conn,
            lock.tenant_id(),
            &transaction,
        )
        .await?;
        Ok(transaction)
    }

    pub(crate) async fn activate_pending_slot_transactions_for_invoice(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        invoice_id: common_domain::ids::InvoiceId,
        effective_at: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<Vec<SlotTransactionRow>> {
        let now = effective_at.unwrap_or(common_clock::now().naive_utc());

        let activated =
            SlotTransactionRow::activate_pending_for_invoice(conn, lock, invoice_id, now).await?;
        for transaction in &activated {
            crate::services::facts::pending::carry_into_set_seats_tx(
                conn,
                lock.tenant_id(),
                transaction,
            )
            .await?;
        }
        Ok(activated)
    }
}

#[async_trait::async_trait]
impl SubscriptionSlotsInterface for Store {
    async fn get_active_slots_value_with_conn(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        unit: String,
        at_ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<u32> {
        // A read: slot changes serialize on the subscription's customer (`add_slot_transaction_tx`).
        SlotTransactionRow::fetch_active_slots(
            conn,
            tenant_id,
            subscription_id,
            unit,
            at_ts.unwrap_or_else(|| common_clock::now().naive_utc()),
        )
        .await
        .map(|slots| slots.unwrap_or(0) as u32)
        .map_err(Into::into)
    }

    async fn list_slot_transactions_with_conn(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        unit: Option<String>,
        status: Option<SlotTransactionStatusEnum>,
    ) -> StoreResult<Vec<SlotTransactionRow>> {
        SlotTransactionRow::list_by_subscription_id(
            conn,
            tenant_id,
            subscription_id,
            unit,
            status.map(Into::into),
        )
        .await
        .map_err(Into::into)
    }

    async fn cancel_slot_transaction_with_conn(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        transaction_id: SlotTransactionId,
    ) -> StoreResult<()> {
        self.transaction_with(conn, |conn| {
            async move {
                let Some(subscription_id) =
                    SlotTransactionRow::subscription_of(conn, tenant_id, transaction_id).await?
                else {
                    return Ok(());
                };
                let lock = SubscriptionRow::lock_customer_of(conn, subscription_id).await?;
                SlotTransactionRow::delete_transaction(conn, tenant_id, transaction_id).await?;
                diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }
}

pub fn validate_slot_limits(
    slot: &SlotForTransaction,
    delta: i32,
    active_slots: i32,
) -> StoreResult<()> {
    let new_slot_count = active_slots + delta;

    // Check minimum slots and positive (min is u32)
    if let Some(min) = slot.min_slots
        && new_slot_count < min as i32
    {
        return Err(StoreError::InvalidArgument(format!(
                "Cannot reduce {} below minimum of {}. Current: {}, Requested change: {}, Would result in: {}",
                slot.unit, min, active_slots, delta, new_slot_count
            )).into());
    }

    // Check maximum slots
    if let Some(max) = slot.max_slots
        && new_slot_count > max as i32
    {
        return Err(StoreError::InvalidArgument(format!(
                "Cannot exceed maximum {} limit of {}. Current: {}, Requested change: {}, Would result in: {}",
                slot.unit, max, active_slots, delta, new_slot_count
            )).into());
    }

    Ok(())
}

/// The ledger rows of new seat transactions, each counting the managed quantity of its unit.
pub(crate) async fn ledger_rows<'a>(
    conn: &mut PgConn,
    tenant_id: TenantId,
    transactions: impl IntoIterator<
        Item = &'a (
            SubscriptionId,
            crate::domain::slot_transactions::SlotTransactionNewInternal,
        ),
    >,
) -> StoreResult<Vec<SlotTransactionRow>> {
    let transactions: Vec<_> = transactions.into_iter().collect();
    let mut codes: Vec<&str> = transactions
        .iter()
        .map(|(_, tx)| tx.unit.as_str())
        .collect();
    codes.sort_unstable();
    codes.dedup();
    let ids = ManagedQuantityRow::ensure_codes(conn, tenant_id, &codes).await?;
    transactions
        .into_iter()
        .map(|(subscription_id, tx)| {
            let quantity = *ids
                .get(&tx.unit)
                .ok_or_else(|| StoreError::ValueNotFound(format!("quantity {}", tx.unit)))?;
            Ok(SlotTransactionRow {
                id: tx.id,
                subscription_id: *subscription_id,
                delta: tx.delta,
                prev_active_slots: tx.prev_active_slots,
                effective_at: tx.effective_at,
                transaction_at: tx.transaction_at,
                unit: tx.unit.clone(),
                status: SlotTransactionStatusEnum::Active.into(),
                invoice_id: None,
                managed_quantity_id: quantity,
                removal_waits_for_renewal: true,
            })
        })
        .collect()
}
