use crate::StoreResult;
use crate::domain::credit_outlook::UpcomingChange;
use crate::domain::enums::{InvoiceType, SubscriptionStatusEnum};
use crate::domain::scheduled_events::{ScheduledEvent, ScheduledEventData};
use crate::domain::{Period, SubscriptionDetails};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::billable_metrics::BillableMetricInterface;
use crate::services::Services;
use crate::services::clients::usage::{UsageClient, WindowedUsageData};
use crate::services::credit::forecast::CreditBooks;
use crate::services::invoice_lines::credit_units::CreditUnitCover;
use crate::services::invoice_lines::invoice_lines::{BilledSpan, ComputedInvoiceContent};
use crate::services::lifecycle::billing_events::Applying;
use crate::services::served_usage::UsagePasses;
use crate::services::subscriptions::terminate::ends_with_final_invoice;
use crate::store::PgConn;
use chrono::NaiveDate;
use common_domain::ids::BillableMetricId;
use diesel_models::enums::CycleActionEnum;
use diesel_models::scheduled_events::ScheduledEventRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
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
        .unwrap_or_else(|| chrono::Utc::now().date_naive());
    (next, invoice_date)
}

/// A free trial ending at the period's end: the invoice ending it opens cycle 0 with no arrears, so
/// the trial's usage is never billed (the cycle transition keeps the cycle on EndTrial). A plan
/// change scheduled for that day makes the subscription Active but leaves the EndTrial transition.
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
    let row = SubscriptionRow::get_subscription_by_id(
        conn,
        &details.subscription.tenant_id,
        details.subscription.id,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    Ok(row.subscription.next_cycle_action == Some(CycleActionEnum::EndTrial))
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
    /// A plan change or amendment the lifecycle applies before billing the renewal.
    apply: Option<ScheduledEvent>,
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
    let pauses = status == SubscriptionStatusEnum::Paused;
    let mut ended = details.clone();
    ended.subscription.status = status;
    ended.subscription.current_period_start = on;
    ended.subscription.current_period_end = None;
    ended.subscription.terminated_at = usage_cutoff.filter(|cutoff| cutoff.date() == on);
    // Never activated, it ends without a final invoice (`terminate_subscription`).
    let billed = !details.subscription.pending_checkout;
    let change = if pauses {
        UpcomingChange::Pauses { on, billed }
    } else {
        UpcomingChange::Ends { on, billed }
    };
    NextInvoiceBasis {
        details: ended,
        invoice_date: on,
        invoice_type,
        change: Some(change),
        apply: None,
    }
}

