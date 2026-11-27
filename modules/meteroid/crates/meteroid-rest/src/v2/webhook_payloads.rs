//! Webhook payloads in API v2's model (ADR-0013 §15), sent to an endpoint whose `api_version` is
//! `V2_WEBHOOK_PAYLOADS` or later: the envelope `{id, type, created_at, api_version, data}`.
//!
//! An event about a resource carries the resource as its v2 GET reads it, rendered by the GET's own
//! renderers ([`data`]): one deleted before the event is fanned out (always the draft of
//! `invoice.deleted`) is named by its `id` alone. A notification (a payment's outcome, credit
//! moving, a balance crossing a threshold, an alert) carries what it observed, in v2's conventions
//! (references, snake_case enums, amounts in minor units), beside the resources it is about as
//! their GETs read them: a wallet's notice its `Wallet`, credit moving its `LedgerEntry` and
//! `CustomerCreditGrant` too, a checkout kept as credit its `CheckoutSession`, a payment's outcome
//! its `InvoicePayment`, a retention notice its `Subscription`. A quote and a cancel flow's
//! session, which v2 does not read, are named by their id.
//!
//! The worker renders `data` once per event when it fans the event out, and each version's
//! envelope once ([`envelope`]); what it stores is what every delivery and retry sends.
use crate::v2::Reader;
use crate::v2::checkout_sessions::CheckoutSession;
use crate::v2::customers::credits::{CustomerCreditGrant, GrantSource, LedgerEntry, Wallet};
use crate::v2::entitlements::model::{LevelKind, LevelResource};
use crate::v2::invoices::model::InvoicePayment;
use crate::v2::problem::{Problem, ProblemCode};
use crate::v2::refs::{
    CreditNoteRef, CreditUnitRef, CustomerRef, FeatureRef, InvoiceRef, PlanRef, ReferenceIds, Refs,
    ResourceRef, SubscriptionRef,
};
use crate::v2::subscriptions::model::{Money, Subscription};
use crate::webhooks::out_model::{
    CreditGrantLapseReason, WebhookOutActor, WebhookOutActorType,
    WebhookOutAutoTopUpCapReachedEventData, WebhookOutAutoTopUpConsentChangedEventData,
    WebhookOutAutoTopUpEventData, WebhookOutBudgetAlertEventData,
    WebhookOutCreditGrantExpiredEventData, WebhookOutEntitlementAlertEventData,
    WebhookOutEntitlementAlertLevel, WebhookOutEventTypeEnum, WebhookOutTooManyGroupsEventData,
    WebhookOutWalletBalanceLowEventData, WebhookOutWalletCreditRecoveredEventData,
    WebhookOutWalletCreditUnrecoveredEventData, WebhookOutWalletDebitedEventData,
    WebhookOutWalletFundedEventData, WebhookOutWalletRecoveredEventData,
};
use chrono::NaiveDate;
use common_domain::ids::{
    BudgetId, CreditGrantId, CustomerId, InvoiceId, LedgerEntryId, PaymentTransactionId, QuoteId,
    RetentionSessionId, SubscriptionId, TenantId,
};
use http::StatusCode;
use meteroid_app::rest::denomination::{Denomination, Denominations};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::outbox_event::{OutboxEvent, PaymentTransactionEvent};
use meteroid_store::domain::{renewals, retention_events};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::CustomerCreditWalletsInterface;
use meteroid_store::repositories::PlansInterface;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::coupons::CouponInterfaceAuto;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::products::ProductInterface;
use meteroid_store::{Services, Store};
use serde::Serialize;
use serde_json::{Value, json};
use utoipa::{OpenApi, ToSchema};

/// What the webhook worker renders `data` with: the store and its services, as a request's
/// `AppState` holds them.
pub struct Renderer {
    pub store: Store,
    pub services: Services,
}

impl Reader for Renderer {
    fn store(&self) -> &Store {
        &self.store
    }

    fn services(&self) -> &Services {
        &self.services
    }
}

pub use meteroid_rest_common::api_versions::webhook_envelope as envelope;

/// What a v2 read refusing for good answers in place of the resource: one gone (deleted since
/// the event) is named by its id.
fn resource_or_id<T: Serialize>(
    read: Result<T, Problem>,
    id: impl ToString,
) -> Result<Value, Problem> {
    match read {
        Ok(resource) => to_value(&resource),
        Err(problem) if problem.status == StatusCode::NOT_FOUND.as_u16() => {
            Ok(json!({ "id": id.to_string() }))
        }
        Err(problem) => Err(problem),
    }
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, Problem> {
    serde_json::to_value(value).map_err(|error| {
        Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ProblemCode::InternalError,
            format!("a webhook payload does not serialize: {error}"),
        )
    })
}

