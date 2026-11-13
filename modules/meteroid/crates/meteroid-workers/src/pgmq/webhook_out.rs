use crate::pgmq::PgmqResult;
use crate::pgmq::error::PgmqError;
use crate::pgmq::outbox::to_outbox_events;
use crate::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::ids::{EventId, TenantId, WebhookEndpointId};
use common_domain::pgmq::MessageId;
use error_stack::{Report, ResultExt};
use meteroid_rest::webhooks::out_model::{
    WebhookOutAddOnEvent, WebhookOutAddOnEventData, WebhookOutCouponEvent,
    WebhookOutCouponEventData, WebhookOutCreditNoteEvent, WebhookOutCreditNoteEventData,
    WebhookOutCustomerEvent, WebhookOutCustomerEventData, WebhookOutEventTypeEnum,
    WebhookOutInvoiceDocumentsEvent, WebhookOutInvoiceDocumentsEventData, WebhookOutInvoiceEvent,
    WebhookOutInvoiceEventData, WebhookOutMetricEvent, WebhookOutMetricEventData,
    WebhookOutPaymentEvent, WebhookOutPlanEvent, WebhookOutPlanEventData, WebhookOutProductEvent,
    WebhookOutProductEventData, WebhookOutQuoteEvent, WebhookOutQuoteEventData,
    WebhookOutRefundEventData, WebhookOutSubscriptionEvent, WebhookOutSubscriptionEventData,
};
use meteroid_rest::webhooks::out_model::{
    WebhookOutAutoTopUpCapReachedEvent, WebhookOutAutoTopUpConsentChangedEvent,
    WebhookOutAutoTopUpEvent, WebhookOutBudgetAlertEvent, WebhookOutCreditGrantExpiredEvent,
    WebhookOutEntitlementAlertEvent, WebhookOutTooManyGroupsEvent, WebhookOutWalletBalanceLowEvent,
    WebhookOutWalletCreditRecoveredEvent, WebhookOutWalletCreditUnrecoveredEvent,
    WebhookOutWalletDebitedEvent, WebhookOutWalletDepletedEvent, WebhookOutWalletFundedEvent,
    WebhookOutWalletRecoveredEvent,
};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::Store;
use meteroid_store::domain::PaymentStatusEnum;
use meteroid_store::domain::outbox_event::{OutboxEvent, PaymentTransactionEvent};
use meteroid_store::domain::pgmq::PgmqMessage;
use meteroid_store::domain::webhooks_out::{WebhookEndpoint, WebhookOutMessageNew};
use meteroid_store::repositories::WebhookOutInterface;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

fn envelope_message<T: Serialize>(
    id: EventId,
    tenant_id: TenantId,
    event_type: WebhookOutEventTypeEnum,
    envelope: T,
) -> Result<WebhookOutMessageNew, serde_json::Error> {
    Ok(WebhookOutMessageNew {
        id,
        tenant_id,
        event_type: event_type.to_string(),
        payload: serde_json::to_value(&envelope)?,
    })
}

