use crate::StoreResult;
use crate::store::{PgConn, Store};
use common_domain::ids::{PriceComponentId, SubscriptionId, TenantId};
use diesel_models::slot_transactions::SlotTransactionRow;
use meteroid_store_macros::with_conn_delegate;

#[with_conn_delegate]
pub trait SubscriptionSlotsInterface {
    #[delegated]
    async fn get_current_slots_value(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        unit: String,
        ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<u32>;

    async fn _add_slot_transaction(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        price_component_id: PriceComponentId,
        slots: i32,
    ) -> StoreResult<i32>;
}

#[async_trait::async_trait]
impl SubscriptionSlotsInterface for Store {
    async fn get_current_slots_value_with_conn(
        &self,
        conn: &mut PgConn,
        _tenant_id: TenantId,
        subscription_id: SubscriptionId,
        unit: String,
        ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<u32> {
        SlotTransactionRow::fetch_by_subscription_id_and_unit(conn, subscription_id, unit, ts)
            .await
            .map(|c| c.current_active_slots as u32)
            .map_err(Into::into)
    }

    async fn _add_slot_transaction(
        &self,
        _tenant_id: TenantId,
        _subscription_id: SubscriptionId,
        _price_component_id: PriceComponentId,
        _slots: i32,
    ) -> StoreResult<i32> {
        todo!()
        /*
        sequenceDiagram
            participant User
            participant Billing Software
            participant Database
            participant Stripe API

            User->>Billing Software: Request to add seats
            Billing Software->>Database: Check current subscription details
            Database-->>Billing Software: Return subscription details
            Billing Software->>Billing Software: Calculate prorated amount for additional seats
            Billing Software->>User: Display prorated charge and request payment approval

            User->>Billing Software: Approve payment
            Billing Software->>Stripe API: Create payment intent with prorated amount
            Stripe API-->>Billing Software: Payment intent created (awaiting confirmation)

            User->>Stripe API: Confirm and process payment
            Stripe API-->>Billing Software: Payment success notification
            Billing Software->>Database: Update subscription (add seats)
            Database-->>Billing Software: Subscription updated
            Billing Software->>Stripe API: Generate invoice for the transaction
            Stripe API-->>Billing Software: Invoice generated
            Billing Software->>User: Confirm seat addition and send invoice

            Billing Software->>Database: Log transaction details
            Database-->>Billing Software: Transaction logged
            Billing Software->>User: Notify transaction completion

                 */
    }
}