/// The `data` of the v2 payload of `event`, an event that has a webhook. The resource is read when
/// the event is fanned out; a refusal that is not the resource being gone is the caller's to retry.
pub async fn data(reader: &impl Reader, event: &OutboxEvent) -> Result<Value, Problem> {
    let store = reader.store();
    let tenant_id = event.tenant_id();
    match event {
        OutboxEvent::CustomerCreated(e) | OutboxEvent::CustomerUpdated(e) => resource_or_id(
            crate::v2::customers::read::render(reader, tenant_id, e.customer_id).await,
            e.customer_id,
        ),
        OutboxEvent::SubscriptionCreated(e)
        | OutboxEvent::SubscriptionUpdated(e)
        | OutboxEvent::SubscriptionCancelled(e)
        | OutboxEvent::SubscriptionEnded(e) => {
            subscription(reader, tenant_id, e.subscription_id).await
        }
        OutboxEvent::InvoiceCreated(e)
        | OutboxEvent::InvoiceFinalized(e)
        | OutboxEvent::InvoicePaid(e)
        | OutboxEvent::InvoiceVoided(e)
        | OutboxEvent::InvoiceClosed(e)
        | OutboxEvent::InvoiceConsolidated(e) => resource_or_id(
            crate::v2::invoices::read::invoice(reader, tenant_id, e.invoice_id).await,
            e.invoice_id,
        ),
        OutboxEvent::InvoiceAccountingPdfGenerated(e) => resource_or_id(
            crate::v2::invoices::read::invoice(reader, tenant_id, e.invoice_id).await,
            e.invoice_id,
        ),
        OutboxEvent::InvoiceDeleted(e) => to_value(&DeletedInvoice { id: e.invoice_id }),
        OutboxEvent::CreditNoteCreated(e)
        | OutboxEvent::CreditNoteFinalized(e)
        | OutboxEvent::CreditNoteVoided(e) => {
            let read = match store
                .get_credit_note_by_id(tenant_id, e.credit_note_id)
                .await
            {
                Ok(note) => crate::v2::credit_notes::detailed(reader, tenant_id, &note).await,
                Err(error) => Err(error.into()),
            };
            resource_or_id(read, e.credit_note_id)
        }
        OutboxEvent::PlanCreated(e)
        | OutboxEvent::PlanPublished(e)
        | OutboxEvent::PlanArchived(e) => {
            // The version the event is about, as `GET /plans/{id}?version=` reads it.
            let version = e.version.to_string();
            let read =
                match crate::v2::plans::full_plan(reader, tenant_id, e.plan_id, Some(&version))
                    .await
                {
                    Ok(plan) => crate::v2::plans::render_plan(reader, tenant_id, &plan, true).await,
                    Err(problem) => Err(problem),
                };
            resource_or_id(read, e.plan_id)
        }
        OutboxEvent::ProductCreated(e)
        | OutboxEvent::ProductUpdated(e)
        | OutboxEvent::ProductArchived(e) => {
            let read = match store.find_product_by_id(e.product_id, tenant_id).await {
                Ok(product) => {
                    let families =
                        crate::v2::product_families::family_codes(reader, tenant_id).await?;
                    let mut entitlements =
                        crate::v2::products::product_entitlements(reader, tenant_id, &[product.id])
                            .await?;
                    Ok(crate::v2::products::to_api(
                        product,
                        &families,
                        &mut entitlements,
                    ))
                }
                Err(error) => Err(error.into()),
            };
            resource_or_id(read, e.product_id)
        }
        OutboxEvent::BillableMetricCreated(e)
        | OutboxEvent::BillableMetricUpdated(e)
        | OutboxEvent::BillableMetricArchived(e) => {
            let read = match store
                .find_billable_metric_by_id(e.metric_id, tenant_id)
                .await
            {
                Ok(metric) => crate::v2::metrics::one(reader, tenant_id, metric).await,
                Err(error) => Err(error.into()),
            };
            resource_or_id(read, e.metric_id)
        }
        OutboxEvent::CouponCreated(e)
        | OutboxEvent::CouponUpdated(e)
        | OutboxEvent::CouponArchived(e) => {
            let read = match store.get_coupon_by_id(tenant_id, e.coupon_id).await {
                Ok(coupon) => crate::v2::coupons::one(reader, tenant_id, coupon).await,
                Err(error) => Err(error.into()),
            };
            resource_or_id(read, e.coupon_id)
        }
        OutboxEvent::AddOnCreated(e)
        | OutboxEvent::AddOnUpdated(e)
        | OutboxEvent::AddOnArchived(e) => {
            let read = match store.get_add_on_by_id(tenant_id, e.add_on_id).await {
                Ok(add_on) => crate::v2::add_ons::one(reader, tenant_id, add_on).await,
                Err(error) => Err(error.into()),
            };
            resource_or_id(read, e.add_on_id)
        }
        OutboxEvent::RefundIssued(e)
        | OutboxEvent::RefundSettled(e)
        | OutboxEvent::RefundFailed(e)
        | OutboxEvent::PaymentReversed(e)
        | OutboxEvent::PaymentReinstated(e)
        | OutboxEvent::PaymentTransactionSaved(e) => {
            to_value(&payment(reader, tenant_id, e).await?)
        }
        OutboxEvent::QuoteAccepted(e) => {
            let refs = refs(reader, tenant_id, |ids| ids.customers.push(e.customer_id)).await?;
            to_value(&QuoteEventData {
                quote: e.quote_id,
                customer: refs.customer(e.customer_id),
                subscription: None,
            })
        }
        OutboxEvent::QuoteConverted(e) => {
            let refs = refs(reader, tenant_id, |ids| {
                ids.customers.push(e.customer_id);
                ids.subscriptions.push(e.subscription_id);
            })
            .await?;
            to_value(&QuoteEventData {
                quote: e.quote_id,
                customer: refs.customer(e.customer_id),
                subscription: Some(refs.subscription(e.subscription_id)),
            })
        }
        OutboxEvent::WalletFunded(e) => {
            let v1 = WebhookOutWalletFundedEventData::from((**e).clone());
            let credit = movement(
                reader,
                tenant_id,
                e.customer_id,
                &e.currency,
                e.ledger_entry_id,
                e.credit_grant_id,
            )
            .await?;
            to_value(&WalletFundedEventData {
                customer: credit.customer,
                wallet: credit.wallet,
                ledger_entry: credit.ledger_entry,
                credit_grant: credit.credit_grant,
                source: v1.source.into(),
            })
        }
        OutboxEvent::WalletDebited(e) => {
            let v1 = WebhookOutWalletDebitedEventData::from((**e).clone());
            let credit = movement(
                reader,
                tenant_id,
                e.customer_id,
                &e.currency,
                e.ledger_entry_id,
                e.credit_grant_id,
            )
            .await?;
            to_value(&WalletDebitedEventData {
                customer: credit.customer,
                wallet: credit.wallet,
                ledger_entry: credit.ledger_entry,
                credit_grant: credit.credit_grant,
                reason: v1.reason,
                actor: v1.actor.map(Into::into),
            })
        }
        OutboxEvent::CreditGrantExpired(e) => {
            let v1 = WebhookOutCreditGrantExpiredEventData::from((**e).clone());
            let refs = refs(reader, tenant_id, |ids| ids.customers.push(e.customer_id)).await?;
            to_value(&CreditGrantExpiredEventData {
                customer: refs.customer(e.customer_id),
                credit_grant: grant(reader, tenant_id, e.customer_id, e.credit_grant_id).await?,
                ledger_entry: entry(
                    reader,
                    tenant_id,
                    e.customer_id,
                    &e.currency,
                    e.ledger_entry_id,
                )
                .await?,
                expired_amount: v1.expired_amount,
                expired_at: v1.expired_at,
                reason: v1.reason.into(),
            })
        }
        OutboxEvent::WalletBalanceLow(e) => {
            let v1 = WebhookOutWalletBalanceLowEventData::from((**e).clone());
            let (customer, wallet) =
                customer_wallet(reader, tenant_id, e.customer_id, &e.currency).await?;
            to_value(&WalletBalanceLowEventData {
                customer,
                wallet,
                balance: v1.balance,
                threshold: v1.threshold,
                shared_balance: v1.shared_balance,
            })
        }
        OutboxEvent::WalletDepleted(e) => {
            let (customer, wallet) =
                customer_wallet(reader, tenant_id, e.customer_id, &e.currency).await?;
            to_value(&WalletDepletedEventData { customer, wallet })
        }
        OutboxEvent::WalletRecovered(e) => {
            let v1 = WebhookOutWalletRecoveredEventData::from((**e).clone());
            let (customer, wallet) =
                customer_wallet(reader, tenant_id, e.customer_id, &e.currency).await?;
            to_value(&WalletRecoveredEventData {
                customer,
                wallet,
                balance: v1.balance,
            })
        }
        OutboxEvent::WalletCreditUnrecovered(e) => {
            let v1 = WebhookOutWalletCreditUnrecoveredEventData::from((**e).clone());
            let refs = refs(reader, tenant_id, |ids| {
                ids.customers.push(v1.customer_id);
                ids.invoices.extend(v1.invoice_id);
                ids.credit_notes.extend(v1.credit_note_id);
            })
            .await?;
            to_value(&WalletCreditUnrecoveredEventData {
                customer: refs.customer(v1.customer_id),
                wallet: wallet(reader, tenant_id, e.customer_id, &e.currency).await?,
                invoice: v1.invoice_id.map(|id| refs.invoice(id)),
                payment: v1.payment_id,
                credit_note: v1.credit_note_id.map(|id| refs.credit_note(id)),
                cause: v1.cause,
                credit_taken_back: v1.credit_taken_back,
                credit_unrecovered: v1.credit_unrecovered,
                credit_unrecovered_total: v1.credit_unrecovered_total,
            })
        }
        OutboxEvent::WalletCreditRecovered(e) => {
            let v1 = WebhookOutWalletCreditRecoveredEventData::from((**e).clone());
            let refs = refs(reader, tenant_id, |ids| {
                ids.customers.push(v1.customer_id);
                ids.invoices.extend(v1.invoice_id);
                ids.credit_notes.extend(v1.credit_note_id);
            })
            .await?;
            to_value(&WalletCreditRecoveredEventData {
                customer: refs.customer(v1.customer_id),
                wallet: wallet(reader, tenant_id, e.customer_id, &e.currency).await?,
                invoice: v1.invoice_id.map(|id| refs.invoice(id)),
                payment: v1.payment_id,
                credit_note: v1.credit_note_id.map(|id| refs.credit_note(id)),
                cause: v1.cause,
                credit_recovered: v1.credit_recovered,
                credit_unrecovered_total: v1.credit_unrecovered_total,
            })
        }
        OutboxEvent::WalletAutoTopUpFailed(e) | OutboxEvent::WalletAutoTopUpSuspended(e) => {
            let v1 = WebhookOutAutoTopUpEventData::from((**e).clone());
            let (customer, wallet) =
                customer_wallet(reader, tenant_id, e.customer_id, &e.currency).await?;
            to_value(&AutoTopUpFailedEventData {
                customer,
                wallet,
                currency: v1.currency,
                amount: v1.amount,
                consecutive_failures: v1.consecutive_failures,
                suspended_at: v1.suspended_at,
                error: v1.error,
            })
        }
        OutboxEvent::WalletAutoTopUpConsentChanged(e) => {
            let v1 = WebhookOutAutoTopUpConsentChangedEventData::from((**e).clone());
            let (customer, wallet) =
                customer_wallet(reader, tenant_id, e.customer_id, &e.currency).await?;
            to_value(&AutoTopUpConsentChangedEventData {
                customer,
                wallet,
                currency: v1.currency,
                enabled: v1.enabled,
                previous_configuration_revision: v1.previous_configuration_revision,
                configuration_revision: v1.configuration_revision,
                actor: v1.actor.into(),
                changed_at: v1.changed_at,
                consent_attested_by: v1.consent_attested_by.map(Into::into),
                consent_attested_at: v1.consent_attested_at,
            })
        }
        OutboxEvent::WalletAutoTopUpCapReached(e) => {
            let v1 = WebhookOutAutoTopUpCapReachedEventData::from((**e).clone());
            // `currency` is the money the cap counts, not the wallet's: the wallet names its own.
            let code = store
                .customer_credit_wallet(tenant_id, e.wallet_id)
                .await?
                .map_or_else(|| e.currency.clone(), |wallet| wallet.currency);
            let (customer, wallet) =
                customer_wallet(reader, tenant_id, e.customer_id, &code).await?;
            to_value(&AutoTopUpCapReachedEventData {
                customer,
                wallet,
                currency: v1.currency,
                // v1's `DAILY`, in v2's snake_case.
                period: v1.period.to_ascii_lowercase(),
                cap: v1.cap,
                attempted_amount: v1.attempted_amount,
                period_start: v1.period_start,
                period_end: v1.period_end,
            })
        }
        OutboxEvent::EntitlementUsageThresholdReached(e)
        | OutboxEvent::EntitlementLimitReached(e) => {
            let v1 = WebhookOutEntitlementAlertEventData::from((**e).clone());
            let refs = refs(reader, tenant_id, |ids| {
                ids.customers.push(v1.customer_id);
                ids.features.push(v1.feature_id);
                ids.subscriptions.extend(v1.subscription_id);
            })
            .await?;
            to_value(&EntitlementAlertEventData {
                customer: refs.customer_or(
                    v1.customer_id,
                    v1.customer_alias.as_deref(),
                    &v1.customer_id.to_string(),
                ),
                feature: refs.feature(v1.feature_id),
                subscription: v1.subscription_id.map(|id| refs.subscription(id)),
                metric_code: v1.metric_code,
                level: v1.level.map(|level| match level {
                    WebhookOutEntitlementAlertLevel::Customer => LevelKind::Customer,
                    WebhookOutEntitlementAlertLevel::ResourceType => LevelKind::ResourceType,
                }),
                resource: v1.resource.map(|resource| LevelResource {
                    resource_type: resource.resource_type,
                    key: resource.key,
                }),
                threshold_pct: v1.threshold_pct,
                used: v1.used,
                limit: v1.limit,
                period: AlertPeriod {
                    start: v1.period.start,
                    end: v1.period.end,
                },
                as_of: v1.as_of,
                exact: v1.exact,
            })
        }
        OutboxEvent::EntitlementTooManyGroups(e) => {
            let v1 = WebhookOutTooManyGroupsEventData::from((**e).clone());
            let refs = refs(reader, tenant_id, |ids| {
                ids.customers.push(v1.customer_id);
                ids.features.extend(v1.feature_id);
                ids.subscriptions.extend(v1.subscription_id);
            })
            .await?;
            to_value(&TooManyGroupsEventData {
                customer: refs.customer_or(
                    v1.customer_id,
                    v1.customer_alias.as_deref(),
                    &v1.customer_id.to_string(),
                ),
                feature: v1.feature_id.map(|id| refs.feature(id)),
                subscription: v1.subscription_id.map(|id| refs.subscription(id)),
                metric_code: v1.metric_code,
                group_by: v1.group_by,
                max_groups: v1.max_groups,
                period: AlertPeriod {
                    start: v1.period.start,
                    end: v1.period.end,
                },
                as_of: v1.as_of,
            })
        }
        OutboxEvent::CheckoutPaymentKeptAsCredit(e) => {
            let version = match e.plan_version_id {
                Some(id) => Some(store.get_plan_version_by_id(id, tenant_id).await?),
                None => None,
            };
            let refs = refs(reader, tenant_id, |ids| {
                ids.customers.push(e.customer_id);
                ids.plans
                    .extend(version.as_ref().map(|version| version.plan_id));
            })
            .await?;
            let resources =
                crate::v2::customers::resources::resource_refs(reader, tenant_id, e.resource_id)
                    .await?;
            // The session is cancelled as the payment is kept: it has no page to read.
            let session = resource_or_id(
                crate::v2::checkout_sessions::session_without_page(
                    reader,
                    tenant_id,
                    e.checkout_session_id,
                )
                .await,
                e.checkout_session_id,
            )?;
            to_value(&CheckoutPaymentKeptAsCreditEventData {
                customer: refs.customer(e.customer_id),
                checkout_session: session,
                payment: e.payment_transaction_id,
                plan: version
                    .map(|version| refs.plan(version.plan_id, Some(version.version), None)),
                resource: e.resource_id.map(|id| {
                    resources
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| ResourceRef::of_id(id))
                }),
                amount: e.amount,
                currency: e.currency.clone(),
                reason: e.reason.clone(),
            })
        }
        OutboxEvent::RetentionSessionCompleted(e) => {
            let refs = refs(reader, tenant_id, |ids| ids.customers.push(e.customer_id)).await?;
            to_value(&RetentionSessionCompletedEventData {
                customer: refs.customer(e.customer_id),
                subscription: subscription(reader, tenant_id, e.subscription_id).await?,
                session: e.session_id,
                outcome: e.outcome.into(),
                reason: e.reason.clone(),
                comment: e.comment.clone(),
                offer: e.offer_kind.map(|kind| RetentionOffer {
                    kind: kind.into(),
                    title: e.offer_title.clone(),
                }),
                effective_date: e.effective_date,
                completed_at: e.completed_at.into(),
            })
        }
        OutboxEvent::SubscriptionRenewalUpcoming(e) => {
            let refs = refs(reader, tenant_id, |ids| ids.customers.push(e.customer_id)).await?;
            to_value(&RenewalUpcomingEventData {
                customer: refs.customer(e.customer_id),
                subscription: subscription(reader, tenant_id, e.subscription_id).await?,
                renewal_date: e.renewal_date,
                mode: e.mode.into(),
                source: e.source.into(),
                days_before: e.days_before,
                opt_out_deadline: e.opt_out_deadline,
                term: e.term.clone().map(|term| RenewalTerm {
                    auto_renew: term.auto_renew,
                    renewal_months: term.renewal_months,
                    uplift_percent: term.uplift_percent,
                    notice_days: term.notice_days,
                }),
                mrr: Money {
                    amount: e.mrr_cents,
                    currency: e.currency.clone(),
                },
                notified_at: e.notified_at.into(),
            })
        }
        OutboxEvent::EntitlementOverrideEndingSoon(e)
        | OutboxEvent::EntitlementOverrideEnded(e) => {
            let refs = refs(reader, tenant_id, |ids| {
                ids.customers.push(e.customer_id);
                ids.features.push(e.feature_id);
            })
            .await?;
            to_value(&OverrideEndingEventData {
                customer: refs.customer(e.customer_id),
                feature: refs.feature(e.feature_id),
                entitlement_override: e.override_id,
                resource: e.resource_id,
                effect: e.effect.to_ascii_lowercase(),
                value: e.value.clone(),
                valid_until: e.valid_until.into(),
                reason: e.reason.clone(),
            })
        }
        OutboxEvent::SubscriptionTrialEndingSoon(e) => {
            let refs = refs(reader, tenant_id, |ids| ids.customers.push(e.customer_id)).await?;
            to_value(&TrialEndingEventData {
                customer: refs.customer(e.customer_id),
                subscription: subscription(reader, tenant_id, e.subscription_id).await?,
                trial_end: e.trial_end,
                free: e.free,
            })
        }
        OutboxEvent::BudgetThresholdReached(e) | OutboxEvent::BudgetExceeded(e) => {
            let v1 = WebhookOutBudgetAlertEventData::from((**e).clone());
            let refs = wallet_refs(reader, tenant_id, v1.customer_id, &v1.credit_unit, |ids| {
                ids.subscriptions.extend(v1.subscription_id);
            })
            .await?;
            to_value(&BudgetAlertEventData {
                customer: refs.customer_or(
                    v1.customer_id,
                    v1.customer_alias.as_deref(),
                    &v1.customer_id.to_string(),
                ),
                budget: v1.budget_id,
                subscription: v1.subscription_id.map(|id| refs.subscription(id)),
                denomination: CreditDenomination::new(
                    &refs,
                    v1.currency,
                    v1.credit_unit,
                    v1.credit_unit_precision,
                ),
                threshold_pct: v1.threshold_pct,
                used: v1.used,
                amount: v1.amount,
                period: AlertPeriod {
                    start: v1.period.start,
                    end: v1.period.end,
                },
                as_of: v1.as_of,
                exact: v1.exact,
            })
        }
    }
}

