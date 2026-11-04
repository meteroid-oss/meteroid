//! Customer and account emails. Customer emails are claim-before-send (D10): one
//! `email_send_claim` row per dedupe key is upserted before SMTP; a released `FAILED` claim
//! is re-won by the retried message, a stale `SENDING` one becomes `delivery_unknown`.

use crate::pgmq::PgmqResult;
use crate::pgmq::outbox::decode_each;
use crate::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CustomerId, EntityActivityId, InvoiceId, QuoteId, TenantId};
use error_stack::{Report, ResultExt};
use itertools::Itertools;
use meteroid_app::features::WhitelabelResolver;
use meteroid_app::services::storage::{self, ObjectStoreService};
use meteroid_mailer::model::{
    Email, EmailAttachment, EmailAttachmentType, EmailRecipient, is_test_address,
};
use meteroid_mailer::render::{Body, Cta, Subject, render};
use meteroid_mailer::service::MailerService;
use meteroid_store::domain::customer_emails::{AudienceSubject, EmailContext, EmailKind};
use meteroid_store::domain::entity_activity::{Activity, ActivityType, AuditInput, EntityType};
use meteroid_store::domain::pgmq::{
    EmailSendAttempts, PgmqMessage, PgmqMessageNew, PgmqQueue, SendEmailRequest,
};
use meteroid_store::errors::StoreError;
use meteroid_store::jwt_claims::{ResourceAccess, generate_portal_token};
use meteroid_store::repositories::customer_emails::{
    CLAIM_IN_FLIGHT_WINDOW, ClaimFinish, ClaimOutcome,
};
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::entity_activity::{
    EntityActivityInterface, EntityActivityInterfaceEmail, SentEmailAttachment, SentEmailNew,
};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::{CustomerEmailsInterface, QuotesInterface, TenantInterface};
use meteroid_store::services::customer_emails::{
    BuildOutcome, DeferReason, EmailRenderInput, EmailTarget, SendOrigin, SkipReason,
    layout_for_entity, sender_ctx, tenant_test_mode,
};
use meteroid_store::services::customer_locale::customer_locale_for;
use meteroid_store::{Services, Store, StoreResult};
use std::sync::Arc;

/// Transport retries: 1 min doubling to 1 h, for about a day.
const SEND_RETRY_MAX_ATTEMPTS: u32 = 30;
const SEND_RETRY_MAX_DELAY_SECS: i32 = 3600;
/// A message that keeps finding a fresh `SENDING` claim is looked at again once that claim
/// can only be `SENT`, released or stale; after this many looks it defers to the live owner.
const IN_FLIGHT_MAX_ATTEMPTS: u32 = 8;
/// Lost a race with a claim released meanwhile: the key is free again, retry shortly.
const RELEASED_RETRY_DELAY_SECS: i32 = 60;

fn in_flight_retry_delay() -> i32 {
    (CLAIM_IN_FLIGHT_WINDOW.num_seconds() + 60)
        .try_into()
        .unwrap_or(i32::MAX)
}

fn send_retry_delay(attempt: u32) -> i32 {
    60i32
        .saturating_mul(1i32 << attempt.min(6))
        .min(SEND_RETRY_MAX_DELAY_SECS)
}

#[derive(Clone)]
pub struct EmailSender {
    mailer: Arc<dyn MailerService>,
    public_url: String,
    rest_api_url: String,
    object_store: Arc<dyn ObjectStoreService>,
    jwt_secret: secrecy::SecretString,
    store: Arc<Store>,
    services: Arc<Services>,
    whitelabel: Arc<dyn WhitelabelResolver>,
}

/// One slim send job; the fat legacy variants are converted into it.
struct CustomerEmailJob {
    tenant_id: TenantId,
    kind: EmailKind,
    context: EmailContext,
    dedupe_key: String,
    resent_from: Option<EntityActivityId>,
    manual: bool,
    attempts: EmailSendAttempts,
}

impl CustomerEmailJob {
    fn message(&self, attempts: EmailSendAttempts) -> StoreResult<PgmqMessageNew> {
        SendEmailRequest::CustomerEmail {
            tenant_id: self.tenant_id,
            kind: self.kind,
            context: self.context.clone(),
            dedupe_key: self.dedupe_key.clone(),
            resent_from: self.resent_from,
            manual: self.manual,
            attempts,
        }
        .try_into()
    }
}

