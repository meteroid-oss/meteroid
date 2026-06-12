use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::{HandleResult, PgmqHandler};
use async_trait::async_trait;
use common_domain::ids::PlanVersionId;
use common_domain::pgmq::{Headers, MessageId};
use error_stack::{Report, ResultExt};
use futures::FutureExt;
use meteroid_store::domain::Invoice;
use meteroid_store::domain::outbox_event::{EventType, OutboxEvent, OutboxPgmqHeaders};
use meteroid_store::domain::pgmq::{
    BiAggregationEvent, BiCreditNoteFinalizedEvent, BiInvoiceFinalizedEvent,
    HubspotSyncRequestEvent, PennylaneSyncInvoice, PennylaneSyncRequestEvent, PgmqMessage,
    PgmqMessageNew, PgmqQueue, QuoteConversionRequestEvent,
};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::{Store, StoreResult};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Dispatches to consumer queues. This allows mimicking kafka consumer groups.
/// Note:
/// The message column is not copied to the consumer queue.
/// The original message id is set in the headers so the consumer queue reader can fetch the messages from the archived outbox queue by ids.
pub struct PgmqOutboxDispatch {
    pub(crate) store: Arc<Store>,
}

impl PgmqOutboxDispatch {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    pub(crate) async fn handle_webhook_out(&self, msgs: &[PgmqMessage]) -> PgmqResult<()> {
        let wh_messages = msgs
            .iter()
            .filter_map(|x| {
                let headers: Headers = DispatchHeaders {
                    outbox_msg_id: x.msg_id,
                }
                .try_into()
                .ok()?;

                Some(PgmqMessageNew {
                    message: None,
                    headers: Some(headers),
                    tenant_id: None,
                })
            })
            .collect();

        self.store
            .pgmq_send_batch(PgmqQueue::WebhookOut, wh_messages)
            .await
            .change_context(PgmqError::HandleMessages)
    }

    pub(crate) async fn handle_hubspot_out(&self, msgs: &[PgmqMessage]) -> PgmqResult<()> {
        let mut new_messages = vec![];

        for msg in msgs {
            let out_headers: StoreResult<Option<OutboxPgmqHeaders>> =
                msg.headers.as_ref().map(TryInto::try_into).transpose();
            if let Ok(Some(out_headers)) = out_headers {
                if let EventType::CustomerCreated = &out_headers.event_type {
                    if let Ok(OutboxEvent::CustomerCreated(evt)) = msg.try_into() {
                        HubspotSyncRequestEvent::CustomerOutbox(evt)
                            .try_into()
                            .map(|msg_new| new_messages.push(msg_new))
                            .change_context(PgmqError::HandleMessages)?;
                    }
                } else if let EventType::CustomerUpdated = &out_headers.event_type {
                    if let Ok(OutboxEvent::CustomerUpdated(evt)) = msg.try_into() {
                        HubspotSyncRequestEvent::CustomerOutbox(evt)
                            .try_into()
                            .map(|msg_new| new_messages.push(msg_new))
                            .change_context(PgmqError::HandleMessages)?;
                    }
                } else if let EventType::SubscriptionCreated = &out_headers.event_type
                    && let Ok(OutboxEvent::SubscriptionCreated(evt)) = msg.try_into()
                {
                    HubspotSyncRequestEvent::SubscriptionOutbox(evt)
                        .try_into()
                        .map(|msg_new| new_messages.push(msg_new))
                        .change_context(PgmqError::HandleMessages)?;
                }
            }
        }

        if !new_messages.is_empty() {
            self.store
                .pgmq_send_batch(PgmqQueue::HubspotSync, new_messages)
                .await
                .change_context(PgmqError::HandleMessages)?;
        }

        Ok(())
    }