/// The subscription as `GET /subscriptions/{id}` reads it: its summary when a charge v2 cannot
/// express refuses the whole read, which the summary never is.
async fn subscription(
    reader: &impl Reader,
    tenant_id: TenantId,
    id: SubscriptionId,
) -> Result<Value, Problem> {
    let read = match crate::v2::subscriptions::render::subscription(reader, tenant_id, id).await {
        Err(problem) if problem.code == ProblemCode::NotExpressible => {
            crate::v2::subscriptions::render::summary(reader, tenant_id, id).await
        }
        read => read,
    };
    resource_or_id(read, id)
}

/// The keys of what `name` collects, read in one round.
async fn refs(
    reader: &impl Reader,
    tenant_id: TenantId,
    name: impl FnOnce(&mut ReferenceIds),
) -> Result<Refs, Problem> {
    let mut ids = ReferenceIds::default();
    name(&mut ids);
    Refs::load(reader.store(), tenant_id, &ids).await
}

/// The keys a wallet's notification names: its customer, its credit unit, and what `name` adds.
async fn wallet_refs(
    reader: &impl Reader,
    tenant_id: TenantId,
    customer_id: CustomerId,
    credit_unit: &Option<String>,
    name: impl FnOnce(&mut ReferenceIds),
) -> Result<Refs, Problem> {
    refs(reader, tenant_id, |ids| {
        ids.customers.push(customer_id);
        ids.credit_unit_codes.extend(credit_unit.clone());
        name(ids);
    })
    .await
}

