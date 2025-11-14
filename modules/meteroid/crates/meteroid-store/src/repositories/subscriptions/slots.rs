#![allow(clippy::too_many_arguments)]

use crate::StoreResult;
use crate::domain::{SlotForTransaction, SlotTransactionStatusEnum};
use crate::errors::StoreError;
use crate::store::{PgConn, Store};
use chrono::NaiveTime;
use common_domain::ids::SlotTransactionId;
use common_domain::ids::{BaseId, InvoiceId, SubscriptionId, TenantId};
use diesel_models::slot_transactions::SlotTransactionRow;
use meteroid_store_macros::with_conn_delegate;

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
}

impl Store {
    pub async fn add_slot_transaction_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        current_period_end: chrono::NaiveDate,
        delta: i32,
        slot: &SlotForTransaction,
        ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<SlotTransactionRow> {
        self.add_slot_transaction_tx_internal(
            conn,
            tenant_id,
            subscription_id,
            None,
            current_period_end,
            delta,
            slot,
            ts,
            SlotTransactionStatusEnum::Active,
        )
        .await
    }

    pub(crate) async fn add_pending_slot_transaction_with_conn(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        current_period_end: chrono::NaiveDate,
        delta: i32,
        slot: &SlotForTransaction,
        invoice_id: InvoiceId,
        at_ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<SlotTransactionRow> {
        self.add_slot_transaction_tx_internal(
            conn,
            tenant_id,
            subscription_id,
            Some(invoice_id),
            current_period_end,
            delta,
            slot,
            at_ts,
            SlotTransactionStatusEnum::Pending,
        )
        .await
    }

    async fn add_slot_transaction_tx_internal(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        invoice_id: Option<InvoiceId>,
        current_period_end: chrono::NaiveDate,
        delta: i32,
        slot: &SlotForTransaction,
        ts: Option<chrono::NaiveDateTime>,
        status: SlotTransactionStatusEnum,
    ) -> StoreResult<SlotTransactionRow> {
        let now = ts.unwrap_or(chrono::Utc::now().naive_utc());
        let period_end = current_period_end.and_time(NaiveTime::MIN); // TODO MIN or MAX

        // Determine effective_at based on upgrade/downgrade
        let effective_at = if delta > 0 {
            // Upgrade: immediate (limited to current_period_end in case of backfilling)
            now.min(period_end)
        } else {
            // Downgrade: defer to next billing period
            period_end
        };

        let current_slots = SlotTransactionRow::fetch_by_subscription_id_and_unit_locked(
            conn,
            tenant_id,
            subscription_id,
            slot.unit.clone(),
            Some(effective_at),
        )
        .await?
        .current_active_slots;

        validate_slot_limits(slot, delta, current_slots)?;

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
        };

        transaction.insert(conn).await.map_err(Into::into)
    }

    pub(crate) async fn activate_pending_slot_transactions_for_invoice(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: common_domain::ids::InvoiceId,
    ) -> StoreResult<Vec<SlotTransactionRow>> {
        SlotTransactionRow::activate_pending_for_invoice(conn, tenant_id, invoice_id)
            .await
            .map_err(Into::into)
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
        SlotTransactionRow::fetch_by_subscription_id_and_unit_locked(
            conn,
            tenant_id,
            subscription_id,
            unit,
            at_ts,
        )
        .await
        .map(|c| c.current_active_slots as u32)
        .map_err(Into::into)
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