    pub(crate) async fn handle_pennylane_out(&self, msgs: &[PgmqMessage]) -> PgmqResult<()> {
        let mut new_messages = vec![];

        for msg in msgs {
            let out_headers: StoreResult<Option<OutboxPgmqHeaders>> =
                msg.headers.as_ref().map(TryInto::try_into).transpose();
            if let Ok(Some(out_headers)) = out_headers {
                if let EventType::CustomerCreated = &out_headers.event_type {
                    if let Ok(OutboxEvent::CustomerCreated(evt)) = msg.try_into() {
                        PennylaneSyncRequestEvent::CustomerOutbox(evt)
                            .try_into()
                            .map(|msg_new| new_messages.push(msg_new))
                            .change_context(PgmqError::HandleMessages)?;
                    }
                } else if let EventType::CustomerUpdated = &out_headers.event_type {
                    if let Ok(OutboxEvent::CustomerUpdated(evt)) = msg.try_into() {
                        PennylaneSyncRequestEvent::CustomerOutbox(evt)
                            .try_into()
                            .map(|msg_new| new_messages.push(msg_new))
                            .change_context(PgmqError::HandleMessages)?;
                    }
                } else if let EventType::InvoiceAccountingPdfGenerated = &out_headers.event_type {
                    if let Ok(OutboxEvent::InvoiceAccountingPdfGenerated(evt)) = msg.try_into() {
                        PennylaneSyncRequestEvent::Invoice(Box::new(PennylaneSyncInvoice {
                            id: evt.invoice_id,
                            tenant_id: evt.tenant_id,
                        }))
                        .try_into()
                        .map(|msg_new| new_messages.push(msg_new))
                        .change_context(PgmqError::HandleMessages)?;
                    }
                } else if let EventType::InvoicePaid = &out_headers.event_type
                    && let Ok(OutboxEvent::InvoicePaid(evt)) = msg.try_into()
                {
                    PennylaneSyncRequestEvent::Invoice(Box::new(PennylaneSyncInvoice {
                        id: evt.invoice_id,
                        tenant_id: evt.tenant_id,
                    }))
                    .try_into()
                    .map(|msg_new| new_messages.push(msg_new))
                    .change_context(PgmqError::HandleMessages)?;
                }
            }
        }

        if !new_messages.is_empty() {
            self.store
                .pgmq_send_batch(PgmqQueue::PennylaneSync, new_messages)
                .await
                .change_context(PgmqError::HandleMessages)?;
        }

        Ok(())
    }

    pub(crate) async fn handle_invoice_orchestration(
        &self,
        msgs: &[PgmqMessage],
    ) -> PgmqResult<()> {
        let mut events = vec![];

        for msg in msgs {
            let out_headers: StoreResult<Option<OutboxPgmqHeaders>> =
                msg.headers.as_ref().map(TryInto::try_into).transpose();
            if let Ok(Some(out_headers)) = out_headers {
                let event_types = [
                    EventType::InvoiceAccountingPdfGenerated,
                    EventType::InvoiceFinalized,
                    EventType::InvoicePaid,
                    EventType::PaymentTransactionReceived,
                    EventType::CreditNoteFinalized,
                ];

                if !event_types.contains(&out_headers.event_type) {
                    continue;
                }

                events.push(PgmqMessageNew {
                    message: None,
                    headers: Some(
                        DispatchHeaders {
                            outbox_msg_id: msg.msg_id,
                        }
                        .try_into()?,
                    ),
                    tenant_id: None,
                });
            }
        }

        if events.is_empty() {
            return Ok(());
        }

        self.store
            .pgmq_send_batch(PgmqQueue::InvoiceOrchestration, events)
            .await
            .change_context(PgmqError::HandleMessages)
    }

    pub(crate) async fn handle_quote_conversion(&self, msgs: &[PgmqMessage]) -> PgmqResult<()> {
        let mut new_messages = vec![];

        for msg in msgs {
            let out_headers: StoreResult<Option<OutboxPgmqHeaders>> =
                msg.headers.as_ref().map(TryInto::try_into).transpose();
            if let Ok(Some(out_headers)) = out_headers
                && let EventType::QuoteAccepted = &out_headers.event_type
                && let Ok(OutboxEvent::QuoteAccepted(evt)) = msg.try_into()
            {
                QuoteConversionRequestEvent::QuoteAccepted(evt)
                    .try_into()
                    .map(|msg_new| new_messages.push(msg_new))
                    .change_context(PgmqError::HandleMessages)?;
            }
        }

        if !new_messages.is_empty() {
            self.store
                .pgmq_send_batch(PgmqQueue::QuoteConversion, new_messages)
                .await
                .change_context(PgmqError::HandleMessages)?;
        }

        Ok(())
    }

