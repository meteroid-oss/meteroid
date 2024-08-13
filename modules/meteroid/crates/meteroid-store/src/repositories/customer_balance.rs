use crate::domain::Customer;
use crate::errors::StoreError;
use crate::store::PgConn;
use crate::StoreResult;
use diesel_models::customer_balance_txs::CustomerBalanceTxRowNew;
use diesel_models::customers::CustomerRow;
use diesel_models::errors::DatabaseError;
use error_stack::Report;
use uuid::Uuid;

pub struct CustomerBalance;

impl CustomerBalance {
    pub async fn update(
        conn: &mut PgConn,
        customer_id: Uuid,
        tenant_id: Uuid,
        cents: i32,
        invoice_id: Option<Uuid>,
    ) -> StoreResult<Customer> {
        let _ = CustomerRow::select_for_update(conn, customer_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let _ = CustomerRow::update_balance(conn, customer_id, cents)
            .await
            .map_err(|err| match err.error.current_context() {
                DatabaseError::CheckViolation(_) => {
                    error_stack::report!(StoreError::NegativeCustomerBalanceError(err.error,))
                }
                _ => Into::<Report<StoreError>>::into(err),
            })?;

        let customer_row_updated = CustomerRow::find_by_id(conn, customer_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let _ = CustomerBalanceTxRowNew {
            id: Uuid::now_v7(),
            amount_cents: cents,
            balance_cents_after: customer_row_updated.balance_value_cents,
            note: None,
            invoice_id,
            tenant_id,
            customer_id,
            created_by: None,
        }
        .insert(conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        customer_row_updated.try_into()
    }
}