/// What a movement of credit names: its customer, its wallet after it, the movement and its
/// grant, each read as its v2 GET reads it.
struct Movement {
    customer: CustomerRef,
    wallet: Value,
    ledger_entry: Value,
    credit_grant: Value,
}

async fn movement(
    reader: &impl Reader,
    tenant_id: TenantId,
    customer_id: CustomerId,
    code: &str,
    ledger_entry_id: LedgerEntryId,
    credit_grant_id: CreditGrantId,
) -> Result<Movement, Problem> {
    let (customer, wallet) = customer_wallet(reader, tenant_id, customer_id, code).await?;
    Ok(Movement {
        customer,
        wallet,
        ledger_entry: entry(reader, tenant_id, customer_id, code, ledger_entry_id).await?,
        credit_grant: grant(reader, tenant_id, customer_id, credit_grant_id).await?,
    })
}

/// The customer's reference and its wallet in `code` (a currency or a credit unit code).
async fn customer_wallet(
    reader: &impl Reader,
    tenant_id: TenantId,
    customer_id: CustomerId,
    code: &str,
) -> Result<(CustomerRef, Value), Problem> {
    let refs = refs(reader, tenant_id, |ids| ids.customers.push(customer_id)).await?;
    Ok((
        refs.customer(customer_id),
        wallet(reader, tenant_id, customer_id, code).await?,
    ))
}

