//! An invoice's collection read back as the retries see it: the failures counted on the card,
//! the retries still planned from the tenant's dunning policy and what follows them.
use crate::StoreResult;
use crate::domain::dunning_policy::{DunningPolicy, ExhaustedAction};
use crate::domain::invoice_collection::{
    AttemptOutcome, CollectionAttempt, CollectionEnd, InvoiceCollection,
};
use crate::errors::StoreError;
use crate::repositories::invoice_account::{self, is_issued};
use crate::services::Services;
use crate::services::orchestration::payment_transaction_failed::hard_declined_on;
use billing_core::settlement::Charge;
use common_domain::ids::{InvoiceId, TenantId};
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::dunning_policy::DunningPolicyRow;
use diesel_models::enums::{PaymentMethodTypeEnum, PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionWithMethodRow};
use error_stack::Report;

impl Services {
    pub(in crate::services) async fn invoice_collection(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<InvoiceCollection> {
        let mut conn = self.store.get_conn().await?;
        let db = |e| Into::<Report<StoreError>>::into(e);
        let invoice = InvoiceRow::find_by_id(&mut conn, tenant_id, invoice_id)
            .await
            .map_err(db)?;
        let mut rows: Vec<PaymentTransactionWithMethodRow> =
            PaymentTransactionRow::list_by_invoice_id(&mut conn, invoice_id, tenant_id)
                .await
                .map_err(db)?
                .into_iter()
                .filter(|row| row.transaction.payment_type == PaymentTypeEnum::Payment)
                .collect();
        rows.sort_by_key(|row| row.transaction.created_at);
        let mut collection = InvoiceCollection {
            attempts: rows.iter().map(attempt).collect(),
            ..InvoiceCollection::default()
        };

        // Only a subscription's issued invoice still due is retried.
        if invoice.subscription_id.is_none()
            || !is_issued(&invoice)
            || !matches!(
                invoice_account::charge_tx(&mut conn, &invoice).await?,
                Charge::Due(_)
            )
        {
            return Ok(collection);
        }
        let Some(last) = rows.iter().rev().find(|row| {
            row.transaction.status == PaymentStatusEnum::Failed
                && row.transaction.initiated_by_customer_id.is_none()
        }) else {
            return Ok(collection);
        };
        let policy: DunningPolicy = DunningPolicyRow::find(&mut conn, tenant_id)
            .await
            .map_err(db)?
            .map(Into::into)
            .unwrap_or_default();
        let end = match policy.exhausted_action {
            ExhaustedAction::CancelSubscription => CollectionEnd::CancelsSubscription,
            ExhaustedAction::None => CollectionEnd::StaysDue,
        };
        let hard_declined = last
            .method
            .as_ref()
            .is_some_and(|method| hard_declined_on(&last.transaction, method));

        if let Some(next) = invoice.next_payment_attempt_at {
            // After a hard decline, a retry set again is a new card's: its ladder starts over.
            let failed = if hard_declined {
                0
            } else {
                failures_on(&mut conn, tenant_id, invoice_id, last).await?
            };
            collection.retries = policy.retries_from(failed, next.and_utc());
            collection.end = Some((end, collection.retries.last().copied()));
        } else if hard_declined {
            collection.stopped_by = last.transaction.decline_kind.map(Into::into);
            let cancel_at = InvoiceRow::dunning_cancel_at(&mut conn, tenant_id, invoice_id)
                .await
                .map_err(db)?;
            collection.end = Some(match cancel_at {
                Some(at) => (CollectionEnd::CancelsSubscription, Some(at.and_utc())),
                None => (CollectionEnd::StaysDue, None),
            });
        } else {
            collection.end = Some((end, Some(last.transaction.created_at)));
        }
        Ok(collection)
    }
}

/// The automatic failures the retries count for `last`'s card, as scheduling them does.
async fn failures_on(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
    last: &PaymentTransactionWithMethodRow,
) -> StoreResult<u32> {
    let counted = match last.transaction.payment_method_id {
        Some(method_id) => {
            PaymentTransactionRow::count_dunning_failures(
                conn,
                invoice_id,
                tenant_id,
                method_id,
                last.method
                    .as_ref()
                    .and_then(|method| method.card_refreshed_at)
                    .map(|at| at.naive_utc()),
            )
            .await
        }
        None => PaymentTransactionRow::count_failed_for_invoice(conn, invoice_id, tenant_id).await,
    }
    .map_err(Into::<Report<StoreError>>::into)?;
    Ok(u32::try_from(counted).unwrap_or(u32::MAX))
}

fn attempt(row: &PaymentTransactionWithMethodRow) -> CollectionAttempt {
    let tx = &row.transaction;
    CollectionAttempt {
        id: tx.id,
        at: tx.created_at,
        outcome: match tx.status {
            PaymentStatusEnum::Settled => AttemptOutcome::Succeeded,
            PaymentStatusEnum::Failed => AttemptOutcome::Failed,
            PaymentStatusEnum::Cancelled => AttemptOutcome::Cancelled,
            PaymentStatusEnum::Refunded => AttemptOutcome::Refunded,
            PaymentStatusEnum::Ready | PaymentStatusEnum::Pending => AttemptOutcome::Pending,
        },
        by_customer: tx.initiated_by_customer_id.is_some(),
        decline: tx.decline_kind.map(Into::into),
        method: row.method.as_ref().map(method_label),
        amount: tx.amount,
        currency: tx.currency.clone(),
    }
}

fn method_label(method: &CustomerPaymentMethodRow) -> String {
    match (&method.card_brand, &method.card_last4) {
        (Some(brand), Some(last4)) => format!("{brand} •••• {last4}"),
        (None, Some(last4)) => format!("Card •••• {last4}"),
        _ => match method.payment_method_type {
            PaymentMethodTypeEnum::Card => "Card".to_owned(),
            PaymentMethodTypeEnum::Transfer => "Bank transfer".to_owned(),
            PaymentMethodTypeEnum::DirectDebitSepa => "SEPA Direct Debit".to_owned(),
            PaymentMethodTypeEnum::DirectDebitAch => "ACH Direct Debit".to_owned(),
            PaymentMethodTypeEnum::DirectDebitBacs => "Bacs Direct Debit".to_owned(),
            PaymentMethodTypeEnum::Other => "Payment method".to_owned(),
        },
    }
}