/// The webhook envelope for one outbox event. `None` means the event is not
/// webhook-enabled.
pub(crate) fn to_webhook_message(
    evt: OutboxEvent,
) -> Option<Result<WebhookOutMessageNew, serde_json::Error>> {
    let id = evt.event_id();
    let tenant_id = evt.tenant_id();
    let timestamp = ApiDateTime::now();

    macro_rules! envelope {
        ($kind:ident, $wrapper:ident, $data:ident, $event:expr) => {
            Some(envelope_message(
                id,
                tenant_id,
                WebhookOutEventTypeEnum::$kind,
                $wrapper {
                    id,
                    event_type: WebhookOutEventTypeEnum::$kind,
                    data: $data::from(*$event),
                    timestamp,
                },
            ))
        };
    }

    macro_rules! typed {
        ($kind:ident, $wrapper:ident, $event:expr) => {
            Some(envelope_message(
                id,
                tenant_id,
                WebhookOutEventTypeEnum::$kind,
                $wrapper {
                    id,
                    event_type: WebhookOutEventTypeEnum::$kind,
                    data: (*$event).into(),
                    timestamp,
                },
            ))
        };
    }

    match evt {
        OutboxEvent::CustomerCreated(e) => envelope!(
            CustomerCreated,
            WebhookOutCustomerEvent,
            WebhookOutCustomerEventData,
            e
        ),
        OutboxEvent::BillableMetricCreated(e) => envelope!(
            BillableMetricCreated,
            WebhookOutMetricEvent,
            WebhookOutMetricEventData,
            e
        ),
        OutboxEvent::BillableMetricUpdated(e) => envelope!(
            BillableMetricUpdated,
            WebhookOutMetricEvent,
            WebhookOutMetricEventData,
            e
        ),
        OutboxEvent::BillableMetricArchived(e) => envelope!(
            BillableMetricArchived,
            WebhookOutMetricEvent,
            WebhookOutMetricEventData,
            e
        ),
        OutboxEvent::InvoiceCreated(e) => envelope!(
            InvoiceCreated,
            WebhookOutInvoiceEvent,
            WebhookOutInvoiceEventData,
            e
        ),
        OutboxEvent::InvoiceFinalized(e) => envelope!(
            InvoiceFinalized,
            WebhookOutInvoiceEvent,
            WebhookOutInvoiceEventData,
            e
        ),
        OutboxEvent::InvoicePaid(e) => envelope!(
            InvoicePaid,
            WebhookOutInvoiceEvent,
            WebhookOutInvoiceEventData,
            e
        ),
        OutboxEvent::InvoiceVoided(e) => envelope!(
            InvoiceVoided,
            WebhookOutInvoiceEvent,
            WebhookOutInvoiceEventData,
            e
        ),
        OutboxEvent::InvoiceClosed(e) => envelope!(
            InvoiceClosed,
            WebhookOutInvoiceEvent,
            WebhookOutInvoiceEventData,
            e
        ),
        OutboxEvent::InvoiceConsolidated(e) => envelope!(
            InvoiceConsolidated,
            WebhookOutInvoiceEvent,
            WebhookOutInvoiceEventData,
            e
        ),
        OutboxEvent::InvoiceDeleted(e) => envelope!(
            InvoiceDeleted,
            WebhookOutInvoiceEvent,
            WebhookOutInvoiceEventData,
            e
        ),
        OutboxEvent::InvoiceAccountingPdfGenerated(e) => envelope!(
            InvoiceAccountingPdfGenerated,
            WebhookOutInvoiceDocumentsEvent,
            WebhookOutInvoiceDocumentsEventData,
            e
        ),
        OutboxEvent::SubscriptionCreated(e) => envelope!(
            SubscriptionCreated,
            WebhookOutSubscriptionEvent,
            WebhookOutSubscriptionEventData,
            e
        ),
        OutboxEvent::SubscriptionUpdated(e) => envelope!(
            SubscriptionUpdated,
            WebhookOutSubscriptionEvent,
            WebhookOutSubscriptionEventData,
            e
        ),
        OutboxEvent::SubscriptionCancelled(e) => envelope!(
            SubscriptionCancelled,
            WebhookOutSubscriptionEvent,
            WebhookOutSubscriptionEventData,
            e
        ),
        OutboxEvent::SubscriptionEnded(e) => envelope!(
            SubscriptionEnded,
            WebhookOutSubscriptionEvent,
            WebhookOutSubscriptionEventData,
            e
        ),
        OutboxEvent::QuoteAccepted(e) => envelope!(
            QuoteAccepted,
            WebhookOutQuoteEvent,
            WebhookOutQuoteEventData,
            e
        ),
        OutboxEvent::QuoteConverted(e) => envelope!(
            QuoteConverted,
            WebhookOutQuoteEvent,
            WebhookOutQuoteEventData,
            e
        ),
        OutboxEvent::CreditNoteCreated(e) => envelope!(
            CreditNoteCreated,
            WebhookOutCreditNoteEvent,
            WebhookOutCreditNoteEventData,
            e
        ),
        OutboxEvent::CreditNoteFinalized(e) => envelope!(
            CreditNoteFinalized,
            WebhookOutCreditNoteEvent,
            WebhookOutCreditNoteEventData,
            e
        ),
        OutboxEvent::CreditNoteVoided(e) => envelope!(
            CreditNoteVoided,
            WebhookOutCreditNoteEvent,
            WebhookOutCreditNoteEventData,
            e
        ),
        OutboxEvent::PlanCreated(e) => {
            envelope!(PlanCreated, WebhookOutPlanEvent, WebhookOutPlanEventData, e)
        }
        OutboxEvent::PlanPublished(e) => envelope!(
            PlanPublished,
            WebhookOutPlanEvent,
            WebhookOutPlanEventData,
            e
        ),
        OutboxEvent::PlanArchived(e) => envelope!(
            PlanArchived,
            WebhookOutPlanEvent,
            WebhookOutPlanEventData,
            e
        ),
        OutboxEvent::ProductCreated(e) => envelope!(
            ProductCreated,
            WebhookOutProductEvent,
            WebhookOutProductEventData,
            e
        ),
        OutboxEvent::ProductUpdated(e) => envelope!(
            ProductUpdated,
            WebhookOutProductEvent,
            WebhookOutProductEventData,
            e
        ),
        OutboxEvent::ProductArchived(e) => envelope!(
            ProductArchived,
            WebhookOutProductEvent,
            WebhookOutProductEventData,
            e
        ),
        OutboxEvent::CouponCreated(e) => envelope!(
            CouponCreated,
            WebhookOutCouponEvent,
            WebhookOutCouponEventData,
            e
        ),
        OutboxEvent::CouponUpdated(e) => envelope!(
            CouponUpdated,
            WebhookOutCouponEvent,
            WebhookOutCouponEventData,
            e
        ),
        OutboxEvent::CouponArchived(e) => envelope!(
            CouponArchived,
            WebhookOutCouponEvent,
            WebhookOutCouponEventData,
            e
        ),
        OutboxEvent::AddOnCreated(e) => envelope!(
            AddOnCreated,
            WebhookOutAddOnEvent,
            WebhookOutAddOnEventData,
            e
        ),
        OutboxEvent::AddOnUpdated(e) => envelope!(
            AddOnUpdated,
            WebhookOutAddOnEvent,
            WebhookOutAddOnEventData,
            e
        ),
        OutboxEvent::AddOnArchived(e) => envelope!(
            AddOnArchived,
            WebhookOutAddOnEvent,
            WebhookOutAddOnEventData,
            e
        ),
        OutboxEvent::RefundIssued(e) => Some(payment_message(
            id,
            tenant_id,
            WebhookOutEventTypeEnum::RefundIssued,
            *e,
            timestamp,
        )),
        OutboxEvent::RefundSettled(e) => Some(payment_message(
            id,
            tenant_id,
            WebhookOutEventTypeEnum::RefundSettled,
            *e,
            timestamp,
        )),
        OutboxEvent::RefundFailed(e) => Some(payment_message(
            id,
            tenant_id,
            WebhookOutEventTypeEnum::RefundFailed,
            *e,
            timestamp,
        )),
        OutboxEvent::PaymentReversed(e) => Some(payment_message(
            id,
            tenant_id,
            WebhookOutEventTypeEnum::PaymentReversed,
            *e,
            timestamp,
        )),
        OutboxEvent::PaymentReinstated(e) => Some(payment_message(
            id,
            tenant_id,
            WebhookOutEventTypeEnum::PaymentReinstated,
            *e,
            timestamp,
        )),
        OutboxEvent::PaymentTransactionSaved(e)
            if e.payment_type.is_collection()
                && e.status == PaymentStatusEnum::Failed
                && e.refunded_at.is_none() =>
        {
            Some(payment_message(
                id,
                tenant_id,
                WebhookOutEventTypeEnum::PaymentFailed,
                *e,
                timestamp,
            ))
        }
        OutboxEvent::WalletFunded(e) => typed!(WalletFunded, WebhookOutWalletFundedEvent, e),
        OutboxEvent::WalletDebited(e) => typed!(WalletDebited, WebhookOutWalletDebitedEvent, e),
        OutboxEvent::CreditGrantExpired(e) => {
            typed!(CreditGrantExpired, WebhookOutCreditGrantExpiredEvent, e)
        }
        OutboxEvent::EntitlementUsageThresholdReached(e) => typed!(
            EntitlementUsageThresholdReached,
            WebhookOutEntitlementAlertEvent,
            e
        ),
        OutboxEvent::EntitlementLimitReached(e) => {
            typed!(EntitlementLimitReached, WebhookOutEntitlementAlertEvent, e)
        }
        OutboxEvent::BudgetThresholdReached(e) => {
            typed!(BudgetThresholdReached, WebhookOutBudgetAlertEvent, e)
        }
        OutboxEvent::BudgetExceeded(e) => typed!(BudgetExceeded, WebhookOutBudgetAlertEvent, e),
        OutboxEvent::EntitlementTooManyGroups(e) => {
            typed!(EntitlementTooManyGroups, WebhookOutTooManyGroupsEvent, e)
        }
        OutboxEvent::WalletBalanceLow(e) => {
            typed!(WalletBalanceLow, WebhookOutWalletBalanceLowEvent, e)
        }
        OutboxEvent::WalletCreditUnrecovered(e) => typed!(
            WalletCreditUnrecovered,
            WebhookOutWalletCreditUnrecoveredEvent,
            e
        ),
        OutboxEvent::WalletCreditRecovered(e) => typed!(
            WalletCreditRecovered,
            WebhookOutWalletCreditRecoveredEvent,
            e
        ),
        OutboxEvent::WalletAutoTopUpFailed(e) => {
            typed!(AutoTopUpFailed, WebhookOutAutoTopUpEvent, e)
        }
        OutboxEvent::WalletAutoTopUpSuspended(e) => {
            typed!(AutoTopUpSuspended, WebhookOutAutoTopUpEvent, e)
        }
        OutboxEvent::WalletAutoTopUpConsentChanged(e) => typed!(
            AutoTopUpConsentChanged,
            WebhookOutAutoTopUpConsentChangedEvent,
            e
        ),
        OutboxEvent::WalletDepleted(e) => typed!(WalletDepleted, WebhookOutWalletDepletedEvent, e),
        OutboxEvent::WalletRecovered(e) => {
            typed!(WalletRecovered, WebhookOutWalletRecoveredEvent, e)
        }
        OutboxEvent::WalletAutoTopUpCapReached(e) => {
            typed!(AutoTopUpCapReached, WebhookOutAutoTopUpCapReachedEvent, e)
        }
        // Not yet webhook-enabled
        OutboxEvent::CustomerUpdated(_) => None,
        // Emitted on every transaction save; the failure, refund and reversal events are
        // the ones integrators subscribe to.
        OutboxEvent::PaymentTransactionSaved(_) => None,
    }
}

