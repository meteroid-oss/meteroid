use crate::StoreResult;
use crate::domain::Customer;
use crate::errors::{StoreError, StoreErrorReport};
use crate::store::PgConn;
use common_domain::ids::{CustomerId, InvoiceId, TenantId};
use diesel_models::customer_balance_txs::CustomerBalanceTxRowNew;
use diesel_models::customers::CustomerRow;
use diesel_models::errors::DatabaseError;
use error_stack::{IntoReport, Report};
use uuid::Uuid;

pub struct CustomerBalanceUpdate {
    pub customer: Customer,
    pub tx_id: Uuid,
}

pub struct CustomerBalance;

impl CustomerBalance {
    pub async fn update(
        conn: &mut PgConn,
        customer_id: CustomerId,
        tenant_id: TenantId,
        cents: i64,
        invoice_id: Option<InvoiceId>,
    ) -> StoreResult<CustomerBalanceUpdate> {
        let _ = CustomerRow::select_for_update(conn, customer_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let _ = CustomerRow::update_balance(conn, customer_id, cents)
            .await
            .map_err(|err| match err.error.current_context() {
                DatabaseError::CheckViolation(_) => {
                    StoreError::NegativeCustomerBalanceError(err.error).into_report()
                }
                _ => Into::<StoreErrorReport>::into(err),
            })?;

        let customer_row_updated = CustomerRow::find_by_id(conn, &customer_id, &tenant_id)
            .await
            .map_err(Into::<StoreErrorReport>::into)?;

        let tx = CustomerBalanceTxRowNew {
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

        Ok(CustomerBalanceUpdate {
            customer: customer_row_updated.try_into()?,
            tx_id: tx.id,
        })
    }
}
