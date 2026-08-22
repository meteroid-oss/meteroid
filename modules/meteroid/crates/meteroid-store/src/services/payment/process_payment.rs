use crate::StoreResult;
use crate::adapters::payment::bridge::payment_intent_from_outcome;
use crate::adapters::payment::initialize_payment_connector;
use crate::adapters::payment::model::{ChargeRequest, IdempotencyKey};
use crate::domain::connectors::Connector;
use crate::domain::entity_activity::Actor;
use crate::domain::payment_transactions::{PaymentIntent, PaymentNextAction, PaymentTransaction};
use crate::errors::StoreError;
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::services::Services;
use crate::store::PgConn;
use common_domain::ids::{
    BaseId, CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, TenantId,
};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowNew};
use error_stack::{Report, ResultExt};
use std::time::Duration;

/// Maximum time to wait for payment provider API calls.
/// This is a safety net in addition to the HTTP client timeout.
const PAYMENT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(45);

impl Services {
    /// Creates a payment intent and the associated payment transaction.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn process_invoice_payment_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
        // True when a customer triggered this payment (portal), so the settled
        // payment is attributed to them; false for system auto-charges.
        customer_initiated: bool,
        on_session: bool,
        // Stable, caller-supplied seed for the provider idempotency key. When
        // set, a retry that mints a fresh transaction id still reuses the SAME
        // provider key, so the provider dedupes instead of double-charging. Only
        // the webhook-driven off-session invoice charge (which can be retried by
        // pgmq) passes this; interactive callers pass None (per-transaction key).
        // Ignored for Stancer connections, which always derive a stable
        // invoice-scoped key — see `create_payment_intent`.
        idempotency_ref: Option<String>,
    ) -> StoreResult<(PaymentTransaction, Option<PaymentNextAction>)> {
        // Get the invoice
        let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // A consolidated child is billed via its parent; paying it directly would double-charge.
        if invoice.invoice.consolidated_into_invoice_id.is_some() {
            return Err(Report::new(StoreError::BillingError)
                .attach("Cannot pay an invoice merged into a consolidated parent"));
        }

        // Allow both draft and finalized invoices
        if invoice.invoice.status != diesel_models::enums::InvoiceStatusEnum::Draft
            && invoice.invoice.status != diesel_models::enums::InvoiceStatusEnum::Finalized
        {
            return Err(Report::new(StoreError::BillingError)
                .attach("Cannot process payment for this invoice status"));
        }

        if invoice.invoice.amount_due <= 0 {
            return Err(Report::new(StoreError::BillingError).attach("Invoice has no amount due"));
        }

        // Check for existing transactions that would prevent a new payment
        let existing_transactions =
            PaymentTransactionRow::list_by_invoice_id(conn, invoice_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        // Check for pending transactions - only one payment attempt at a time
        let has_pending_transaction = existing_transactions
            .iter()
            .any(|tx| tx.transaction.status == PaymentStatusEnum::Pending);

        if has_pending_transaction {
            return Err(Report::new(StoreError::PaymentError(
                "A payment for this invoice is already being processed. Please wait for it to complete before attempting another payment.".to_string()
            )));
        }

        // Calculate total of active payments (pending/ready/settled) to prevent over-payment.
        // This check, combined with SELECT FOR UPDATE on the invoice, ensures atomicity
        // in a distributed environment. Refunds are netted out (mirroring
        // `recompute_amount_due_from_settled_payments`) so a partially refunded
        // settlement doesn't block collecting the reopened balance.
        let active_payment_sum: i64 = existing_transactions
            .iter()
            .filter(|tx| {
                matches!(
                    tx.transaction.status,
                    PaymentStatusEnum::Pending
                        | PaymentStatusEnum::Ready
                        | PaymentStatusEnum::Settled
                )
            })
            .map(|tx| tx.transaction.amount - tx.transaction.amount_refunded)
            .sum();

        // Prevent payment if invoice is already fully covered
        if active_payment_sum >= invoice.invoice.total {
            return Err(Report::new(StoreError::PaymentError(format!(
                "Invoice already has sufficient payments. Total: {}, Already paid: {}",
                invoice.invoice.total, active_payment_sum
            ))));
        }

        // Prevent payment if this would exceed the invoice total
        let proposed_payment = invoice.invoice.amount_due;
        if active_payment_sum + proposed_payment > invoice.invoice.total {
            return Err(Report::new(StoreError::PaymentError(format!(
                "Payment of {} would exceed invoice total. Already paid: {}, Total: {}",
                proposed_payment, active_payment_sum, invoice.invoice.total
            ))));
        }

        // Committed prior attempts (never the uncommitted row inserted below):
        // seeds the per-attempt component of the Stancer idempotency key in
        // `create_payment_intent`. Rollback-stable — a retry after a rolled-back
        // charge recomputes the same value and dedupes at the provider.
        let prior_invoice_attempts = existing_transactions.len();

        // Persist the Pending row BEFORE the external charge so the provider
        // idempotency key `charge:{id}` is derived from a row that already exists:
        // a retry that reuses this id can never double-charge.
        //
        // Residual window: this insert shares the surrounding DB transaction. When
        // the invoice is created in that same (uncommitted) transaction — the
        // checkout `FinalizeAfterPayment` path — a fresh connection can't see it
        // (FK invisible), so the row genuinely can't be committed ahead of the
        // charge. If the surrounding transaction later rolls back after the charge
        // succeeded, the row is lost; recovery is via the provider webhook /
        // reconciliation (which re-drive from the provider's own record). A retry
        // through this function mints a NEW id only if the row did not survive —
        // that is the uncovered window, bounded to a crash between charge and commit.
        // For Stancer (no webhook, reconcile polls local rows only) that window is
        // closed instead by the invoice-derived key in `create_payment_intent`.
        let transaction = PaymentTransactionRowNew {
            id: PaymentTransactionId::new(),
            tenant_id,
            invoice_id: Some(invoice_id),
            provider_transaction_id: None,
            amount: invoice.invoice.amount_due,
            currency: invoice.invoice.currency.clone(),
            payment_method_id: Some(payment_method_id),
            status: PaymentStatusEnum::Pending,
            payment_type: PaymentTypeEnum::Payment,
            error_type: None,
            processed_at: None,
            checkout_session_id: None,
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: customer_initiated.then_some(invoice.invoice.customer_id),
            pending_provider_intent_id: None,
            pending_connection_id: None,
        };

        let inserted_transaction = transaction
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Create payment intent with payment provider
        let payment_intent = self
            .create_payment_intent(
                conn,
                &tenant_id,
                &invoice_id,
                &payment_method_id,
                &inserted_transaction.id,
                inserted_transaction.amount as u64,
                inserted_transaction.currency.clone(),
                on_session,
                idempotency_ref.as_deref(),
                prior_invoice_attempts,
            )
            .await?;

        // Transient next_action (carries the client secret) — surfaced to the
        // on-session caller so the portal can complete 3DS; never persisted.
        let next_action = payment_intent.next_action.clone();

        // Consolidate the transaction
        let tx = self
            .store
            .consolidate_intent_and_transaction_tx(
                conn,
                &Actor::System,
                inserted_transaction.into(),
                payment_intent,
            )
            .await?;

        Ok((tx, next_action))
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_payment_intent(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        invoice_id: &InvoiceId,
        payment_method_id: &CustomerPaymentMethodId,
        transaction_id: &PaymentTransactionId,
        amount: u64,
        currency: String,
        on_session: bool,
        idempotency_ref: Option<&str>,
        prior_invoice_attempts: usize,
    ) -> StoreResult<PaymentIntent> {
        let method = CustomerPaymentMethodRow::get_by_id(conn, tenant_id, payment_method_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connection =
            CustomerConnectionDetailsRow::get_by_id(conn, tenant_id, &method.connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connector = Connector::from_row(&self.store.settings.crypt_key, connection.connector)?;

        let connector_impl = initialize_payment_connector(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        // Provider idempotency key. Default: the transaction id, so the client's
        // own internal retries within this attempt dedupe. When the caller passes
        // a stable `idempotency_ref` (the webhook charge does — see below), use
        // that instead: it survives a full DB-transaction rollback + pgmq retry,
        // where a fresh transaction id would NOT, and lets the provider dedupe a
        // charge it already processed rather than creating a second one.
        //
        // Stancer override: no webhook exists and reconciliation only polls
        // locally-known rows, so a rollback after an accepted charge would
        // orphan the provider payment. EVERY Stancer invoice charge derives
        // its key from committed state only — (method, invoice, #prior
        // committed attempts) — so any retry of the same attempt reuses the
        // key (adapter adopts) while a new attempt after a committed decline
        // gets a fresh one.
        let idempotency_key =
            if connector.provider == crate::domain::enums::ConnectorProviderEnum::Stancer {
                IdempotencyKey::new(format!(
                    "charge:{}",
                    stancer_invoice_idempotency_seed(
                        payment_method_id,
                        invoice_id,
                        prior_invoice_attempts
                    )
                ))
            } else {
                match idempotency_ref {
                    Some(seed) => IdempotencyKey::new(format!("charge:{seed}")),
                    None => IdempotencyKey::new(format!("charge:{}", transaction_id.as_base62())),
                }
            };
        let request = ChargeRequest {
            transaction_id: *transaction_id,
            customer_external_id: &connection.external_customer_id,
            payment_method_external_id: &method.external_payment_method_id,
            payment_method_type: method.payment_method_type.clone().into(),
            amount_minor: amount as i64,
            currency: &currency,
            idempotency_key,
            on_session,
        };

        let outcome = tokio::time::timeout(
            PAYMENT_PROVIDER_TIMEOUT,
            connector_impl.charge_off_session(&connector, request),
        )
        .await
        .map_err(|_| {
            Report::new(StoreError::PaymentProviderError)
                .attach("Payment provider request timed out")
        })?
        .change_context_lazy(|| StoreError::PaymentProviderError)?;

        Ok(payment_intent_from_outcome(
            outcome,
            *transaction_id,
            *tenant_id,
            amount as i64,
            currency,
        ))
    }
}

/// Stable provider-idempotency seed for a Stancer invoice charge. Derived
/// exclusively from committed state — never the per-attempt transaction id —
/// so the return-handler first charge and any later retry/renewal of the same
/// (invoice, method, attempt) reuse the SAME provider key and Stancer dedupes.
fn stancer_invoice_idempotency_seed(
    payment_method_id: &CustomerPaymentMethodId,
    invoice_id: &InvoiceId,
    prior_invoice_attempts: usize,
) -> String {
    format!(
        "stancer-charge:{}:{}:{prior_invoice_attempts}",
        payment_method_id.as_base62(),
        invoice_id.as_base62(),
    )
}

#[cfg(test)]
mod tests {
    use super::stancer_invoice_idempotency_seed;
    use common_domain::ids::{BaseId, CustomerPaymentMethodId, InvoiceId};

    /// The double-charge guard: two invocations for the same invoice+method
    /// derive the SAME seed — Stancer's `unique_id` unicity then dedupes.
    #[test]
    fn stancer_seed_is_stable_across_invocations() {
        let method = CustomerPaymentMethodId::new();
        let invoice = InvoiceId::new();

        let first = stancer_invoice_idempotency_seed(&method, &invoice, 0);
        let second = stancer_invoice_idempotency_seed(&method, &invoice, 0);
        assert_eq!(first, second);
        assert_eq!(
            first,
            format!(
                "stancer-charge:{}:{}:0",
                method.as_base62(),
                invoice.as_base62()
            )
        );
    }

    /// A genuinely new attempt (after a COMMITTED failed one) must get a fresh
    /// seed, or dunning could never retry a declined Stancer charge; different
    /// invoices/methods must never share a seed.
    #[test]
    fn stancer_seed_distinguishes_attempts_and_targets() {
        let method = CustomerPaymentMethodId::new();
        let invoice = InvoiceId::new();

        let base = stancer_invoice_idempotency_seed(&method, &invoice, 0);
        assert_ne!(base, stancer_invoice_idempotency_seed(&method, &invoice, 1));
        assert_ne!(
            base,
            stancer_invoice_idempotency_seed(&CustomerPaymentMethodId::new(), &invoice, 0)
        );
        assert_ne!(
            base,
            stancer_invoice_idempotency_seed(&method, &InvoiceId::new(), 0)
        );
    }
}
