use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::processor::{HandleResult, PgmqHandler};
use error_stack::ResultExt;
use meteroid_store::Store;
use meteroid_store::domain::VatNumberValidationStatus;
use meteroid_store::domain::pgmq::{
    PgmqMessage, PgmqMessageNew, PgmqQueue, VatValidationRequestEvent,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_tax::VatNumberExternalValidationResult;
use std::sync::Arc;

/// Delay (seconds) before attempt N+1 when attempt N found VIES unreachable:
/// one quick retry for transient blips, then exponentially spaced re-checks
/// spanning several days, since VIES outages routinely last hours.
/// After the last attempt the customer stays `UNAVAILABLE` until the periodic
/// revalidation worker (or a manual re-check) picks it up again.
const RETRY_DELAYS_SECS: &[i32] = &[
    120,        // 2 minutes
    3_600,      // 1 hour
    6 * 3_600,  // 6 hours
    24 * 3_600, // 1 day
    48 * 3_600, // 2 days
    72 * 3_600, // 3 days
];

pub(crate) struct VatValidation {
    store: Arc<Store>,
}

impl VatValidation {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    /// Processes a single job. VIES is rate-limited, so callers run these
    /// sequentially. Returns `Ok(true)` when the message should be acked, or
    /// `Ok(false)` to leave it for a visibility-timeout retry (transient
    /// store errors only — VIES unavailability is handled via delayed
    /// re-enqueues, never by blocking the queue).
    async fn process(&self, event: &VatValidationRequestEvent) -> PgmqResult<bool> {
        let customer = match self
            .store
            .find_customer_by_id(event.customer_id, event.tenant_id)
            .await
        {
            Ok(customer) => customer,
            // Only a genuinely missing customer is a terminal ack; any other error
            // (connection/DB hiccup) must retry rather than silently drop the job.
            Err(err) => {
                return match err.current_context() {
                    StoreError::ValueNotFound(_) => Ok(true),
                    _ => Ok(false),
                };
            }
        };

        // A job is stale once the number it carries is no longer the customer's
        // current number — a later edit reset the status and enqueued its own job.
        if customer.vat_number.as_deref() != Some(event.vat_number.as_str()) {
            return Ok(true);
        }

        let status = customer.vat_number_validation_status;
        let actionable = match status {
            Some(VatNumberValidationStatus::Pending) => true,
            // Delayed retries and periodic re-checks may act on a parked number.
            Some(VatNumberValidationStatus::Unavailable) => event.attempt > 0 || event.revalidate,
            // Definitive answers (and pre-feature rows without a status) are only
            // ever re-checked by an explicit revalidation pass.
            Some(VatNumberValidationStatus::Valid)
            | Some(VatNumberValidationStatus::Invalid)
            | None => event.revalidate,
        };
        if !actionable {
            return Ok(true);
        }

        let now = chrono::Utc::now().naive_utc();

        // Run a qualified check on behalf of the customer's invoicing entity when
        // it has a usable VAT number, so VIES returns a consultation number
        // (audit proof). A missing/ineligible requester falls back to unqualified.
        let requester_vat = self
            .store
            .get_invoicing_entity(event.tenant_id, Some(customer.invoicing_entity_id))
            .await
            .ok()
            .and_then(|entity| entity.vat_number)
            .filter(|vat| {
                meteroid_tax::validation::validate_vat_number_format(vat)
                    && meteroid_tax::vies::is_vies_eligible(vat)
            });

        let validation = meteroid_tax::vies::validate_with_requester(
            &event.vat_number,
            requester_vat.as_deref(),
        )
        .await;
        match validation.result {
            VatNumberExternalValidationResult::Valid => {
                self.persist(
                    event,
                    VatNumberValidationStatus::Valid,
                    now,
                    validation.check,
                )
                .await?;
            }
            VatNumberExternalValidationResult::Invalid => {
                self.persist(
                    event,
                    VatNumberValidationStatus::Invalid,
                    now,
                    validation.check,
                )
                .await?;
            }
            VatNumberExternalValidationResult::ServiceUnavailable => {
                // Fail open, but never downgrade a definitive answer: a VALID
                // customer being revalidated stays VALID through a VIES outage.
                if !matches!(
                    status,
                    Some(VatNumberValidationStatus::Valid)
                        | Some(VatNumberValidationStatus::Invalid)
                ) {
                    self.persist(event, VatNumberValidationStatus::Unavailable, now, None)
                        .await?;
                }

                // Revalidation passes are single-shot — the periodic worker owns
                // their cadence. Initial validations get the full retry chain.
                if !event.revalidate {
                    self.schedule_retry(event).await?;
                }
            }
        }

        Ok(true)
    }

    async fn schedule_retry(&self, event: &VatValidationRequestEvent) -> PgmqResult<()> {
        let Some(&delay) = RETRY_DELAYS_SECS.get(event.attempt as usize) else {
            log::warn!(
                "VIES still unavailable after {} attempts for customer {}; parking as UNAVAILABLE",
                event.attempt + 1,
                event.customer_id
            );
            return Ok(());
        };

        let retry = VatValidationRequestEvent {
            attempt: event.attempt + 1,
            ..event.clone()
        };
        let message: PgmqMessageNew = retry
            .try_into()
            .change_context(crate::workers::pgmq::error::PgmqError::HandleMessages)?;

        self.store
            .pgmq_send_batch_delayed(PgmqQueue::VatValidation, vec![message], delay)
            .await
            .change_context(crate::workers::pgmq::error::PgmqError::HandleMessages)
    }

    async fn persist(
        &self,
        event: &VatValidationRequestEvent,
        status: VatNumberValidationStatus,
        now: chrono::NaiveDateTime,
        vies_check: Option<meteroid_tax::ViesCheckData>,
    ) -> PgmqResult<()> {
        self.store
            .update_vat_number_validation(
                event.tenant_id,
                event.customer_id,
                &event.vat_number,
                status,
                now,
                vies_check,
            )
            .await
            .change_context(crate::workers::pgmq::error::PgmqError::HandleMessages)
    }
}

#[async_trait::async_trait]
impl PgmqHandler for VatValidation {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let mut succeeded = vec![];
        let mut failed = vec![];

        for msg in msgs {
            let event: VatValidationRequestEvent = match msg.try_into() {
                Ok(event) => event,
                Err(err) => {
                    failed.push(HandleResult::fail(msg.msg_id, &err));
                    continue;
                }
            };

            match self.process(&event).await {
                Ok(true) => succeeded.push(msg.msg_id),
                Ok(false) => {
                    failed.push((msg.msg_id, "transient store error, will retry".to_string()))
                }
                Err(err) => failed.push(HandleResult::fail(msg.msg_id, &err)),
            }
        }

        Ok(HandleResult { succeeded, failed })
    }
}