/// The customer's wallet in `code` as `GET /customers/{id}/wallets/{code}` reads it; named by
/// its currency or credit unit alone (a wallet has no id) once the customer is gone.
async fn wallet(
    reader: &impl Reader,
    tenant_id: TenantId,
    customer_id: CustomerId,
    code: &str,
) -> Result<Value, Problem> {
    let read = match reader
        .store()
        .find_customer_by_id(customer_id, tenant_id)
        .await
    {
        Ok(customer) => {
            crate::v2::customers::credits::wallet_in(reader, tenant_id, &customer, code).await
        }
        Err(error) => Err(error.into()),
    };
    match read {
        Ok(wallet) => to_value(&wallet),
        Err(problem) if problem.status == StatusCode::NOT_FOUND.as_u16() => {
            let denomination = Denominations::load(reader.store(), tenant_id, [code])
                .await?
                .of(code)
                .unwrap_or_else(|_| Denomination {
                    currency: None,
                    credit_unit: Some(code.to_owned()),
                    credit_unit_precision: None,
                });
            let refs = refs(reader, tenant_id, |ids| {
                ids.credit_unit_codes
                    .extend(denomination.credit_unit.clone());
            })
            .await?;
            to_value(&CreditDenomination::new(
                &refs,
                denomination.currency.map(|currency| currency.to_string()),
                denomination.credit_unit,
                denomination.credit_unit_precision,
            ))
        }
        Err(problem) => Err(problem),
    }
}

/// The grant as `GET /customers/{id}/credit-grants/{grant_id}` reads it.
async fn grant(
    reader: &impl Reader,
    tenant_id: TenantId,
    customer_id: CustomerId,
    grant_id: CreditGrantId,
) -> Result<Value, Problem> {
    resource_or_id(
        crate::v2::customers::credits::credit_grant(reader, tenant_id, customer_id, grant_id).await,
        grant_id,
    )
}

/// The movement as `GET /customers/{id}/credit-ledger` lists it in the wallet of `code`.
async fn entry(
    reader: &impl Reader,
    tenant_id: TenantId,
    customer_id: CustomerId,
    code: &str,
    entry_id: LedgerEntryId,
) -> Result<Value, Problem> {
    resource_or_id(
        crate::v2::customers::credits::ledger_entry(reader, tenant_id, customer_id, code, entry_id)
            .await,
        entry_id,
    )
}

/// The payment as `GET /invoices/{id}` lists it, read now.
async fn payment(
    reader: &impl Reader,
    tenant_id: TenantId,
    event: &PaymentTransactionEvent,
) -> Result<PaymentEventData, Problem> {
    let tx = reader
        .store()
        .get_payment_tx_by_id(tenant_id, event.payment_transaction_id)
        .await?;
    let refs = refs(reader, tenant_id, |ids| {
        ids.invoices.extend(tx.invoice_id);
        ids.credit_notes.extend(tx.credit_note_id);
    })
    .await?;
    Ok(PaymentEventData {
        invoice: tx.invoice_id.map(|id| refs.invoice(id)),
        payment: crate::v2::invoices::read::payment(tx, &refs),
    })
}

/// What a wallet or a budget counts: a currency, or a credit unit with its precision.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct CreditDenomination {
    /// Absent for a credit unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// Absent for a currency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<CreditUnitRef>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_unit_precision: Option<u8>,
}

impl CreditDenomination {
    fn new(
        refs: &Refs,
        currency: Option<String>,
        credit_unit: Option<String>,
        credit_unit_precision: Option<u8>,
    ) -> Self {
        CreditDenomination {
            currency,
            credit_unit: credit_unit
                .as_deref()
                .and_then(|code| refs.credit_unit_coded(code)),
            credit_unit_precision,
        }
    }
}

/// The draft invoice `invoice.deleted` is about: it is gone, so only its id is known.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct DeletedInvoice {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: InvoiceId,
}

/// A payment's outcome (refunded, reversed, reinstated, failed): the payment as its invoice lists
/// it, and the invoice when it pays one.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct PaymentEventData {
    pub payment: InvoicePayment,
    /// Absent for a payment of no invoice (a top-up, a checkout).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice: Option<InvoiceRef>,
}

/// A quote accepted, or converted into its subscription.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct QuoteEventData {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub quote: QuoteId,
    pub customer: CustomerRef,
    /// Set on `quote.converted`: the subscription it created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionRef>,
}

/// Who changed something: a user, the customer, an API key, an OAuth app, or Meteroid itself.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventActorType {
    User,
    Customer,
    System,
    ApiKey,
    OauthApp,
}

#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct EventActor {
    #[serde(rename = "type")]
    pub actor_type: EventActorType,
    /// Its prefixed id; absent for `system`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl From<WebhookOutActor> for EventActor {
    fn from(actor: WebhookOutActor) -> Self {
        EventActor {
            actor_type: match actor.actor_type {
                WebhookOutActorType::User => EventActorType::User,
                WebhookOutActorType::Customer => EventActorType::Customer,
                WebhookOutActorType::System => EventActorType::System,
                WebhookOutActorType::ApiKey => EventActorType::ApiKey,
                WebhookOutActorType::OauthApp => EventActorType::OauthApp,
            },
            id: actor.id,
        }
    }
}

/// Credit added to a wallet: a purchase, a top-up, a plan's credit, a coupon, a credit note, a
/// grant by hand, or what a voided invoice gave back.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct WalletFundedEventData {
    pub customer: CustomerRef,
    /// The wallet after the movement, as `GET /customers/{id}/wallets/{wallet}` reads it.
    #[schema(value_type = Wallet)]
    pub wallet: Value,
    /// The movement, as `GET /customers/{id}/credit-ledger` lists it.
    #[schema(value_type = LedgerEntry)]
    pub ledger_entry: Value,
    /// The grant credited, as `GET /customers/{id}/credit-grants/{grant_id}` reads it.
    #[schema(value_type = CustomerCreditGrant)]
    pub credit_grant: Value,
    /// Why the credit came in: the grant's source, or what gave a draw back (`credit_note`;
    /// `adjustment` for a voided invoice, or a usage settlement returning what it drew).
    pub source: GrantSource,
}

/// Credit leaving a wallet: applied to an invoice, taken back by a credit note, or removed by hand.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct WalletDebitedEventData {
    pub customer: CustomerRef,
    /// The wallet after the movement, as `GET /customers/{id}/wallets/{wallet}` reads it.
    #[schema(value_type = Wallet)]
    pub wallet: Value,
    /// The movement, as `GET /customers/{id}/credit-ledger` lists it.
    #[schema(value_type = LedgerEntry)]
    pub ledger_entry: Value,
    /// The grant the credit left, as `GET /customers/{id}/credit-grants/{grant_id}` reads it.
    #[schema(value_type = CustomerCreditGrant)]
    pub credit_grant: Value,
    /// Why the credit left, as it was recorded.
    pub reason: String,
    /// Who removed it by hand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<EventActor>,
}

/// Why a grant's credit lapsed.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GrantLapseReason {
    Expired,
    SubscriptionEnded,
}

impl From<CreditGrantLapseReason> for GrantLapseReason {
    fn from(reason: CreditGrantLapseReason) -> Self {
        match reason {
            CreditGrantLapseReason::Expired => GrantLapseReason::Expired,
            CreditGrantLapseReason::SubscriptionEnded => GrantLapseReason::SubscriptionEnded,
        }
    }
}

/// What was left of a grant lapsed: it expired, or the subscription it was for ended.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct CreditGrantExpiredEventData {
    pub customer: CustomerRef,
    /// The grant, as `GET /customers/{id}/credit-grants/{grant_id}` reads it.
    #[schema(value_type = CustomerCreditGrant)]
    pub credit_grant: Value,
    /// The lapse, as `GET /customers/{id}/credit-ledger` lists it.
    #[schema(value_type = LedgerEntry)]
    pub ledger_entry: Value,
    /// In minor units of the wallet.
    pub expired_amount: i64,
    pub expired_at: ApiDateTime,
    pub reason: GrantLapseReason,
}