fn payment_message(
    id: EventId,
    tenant_id: TenantId,
    event_type: WebhookOutEventTypeEnum,
    event: PaymentTransactionEvent,
    timestamp: ApiDateTime,
) -> Result<WebhookOutMessageNew, serde_json::Error> {
    envelope_message(
        id,
        tenant_id,
        event_type,
        WebhookOutPaymentEvent {
            id,
            event_type,
            data: WebhookOutRefundEventData::from(event),
            timestamp,
        },
    )
}

/// Turns each outbox event into a stored `webhook_out_message` plus one
/// PENDING delivery per matching endpoint, which the delivery worker then picks up.
pub(crate) struct WebhookOut {
    store: Arc<Store>,
}

impl WebhookOut {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    #[tracing::instrument(skip_all, fields(
        tenant_id = %event.tenant_id(),
        customer_id = event.customer_id().map(tracing::field::display),
        subscription_id = event.subscription_id().map(tracing::field::display),
        invoice_id = event.invoice_id().map(tracing::field::display),
        event_type = %event.event_type(),
    ))]
    async fn publish_webhook_event(
        &self,
        msg_id: MessageId,
        event: OutboxEvent,
        endpoints: &[WebhookEndpoint],
    ) -> Result<(), (MessageId, String)> {
        let event_type = event.event_type();
        let Some(message) = to_webhook_message(event) else {
            return Ok(());
        };
        let message = message.map_err(|e| {
            log::error!(
                "Failed to build webhook_out {event_type} message with msg_id={}: {e:?}",
                msg_id.0
            );
            HandleResult::fail(msg_id, &e)
        })?;
        self.fan_out(message, endpoints).await.map_err(|e| {
            tracing::warn!(
                "Failed to fan out webhook_out {event_type} event with msg_id={}: {e:?}",
                msg_id.0
            );
            HandleResult::fail(msg_id, &e)
        })
    }

    async fn fan_out(
        &self,
        message: WebhookOutMessageNew,
        endpoints: &[WebhookEndpoint],
    ) -> Result<(), Report<PgmqError>> {
        let beta = is_beta_event_type(&message.event_type);
        let targets: Vec<WebhookEndpointId> = endpoints
            .iter()
            .filter(|e| receives(e, &message.event_type, beta))
            .map(|e| e.id)
            .collect();

        if targets.is_empty() {
            return Ok(());
        }

        self.store
            .insert_webhook_message_with_deliveries(message, &targets)
            .await
            .change_context(PgmqError::HandleMessages)
    }
}

