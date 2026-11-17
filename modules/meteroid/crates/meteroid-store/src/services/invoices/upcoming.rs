use crate::StoreResult;
use crate::domain::credit_outlook::UpcomingChange;
use crate::domain::enums::{InvoiceType, SubscriptionStatusEnum};
use crate::domain::scheduled_events::{ScheduledEvent, ScheduledEventData};
use crate::domain::{Period, SubscriptionDetails};
use crate::errors::StoreError;
use crate::repositories::billable_metrics::BillableMetricInterface;
use crate::services::Services;
use crate::services::clients::usage::{UsageClient, WindowedUsageData};
use crate::services::credit::forecast::CreditBooks;
use crate::services::invoice_lines::credit_units::CreditUnitCover;
use crate::services::invoice_lines::invoice_lines::{BilledSpan, ComputedInvoiceContent};
use crate::services::subscriptions::terminate::ends_with_final_invoice;
use crate::store::PgConn;
use chrono::NaiveDate;
use common_domain::ids::BillableMetricId;
use diesel_models::subscription_changes::SubscriptionChangeRow;
use diesel_models::subscription_facts::SubscriptionEndingRow;
use error_stack::Report;
use std::sync::Arc;

/// The subscription as its upcoming invoice bills it: dated `current_period_end`, with the next
/// `cycle_index`, so arrear/usage charges cover the current period and advance charges cover the
/// next one (full, no proration).
pub fn upcoming_invoice_details(details: &SubscriptionDetails) -> (SubscriptionDetails, NaiveDate) {
    let mut next = details.clone();
    next.subscription.cycle_index = Some(details.subscription.cycle_index.unwrap_or(0) + 1);
    let invoice_date = details
        .subscription
        .current_period_end
        .unwrap_or_else(|| common_clock::now().date_naive());
    (next, invoice_date)
}

/// A free trial ending at the period's end: the invoice ending it opens cycle 0 with no arrears, so
/// the trial's usage is never billed. A plan change scheduled for that day makes the subscription
/// Active, its period still the trial.
async fn ends_free_trial(conn: &mut PgConn, details: &SubscriptionDetails) -> StoreResult<bool> {
    let subscription = &details.subscription;
    let in_first_cycle = subscription.cycle_index.unwrap_or(0) == 0
        && matches!(
            subscription.status,
            SubscriptionStatusEnum::TrialActive | SubscriptionStatusEnum::Active
        );
    if !in_first_cycle {
        return Ok(false);
    }
    crate::services::subscriptions::in_free_trial_period_tx(conn, subscription).await
}

/// The subscription as its upcoming invoice bills it, as [`upcoming_invoice_details`] but keeping
/// the cycle when the invoice ends a free trial.
pub(crate) async fn upcoming_invoice_details_on(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
) -> StoreResult<(SubscriptionDetails, NaiveDate)> {
    let (mut next, invoice_date) = upcoming_invoice_details(details);
    if ends_free_trial(conn, details).await? {
        next.subscription.cycle_index = details.subscription.cycle_index;
    }
    Ok((next, invoice_date))
}

/// What a subscription's next invoice bills: the renewal of its period, or the changes scheduled
/// on or before its date, as the lifecycle will apply them.
#[derive(Debug, Clone)]
pub(crate) struct NextInvoiceBasis {
    /// The subscription as the invoice is computed for (before a change `apply` makes).
    pub details: SubscriptionDetails,
    pub invoice_date: NaiveDate,
    /// `Final` for an immediate cancellation on the first day of a period already invoiced.
    pub invoice_type: InvoiceType,
    pub change: Option<UpcomingChange>,
}

impl NextInvoiceBasis {
    pub(crate) fn span(&self) -> BilledSpan {
        BilledSpan::of(&self.invoice_type)
    }
}

/// The subscription settling on `on` with `status`, as termination and completion leave it.
fn ending(
    details: &SubscriptionDetails,
    on: NaiveDate,
    status: SubscriptionStatusEnum,
    usage_cutoff: Option<chrono::NaiveDateTime>,
    invoice_type: InvoiceType,
) -> NextInvoiceBasis {
    let mut ended = details.clone();
    ended.subscription.status = status;
    ended.subscription.current_period_start = on;
    ended.subscription.current_period_end = None;
    ended.subscription.terminated_at = usage_cutoff.filter(|cutoff| cutoff.date() == on);
    // Never activated, it ends without a final invoice (R-013).
    let billed = !details.subscription.pending_checkout;
    let change = UpcomingChange::Ends { on, billed };
    NextInvoiceBasis {
        details: ended,
        invoice_date: on,
        invoice_type,
        change: Some(change),
    }
}