/// The day a subscription stops consuming, when a cancellation, a pause or its end date falls on
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
            apply: None,
        };
        let subscription = &details.subscription;
        let Some(period_end) = subscription.current_period_end else {
            return Ok(plain(renewal));
        };
        let events = ScheduledEventRow::get_pending_user_visible_events_for_subscription(
            conn,
            subscription.id,
            &subscription.tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        for row in events {
            let event: ScheduledEvent = row.try_into()?;
            let on = event.scheduled_time.date();
            if on > period_end {
                break;
            }
            match &event.event_data {
                ScheduledEventData::CancelSubscription { usage_cutoff, .. } => {
                    let invoice_type = if ends_with_final_invoice(
                        conn,
                        subscription.tenant_id,
                        subscription.id,
                        on,
                        *usage_cutoff,
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
                        *usage_cutoff,
                        invoice_type,
                    ));
                }
                ScheduledEventData::PauseSubscription => {
                    return Ok(ending(
                        details,
                        on,
                        SubscriptionStatusEnum::Paused,
                        None,
                        InvoiceType::Recurring,
                    ));
                }
                ScheduledEventData::ApplyPlanChange {
                    new_plan_version_id,
                    ..
                } if on == period_end => {
                    let change = UpcomingChange::PlanChange {
                        plan_version_id: *new_plan_version_id,
                    };
                    return Ok(NextInvoiceBasis {
                        details: details.clone(),
                        invoice_date,
                        invoice_type: InvoiceType::Recurring,
                        change: Some(change),
                        apply: Some(event),
                    });
                }
                ScheduledEventData::ApplyAmendment { .. } if on == period_end => {
                    return Ok(NextInvoiceBasis {
                        details: details.clone(),
                        invoice_date,
                        invoice_type: InvoiceType::Recurring,
                        change: Some(UpcomingChange::Amendment),
                        apply: Some(event),
                    });
                }
                _ => {}
            }
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

    /// The next invoice of `basis`, its credit estimated on `book` when given. A plan change or
    /// amendment is applied by the lifecycle's own code inside a transaction that is rolled back,
    /// so the preview bills exactly what executing it will (its parameters, add-ons, seeded seats).
    pub(crate) async fn compute_next_invoice_on(
        &self,
        conn: &mut PgConn,
        basis: &NextInvoiceBasis,
        book: Option<&CreditBooks>,
    ) -> StoreResult<ComputedInvoiceContent> {
        if matches!(
            basis.change,
            Some(
                UpcomingChange::Ends { billed: false, .. }
                    | UpcomingChange::Pauses { billed: false, .. }
            )
        ) {
            return Ok(ComputedInvoiceContent::empty());
        }
        let Some(event) = basis.apply.as_ref() else {
            return self
                .compute_invoice_on(
                    conn,
                    &basis.invoice_date,
                    &basis.details,
                    None,
                    None,
                    basis.invoice_type.clone(),
                    CreditUnitCover::Estimate { invoice_id: None },
                    book,
                )
                .await;
        };
        // The transaction holds the subscription, so it never reads metering: a pass records the
        // reads the change prices, they run unlocked, and the next pass is served what they read.
        let mut passes = UsagePasses::default();
        loop {
            let usage = passes.next_pass()?;
            let content = self
                .reading_usage(usage.clone())
                .compute_change_rolled_back(conn, event, book)
                .await;
            if passes.served(self.usage_client.as_ref(), &usage).await? {
                return content;
            }
            if let Some(book) = book {
                book.abandon_document()?;
            }
        }
    }

    async fn compute_change_rolled_back(
        &self,
        conn: &mut PgConn,
        event: &ScheduledEvent,
        book: Option<&CreditBooks>,
    ) -> StoreResult<ComputedInvoiceContent> {
        let mut computed = None;
        let rolled_back = self
            .store
            .transaction_with(conn, |conn| {
                let computed = &mut computed;
                async move {
                    match &event.event_data {
                        ScheduledEventData::ApplyPlanChange { .. } => {
                            self.process_apply_plan_change(conn, event, Applying::Preview)
                                .await?
                        }
                        ScheduledEventData::ApplyAmendment { .. } => {
                            self.process_apply_amendment(conn, event, Applying::Preview)
                                .await?
                        }
                        _ => {
                            return Err(Report::new(StoreError::InvalidArgument(
                                "only a plan change or an amendment is applied ahead".into(),
                            )));
                        }
                    }
                    let changed = self
                        .store
                        .get_subscription_details_with_conn(
                            conn,
                            event.tenant_id,
                            event.subscription_id,
                        )
                        .await?;
                    let (renewal, invoice_date) =
                        upcoming_invoice_details_on(conn, &changed).await?;
                    *computed = Some(
                        self.compute_invoice_on(
                            conn,
                            &invoice_date,
                            &renewal,
                            None,
                            None,
                            InvoiceType::Recurring,
                            CreditUnitCover::Estimate { invoice_id: None },
                            book,
                        )
                        .await?,
                    );
                    Err::<(), _>(Report::new(StoreError::FailedPrecondition(
                        "a previewed change is rolled back".into(),
                    )))
                }
                .scope_boxed()
            })
            .await;
        match (computed, rolled_back) {
            (Some(content), _) => Ok(content),
            (None, Err(error)) => Err(error),
            (None, Ok(())) => Err(Report::new(StoreError::InvoiceComputationError)
                .attach("a previewed change was not rolled back")),
        }
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
        self.compute_invoice(
            conn,
            &invoice_date,
            &details,
            None,
            None,
            crate::domain::enums::InvoiceType::Recurring,
        )
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