    pub(crate) async fn handle_bi_aggregation(&self, msgs: &[PgmqMessage]) -> PgmqResult<()> {
        let mut new_messages = vec![];

        for msg in msgs {
            let out_headers: StoreResult<Option<OutboxPgmqHeaders>> =
                msg.headers.as_ref().map(TryInto::try_into).transpose();
            if let Ok(Some(out_headers)) = out_headers {
                match &out_headers.event_type {
                    EventType::InvoiceFinalized => {
                        if let Ok(OutboxEvent::InvoiceFinalized(evt)) = msg.try_into() {
                            // Only process if finalized_at exists (should always be set on finalization)
                            if let Some(finalized_at) = evt.finalized_at {
                                // A consolidated parent has no plan_version_id; split its revenue
                                // across members to keep per-plan attribution.
                                let members = if evt.plan_version_id.is_none() {
                                    self.store
                                        .list_consolidated_children(evt.tenant_id, evt.invoice_id)
                                        .await
                                        .change_context(PgmqError::HandleMessages)?
                                } else {
                                    vec![]
                                };

                                for (plan_version_id, amount_due) in allocate_revenue_across_members(
                                    &members,
                                    evt.plan_version_id,
                                    evt.amount_due,
                                ) {
                                    BiAggregationEvent::InvoiceFinalized(Box::new(
                                        BiInvoiceFinalizedEvent {
                                            tenant_id: evt.tenant_id,
                                            customer_id: evt.customer_id,
                                            plan_version_id,
                                            currency: evt.currency.clone(),
                                            amount_due,
                                            finalized_at,
                                        },
                                    ))
                                    .try_into()
                                    .map(|msg_new| new_messages.push(msg_new))
                                    .change_context(PgmqError::HandleMessages)?;
                                }
                            }
                        }
                    }
                    EventType::CreditNoteFinalized => {
                        if let Ok(OutboxEvent::CreditNoteFinalized(evt)) = msg.try_into() {
                            // Only process if finalized_at exists
                            if let Some(finalized_at) = evt.finalized_at {
                                // A credit note against a consolidated parent inherits the
                                // parent's null plan_version_id. Split the refund across the
                                // parent's members (pro-rata by member total) so per-plan revenue
                                // stays symmetric with the finalize-time split — otherwise the
                                // negative revenue would be booked entirely against "no plan".
                                let members = if evt.plan_version_id.is_none() {
                                    self.store
                                        .list_consolidated_children(evt.tenant_id, evt.invoice_id)
                                        .await
                                        .change_context(PgmqError::HandleMessages)?
                                } else {
                                    vec![]
                                };

                                for (plan_version_id, refunded_amount_cents) in
                                    allocate_revenue_across_members(
                                        &members,
                                        evt.plan_version_id,
                                        evt.refunded_amount_cents,
                                    )
                                {
                                    BiAggregationEvent::CreditNoteFinalized(Box::new(
                                        BiCreditNoteFinalizedEvent {
                                            tenant_id: evt.tenant_id,
                                            customer_id: evt.customer_id,
                                            plan_version_id,
                                            currency: evt.currency.clone(),
                                            refunded_amount_cents,
                                            finalized_at,
                                        },
                                    ))
                                    .try_into()
                                    .map(|msg_new| new_messages.push(msg_new))
                                    .change_context(PgmqError::HandleMessages)?;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if !new_messages.is_empty() {
            self.store
                .pgmq_send_batch(PgmqQueue::BiAggregation, new_messages)
                .await
                .change_context(PgmqError::HandleMessages)?;
        }

        Ok(())
    }
}

#[async_trait]
impl PgmqHandler for PgmqOutboxDispatch {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let ids = msgs.iter().map(|x| x.msg_id).collect::<Vec<_>>();

        // todo handle errors, some handlers might fail, we don't want to reprocess again everything
        let handlers = vec![
            self.handle_webhook_out(msgs).boxed(),
            self.handle_hubspot_out(msgs).boxed(),
            self.handle_pennylane_out(msgs).boxed(),
            self.handle_invoice_orchestration(msgs).boxed(),
            self.handle_quote_conversion(msgs).boxed(),
            self.handle_bi_aggregation(msgs).boxed(),
        ];

        // Run the functions concurrently
        let joined = futures::future::join_all(handlers).await;

        // Check for errors
        for result in joined {
            result?;
        }

        Ok(HandleResult::from_succeeded(ids))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DispatchHeaders {
    pub outbox_msg_id: MessageId,
}

impl TryInto<Headers> for DispatchHeaders {
    type Error = Report<PgmqError>;

    fn try_into(self) -> Result<Headers, Self::Error> {
        serde_json::to_value(&self)
            .map(Headers)
            .change_context(PgmqError::HandleMessages)
    }
}

impl TryInto<DispatchHeaders> for &PgmqMessage {
    type Error = Report<PgmqError>;

    fn try_into(self) -> Result<DispatchHeaders, Self::Error> {
        let headers = &self.headers.as_ref().ok_or(PgmqError::EmptyHeaders)?.0;

        DispatchHeaders::deserialize(headers.clone()).map_err(|e| Report::new(PgmqError::Serde(e)))
    }
}

/// Proxy Handler that lists the archived outbox messages, passes them to the underlying handler,
/// then matches the results to the original messages and returns the original message ids so they can be acked.
pub(crate) struct PgmqOutboxProxy {
    pub(crate) store: Arc<Store>,
    pub(crate) underlying: Arc<dyn PgmqHandler>,
}

impl PgmqOutboxProxy {
    pub fn new(store: Arc<Store>, underlying: Arc<dyn PgmqHandler>) -> Self {
        Self { store, underlying }
    }
}

#[async_trait::async_trait]
impl PgmqHandler for PgmqOutboxProxy {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        if msgs.is_empty() {
            return Ok(HandleResult::from_succeeded(vec![]));
        }

        let msg_ids = msgs.iter().try_fold(Vec::new(), |mut acc, msg| {
            let DispatchHeaders { outbox_msg_id } = msg.try_into()?;
            acc.push((msg.msg_id, outbox_msg_id));
            Ok::<Vec<_>, Report<PgmqError>>(acc)
        })?;

        let (_, outbox_msg_ids): (Vec<_>, Vec<_>) = msg_ids.iter().copied().unzip();

        let outbox_archived: Vec<PgmqMessage> = self
            .store
            .pgmq_list_archived(PgmqQueue::OutboxEvent, outbox_msg_ids)
            .await
            .change_context(PgmqError::ListArchived)?;

        let result = self.underlying.handle(&outbox_archived).await?;

        let succeeded = msg_ids
            .iter()
            .filter_map(|(orig, out)| {
                if result.succeeded.contains(out) {
                    Some(*orig)
                } else {
                    None
                }
            })
            .collect();

        let failed = msg_ids
            .iter()
            .filter_map(|(orig, out)| {
                result
                    .failed
                    .iter()
                    .find(|(id, _)| id == out)
                    .map(|(_, err)| (*orig, err.clone()))
            })
            .collect();

        Ok(HandleResult { succeeded, failed })
    }
}

/// Allocates a consolidated parent invoice's net amount (finalized revenue or refunded credit)
/// across its member subscriptions as `(plan_version_id, amount)` pairs, for per-plan BI
/// attribution. Allocation is pro-rata by member gross total; the largest-total member absorbs the
/// rounding remainder so the parts sum exactly to `net_amount`.
///
/// Returns a single `(fallback_plan_version_id, net_amount)` entry when there is nothing to split —
/// no members, or a non-positive group gross (e.g. a member's negative total nets the group to
/// zero) — keeping the aggregate exact even if per-plan attribution is lost for that degenerate case.
fn allocate_revenue_across_members(
    members: &[Invoice],
    fallback_plan_version_id: Option<PlanVersionId>,
    net_amount: i64,
) -> Vec<(Option<PlanVersionId>, i64)> {
    let parent_gross: i64 = members.iter().map(|m| m.total).sum();
    if members.is_empty() || parent_gross <= 0 {
        return vec![(fallback_plan_version_id, net_amount)];
    }

    // Absorb the rounding remainder into the largest-total member, so a zero/low-weight plan isn't
    // attributed leftover cents.
    let remainder_idx = members
        .iter()
        .enumerate()
        .max_by_key(|(_, m)| m.total)
        .map(|(idx, _)| idx)
        .unwrap_or(0);

    let mut allocations: Vec<(Option<PlanVersionId>, i64)> = Vec::with_capacity(members.len());
    let mut allocated: i64 = 0;
    for (idx, member) in members.iter().enumerate() {
        if idx == remainder_idx {
            allocations.push((member.plan_version_id, 0)); // fixed up below
            continue;
        }
        let share = (net_amount as i128 * member.total as i128 / parent_gross as i128) as i64;
        allocated += share;
        allocations.push((member.plan_version_id, share));
    }
    allocations[remainder_idx].1 = net_amount - allocated;
    allocations
}

pub(crate) async fn to_outbox_events(
    msgs: &[PgmqMessage],
) -> PgmqResult<Vec<(MessageId, OutboxEvent)>> {
    msgs.iter().try_fold(vec![], |mut acc, msg| {
        let outbox_event: StoreResult<OutboxEvent> = msg.try_into();
        let outbox_event = outbox_event.change_context(PgmqError::HandleMessages)?;

        acc.push((msg.msg_id, outbox_event));

        Ok::<Vec<_>, Report<PgmqError>>(acc)
    })
}