impl EmailSender {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mailer: Arc<dyn MailerService>,
        object_store: Arc<dyn ObjectStoreService>,
        public_url: String,
        rest_api_url: String,
        jwt_secret: secrecy::SecretString,
        store: Arc<Store>,
        services: Arc<Services>,
        whitelabel: Arc<dyn WhitelabelResolver>,
    ) -> Self {
        Self {
            mailer,
            public_url,
            rest_api_url,
            object_store,
            jwt_secret,
            store,
            services,
            whitelabel,
        }
    }

    fn pay_link(&self, tenant_id: TenantId, invoice_id: InvoiceId) -> StoreResult<String> {
        let token = generate_portal_token(
            &self.jwt_secret,
            tenant_id,
            ResourceAccess::Invoice(invoice_id),
            None,
            None,
            false,
        )?;
        Ok(format!(
            "{}/portal/invoice-payment?token={}",
            self.public_url.trim_end_matches('/'),
            token
        ))
    }

    #[tracing::instrument(skip_all, fields(tenant_id = %ev.tenant_id()))]
    async fn send_email_event(&self, ev: SendEmailRequest) -> Result<(), Report<StoreError>> {
        let result = self.send_email(ev).await;
        if let Err(e) = &result {
            log::warn!("Failed to send email: {e:?}");
        }
        result
    }

    async fn send_email(&self, ev: SendEmailRequest) -> Result<(), Report<StoreError>> {
        match ev {
            SendEmailRequest::CustomerEmail {
                tenant_id,
                kind,
                context,
                dedupe_key,
                resent_from,
                manual,
                attempts,
            } => {
                self.send_customer_email(CustomerEmailJob {
                    tenant_id,
                    kind,
                    context,
                    dedupe_key,
                    resent_from,
                    manual,
                    attempts,
                })
                .await
            }
            // Fat messages enqueued before this release: same path, live state (D25).
            SendEmailRequest::InvoiceReady {
                tenant_id,
                invoice_id,
                ..
            } => {
                self.send_customer_email(CustomerEmailJob {
                    tenant_id,
                    kind: EmailKind::InvoiceReady,
                    context: EmailContext::Invoice { invoice_id },
                    dedupe_key: format!("{}:{invoice_id}", EmailKind::InvoiceReady),
                    resent_from: None,
                    manual: false,
                    attempts: EmailSendAttempts::default(),
                })
                .await
            }
            SendEmailRequest::InvoicePaid {
                tenant_id,
                invoice_id,
                ..
            } => {
                self.send_customer_email(CustomerEmailJob {
                    tenant_id,
                    kind: EmailKind::InvoicePaid,
                    context: EmailContext::Invoice { invoice_id },
                    dedupe_key: format!("{}:{invoice_id}", EmailKind::InvoicePaid),
                    resent_from: None,
                    manual: false,
                    attempts: EmailSendAttempts::default(),
                })
                .await
            }
            SendEmailRequest::QuoteReady {
                tenant_id,
                quote_id,
                invoicing_entity_id,
                quote_number,
                expires_at,
                recipient_emails,
                portal_url,
                custom_message,
                ..
            } => {
                let delivery = self.store.email_delivery(tenant_id).await?;
                if recipient_emails.is_empty() {
                    log::warn!("No recipient emails found for quote {quote_number}");
                    return Ok(());
                }
                let entity = self
                    .store
                    .get_invoicing_entity(tenant_id, Some(invoicing_entity_id))
                    .await?;
                let settings = self.store.get_email_settings(tenant_id, entity.id).await?;
                if !settings.is_enabled(EmailKind::QuoteReady) {
                    return Ok(());
                }
                let customer_id = self
                    .store
                    .get_quote_by_id(tenant_id, quote_id)
                    .await?
                    .customer_id;
                if self
                    .store
                    .email_audience_suppresses(
                        tenant_id,
                        entity.id,
                        EmailKind::QuoteReady,
                        AudienceSubject::Customer(customer_id),
                    )
                    .await?
                {
                    return self
                        .record_quote_skip(
                            tenant_id,
                            quote_id,
                            customer_id,
                            SkipReason::AudienceExcluded,
                        )
                        .await;
                }
                let recipients = delivery.deliverable(&deliverable(&recipient_emails));
                if recipients.is_empty() {
                    let reason = if delivery.is_off() {
                        SkipReason::EmailsOff
                    } else {
                        SkipReason::TestMode
                    };
                    return self
                        .record_quote_skip(tenant_id, quote_id, customer_id, reason)
                        .await;
                }
                let customer = self
                    .store
                    .find_customer_by_id(customer_id, tenant_id)
                    .await?;
                let lang = customer_locale_for(&entity, &customer.preferred_locales);
                let whitelabel = self.whitelabel.is_whitelabel(tenant_id).await;
                let test_mode = tenant_test_mode(&self.store, tenant_id).await?;
                let layout =
                    layout_for_entity(&entity, lang, &self.rest_api_url, whitelabel, test_mode);
                let rendered = render(
                    &layout,
                    Subject::Fixed,
                    Body::QuoteReady {
                        quote_number,
                        expires_at,
                        custom_message,
                        cta: Cta::new(portal_url),
                    },
                )
                .change_context(StoreError::MailServiceError)?;
                let mut sender = sender_ctx(&settings, &entity, self.mailer.customer_from_addr())?;
                sender.bcc = delivery.deliverable(&sender.bcc);
                let (from, reply_to) = (sender.from.clone(), sender.reply_to.clone());
                let email = sender.email(
                    recipients.iter().cloned().map(recipient).collect(),
                    rendered.clone(),
                    vec![],
                );
                let delivered = self
                    .mailer
                    .send(email)
                    .await
                    .change_context(StoreError::MailServiceError)?;
                if delivered {
                    // Best effort: a missed audit row is better than a redelivery.
                    if let Err(e) = self
                        .store
                        .record_email_sent(
                            Actor::System,
                            SentEmailNew {
                                tenant_id,
                                entity_type: EntityType::Quote,
                                entity_id: quote_id.as_uuid(),
                                agg_customer_id: Some(customer_id),
                                agg_subscription_id: None,
                                kind: EmailKind::QuoteReady.to_string(),
                                subject: rendered.subject,
                                from_addr: from,
                                reply_to,
                                recipients,
                                body_html: rendered.body_html,
                                attachments: vec![],
                                invoicing_entity_id: Some(entity.id),
                                locale: Some(lang.to_string()),
                                context: None,
                                dedupe_key: None,
                                resent_from: None,
                            },
                        )
                        .await
                    {
                        log::warn!("Quote {quote_id} email sent but not recorded: {e:?}");
                    }
                }
                Ok(())
            }
            SendEmailRequest::AlertTriggered {
                tenant_id,
                alert_name,
                metric_key,
                severity,
                summary,
                window_date,
                recipient: to,
                ..
            } => {
                // Alerts go to the workspace's own team: the delivery mode does not apply.
                // Deep-link to the tenant's in-app alerts feed:
                // `{public_url}/{org_slug}/{tenant_slug}/alerts`.
                let tenant = self.store.find_tenant_by_id(tenant_id).await?;
                let alerts_url = format!(
                    "{}/{}/{}/alerts",
                    self.public_url.trim_end_matches('/'),
                    tenant.organization.slug,
                    tenant.tenant.slug,
                );
                // Alerts aren't tied to a billing entity, so there's no `sent_email` row.
                let _ = self
                    .mailer
                    .send_alert_triggered(meteroid_mailer::model::AlertTriggered {
                        alert_name,
                        severity,
                        summary,
                        metric_key,
                        window_date,
                        alerts_url,
                        recipients: vec![recipient(to)],
                    })
                    .await
                    .change_context(StoreError::MailServiceError)?;
                Ok(())
            }
        }
    }

    /// §3.9 steps 1–6.
    async fn send_customer_email(&self, job: CustomerEmailJob) -> Result<(), Report<StoreError>> {
        let tenant_id = job.tenant_id;
        let delivery = self.store.email_delivery(tenant_id).await?;
        if delivery.is_off() {
            // Not claimed: the key stays free for a send once delivery is back on.
            let target = self.services.email_target(tenant_id, &job.context).await?;
            return self.record_skip(&job, SkipReason::EmailsOff, &target).await;
        }

        // Read before claiming: past a won claim, every early return has to release it.
        let locale_override = match job.resent_from {
            Some(id) => {
                self.store
                    .get_sent_email_detail(tenant_id, id)
                    .await?
                    .locale
            }
            None => None,
        };

        let now = chrono::Utc::now().naive_utc();
        match self
            .store
            .claim_email_send(tenant_id, &job.dedupe_key, now)
            .await?
        {
            ClaimOutcome::Won => {}
            ClaimOutcome::AlreadySent => return Ok(()),
            // A sender that died after winning the claim would otherwise be acked away here:
            // look again once the claim has either finished or gone stale.
            ClaimOutcome::InFlight => {
                if job.attempts.in_flight >= IN_FLIGHT_MAX_ATTEMPTS {
                    // Fresh on every look, so re-won each time: its live owner finishes or
                    // retries the key with its own message.
                    log::warn!(
                        "Dropping duplicate {}: claim kept in flight",
                        job.dedupe_key
                    );
                    return Ok(());
                }
                let attempts = EmailSendAttempts {
                    in_flight: job.attempts.in_flight + 1,
                    ..job.attempts
                };
                return self.requeue(&job, attempts, in_flight_retry_delay()).await;
            }
            // Nobody is sending: never delivery-unknown, just try to win it again.
            ClaimOutcome::Released => {
                if job.attempts.in_flight >= IN_FLIGHT_MAX_ATTEMPTS {
                    return Err(Report::new(StoreError::InvalidArgument(format!(
                        "claim {} released but never re-won",
                        job.dedupe_key
                    ))));
                }
                let attempts = EmailSendAttempts {
                    in_flight: job.attempts.in_flight + 1,
                    ..job.attempts
                };
                return self
                    .requeue(&job, attempts, RELEASED_RETRY_DELAY_SECS)
                    .await;
            }
            ClaimOutcome::Stale => return self.delivery_unknown(&job).await,
        }

        let whitelabel = self.whitelabel.is_whitelabel(tenant_id).await;
        let pay_link = |invoice_id: InvoiceId| self.pay_link(tenant_id, invoice_id);
        let origin = if job.resent_from.is_some() || job.manual {
            SendOrigin::Manual
        } else {
            SendOrigin::Automatic
        };
        let outcome = self
            .services
            .build_customer_email(
                tenant_id,
                job.kind,
                &job.context,
                locale_override.as_deref(),
                None,
                &pay_link,
                whitelabel,
                origin,
            )
            .await;
        let input = match outcome {
            Ok(BuildOutcome::Render(input)) => input,
            Ok(BuildOutcome::Skip(reason, target)) => {
                self.release(&job).await?;
                return self.record_skip(&job, reason, &target).await;
            }
            Ok(BuildOutcome::Defer(reason, target)) => {
                self.release(&job).await?;
                return self.defer(&job, reason, &target).await;
            }
            Err(e) => {
                self.release(&job).await?;
                return Err(e);
            }
        };

        let rendered = match render(&input.layout, input.subject.clone(), input.body.clone()) {
            Ok(r) => r,
            Err(e) => {
                self.release(&job).await?;
                return Err(Report::new(e).change_context(StoreError::MailServiceError));
            }
        };

        let mut attachments = Vec::with_capacity(input.attachments.len());
        let mut sent_attachments = Vec::with_capacity(input.attachments.len());
        for a in &input.attachments {
            match storage::read(self.object_store.as_ref(), &self.store, a.document_id).await {
                Ok(bytes) => {
                    attachments.push(EmailAttachment {
                        filename: a.filename.clone(),
                        content: bytes.to_vec(),
                        type_: EmailAttachmentType::Pdf,
                    });
                    sent_attachments.push(SentEmailAttachment {
                        filename: a.filename.clone(),
                        id: a.document_id,
                        kind: a.kind,
                    });
                }
                Err(e) => {
                    log::warn!(
                        "Attachment {} unavailable for {}: {e:?}",
                        a.document_id,
                        job.dedupe_key
                    );
                    self.release(&job).await?;
                    return self
                        .defer(&job, DeferReason::PdfMissing, &input.target)
                        .await;
                }
            }
        }

        let recipients = delivery.deliverable(&deliverable(&input.recipients));
        if recipients.is_empty() && !input.recipients.iter().all(|r| is_test_address(r)) {
            self.release(&job).await?;
            return self
                .record_skip(&job, SkipReason::TestMode, &input.target)
                .await;
        }
        let mut sender = match input.sender(self.mailer.customer_from_addr()) {
            Ok(s) => s,
            Err(e) => {
                self.release(&job).await?;
                return Err(e);
            }
        };
        sender.bcc = delivery.deliverable(&sender.bcc);
        let (from, reply_to) = (sender.from.clone(), sender.reply_to.clone());
        let email: Email = sender.email(
            recipients.iter().cloned().map(recipient).collect(),
            rendered.clone(),
            attachments,
        );

        match self.mailer.send(email).await {
            Ok(true) => {
                // The email left: a missing audit row must not strand the claim in `SENDING`.
                let row = sent_row(
                    &job,
                    &input,
                    rendered,
                    from,
                    reply_to,
                    recipients,
                    sent_attachments,
                );
                let sent_id = match self.store.record_email_sent(Actor::System, row).await {
                    Ok(id) => Some(id),
                    Err(e) => {
                        log::warn!("{} sent but not recorded: {e:?}", job.dedupe_key);
                        None
                    }
                };
                self.store
                    .finish_email_send(tenant_id, &job.dedupe_key, ClaimFinish::Sent, sent_id)
                    .await
            }
            // Only test addresses: nothing left the process, and nothing ever will for this key.
            Ok(false) => {
                self.store
                    .finish_email_send(tenant_id, &job.dedupe_key, ClaimFinish::Sent, None)
                    .await
            }
            Err(e) => {
                log::warn!("Transport error for {}: {e:?}", job.dedupe_key);
                self.release(&job).await?;
                let tried = job.attempts.transport;
                if tried >= SEND_RETRY_MAX_ATTEMPTS {
                    return self
                        .record_skip(&job, SkipReason::SendFailed, &input.target)
                        .await;
                }
                let attempts = EmailSendAttempts {
                    transport: tried + 1,
                    ..job.attempts
                };
                self.requeue(&job, attempts, send_retry_delay(tried)).await
            }
        }
    }

    /// At-most-once (D10): the claim turns `UNKNOWN`, terminal, and the operator resends.
    async fn delivery_unknown(&self, job: &CustomerEmailJob) -> Result<(), Report<StoreError>> {
        let target = self
            .services
            .email_target(job.tenant_id, &job.context)
            .await?;
        // Finished or released since it was read as stale: nothing is unknown.
        if !self
            .store
            .mark_claim_unknown(job.tenant_id, &job.dedupe_key)
            .await?
        {
            return Ok(());
        }
        self.store
            .record(
                job.tenant_id,
                Actor::System,
                AuditInput::Activity(activity(
                    ActivityType::EmailDeliveryUnknown,
                    &target,
                    serde_json::json!({
                        "kind": job.kind,
                        "dedupe_key": job.dedupe_key,
                        "context": job.context,
                    }),
                )),
            )
            .await
    }

    async fn release(&self, job: &CustomerEmailJob) -> Result<(), Report<StoreError>> {
        self.store
            .finish_email_send(job.tenant_id, &job.dedupe_key, ClaimFinish::Failed, None)
            .await
    }

    async fn requeue(
        &self,
        job: &CustomerEmailJob,
        attempts: EmailSendAttempts,
        delay_seconds: i32,
    ) -> Result<(), Report<StoreError>> {
        let msg = job.message(attempts)?;
        self.store
            .pgmq_send_batch_delayed(PgmqQueue::SendEmailRequest, vec![msg], delay_seconds)
            .await
    }

    async fn defer(
        &self,
        job: &CustomerEmailJob,
        reason: DeferReason,
        target: &EmailTarget,
    ) -> Result<(), Report<StoreError>> {
        let mut attempts = job.attempts;
        let tried = match reason {
            DeferReason::PaymentPending => &mut attempts.payment_pending,
            DeferReason::PdfMissing => &mut attempts.pdf_missing,
        };
        if *tried >= reason.max_attempts() {
            return self
                .record_skip(job, reason.exhausted_reason(), target)
                .await;
        }
        *tried += 1;
        self.requeue(job, attempts, reason.delay_seconds()).await
    }

    async fn record_quote_skip(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        customer_id: CustomerId,
        reason: SkipReason,
    ) -> Result<(), Report<StoreError>> {
        log::info!("Skipping quote_ready for quote {quote_id}: {reason}");
        let target = EmailTarget {
            entity_type: EntityType::Quote,
            entity_id: quote_id.as_uuid(),
            agg_customer_id: Some(customer_id),
            agg_subscription_id: None,
        };
        self.store
            .record(
                tenant_id,
                Actor::System,
                AuditInput::Activity(activity(
                    ActivityType::EmailSkipped,
                    &target,
                    serde_json::json!({ "kind": EmailKind::QuoteReady, "reason": reason }),
                )),
            )
            .await
    }

    async fn record_skip(
        &self,
        job: &CustomerEmailJob,
        reason: SkipReason,
        target: &EmailTarget,
    ) -> Result<(), Report<StoreError>> {
        log::info!("Skipping {} ({}): {reason}", job.dedupe_key, job.kind);
        let activity_type = if job.kind == EmailKind::PaymentReminder {
            ActivityType::PaymentReminderSkipped
        } else {
            ActivityType::EmailSkipped
        };
        self.store
            .record(
                job.tenant_id,
                Actor::System,
                AuditInput::Activity(activity(
                    activity_type,
                    target,
                    serde_json::json!({
                        "kind": job.kind,
                        "reason": reason,
                        "dedupe_key": job.dedupe_key,
                    }),
                )),
            )
            .await
    }
}

