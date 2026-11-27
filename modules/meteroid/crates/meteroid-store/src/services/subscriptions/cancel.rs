use crate::StoreResult;
use crate::domain::customer_emails::{EmailContext, EmailKind};
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::SubscriptionEventType;
use crate::domain::outbox_event::SubscriptionUpdateType;
use crate::domain::subscription_changes::CancellationOutcome;
use crate::domain::{Subscription, SubscriptionDetails};
use crate::errors::StoreError;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use diesel_models::query::credit_inputs;
use error_stack::Report;
use futures::FutureExt;
use scoped_futures::ScopedFutureExt;
use uuid::Uuid;

use crate::repositories::SubscriptionInterface;
use crate::repositories::customer_emails::CustomerEmailsInterface;
use crate::repositories::entity_activity::EntityActivityInterface;
use crate::repositories::subscriptions::CancellationEffectiveAt;
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::credit_notes::CreditNoteRow;
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::invoices::InvoiceRow;
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscription_facts::SubscriptionEndingRow;
use diesel_models::subscriptions::{SubscriptionCycleRowPatch, SubscriptionRow};
// TODO we need to always pass the tenant id and match it with the resource, if not within the resource.
// and even within it's probably still unsafe no ? Ex: creating components against a wrong subscription within a different tenant
use crate::services::Services;
use crate::services::credit::scope::credit_ended_at;
use crate::services::credit::subscription_end::end_subscription_credit_tx;
use common_domain::ids::{BaseId, CreditNoteId, InvoiceId, SubscriptionId, TenantId};