pub(crate) fn is_beta_event_type(event_type: &str) -> bool {
    WebhookOutEventTypeEnum::from_str(event_type).is_ok_and(|event| event.group().is_beta())
}

/// Beta event types are not in the published spec, so an "all events" endpoint never chose
/// them: they reach only the endpoints that list them.
fn receives(endpoint: &WebhookEndpoint, event_type: &str, beta: bool) -> bool {
    if beta {
        endpoint.event_types.iter().any(|t| t == event_type)
    } else {
        endpoint.matches_event_type(event_type)
    }
}

#[async_trait::async_trait]
impl PgmqHandler for WebhookOut {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let decoded = to_outbox_events(msgs);
        let msg_id_to_out_evt = decoded.events;

        let tenant_ids: Vec<TenantId> = msg_id_to_out_evt
            .iter()
            .map(|(_, e)| e.tenant_id())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let mut by_tenant: HashMap<TenantId, Vec<WebhookEndpoint>> = HashMap::new();
        for endpoint in self
            .store
            .list_active_webhook_endpoints_for_tenants(&tenant_ids)
            .await
            .change_context(PgmqError::HandleMessages)?
        {
            by_tenant
                .entry(endpoint.tenant_id)
                .or_default()
                .push(endpoint);
        }

        let mut succeeded = Vec::new();
        let mut failed = decoded.failed;