/// A wallet's balance (credit on account net of usage not invoiced yet) fell below one of its
/// low-balance thresholds. Amounts in minor units of the wallet.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct WalletBalanceLowEventData {
    pub customer: CustomerRef,
    /// The wallet, as `GET /customers/{id}/wallets/{wallet}` reads it.
    #[schema(value_type = Wallet)]
    pub wallet: Value,
    /// The balance that crossed the threshold.
    pub balance: i64,
    pub threshold: i64,
    /// The balance any subscription can use, when only part of `balance` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_balance: Option<i64>,
}

/// A wallet's balance reached zero.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct WalletDepletedEventData {
    pub customer: CustomerRef,
    /// The wallet, as `GET /customers/{id}/wallets/{wallet}` reads it.
    #[schema(value_type = Wallet)]
    pub wallet: Value,
}

/// A wallet's balance came back above zero.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct WalletRecoveredEventData {
    pub customer: CustomerRef,
    /// The wallet, as `GET /customers/{id}/wallets/{wallet}` reads it.
    #[schema(value_type = Wallet)]
    pub wallet: Value,
    /// The balance it came back to, in minor units of the wallet.
    pub balance: i64,
}

/// Credit bought was spent when its money came back (a reversed payment, a refund): part of it
/// could not be taken back. Amounts in minor units of the wallet.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct WalletCreditUnrecoveredEventData {
    pub customer: CustomerRef,
    /// The wallet, as `GET /customers/{id}/wallets/{wallet}` reads it.
    #[schema(value_type = Wallet)]
    pub wallet: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice: Option<InvoiceRef>,
    #[serde(
        serialize_with = "common_domain::ids::string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub payment: Option<PaymentTransactionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_note: Option<CreditNoteRef>,
    pub cause: String,
    pub credit_taken_back: i64,
    pub credit_unrecovered: i64,
    /// What stays unrecovered of this money, every notice of it counted.
    pub credit_unrecovered_total: i64,
}

/// Credit left unrecovered was taken back since. Amounts in minor units of the wallet.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct WalletCreditRecoveredEventData {
    pub customer: CustomerRef,
    /// The wallet, as `GET /customers/{id}/wallets/{wallet}` reads it.
    #[schema(value_type = Wallet)]
    pub wallet: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice: Option<InvoiceRef>,
    #[serde(
        serialize_with = "common_domain::ids::string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub payment: Option<PaymentTransactionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_note: Option<CreditNoteRef>,
    pub cause: String,
    pub credit_recovered: i64,
    pub credit_unrecovered_total: i64,
}

/// An automatic top-up's charge failed (`wallet.auto_top_up_failed`), or failed often enough to
/// suspend it (`wallet.auto_top_up_suspended`).
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct AutoTopUpFailedEventData {
    pub customer: CustomerRef,
    /// The wallet, with its automatic top-up, as `GET /customers/{id}/wallets/{wallet}` reads it.
    #[schema(value_type = Wallet)]
    pub wallet: Value,
    /// The wallet's currency, or its credit unit's code.
    pub currency: String,
    /// The credit the charge would have bought, in minor units of the wallet.
    pub amount: i64,
    pub consecutive_failures: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suspended_at: Option<ApiDateTime>,
    pub error: String,
}

/// An automatic top-up was turned on or off, or its terms changed.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct AutoTopUpConsentChangedEventData {
    pub customer: CustomerRef,
    /// The wallet, with its automatic top-up as changed, as `GET
    /// /customers/{id}/wallets/{wallet}` reads it.
    #[schema(value_type = Wallet)]
    pub wallet: Value,
    /// The wallet's currency, or its credit unit's code.
    pub currency: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_configuration_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration_revision: Option<String>,
    pub actor: EventActor,
    pub changed_at: ApiDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consent_attested_by: Option<EventActor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consent_attested_at: Option<ApiDateTime>,
}

/// A spending cap held back an automatic top-up. Amounts in minor units of `currency`.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct AutoTopUpCapReachedEventData {
    pub customer: CustomerRef,
    /// The wallet, with its automatic top-up, as `GET /customers/{id}/wallets/{wallet}` reads it.
    #[schema(value_type = Wallet)]
    pub wallet: Value,
    /// The money the cap counts (ISO 4217): the customer's currency for a credit unit wallet.
    pub currency: String,
    /// The cap's period: `daily` or `monthly`.
    pub period: String,
    pub cap: i64,
    pub attempted_amount: i64,
    pub period_start: ApiDateTime,
    pub period_end: ApiDateTime,
}

/// The period an alert was measured over; `end` absent for one that never ends.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct AlertPeriod {
    pub start: ApiDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<ApiDateTime>,
}

/// A feature's usage reached a percentage of its limit (`entitlement.usage_threshold_reached`),
/// or the limit itself (`entitlement.limit_reached`).
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct EntitlementAlertEventData {
    pub customer: CustomerRef,
    pub feature: FeatureRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionRef>,
    /// The metric counting the usage; absent for a limit your app counts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_code: Option<String>,
    /// Where the threshold was crossed: across the customer, or for one of its resources (`resource`).
    /// Absent on an alert recorded before resources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<LevelKind>,
    /// With `level: resource_type`: the resource whose limit the usage crossed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<LevelResource>,
    /// 100 is the limit itself. Rely on it rather than on the order of arrival.
    pub threshold_pct: u32,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub used: rust_decimal::Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub limit: rust_decimal::Decimal,
    pub period: AlertPeriod,
    pub as_of: ApiDateTime,
    /// Always false: usage is aggregated asynchronously.
    pub exact: bool,
}

/// A grouped limit holds as many groups as it may: callers of another group are refused until
/// the period ends.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct TooManyGroupsEventData {
    pub customer: CustomerRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature: Option<FeatureRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionRef>,
    pub metric_code: String,
    pub group_by: String,
    pub max_groups: u32,
    pub period: AlertPeriod,
    pub as_of: ApiDateTime,
}

/// A self-serve checkout was paid but its subscription could not be created (its resource was
/// sold meanwhile, or is no longer the customer's): the payment is kept as the customer's credit
/// and the session is cancelled.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct CheckoutPaymentKeptAsCreditEventData {
    pub customer: CustomerRef,
    /// The checkout session, cancelled, as `GET /checkout-sessions/{id}` reads it.
    #[schema(value_type = CheckoutSession)]
    pub checkout_session: Value,
    /// The payment kept as credit.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub payment: PaymentTransactionId,
    /// The plan the session sold, with its version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanRef>,
    /// The resource the subscription was for; absent for the whole customer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceRef>,
    /// Credited to the customer, in minor units of `currency`.
    pub amount: i64,
    pub currency: String,
    /// Why the subscription was refused.
    pub reason: String,
}

/// How a portal cancel flow ended.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetentionOutcome {
    /// An offer kept the subscription.
    Saved,
    /// The customer asked to be contacted; the subscription is kept.
    ContactRequested,
    /// The customer cancelled, at the end of the period.
    Cancelled,
}