/// The ids of the subscription's invoices and credit notes: what a step issued is the difference
/// of a read before it and one after.
async fn documents_of(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<(Vec<InvoiceId>, Vec<CreditNoteId>)> {
    let invoices = InvoiceRow::list_ids_of_subscription(conn, tenant_id, subscription_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let notes = CreditNoteRow::list_ids_of_subscription(conn, tenant_id, subscription_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok((invoices, notes))
}

/// An ended subscription (cancelled, completed or aborted) has nothing left to cancel (R-141).
fn refuse_ended(details: &SubscriptionDetails) -> StoreResult<()> {
    if details.subscription.status.is_terminal() {
        return Err(Report::new(StoreError::Conflict(
            "The subscription has ended: there is nothing to cancel".to_string(),
        )));
    }
    Ok(())
}

impl Services {
    pub(in crate::services) async fn cancel_subscription(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
    ) -> StoreResult<Subscription> {
        self.cancel_subscription_at(
            actor,
            subscription_id,
            tenant_id,
            reason,
            effective_at,
            common_clock::now(),
        )
        .await
    }

    /// The cancellation requested at `requested_at`, the clock or a replay's earlier moment: it
    /// applies in the request when its ending is reached by then.
    pub(in crate::services) async fn cancel_subscription_at(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
        requested_at: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<Subscription> {
        self.cancel_subscription_outcome_at(
            actor,
            subscription_id,
            tenant_id,
            reason,
            effective_at,
            requested_at,
        )
        .await
        .map(|outcome| outcome.subscription)
    }

    /// [`Self::cancel_subscription_at`] with the documents its ending issued in the request. An
    /// ended subscription is refused; a cancellation still to apply is replaced, its one
    /// `Cancelled` event moved to the new ending (R-141). Once its transaction commits the call
    /// never fails: a retry would cancel again. What follows the commit (the ending applied in
    /// the request, the read back) is best-effort: a step failing is the subscription's failure,
    /// retried by the controller (R-121), and the committed subscription is answered.
    pub(in crate::services) async fn cancel_subscription_outcome_at(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
        requested_at: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<CancellationOutcome> {
        Box::pin(self.cancel_subscription_unless(
            actor,
            subscription_id,
            tenant_id,
            reason,
            effective_at,
            requested_at,
            None,
            true,
        ))
        .await?
        .ok_or_else(|| Report::new(StoreError::CancellationError))
    }

    /// A cancellation the customer never asked for, as a migration rolled back to its source:
    /// nothing confirms it to them, the subscription simply stops at `effective_at`.
    pub(in crate::services) async fn cancel_subscription_unconfirmed(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
    ) -> StoreResult<Subscription> {
        Box::pin(self.cancel_subscription_unless(
            actor,
            subscription_id,
            tenant_id,
            reason,
            effective_at,
            common_clock::now(),
            None,
            false,
        ))
        .await?
        .map(|outcome| outcome.subscription)
        .ok_or_else(|| Report::new(StoreError::CancellationError))
    }

    /// The cancellation, unless `still_unpaid` is no longer an unpaid invoice whose retries ran
    /// out once the customer is locked: `None` then, nothing changed.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn cancel_subscription_unless(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
        requested_at: chrono::DateTime<chrono::Utc>,
        still_unpaid: Option<InvoiceId>,
        confirm_to_customer: bool,
    ) -> StoreResult<Option<CancellationOutcome>> {
        if requested_at > common_clock::now() {
            return Err(Report::new(StoreError::InvalidArgument(
                "a cancellation cannot be requested at a future moment".to_string(),
            )));
        }
        let cancelled = self
            .store
            .transaction(|conn| {
                let actor = &actor;
                let reason = reason.clone();
                async move {
                    // Customer first, as every transaction writing the subscription or its events.
                    let lock =
                        SubscriptionRow::lock_customer_of_tenant(conn, tenant_id, subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    credit_inputs::bump_customer(conn, &lock).await?;
                    let subscription: SubscriptionDetails = self
                        .store
                        .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                        .await?;
                    let customer_id = subscription.subscription.customer_id;
                    let reason_for_audit = reason.clone();
                    if let Some(invoice_id) = still_unpaid
                        && !crate::services::orchestration::payment_transaction_failed::retries_exhausted_tx(
                            conn, &lock, &subscription, invoice_id,
                        )
                        .await?
                    {
                        return Ok(None);
                    }

                    // An ended subscription's claim on usage is settled: an ending would move it.
                    refuse_ended(&subscription)?;
                    // A cancellation still to apply is replaced, but a customer never moves one
                    // the account manager scheduled, as they never revoke it.
                    if actor.is_customer()
                        && SubscriptionEndingRow::list_pending_cancellations(
                            conn,
                            tenant_id,
                            &[subscription_id],
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .iter()
                        .any(|pending| {
                            pending.actor_type != diesel_models::enums::ActorTypeEnum::Customer
                        })
                    {
                        return Err(Report::new(StoreError::Conflict(
                            "This subscription's cancellation was scheduled by your account manager: only they can change it"
                                .to_string(),
                        )));
                    }

                    // A cancellation replaces the changes made alone still to come; the timeline
                    // stays, written again over what they leave, its ending cutting it (R-146, R-162).
                    crate::services::subscriptions::timeline_rewrite::withdraw_one_offs_under_timeline_tx(
                        self,
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        crate::services::facts::pending::CHANGES,
                        "Replaced by subscription cancellation",
                    )
                    .await?;

                    let now = requested_at.naive_utc();

                    // A never-activated subscription (awaiting checkout / pending activation) was
                    // never counted in the active population: abort it immediately with no CANCELLED
                    // lifecycle event (which would wrongly decrement the active-subscriptions total).
                    if subscription.subscription.activated_at.is_none() {
                        crate::repositories::subscriptions::usage_attribution::ensure_claim_releasable(
                            conn,
                            tenant_id,
                            subscription_id,
                            "Aborting this subscription",
                        )
                        .await?;
                        crate::repositories::subscriptions::usage_attribution::claims_moved(
                            conn,
                            tenant_id,
                            subscription_id,
                        )
                        .await?;
                        SubscriptionCycleRowPatch {
                            id: subscription_id,
                            tenant_id,
                            status: Some(SubscriptionStatusEnum::Aborted),
                            current_period_start: None,
                            current_period_end: None,
                            pending_checkout: Some(false),
                            billing_start_date: None,
                            billing_day_anchor: None,
                            terminated_at: None,
                        }
                        .patch(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                        // Kill any still-open checkout session so a late payment can't resurrect
                        // (re-activate + bill) the aborted subscription.
                        CheckoutSessionRow::cancel_open_for_subscription(
                            conn,
                            tenant_id,
                            subscription_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                        let activity = {
                            let mut a = Activity::new(
                                ActivityType::SubscriptionCancelled,
                                EntityType::Subscription,
                                subscription_id.as_uuid(),
                            )
                            .agg_customer(customer_id);
                            if let Some(reason) = reason_for_audit {
                                a = a.with_metadata(serde_json::json!({ "reason": reason }));
                            }
                            a
                        };
                        self.store
                            .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                            .await?;

                        self.emit_subscription_cancelled_tx(
                            conn,
                            tenant_id,
                            subscription_id,
                            reason.clone(),
                        )
                        .await?;

                        let res = SubscriptionRow::get_subscription_by_id(
                            conn,
                            &tenant_id,
                            subscription_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                        if let Some(ended_at) = credit_ended_at(&res.subscription) {
                            end_subscription_credit_tx(
                                conn,
                                &*self.store.hooks.credit,
                                tenant_id,
                                subscription_id,
                                ended_at,
                                common_clock::now(),
                            )
                            .await?;
                        }

                        let aborted: Subscription = res.try_into()?;
                        return Ok(Some((aborted, None)));
                    }

                    // `usage_cutoff` is set only for an immediate cancel: it pins the terminal
                    // period's usage window to the request instant, so same-day usage is billed
                    // rather than truncated at the effective date's midnight. A date-effective
                    // cancel keeps the midnight boundary (None).
                    let (mut billing_end_date, usage_cutoff): (NaiveDate, Option<NaiveDateTime>) =
                        match effective_at {
                            // The period the subscription is in ends it, as Stripe's
                            // cancel_at_period_end: counting a cadence from today overshoots it.
                            CancellationEffectiveAt::EndOfBillingPeriod => (
                                period_end_in_force(conn, &subscription, requested_at)
                                    .await?
                                    .or_else(|| {
                                        subscription
                                            .calculate_cancellable_end_of_period_date(now.date())
                                    })
                                    .ok_or(Report::from(StoreError::CancellationError))?,
                                None,
                            ),
                            CancellationEffectiveAt::Date(date) => (date, None),
                            CancellationEffectiveAt::Immediate => (now.date(), Some(now)),
                        };

                    // A fixed-term subscription can't be cancelled effective after its natural end:
                    // clamp so the cancellation (-1) and the natural completion (Ended -1) can't both
                    // land, which would double-decrement the active-subscriptions count.
                    if let Some(end_date) = subscription.subscription.end_date {
                        billing_end_date = billing_end_date.min(end_date);
                    }

                    // Keep the instant cutoff only when it lands on the (possibly clamped) settled
                    // day. A fixed-term sub cancelled after its end_date clamps billing_end_date into
                    // the past, so `now` no longer belongs to the settled period; fall back to the
                    // date's midnight, keeping `terminated_at` consistent with the period boundary.
                    let usage_cutoff = usage_cutoff.filter(|c| c.date() == billing_end_date);
                    let ends_at =
                        usage_cutoff.unwrap_or_else(|| billing_end_date.and_time(NaiveTime::MIN));

                    crate::services::facts::record_cancellation_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                        ends_at,
                        now,
                        reason,
                        actor,
                    )
                    .await?;

                    // Dunning ends the subscription for the merchant: no cancellation was received.
                    if still_unpaid.is_none() && confirm_to_customer {
                        confirm_cancellation_tx(&self.store, conn, &subscription, now, ends_at)
                            .await?;
                    }

                    let res =
                        SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                    let mrr = subscription.subscription.mrr_cents;

                    // One `Cancelled` event per cancellation still to apply: one it replaces has
                    // its event moved to this ending, never a second churn (R-141).
                    SubscriptionEventRow::withdraw_pending_cancellation(conn, subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let event = SubscriptionEventRow {
                        id: Uuid::now_v7(),
                        subscription_id,
                        event_type: SubscriptionEventType::Cancelled.into(),
                        details: None,
                        created_at: common_clock::now().naive_utc(),
                        mrr_delta: Some(-(mrr as i64)),
                        bi_mrr_movement_log_id: None,
                        applies_to: billing_end_date,
                    };

                    event
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let activity = {
                        let mut a = Activity::new(
                            ActivityType::SubscriptionCancellationScheduled,
                            EntityType::Subscription,
                            subscription_id.as_uuid(),
                        )
                        .agg_customer(customer_id);
                        if let Some(reason) = reason_for_audit {
                            a = a.with_metadata(serde_json::json!({ "reason": reason }));
                        }
                        a
                    };
                    self.store
                        .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                        .await?;

                    self.emit_subscription_updated_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                        SubscriptionUpdateType::CancellationScheduled,
                    )
                    .await?;

                    // The controller wakes at the ending, before its period's end.
                    self.derive_state_tx(conn, actor, tenant_id, subscription_id, requested_at)
                        .await?;

                    // Reached already, it applies in the request: what it issues is what the
                    // subscription holds after less what it holds now.
                    let reached = if ends_at.and_utc() <= requested_at {
                        Some(documents_of(conn, tenant_id, subscription_id).await?)
                    } else {
                        None
                    };
                    let cancelled: Subscription = res.try_into()?;
                    Ok(Some((cancelled, reached)))
                }
                .scope_boxed()
            })
            .await?;
        let Some((subscription, reached)) = cancelled else {
            return Ok(None);
        };

        let Some((invoices_before, notes_before)) = reached else {
            return Ok(Some(CancellationOutcome {
                subscription,
                invoice_ids: Vec::new(),
                credit_note_ids: Vec::new(),
                applies_later: false,
            }));
        };

        // Committed: nothing from here fails the call. The ending applies now, as the controller
        // would: an ending past the stored period's end first renews each period before it, one
        // boundary a step. A step failing (or panicking) is the subscription's failure, recorded
        // and retried by the controller, which applies the ending then (R-121).
        let stepped = std::panic::AssertUnwindSafe(self.step_until_idle(
            tenant_id,
            subscription_id,
            crate::services::lifecycle::controller::CATCH_UP,
            crate::services::lifecycle::controller::StepOf::Request,
        ))
        .catch_unwind()
        .await
        .unwrap_or_else(|panic| Err(crate::services::lifecycle::controller::panicked(panic)));
        let applies_later = match stepped {
            Ok(_) => false,
            Err(error) => {
                log::error!(
                    "The cancellation of subscription {subscription_id} is recorded but did not apply in its request; the controller retries it: {error:?}"
                );
                if let Err(error) = self
                    .subscription_failed(tenant_id, subscription_id, &error)
                    .await
                {
                    log::error!(
                        "The failure of subscription {subscription_id}'s cancellation was not recorded: {error:?}"
                    );
                }
                true
            }
        };

        let read = async {
            let mut conn = self.store.get_conn().await?;
            let after: Subscription =
                SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, subscription_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .try_into()?;
            let documents = documents_of(&mut conn, tenant_id, subscription_id).await?;
            Ok::<_, Report<StoreError>>((after, documents))
        }
        .await;
        Ok(Some(match read {
            Ok((after, (invoices, notes))) => CancellationOutcome {
                subscription: after,
                invoice_ids: invoices
                    .into_iter()
                    .filter(|id| !invoices_before.contains(id))
                    .collect(),
                credit_note_ids: notes
                    .into_iter()
                    .filter(|id| !notes_before.contains(id))
                    .collect(),
                applies_later,
            },
            Err(error) => {
                log::warn!(
                    "Subscription {subscription_id} was cancelled but is not read back: {error:?}"
                );
                CancellationOutcome {
                    subscription,
                    invoice_ids: Vec::new(),
                    credit_note_ids: Vec::new(),
                    applies_later,
                }
            }
        }))
    }
}

impl Services {
    /// What cancelling the subscription `effective_at` would issue, nothing written: its ending
    /// laid over the facts as recording it would, the period's last invoice priced on the
    /// ending's day (a final invoice of its own when that day's invoice is written already) and
    /// the unused time of the fees billed in advance credited as the ending gives it back
    /// (R-016, R-077). Each document is best-effort: one the facts cannot price is left out,
    /// named in `missing`.
    pub(in crate::services) async fn preview_cancellation(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        effective_at: CancellationEffectiveAt,
    ) -> StoreResult<crate::domain::subscription_changes::CancellationPreview> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            use crate::domain::subscription_changes::{CancellationPreview, PreviewPart};
            use crate::services::evaluate::load::Overlay;
            use crate::services::invoice_lines::credit_units::CreditUnitCover;

            let requested_at = common_clock::now();
            let now = requested_at.naive_utc();
            let mut conn = self.store.get_conn().await?;
            let details: SubscriptionDetails = self
                .store
                .get_subscription_details_with_conn(&mut conn, tenant_id, subscription_id)
                .await?;
            refuse_ended(&details)?;
            if details.subscription.activated_at.is_none() {
                return Ok(CancellationPreview {
                    effective_date: now.date(),
                    aborts: true,
                    final_invoice: None,
                    credit_note: None,
                    missing: Vec::new(),
                });
            }
            // As `cancel_subscription_at` dates it.
            let (mut on, usage_cutoff) = match effective_at {
                CancellationEffectiveAt::EndOfBillingPeriod => (
                    period_end_in_force(&mut conn, &details, requested_at)
                        .await?
                        .or_else(|| details.calculate_cancellable_end_of_period_date(now.date()))
                        .ok_or(Report::from(StoreError::CancellationError))?,
                    None,
                ),
                CancellationEffectiveAt::Date(date) => (date, None),
                CancellationEffectiveAt::Immediate => (now.date(), Some(now)),
            };
            if let Some(end_date) = details.subscription.end_date {
                on = on.min(end_date);
            }
            let at = usage_cutoff
                .filter(|cutoff| cutoff.date() == on)
                .unwrap_or_else(|| on.and_time(NaiveTime::MIN))
                .and_utc();
            let overlay = Overlay::of_cancellation(tenant_id, subscription_id, now.date(), at);

            let final_on_its_own = diesel_models::invoices::InvoiceRow::find_existing_recurring_invoice(
                &mut conn,
                tenant_id,
                subscription_id,
                on,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .is_some();
            let mut missing = Vec::new();
            let invoice_type = if final_on_its_own {
                crate::domain::enums::InvoiceType::Final
            } else {
                crate::domain::enums::InvoiceType::Recurring
            };
            let final_invoice = match self
                .compute_invoice_from_facts_with(
                    &mut conn,
                    &on,
                    &details,
                    None,
                    None,
                    invoice_type,
                    CreditUnitCover::Estimate { invoice_id: None },
                    None,
                    Some(&overlay),
                )
                .await
            {
                Ok(invoice) if !invoice.invoice_lines.is_empty() => Some(invoice),
                Ok(_) => None,
                Err(error) => {
                    log::warn!(
                        "The final invoice of a cancellation of subscription {subscription_id} was not previewed: {error:?}"
                    );
                    missing.push(PreviewPart::Invoice);
                    None
                }
            };
            // The note the ending issues (`issue_cancellation_refund`): none where it refunds
            // nothing, else the fees' give-backs as the facts with the ending price them, and
            // what the settlement credits beside them — a shortfall charged up front, each pack's
            // fee capped by its unused paid credit — by the settlement's own computation.
            let credit_note = match self
                .preview_cancellation_note(&mut conn, &details, &overlay, on, at)
                .await
            {
                Ok(note) => note.filter(|note| !note.invoice_lines.is_empty()),
                Err(error) => {
                    log::warn!(
                        "The credit of a cancellation of subscription {subscription_id} was not previewed: {error:?}"
                    );
                    missing.push(PreviewPart::CreditNote);
                    None
                }
            };
            Ok(CancellationPreview {
                effective_date: on,
                aborts: false,
                final_invoice,
                credit_note,
                missing,
            })
        })
        .await
    }
}

impl Services {
    /// The credit note a cancellation ending on `on` (its usage at `at`) issues, nothing written.
    async fn preview_cancellation_note(
        &self,
        conn: &mut crate::store::PgConn,
        details: &SubscriptionDetails,
        overlay: &crate::services::evaluate::load::Overlay,
        on: NaiveDate,
        at: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<Option<crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent>>
    {
        if !super::terminate::refunds_on_cancellation(conn, &details.subscription, on).await? {
            return Ok(None);
        }
        let beyond_fees = self
            .preview_cancellation_credits(conn, details, on, at)
            .await?;
        let documents = self
            .preview_change_documents(
                conn,
                details,
                overlay,
                crate::services::evaluate::change::Change::of_ending(on, at),
                beyond_fees,
            )
            .await?;
        Ok(documents.credit_note)
    }
}

/// Queues the customer's confirmation of the cancellation (§ 312k BGB), once per request: a moved
/// ending, or one requested again after an undo, confirms again.
async fn confirm_cancellation_tx(
    store: &crate::Store,
    conn: &mut crate::store::PgConn,
    details: &SubscriptionDetails,
    received_at: NaiveDateTime,
    ends_at: NaiveDateTime,
) -> StoreResult<()> {
    let subscription = &details.subscription;
    let kind = EmailKind::SubscriptionCancelled;
    store
        .enqueue_customer_email_tx(
            conn,
            subscription.tenant_id,
            kind,
            EmailContext::SubscriptionCancelled {
                customer_id: subscription.customer_id,
                subscription_id: subscription.id,
                plan_name: subscription.plan_name.clone(),
                received_at,
                ends_at,
            },
            format!(
                "{kind}:{}:{}:{}",
                subscription.id,
                ends_at.and_utc().timestamp(),
                received_at.and_utc().timestamp()
            ),
        )
        .await
}

/// The end of the period the subscription is in at `at`, on its grid (R-084): the stored one,
/// unless the controller has not renewed past it yet (it is behind, or retrying), then the one its
/// facts hold at `at`. None without a period.
pub(in crate::services) async fn period_end_in_force(
    conn: &mut crate::store::PgConn,
    details: &SubscriptionDetails,
    at: chrono::DateTime<chrono::Utc>,
) -> StoreResult<Option<NaiveDate>> {
    let stored = details.subscription.current_period_end;
    if stored.is_none_or(|end| end > at.date_naive()) {
        return Ok(stored);
    }
    let loaded = crate::services::evaluate::load::load_facts(
        conn,
        details.subscription.tenant_id,
        details.subscription.id,
        at.date_naive(),
        None,
    )
    .await?;
    let evaluation = billing_core::evaluate::evaluate(
        &loaded.facts,
        at,
        None,
        &crate::services::evaluate::pricer::StorePricer,
    )
    .map_err(crate::services::invoice_lines::fees::pricing_error)?;
    Ok(evaluation
        .state
        .current_period
        .and_then(|(_, end)| end)
        .filter(|end| *end > at.date_naive())
        .or(stored))
}