        for (msg_id, event) in msg_id_to_out_evt {
            let endpoints = by_tenant
                .get(&event.tenant_id())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            match self.publish_webhook_event(msg_id, event, endpoints).await {
                Ok(()) => succeeded.push(msg_id),
                Err(failure) => failed.push(failure),
            }
        }

        Ok(HandleResult { succeeded, failed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::{BaseId, InvoiceId, PaymentTransactionId};
    use meteroid_store::domain::PaymentTypeEnum;
    use meteroid_store::domain::enums::DeclineKind;

    fn transaction(status: PaymentStatusEnum, payment_type: PaymentTypeEnum) -> OutboxEvent {
        OutboxEvent::payment_transaction_saved(PaymentTransactionEvent {
            id: EventId::new(),
            payment_transaction_id: PaymentTransactionId::new(),
            tenant_id: TenantId::new(),
            invoice_id: Some(InvoiceId::new()),
            checkout_session_id: None,
            provider_transaction_id: Some("pi_1".to_string()),
            processed_at: None,
            refunded_at: None,
            amount: 2500,
            currency: "EUR".to_string(),
            payment_method_id: None,
            status,
            payment_type,
            error_type: Some("Your card has expired.".to_string()),
            receipt_pdf_id: None,
            pending_plan_version_id: None,
            amount_refunded: 0,
            amount_reversed: 0,
            amount_disputed: 0,
            amount_reversed_excess: 0,
            reversal_kind: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            decline_kind: Some(DeclineKind::CardExpired),
        })
    }

    #[test]
    fn a_failed_payment_emits_payment_failed() {
        let message = to_webhook_message(transaction(
            PaymentStatusEnum::Failed,
            PaymentTypeEnum::Payment,
        ))
        .expect("webhook-enabled")
        .expect("serializes");

        assert_eq!(message.event_type, "payment.failed");
        assert_eq!(message.payload["decline_kind"], "CARD_EXPIRED");
        assert_eq!(message.payload["error"], "Your card has expired.");
    }

    #[test]
    fn a_failed_top_up_emits_payment_failed() {
        for payment_type in [PaymentTypeEnum::TopUp, PaymentTypeEnum::AutoTopup] {
            let message = to_webhook_message(transaction(PaymentStatusEnum::Failed, payment_type))
                .expect("webhook-enabled")
                .expect("serializes");

            assert_eq!(message.event_type, "payment.failed");
            assert_eq!(message.payload["payment_type"], "PAYMENT");
        }
    }

    #[test]
    fn usage_alert_payload_snapshots() {
        use chrono::TimeZone;
        use common_domain::ids::{BudgetId, CustomerId, FeatureId, SubscriptionId};
        use meteroid_store::domain::usage_alert_events::{
            AlertReading, BudgetAlertEvent, EntitlementAlertEvent,
        };
        use rust_decimal::Decimal;
        use std::collections::BTreeMap;

        // `timestamp` is stamped at send time, so it is checked for presence only.
        fn deliver(event: OutboxEvent) -> WebhookOutMessageNew {
            let mut message = to_webhook_message(event)
                .expect("webhook-enabled")
                .expect("serializes");
            let payload = message.payload.as_object_mut().expect("object payload");
            assert!(payload.remove("timestamp").is_some());
            message
        }

        let start = chrono::NaiveDate::from_ymd_opt(2026, 3, 1)
            .unwrap()
            .and_time(chrono::NaiveTime::MIN);
        let end = start + chrono::Duration::days(1);
        let as_of = chrono::Utc.with_ymd_and_hms(2026, 3, 1, 10, 30, 0).unwrap();
        let reading = |pct: u32, used: i64, limit: i64, group: Option<(&str, &str)>| AlertReading {
            customer_alias: Some("acme".to_owned()),
            metric_code: Some("api_calls".to_owned()),
            group: group.map(|(k, v)| BTreeMap::from([(k.to_owned(), v.to_owned())])),
            threshold_pct: pct,
            used: Decimal::from(used),
            limit: Decimal::from(limit),
            window_start: start,
            window_end: Some(end),
            as_of,
        };
        let tenant_id = TenantId::new();
        let customer = CustomerId::new();
        let event_id = EventId::new();

        let entitlement = EntitlementAlertEvent::new(
            event_id,
            tenant_id,
            customer,
            FeatureId::new(),
            Some(SubscriptionId::new()),
            reading(80, 800, 1_000, Some(("region", "eu"))),
        );
        let feature_id = entitlement.feature_id.to_string();
        let subscription_id = entitlement.subscription_id.unwrap().to_string();
        let message = deliver(OutboxEvent::EntitlementUsageThresholdReached(Box::new(
            entitlement,
        )));
        assert_eq!(message.id, event_id);
        assert_eq!(message.event_type, "entitlement.usage_threshold_reached");
        assert_eq!(
            message.payload,
            serde_json::json!({
                "id": event_id.to_string(),
                "type": "entitlement.usage_threshold_reached",
                "customer_id": customer.to_string(),
                "customer_alias": "acme",
                "feature_id": feature_id,
                "subscription_id": subscription_id,
                "metric_code": "api_calls",
                "group": {"region": "eu"},
                "threshold_pct": 80,
                "used": "800",
                "limit": "1000",
                "period": {"start": "2026-03-01T00:00:00.000Z", "end": "2026-03-02T00:00:00.000Z"},
                "as_of": "2026-03-01T10:30:00.000Z",
                "exact": false,
            })
        );

        let limit = EntitlementAlertEvent::new(
            event_id,
            tenant_id,
            customer,
            FeatureId::new(),
            Some(SubscriptionId::new()),
            reading(100, 1_000, 1_000, None),
        );
        let feature_id = limit.feature_id.to_string();
        let subscription_id = limit.subscription_id.unwrap().to_string();
        let message = deliver(OutboxEvent::EntitlementLimitReached(Box::new(limit)));
        assert_eq!(message.event_type, "entitlement.limit_reached");
        assert_eq!(
            message.payload,
            serde_json::json!({
                "id": event_id.to_string(),
                "type": "entitlement.limit_reached",
                "customer_id": customer.to_string(),
                "customer_alias": "acme",
                "feature_id": feature_id,
                "subscription_id": subscription_id,
                "metric_code": "api_calls",
                "threshold_pct": 100,
                "used": "1000",
                "limit": "1000",
                "period": {"start": "2026-03-01T00:00:00.000Z", "end": "2026-03-02T00:00:00.000Z"},
                "as_of": "2026-03-01T10:30:00.000Z",
                "exact": false,
            })
        );

        let scope = SubscriptionId::new();
        let budget = BudgetAlertEvent {
            subscription_id: Some(scope),
            ..BudgetAlertEvent::new(
                event_id,
                tenant_id,
                customer,
                BudgetId::new(),
                "EUR".to_owned(),
                None,
                reading(100, 10_850, 10_000, None),
            )
        };
        let budget_id = budget.budget_id.to_string();
        let message = deliver(OutboxEvent::BudgetExceeded(Box::new(budget)));
        assert_eq!(message.event_type, "budget.exceeded");
        assert_eq!(
            message.payload,
            serde_json::json!({
                "id": event_id.to_string(),
                "type": "budget.exceeded",
                "customer_id": customer.to_string(),
                "customer_alias": "acme",
                "budget_id": budget_id,
                "subscription_id": scope.to_string(),
                "currency": "EUR",
                "threshold_pct": 100,
                "used": 10_850,
                "amount": 10_000,
                "period": {"start": "2026-03-01T00:00:00.000Z", "end": "2026-03-02T00:00:00.000Z"},
                "as_of": "2026-03-01T10:30:00.000Z",
                "exact": false,
            })
        );
    }

    #[test]
    fn other_transaction_saves_emit_nothing() {
        for (status, payment_type) in [
            (PaymentStatusEnum::Settled, PaymentTypeEnum::Payment),
            (PaymentStatusEnum::Pending, PaymentTypeEnum::Payment),
            (PaymentStatusEnum::Failed, PaymentTypeEnum::Refund),
        ] {
            assert!(to_webhook_message(transaction(status, payment_type)).is_none());
        }
    }
}

#[cfg(test)]
mod fan_out_tests {
    use super::*;
    use common_domain::ids::BaseId;
    use strum::IntoEnumIterator;

    fn endpoint(event_types: &[&str]) -> WebhookEndpoint {
        WebhookEndpoint {
            id: WebhookEndpointId::new(),
            tenant_id: TenantId::new(),
            url: "https://example.com/hook".to_owned(),
            description: None,
            event_types: event_types.iter().map(|t| (*t).to_owned()).collect(),
            disabled: false,
            disabled_reason: None,
            disabled_at: None,
            consecutive_failures: 0,
            last_success_at: None,
            last_failure_at: None,
            created_at: common_domain::clock::wall().naive_utc(),
            updated_at: None,
            archived_at: None,
            max_in_flight: 10,
            headers: vec![],
            legacy_svix_headers: false,
            failing_since: None,
            unreachable_streak: 0,
            paused_until: None,
            rate_limit_per_sec: None,
        }
    }

    #[test]
    fn beta_events_reach_only_endpoints_that_list_them() {
        let all = endpoint(&[]);
        for event in WebhookOutEventTypeEnum::iter() {
            let name = event.to_string();
            let beta = is_beta_event_type(&name);
            assert_eq!(beta, event.group().is_beta(), "{name}");
            assert_eq!(
                receives(&all, &name, beta),
                !beta,
                "{name} on an all-events endpoint"
            );
            assert!(
                receives(&endpoint(&[&name]), &name, beta),
                "{name} when listed"
            );
            assert!(
                !receives(&endpoint(&["invoice.finalized"]), &name, beta)
                    || name == "invoice.finalized",
                "{name} when not listed"
            );
        }
    }
}
