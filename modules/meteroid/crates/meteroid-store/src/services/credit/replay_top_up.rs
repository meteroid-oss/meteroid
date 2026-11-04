//! The card charge of a top-up a sandbox replay of past billing records: off-session, through
//! the card's gateway, keyed by the replay's payment so a retry never charges twice.
use crate::Store;
use crate::StoreResult;
use crate::adapters::payment::bridge::payment_intent_from_outcome;
use crate::adapters::payment::model::{ChargeRequest, IdempotencyKey, connector_webhook_url};
use crate::domain::payment_transactions::PaymentIntent;
use crate::errors::StoreError;
use crate::services::payment::resolve_connector_for_method;
use crate::store::PgConn;
use common_domain::ids::{BaseId, CustomerPaymentMethodId, PaymentTransactionId, TenantId};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use error_stack::ResultExt;

/// Charges `method_id` off-session through its gateway for the replay's `payment_id`, keyed by it.
pub(super) async fn charge_card_off_session(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    method_id: CustomerPaymentMethodId,
    payment_id: PaymentTransactionId,
    amount_minor: i64,
    currency: &str,
) -> StoreResult<PaymentIntent> {
    let (connector, gateway) =
        resolve_connector_for_method(store, conn, tenant_id, method_id).await?;
    let method = CustomerPaymentMethodRow::get_by_id(conn, &tenant_id, &method_id)
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;
    let connection =
        CustomerConnectionDetailsRow::get_by_id(conn, &tenant_id, &method.connection_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;
    let outcome = gateway
        .charge_off_session(ChargeRequest {
            transaction_id: payment_id,
            customer_external_id: &connection.external_customer_id,
            payment_method_external_id: &method.external_payment_method_id,
            payment_method_type: method.payment_method_type.clone().into(),
            amount_minor,
            currency,
            idempotency_key: IdempotencyKey::new(format!("charge:{}", payment_id.as_base62())),
            on_session: false,
            descriptor: None,
            webhook_url: connector_webhook_url(store.settings.webhook_base_url(), &connector),
        })
        .await
        .change_context(StoreError::PaymentProviderError)?;
    Ok(payment_intent_from_outcome(
        outcome,
        payment_id,
        tenant_id,
        amount_minor,
        currency.to_owned(),
    ))
}
