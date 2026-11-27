#![allow(clippy::too_many_arguments)]

use crate::StoreResult;
use crate::domain::{SlotForTransaction, SlotTransactionStatusEnum};
use crate::errors::StoreError;
use crate::store::{PgConn, Store};
use chrono::NaiveTime;
use common_domain::ids::SlotTransactionId;
use common_domain::ids::{BaseId, InvoiceId, ManagedQuantityId, SubscriptionId, TenantId};
use diesel_models::lock::CustomerLock;
use diesel_models::managed_quantities::ManagedQuantityRow;
use diesel_models::slot_transactions::SlotTransactionRow;
use diesel_models::subscriptions::SubscriptionRow;
use meteroid_store_macros::with_conn_delegate;
use scoped_futures::ScopedFutureExt;

#[with_conn_delegate]
pub trait SubscriptionSlotsInterface {
    /// The seats of `quantity` the subscription holds at `at_ts` (now when None).
    #[delegated]
    async fn get_active_slots_value(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        quantity: ManagedQuantityId,
        // default to now
        at_ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<u32>;

    #[delegated]
    async fn list_slot_transactions(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        quantity: Option<ManagedQuantityId>,
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
    /// period's end when `deferred`. Only a deferred removal waits for the renewal (R-108). The
    /// period's end is the earliest of the charges counting the quantity; `renewals` are each
    /// one's, where the bounds hold too (R-137).
    #[allow(clippy::too_many_arguments)]
    pub async fn add_slot_transaction_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        subscription_id: SubscriptionId,
        current_period_end: chrono::NaiveDate,
        renewals: &[chrono::NaiveDate],
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
            renewals,
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
        renewals: &[chrono::NaiveDate],
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
            renewals,
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
        renewals: &[chrono::NaiveDate],
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

        let current_slots = SlotTransactionRow::fetch_by_subscription_id_and_quantity_locked(
            conn,
            lock,
            subscription_id,
            slot.quantity_id,
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
                slot.quantity_id,
            )
            .await?
        } else {
            0
        };
        validate_slot_limits(slot, delta, current_slots + pending)?;
        // A change waiting for a renewal moves the count from there: the bounds hold at the
        // renewal of every charge counting the quantity too.
        let mut renewals: Vec<chrono::NaiveDateTime> = renewals
            .iter()
            .map(|day| day.and_time(NaiveTime::MIN))
            .chain(std::iter::once(period_end))
            .filter(|at| *at > effective_at)
            .collect();
        renewals.sort();
        renewals.dedup();
        for renewal in renewals {
            let at_renewal = SlotTransactionRow::fetch_by_subscription_id_and_quantity_locked(
                conn,
                lock,
                subscription_id,
                slot.quantity_id,
                Some(renewal),
            )
            .await?
            .current_active_slots;
            validate_slot_limits(slot, delta, at_renewal + pending)?;
        }

        // The ledger's unit is its quantity's code (I32).
        let code = ManagedQuantityRow::codes_of(conn, lock.tenant_id(), &[slot.quantity_id])
            .await?
            .remove(&slot.quantity_id)
            .ok_or_else(|| StoreError::ValueNotFound(format!("quantity {}", slot.quantity_id)))?;
        let transaction = SlotTransactionRow {
            id: SlotTransactionId::new(),
            subscription_id,
            delta,
            prev_active_slots: current_slots,
            effective_at,
            transaction_at: now,
            unit: code,
            status: status.into(),
            invoice_id,
            managed_quantity_id: slot.quantity_id,
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
        quantity: ManagedQuantityId,
        at_ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<u32> {
        // A read: slot changes serialize on the subscription's customer (`add_slot_transaction_tx`).
        SlotTransactionRow::fetch_active_slots(
            conn,
            tenant_id,
            subscription_id,
            quantity,
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
        quantity: Option<ManagedQuantityId>,
        status: Option<SlotTransactionStatusEnum>,
    ) -> StoreResult<Vec<SlotTransactionRow>> {
        SlotTransactionRow::list_by_subscription_id(
            conn,
            tenant_id,
            subscription_id,
            quantity,
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

    // A reduction never goes below the minimum; held below a phase's floor, seats may still grow.
    if let Some(min) = slot.min_slots
        && delta < 0
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

/// The ledger rows of new seat transactions, each counting the managed quantity it names, else
/// (a fee recorded before quantities were named) the one its unit names (`ensure_quantity_of_unit`);
/// a row's unit is its quantity's code (I32).
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
    let mut units: Vec<&str> = transactions
        .iter()
        .filter(|(_, tx)| tx.quantity_id.is_none())
        .map(|(_, tx)| tx.unit.as_str())
        .collect();
    units.sort_unstable();
    units.dedup();
    // The quantity each unit names, as a charge naming none counts it.
    let mut by_unit = std::collections::HashMap::new();
    for unit in units {
        let row =
            crate::repositories::managed_quantities::ensure_quantity_of_unit(conn, tenant_id, unit)
                .await?;
        by_unit.insert(unit.to_owned(), (row.id, row.code));
    }
    let mut named: Vec<ManagedQuantityId> = transactions
        .iter()
        .filter_map(|(_, tx)| tx.quantity_id)
        .collect();
    named.sort_by_key(|id| id.as_uuid());
    named.dedup();
    let code_of = ManagedQuantityRow::codes_of(conn, tenant_id, &named).await?;
    transactions
        .into_iter()
        .map(|(subscription_id, tx)| {
            let (quantity, code) = match tx.quantity_id {
                Some(id) => (
                    id,
                    code_of
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| StoreError::ValueNotFound(format!("quantity {id}")))?,
                ),
                None => by_unit
                    .get(&tx.unit)
                    .cloned()
                    .ok_or_else(|| StoreError::ValueNotFound(format!("quantity {}", tx.unit)))?,
            };
            Ok(SlotTransactionRow {
                id: tx.id,
                subscription_id: *subscription_id,
                delta: tx.delta,
                prev_active_slots: tx.prev_active_slots,
                effective_at: tx.effective_at,
                transaction_at: tx.transaction_at,
                unit: code,
                status: SlotTransactionStatusEnum::Active.into(),
                invoice_id: None,
                managed_quantity_id: quantity,
                removal_waits_for_renewal: true,
            })
        })
        .collect()
}

/// Names the quantity of each seat fee recorded before quantities were named: its tenant's
/// quantity its unit names (`quantity_of_unit`: coded as it first, which counted its seats so
/// far, I32). A fee naming one is left alone; a unit with no quantity has no seat ledger either,
/// so stays unnamed.
pub(crate) async fn name_seat_quantities<'a>(
    conn: &mut PgConn,
    tenant_id: TenantId,
    // Collected by the caller: an iterator of closures held across the await would not be Send.
    fees: Vec<&'a mut crate::domain::SubscriptionFee>,
) -> StoreResult<()> {
    use crate::domain::SubscriptionFee;

    let unnamed: Vec<&'a mut SubscriptionFee> = fees
        .into_iter()
        .filter(|fee| {
            matches!(
                fee,
                SubscriptionFee::Slot {
                    quantity_id: None,
                    ..
                }
            )
        })
        .collect();
    if unnamed.is_empty() {
        return Ok(());
    }
    let mut units: Vec<String> = unnamed
        .iter()
        .filter_map(|fee| match &**fee {
            SubscriptionFee::Slot { unit, .. } => Some(unit.clone()),
            _ => None,
        })
        .collect();
    units.sort_unstable();
    units.dedup();
    let unit_refs: Vec<&str> = units.iter().map(String::as_str).collect();
    let quantities =
        crate::repositories::managed_quantities::quantities_of_units(conn, tenant_id, &unit_refs)
            .await?;
    for fee in unnamed {
        if let SubscriptionFee::Slot {
            unit, quantity_id, ..
        } = fee
        {
            *quantity_id = quantities.get(unit.as_str()).map(|row| row.id);
        }
    }
    Ok(())
}