impl From<retention_events::RetentionOutcome> for RetentionOutcome {
    fn from(outcome: retention_events::RetentionOutcome) -> Self {
        match outcome {
            retention_events::RetentionOutcome::Saved => RetentionOutcome::Saved,
            retention_events::RetentionOutcome::ContactRequested => {
                RetentionOutcome::ContactRequested
            }
            retention_events::RetentionOutcome::Cancelled => RetentionOutcome::Cancelled,
        }
    }
}

#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetentionOfferKind {
    Coupon,
    PlanChange,
    Contact,
}

impl From<retention_events::RetentionOfferKind> for RetentionOfferKind {
    fn from(kind: retention_events::RetentionOfferKind) -> Self {
        match kind {
            retention_events::RetentionOfferKind::Coupon => RetentionOfferKind::Coupon,
            retention_events::RetentionOfferKind::PlanChange => RetentionOfferKind::PlanChange,
            retention_events::RetentionOfferKind::Contact => RetentionOfferKind::Contact,
        }
    }
}

/// An offer of the cancel flow, as the customer was shown it.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct RetentionOffer {
    pub kind: RetentionOfferKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// A customer finished the portal cancel flow: kept by an offer, asking to be contacted, or
/// cancelled.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct RetentionSessionCompletedEventData {
    pub customer: CustomerRef,
    /// The subscription, as `GET /subscriptions/{id}` reads it.
    #[schema(value_type = Subscription)]
    pub subscription: Value,
    /// The cancel flow's session.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub session: RetentionSessionId,
    pub outcome: RetentionOutcome,
    /// The reason the customer picked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// The offer accepted, or the one declined before cancelling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offer: Option<RetentionOffer>,
    /// When the accepted offer or the cancellation takes effect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_date: Option<NaiveDate>,
    pub completed_at: ApiDateTime,
}

/// What the date of a renewal notice is.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenewalMode {
    /// The subscription renews on the date: its term auto-renews, or an annual or semiannual
    /// period ends.
    Renewing,
    /// The subscription ends on the date: a term not renewing, or an end date.
    Ending,
    /// The subscription is set to cancel on the date.
    Cancelling,
}

impl From<renewals::RenewalMode> for RenewalMode {
    fn from(mode: renewals::RenewalMode) -> Self {
        match mode {
            renewals::RenewalMode::Renewing => RenewalMode::Renewing,
            renewals::RenewalMode::Ending => RenewalMode::Ending,
            renewals::RenewalMode::Cancelling => RenewalMode::Cancelling,
        }
    }
}

/// What dates a renewal: the contract term, an annual or semiannual period, an end date, or a
/// scheduled cancellation.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenewalSource {
    Term,
    Period,
    EndDate,
    Cancellation,
}

impl From<renewals::RenewalSource> for RenewalSource {
    fn from(source: renewals::RenewalSource) -> Self {
        match source {
            renewals::RenewalSource::Term => RenewalSource::Term,
            renewals::RenewalSource::Period => RenewalSource::Period,
            renewals::RenewalSource::EndDate => RenewalSource::EndDate,
            renewals::RenewalSource::Cancellation => RenewalSource::Cancellation,
        }
    }
}

/// How the contract term renews.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct RenewalTerm {
    pub auto_renew: bool,
    pub renewal_months: u32,
    /// Percent added at the renewal to each fee priced by one amount.
    #[serde(
        with = "rust_decimal::serde::str_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub uplift_percent: Option<rust_decimal::Decimal>,
    /// Days before the term's end an opt-out is still taken.
    pub notice_days: u32,
}

/// A subscription reached a notice checkpoint before it renews, its term ends or it cancels
/// (`subscription.renewal_upcoming`, R-157).
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct RenewalUpcomingEventData {
    pub customer: CustomerRef,
    /// The subscription, as `GET /subscriptions/{id}` reads it.
    #[schema(value_type = Subscription)]
    pub subscription: Value,
    /// The day the subscription renews, ends or cancels.
    pub renewal_date: NaiveDate,
    pub mode: RenewalMode,
    pub source: RenewalSource,
    /// The checkpoint reached: how many days before the opt-out deadline, else the date.
    pub days_before: u32,
    /// The last day the renewal can be opted out of, for a term renewing with notice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opt_out_deadline: Option<NaiveDate>,
    /// The contract term, for a subscription under one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub term: Option<RenewalTerm>,
    /// The subscription's monthly recurring revenue when the notice was made, tax excluded.
    pub mrr: Money,
    pub notified_at: ApiDateTime,
}

/// A customer or resource override with an end ends within a week
/// (`entitlement.override_ending_soon`) or has ended (`entitlement.override_ended`), once per end.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct OverrideEndingEventData {
    pub customer: CustomerRef,
    pub feature: FeatureRef,
    /// The override, as `/customers/{id}/entitlement-overrides/{id}` names it across its edits.
    pub entitlement_override: uuid::Uuid,
    /// The stored resource it overrides; absent for the whole customer.
    #[serde(
        serialize_with = "common_domain::ids::string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>)]
    pub resource: Option<common_domain::ids::ResourceId>,
    pub effect: String,
    #[schema(value_type = Object)]
    pub value: Value,
    pub valid_until: ApiDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A subscription's trial ends in 3 days, once per trial end.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct TrialEndingEventData {
    pub customer: CustomerRef,
    /// The subscription, as `GET /subscriptions/{id}` reads it.
    #[schema(value_type = Subscription)]
    pub subscription: Value,
    /// The trial's end, exclusive: billing starts on it.
    pub trial_end: NaiveDate,
    pub free: bool,
}

/// A budget's spend reached a percentage of it (`budget.threshold_reached`), or the budget
/// itself (`budget.exceeded`). Amounts in minor units of the budget.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct BudgetAlertEventData {
    pub customer: CustomerRef,
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub budget: BudgetId,
    /// Set when the budget counts this subscription's usage only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionRef>,
    #[serde(flatten)]
    pub denomination: CreditDenomination,
    pub threshold_pct: u32,
    /// Spend of the period, usage not invoiced yet included.
    pub used: i64,
    pub amount: i64,
    pub period: AlertPeriod,
    pub as_of: ApiDateTime,
    pub exact: bool,
}