fn activity(
    activity_type: ActivityType,
    target: &EmailTarget,
    metadata: serde_json::Value,
) -> Activity {
    let mut a =
        Activity::new(activity_type, target.entity_type, target.entity_id).with_metadata(metadata);
    a.agg_customer_id = target.agg_customer_id;
    a.agg_subscription_id = target.agg_subscription_id;
    a
}

/// `.test` / `.invalid` recipients never reach the transport; the others still get the email.
fn deliverable(recipients: &[String]) -> Vec<String> {
    recipients
        .iter()
        .filter(|r| !is_test_address(r))
        .cloned()
        .collect()
}

fn recipient(email: String) -> EmailRecipient {
    EmailRecipient {
        email,
        first_name: None,
        last_name: None,
    }
}

fn sent_row(
    job: &CustomerEmailJob,
    input: &EmailRenderInput,
    rendered: meteroid_mailer::render::Rendered,
    from_addr: String,
    reply_to: Option<String>,
    recipients: Vec<String>,
    attachments: Vec<SentEmailAttachment>,
) -> SentEmailNew {
    SentEmailNew {
        tenant_id: job.tenant_id,
        entity_type: input.target.entity_type,
        entity_id: input.target.entity_id,
        agg_customer_id: input.target.agg_customer_id,
        agg_subscription_id: input.target.agg_subscription_id,
        kind: job.kind.to_string(),
        subject: rendered.subject,
        from_addr,
        reply_to,
        recipients,
        body_html: rendered.body_html,
        attachments,
        invoicing_entity_id: Some(input.entity.id),
        locale: Some(input.lang.to_string()),
        context: Some(job.context.clone()),
        dedupe_key: Some(job.dedupe_key.clone()),
        resent_from: job.resent_from,
    }
}

