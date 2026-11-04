pub mod hosted_invoice;
pub mod hosted_payment_sweep;
pub mod hosted_return;
pub mod hosted_setup;
pub(in crate::services) mod manual;
pub mod method;
pub mod process_payment;
pub mod reconcile;
pub mod refund;
mod resume_authentication;
mod sepa;
pub mod webhook_backed_setup;

use crate::StoreResult;
use crate::adapters::payment::{PaymentConnector, initialize_payment_connector};
use crate::domain::connectors::Connector;
use crate::errors::StoreError;
use crate::store::{PgConn, Store};
use common_domain::ids::{CustomerPaymentMethodId, TenantId};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use error_stack::{Report, ResultExt};

/// The connector a stored payment method runs through: method → customer connection →
/// connector. Every provider call on an existing transaction resolves it this way.
pub(in crate::services) async fn resolve_connector_for_method(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    method_id: CustomerPaymentMethodId,
) -> StoreResult<(Connector, Box<dyn PaymentConnector>)> {
    let method = CustomerPaymentMethodRow::get_by_id(conn, &tenant_id, &method_id)
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;
    let connection =
        CustomerConnectionDetailsRow::get_by_id(conn, &tenant_id, &method.connection_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

    let connector = Connector::from_row(&store.settings.crypt_key, connection.connector)?;
    let connector_impl = initialize_payment_connector(&connector)
        .change_context(StoreError::PaymentProviderError)?;
    Ok((connector, connector_impl))
}

/// `resolve_connector_for_method` for a transaction row, which must carry a payment method.
pub(in crate::services) async fn resolve_connector_for_transaction(
    store: &Store,
    conn: &mut PgConn,
    row: &diesel_models::payments::PaymentTransactionRow,
) -> StoreResult<(Connector, Box<dyn PaymentConnector>)> {
    let method_id = row.payment_method_id.ok_or_else(|| {
        Report::new(StoreError::InvalidArgument(
            "transaction has no payment method".into(),
        ))
    })?;
    resolve_connector_for_method(store, conn, row.tenant_id, method_id).await
}