/// The schema of an event type's `data`: a resource's own for an event about one, else the
/// notification's.
pub fn data_schema(event: WebhookOutEventTypeEnum) -> &'static str {
    use WebhookOutEventTypeEnum as E;
    match event {
        E::CustomerCreated => "Customer",
        E::SubscriptionCreated
        | E::SubscriptionUpdated
        | E::SubscriptionCancelled
        | E::SubscriptionEnded => "Subscription",
        E::InvoiceCreated
        | E::InvoiceFinalized
        | E::InvoicePaid
        | E::InvoiceVoided
        | E::InvoiceClosed
        | E::InvoiceConsolidated
        | E::InvoiceAccountingPdfGenerated => "Invoice",
        E::InvoiceDeleted => "DeletedInvoice",
        E::CreditNoteCreated | E::CreditNoteFinalized | E::CreditNoteVoided => "CreditNote",
        E::PlanCreated | E::PlanPublished | E::PlanArchived => "Plan",
        E::ProductCreated | E::ProductUpdated | E::ProductArchived => "Product",
        E::BillableMetricCreated | E::BillableMetricUpdated | E::BillableMetricArchived => "Metric",
        E::CouponCreated | E::CouponUpdated | E::CouponArchived => "Coupon",
        E::AddOnCreated | E::AddOnUpdated | E::AddOnArchived => "AddOn",
        E::RefundIssued
        | E::RefundSettled
        | E::RefundFailed
        | E::PaymentReversed
        | E::PaymentReinstated
        | E::PaymentFailed => "PaymentEventData",
        E::QuoteAccepted | E::QuoteConverted => "QuoteEventData",
        E::WalletFunded => "WalletFundedEventData",
        E::WalletDebited => "WalletDebitedEventData",
        E::CreditGrantExpired => "CreditGrantExpiredEventData",
        E::WalletBalanceLow => "WalletBalanceLowEventData",
        E::WalletDepleted => "WalletDepletedEventData",
        E::WalletRecovered => "WalletRecoveredEventData",
        E::WalletCreditUnrecovered => "WalletCreditUnrecoveredEventData",
        E::WalletCreditRecovered => "WalletCreditRecoveredEventData",
        E::AutoTopUpFailed | E::AutoTopUpSuspended => "AutoTopUpFailedEventData",
        E::AutoTopUpConsentChanged => "AutoTopUpConsentChangedEventData",
        E::AutoTopUpCapReached => "AutoTopUpCapReachedEventData",
        E::EntitlementUsageThresholdReached | E::EntitlementLimitReached => {
            "EntitlementAlertEventData"
        }
        E::EntitlementTooManyGroups => "TooManyGroupsEventData",
        E::CheckoutPaymentKeptAsCredit => "CheckoutPaymentKeptAsCreditEventData",
        E::BudgetThresholdReached | E::BudgetExceeded => "BudgetAlertEventData",
        E::RetentionSessionCompleted => "RetentionSessionCompletedEventData",
        E::SubscriptionRenewalUpcoming => "RenewalUpcomingEventData",
        E::EntitlementOverrideEndingSoon | E::EntitlementOverrideEnded => "OverrideEndingEventData",
        E::SubscriptionTrialEndingSoon => "TrialEndingEventData",
    }
}

/// The GET whose answer an event about a resource carries in `data`; none for a notification.
pub fn read_as(event: WebhookOutEventTypeEnum) -> Option<&'static str> {
    match data_schema(event) {
        "Customer" => Some("GET /api/v2/customers/{id_or_alias}"),
        "Subscription" => Some("GET /api/v2/subscriptions/{id_or_external_id}"),
        "Invoice" => Some("GET /api/v2/invoices/{id}"),
        "CreditNote" => Some("GET /api/v2/credit-notes/{id}"),
        "Plan" => Some("GET /api/v2/plans/{id_or_code}?version=<the event's version>"),
        "Product" => Some("GET /api/v2/products/{id_or_code}"),
        "Metric" => Some("GET /api/v2/metrics/{id_or_code}"),
        "Coupon" => Some("GET /api/v2/coupons/{id_or_code}"),
        "AddOn" => Some("GET /api/v2/add-ons/{id_or_code}"),
        _ => None,
    }
}

/// What the `Webhooks` tag says of payload versions, a paragraph of its own.
pub const VERSIONS_NOTE: &str = "Payload versions. An endpoint's `api_version` pins the shape of \
     what it receives. From `2026-12-15` on, each event is the envelope `{id, type, created_at, \
     api_version, data}` documented here: `data` is the resource as its v2 GET reads it (the \
     same schema, references, typed ids and decimals), read when the event is processed, or \
     for a notification what it observed beside the resources it is about, read alike (a \
     wallet's notice its wallet, credit moving its ledger entry and grant too). An endpoint created through v2 holds the \
     `Meteroid-Version` its request named, or the latest version when it named none; one created \
     from the dashboard the latest version too; one holding `2026-12-01`, or none (created through v1, or \
     before versions were recorded), receives API v1's payloads, which v1's document describes. \
     `PATCH /api/v2/webhooks/endpoints/{id}` with `api_version` moves an endpoint to another \
     version from its next event on; nothing else does: a pinned endpoint never changes shape, \
     and a payload is the same bytes on every retry and resend. A resource deleted before its \
     event is processed is named by its `id` alone (a wallet, which has none, by its currency \
     or credit unit).";

/// The schema of an event type's whole payload: `<Resource>Event` for an event about a resource,
/// `<Notice>Event` for a notification's `<Notice>EventData`.
pub fn envelope_schema(event: WebhookOutEventTypeEnum) -> String {
    let data = data_schema(event);
    data.strip_suffix("Data")
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{data}Event"))
}

/// The notifications' `data` schemas, which the v2 document holds beside the resources' for the
/// webhooks it publishes (`document::attach_webhooks`).
#[derive(OpenApi)]
#[openapi(components(schemas(
    DeletedInvoice,
    PaymentEventData,
    QuoteEventData,
    WalletFundedEventData,
    WalletDebitedEventData,
    CreditGrantExpiredEventData,
    WalletBalanceLowEventData,
    WalletDepletedEventData,
    WalletRecoveredEventData,
    WalletCreditUnrecoveredEventData,
    WalletCreditRecoveredEventData,
    AutoTopUpFailedEventData,
    AutoTopUpConsentChangedEventData,
    AutoTopUpCapReachedEventData,
    EntitlementAlertEventData,
    TooManyGroupsEventData,
    BudgetAlertEventData,
    CheckoutPaymentKeptAsCreditEventData,
    RetentionSessionCompletedEventData,
    RenewalUpcomingEventData,
    OverrideEndingEventData,
    TrialEndingEventData,
)))]
pub struct WebhookPayloadsDoc;

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::EventId;
    use strum::IntoEnumIterator;

    #[test]
    fn every_event_type_names_its_schemas() {
        let documented = serde_json::to_value(WebhookPayloadsDoc::openapi()).unwrap();
        let schemas = documented["components"]["schemas"].as_object().unwrap();
        for event in WebhookOutEventTypeEnum::iter() {
            let data = data_schema(event);
            let envelope = envelope_schema(event);
            assert!(envelope.ends_with("Event"), "{event}: {envelope}");
            assert_ne!(envelope, data, "{event}");
            if data.ends_with("EventData") || data == "DeletedInvoice" {
                assert!(schemas.contains_key(data), "{event}: {data} is documented");
            }
        }
        assert_eq!(
            envelope_schema(WebhookOutEventTypeEnum::InvoicePaid),
            "InvoiceEvent"
        );
        assert_eq!(
            envelope_schema(WebhookOutEventTypeEnum::WalletFunded),
            "WalletFundedEvent"
        );
    }

    #[test]
    fn an_envelope_carries_its_version_and_data() {
        let id = EventId::default();
        let payload = envelope(
            id,
            "invoice.paid",
            ApiDateTime::default(),
            "2026-12-15",
            &json!({"id": "inv_1"}),
        );
        assert_eq!(
            payload,
            json!({
                "id": id.to_string(),
                "type": "invoice.paid",
                "created_at": "1970-01-01T00:00:00.000Z",
                "api_version": "2026-12-15",
                "data": {"id": "inv_1"},
            })
        );
    }
}