/// The day a subscription stops consuming, when a cancellation or its end date falls on
/// or before `until`.
pub fn scheduled_stop(
    end_date: Option<NaiveDate>,
    stops: impl IntoIterator<Item = NaiveDate>,
    until: NaiveDate,
) -> Option<NaiveDate> {
    stops
        .into_iter()
        .chain(end_date)
        .filter(|day| *day <= until)
        .min()
}

impl Services {
    pub(crate) async fn next_invoice_basis(
        &self,
        conn: &mut PgConn,
        details: &SubscriptionDetails,
    ) -> StoreResult<NextInvoiceBasis> {
        let (renewal, invoice_date) = upcoming_invoice_details_on(conn, details).await?;
        let plain = |renewal: SubscriptionDetails| NextInvoiceBasis {
            details: renewal,
            invoice_date,
            invoice_type: InvoiceType::Recurring,
            change: None,
        };
        let subscription = &details.subscription;
        let Some(period_end) = subscription.current_period_end else {
            return Ok(plain(renewal));
        };
        let cancellation = SubscriptionEndingRow::list_pending_cancellations(
            conn,
            subscription.tenant_id,
            &[subscription.id],
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .next();
        if let Some(cancellation) = cancellation.filter(|c| c.at.date() <= period_end) {
            let on = cancellation.at.date();
            // An immediate cancellation stops at its instant, a dated one at its midnight.
            let usage_cutoff =
                Some(cancellation.at).filter(|at| *at > on.and_time(chrono::NaiveTime::MIN));
            let invoice_type = if ends_with_final_invoice(
                conn,
                subscription.tenant_id,
                subscription.id,
                on,
                usage_cutoff,
            )
            .await?
            {
                InvoiceType::Final
            } else {
                InvoiceType::Recurring
            };
            return Ok(ending(
                details,
                on,
                SubscriptionStatusEnum::Cancelled,
                usage_cutoff,
                invoice_type,
            ));
        }
        let change = SubscriptionChangeRow::list_pending_user_visible(
            conn,
            subscription.id,
            &subscription.tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .find(|row| row.effective_at.date() == period_end)
        .map(ScheduledEvent::try_from)
        .transpose()?;
        if let Some(event) = change {
            let on = event.scheduled_time.date();
            let change = match &event.event_data {
                ScheduledEventData::ApplyPlanChange {
                    new_plan_version_id,
                    ..
                } => UpcomingChange::PlanChange {
                    plan_version_id: *new_plan_version_id,
                },
                ScheduledEventData::ApplyAmendment { .. } => UpcomingChange::Amendment,
            };
            // Written as facts, the renewal reads its rows.
            let changed = self.details_on(conn, details, on).await?;
            let (renewal, invoice_date) = upcoming_invoice_details_on(conn, &changed).await?;
            return Ok(NextInvoiceBasis {
                details: renewal,
                invoice_date,
                invoice_type: InvoiceType::Recurring,
                change: Some(change),
            });
        }
        if subscription.end_date.is_some_and(|end| end <= period_end) {
            return Ok(ending(
                details,
                period_end,
                SubscriptionStatusEnum::Completed,
                None,
                InvoiceType::Recurring,
            ));
        }
        Ok(plain(renewal))
    }

    /// These services reading metering through `usage_client`.
    pub(in crate::services) fn reading_usage(
        &self,
        usage_client: Arc<dyn UsageClient>,
    ) -> Services {
        Services {
            store: self.store.clone(),
            usage_client,
            served: None,
            reminder_scan_resume: self.reminder_scan_resume.clone(),
        }
    }

    /// The subscription as its rows and plan window in force on `on` read it: a renewal on `on`
    /// after a change written ahead for that day.
    async fn details_on(
        &self,
        conn: &mut PgConn,
        details: &SubscriptionDetails,
        on: NaiveDate,
    ) -> StoreResult<SubscriptionDetails> {
        let subscription = &details.subscription;
        let mut renewal = self
            .store
            .subscription_details_as_of(conn, subscription.tenant_id, subscription.id, on)
            .await?;
        let windows =
            diesel_models::subscription_facts::SubscriptionPlanWindowRow::list_by_subscription(
                conn,
                subscription.id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if let Some(window) = windows
            .into_iter()
            .find(|w| w.from_date <= on && w.to_date.is_none_or(|to| on < to))
        {
            renewal.subscription.plan_version_id = window.plan_version_id;
            renewal.subscription.period = window.cadence.into();
        }
        Ok(renewal)
    }

    /// The next invoice of `basis`, its credit estimated on `book` when given.
    pub(crate) async fn compute_next_invoice_on(
        &self,
        conn: &mut PgConn,
        basis: &NextInvoiceBasis,
        book: Option<&CreditBooks>,
    ) -> StoreResult<ComputedInvoiceContent> {
        if matches!(
            basis.change,
            Some(UpcomingChange::Ends { billed: false, .. })
        ) {
            return Ok(ComputedInvoiceContent::empty());
        }
        self.compute_invoice_from_facts(
            conn,
            &basis.invoice_date,
            &basis.details,
            None,
            None,
            basis.invoice_type.clone(),
            CreditUnitCover::Estimate { invoice_id: None },
            book,
        )
        .await
    }
    /// Compute the upcoming invoice — i.e. the next invoice that will be generated
    /// at the end of the current billing period, from [`upcoming_invoice_details`].
    pub async fn compute_upcoming_invoice(
        &self,
        conn: &mut PgConn,
        subscription_details: &SubscriptionDetails,
    ) -> StoreResult<ComputedInvoiceContent> {
        let (details, invoice_date) =
            upcoming_invoice_details_on(conn, subscription_details).await?;
        self.compute_invoice_from_facts(
            conn,
            &invoice_date,
            &details,
            None,
            None,
            crate::domain::enums::InvoiceType::Recurring,
            CreditUnitCover::Estimate { invoice_id: None },
            None,
        )
        .await
    }

    /// The upcoming invoice with `prepared`, a plan change to `target` not written, scheduled for
    /// the end of the current period: its rows laid over the facts, as a pending change's next
    /// invoice is previewed. The period's arrears are the old plan's, the next period the target's.
    pub(in crate::services) async fn upcoming_invoice_after_plan_change(
        &self,
        conn: &mut PgConn,
        prepared: &crate::services::subscriptions::plan_change::PreparedPlanChange,
        target: common_domain::ids::PlanVersionId,
    ) -> StoreResult<ComputedInvoiceContent> {
        let on = prepared
            .subscription_details
            .subscription
            .current_period_end
            .unwrap_or_else(|| prepared.change_date());
        let overlay = self.plan_change_overlay(conn, prepared, target, on).await?;
        let after = crate::services::evaluate::preview::details_after(
            &prepared.subscription_details,
            &overlay,
        )?;
        self.preview_next_invoice(conn, &after, &overlay, None)
            .await
    }

    /// The upcoming invoice with one more `add_on` from the current period's start, priced as an
    /// attach prices it: its row laid over the facts, never written.
    pub(in crate::services) async fn upcoming_invoice_with_add_on(
        &self,
        conn: &mut PgConn,
        details: &SubscriptionDetails,
        add_on: common_domain::ids::AddOnId,
    ) -> StoreResult<ComputedInvoiceContent> {
        use crate::domain::subscription_add_ons::{
            CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
        };
        use crate::services::facts::pending::{MinimumChange, PendingFacts};
        let sub = &details.subscription;
        let on = sub.current_period_start;
        let mut change = PendingFacts {
            event_id: <common_domain::ids::ScheduledEventId as common_domain::ids::BaseId>::new(),
            tenant_id: sub.tenant_id,
            subscription_id: sub.id,
            on,
            plan_window: None,
            closes_components: Vec::new(),
            opens_components: Vec::new(),
            closes_add_ons: Vec::new(),
            opens_add_ons: crate::repositories::subscription_add_ons::checkout_add_on_rows(
                conn,
                sub.tenant_id,
                sub.id,
                &[CreateSubscriptionAddOn {
                    add_on_id: add_on,
                    customization: SubscriptionAddOnCustomization::None,
                    quantity: 1,
                }],
                on,
            )
            .await?,
            seat_seeds: Vec::new(),
            set_seats: Vec::new(),
            scheduled_at: None,
            minimum: MinimumChange::Kept,
        };
        change.plan_window = crate::services::facts::pending::amendment_cadence_window_tx(
            conn,
            sub.tenant_id,
            sub.id,
            on,
            Some(&change),
        )
        .await?;
        let overlay = crate::services::evaluate::load::Overlay::of(change);
        let after = crate::services::evaluate::preview::details_after(details, &overlay)?;
        self.preview_next_invoice(conn, &after, &overlay, None)
            .await
    }

    pub async fn get_subscription_component_usage(
        &self,
        subscription_details: &SubscriptionDetails,
        metric_id: BillableMetricId,
        period: Period,
    ) -> StoreResult<WindowedUsageData> {
        let metric = if let Some(m) = subscription_details
            .metrics
            .iter()
            .find(|m| m.id == metric_id)
        {
            m.clone()
        } else {
            subscription_details.usage_scopes.apply(
                self.store
                    .find_billable_metric_by_id(
                        metric_id,
                        subscription_details.subscription.tenant_id,
                    )
                    .await
                    .map_err(|_| {
                        Report::new(StoreError::ValueNotFound(format!(
                            "Metric {} not found on subscription",
                            metric_id
                        )))
                    })?,
            )
        };

        if period.start >= period.end {
            return Ok(WindowedUsageData {
                data: vec![],
                period: period.into(),
            });
        }

        self.usage_client
            .fetch_windowed_usage(
                &subscription_details.subscription.tenant_id,
                &subscription_details.subscription.customer_id,
                &metric,
                period.into(),
            )
            .await
    }
}