#[async_trait::async_trait]
impl PgmqHandler for EmailSender {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let decoded = decode_each(msgs, |msg| {
            let evt: StoreResult<SendEmailRequest> = msg.try_into();
            evt.map(Some)
        });

        let mut result = HandleResult {
            succeeded: vec![],
            failed: decoded.failed,
        };

        let tasks = decoded
            .events
            .into_iter()
            .map(|(id, ev)| {
                let value = self.clone();
                (
                    id,
                    tokio::spawn(async move { value.send_email_event(ev).await }),
                )
            })
            .collect_vec();

        for (id, task) in tasks {
            match task.await {
                Ok(Ok(())) => result.succeeded.push(id),
                Ok(Err(e)) => result.failed.push(HandleResult::fail(id, &e)),
                Err(e) => {
                    log::warn!("Email send task panicked: {e:?}");
                    result.failed.push(HandleResult::fail(id, &e));
                }
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::{in_flight_retry_delay, send_retry_delay};

    #[test]
    fn in_flight_retry_outlives_the_claim_window() {
        assert_eq!(in_flight_retry_delay(), 16 * 60);
    }

    #[test]
    fn transport_backoff_doubles_from_a_minute_and_caps_at_an_hour() {
        assert_eq!(send_retry_delay(0), 60);
        assert_eq!(send_retry_delay(1), 120);
        assert_eq!(send_retry_delay(5), 1920);
        assert_eq!(send_retry_delay(6), 3600);
        assert_eq!(send_retry_delay(29), 3600);
    }
}
