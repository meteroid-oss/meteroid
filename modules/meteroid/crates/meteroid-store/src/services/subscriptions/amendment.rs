use crate::StoreResult;
use crate::domain::LineItemKind;
use crate::domain::SubscriptionStatusEnum;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::SubscriptionFeeBillingPeriod;
use crate::domain::enums::{CreditType, InvoicePaymentStatus, InvoiceStatusEnum};
use crate::domain::invoice_lines::LineItem;
use crate::domain::invoices::Invoice;
use crate::domain::price_components::{PriceComponentNewInternal, PriceEntry, ProductRef};
use crate::domain::prices::FeeStructure;
use crate::domain::scheduled_events::{
    ResolvedAddOnInsert, ResolvedComponentInsert, ScheduledEvent, ScheduledEventData,
    ScheduledEventNew,
};
use crate::domain::subscription_add_ons::{SubscriptionAddOn, SubscriptionAddOnNew};
use crate::domain::subscription_amendment::{
    AmendmentPreview, AmendmentPreviewExtended, ImmediateAmendmentResult, SubscriptionAmendment,
};
use crate::domain::subscription_changes::{
    AddedComponent, PlanChangeMode, ProrationSummary, RemovedComponent,
};
use crate::domain::subscription_components::{
    SubscriptionComponent, SubscriptionComponentNew, SubscriptionComponentNewInternal,
    SubscriptionFee,
};
use crate::domain::{Product, SubscriptionDetails};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, create_user_credit_note_tx,
};
use crate::repositories::entity_activity::EntityActivityInterface;
use crate::repositories::price_components::resolve_component_internal;
use crate::repositories::subscriptions::fetch_prices_and_products;
use crate::services::Services;
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use crate::services::subscriptions::insert::context::resolve_fee_read_only;
use crate::services::subscriptions::plan_change::calculate_components_mrr_with_slots;
use crate::services::subscriptions::utils::{
    MrrLine, calculate_mrr, fee_period_total_cents, scale_fee, subscription_mrr, with_add_on_lines,
};
use crate::store::PgConn;
use chrono::{NaiveDate, NaiveTime};
use common_domain::ids::{
    AddOnId, BaseId, CreditNoteId, InvoiceId, PriceComponentId, PriceId, ProductId,
    SubscriptionAddOnId, SubscriptionId, SubscriptionPriceComponentId, TenantId,
};
use common_utils::decimals::ToSubunit;
use diesel_models::credit_notes::CreditNoteRow;
use diesel_models::enums::CreditNoteStatus;
use diesel_models::invoices::InvoiceRow;
use diesel_models::query::credit_inputs;
use diesel_models::scheduled_events::ScheduledEventRow;
use diesel_models::slot_transactions::SlotTransactionRow;
use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;
use std::collections::HashMap;

/// A component to insert, with the source PriceEntry still needing materialization
/// (for PriceEntry::New) before it can become a concrete row.
struct PendingComponentInsert {
    price_component_id: Option<PriceComponentId>,
    product_ref: ProductRef,
    price_entry: PriceEntry,
    name: String,
    period: SubscriptionFeeBillingPeriod,
    fee: SubscriptionFee,
    is_override: bool,
    /// Lineage root for an override insert (the closed component's lineage); `None`
    /// for a genuinely new component, which becomes its own root.
    lineage_id: Option<SubscriptionPriceComponentId>,
    /// Pre-generated id this component will be inserted with, for a genuine
    /// immediate add. Lets the same amendment's adjustment invoice stamp the id
    /// onto its prorated charge line so a later removal can credit it. `None`
    /// lets the row generate its own id.
    subscription_component_id: Option<SubscriptionPriceComponentId>,
}

/// An add-on to insert. `price_id` is filled when the source is an existing price;
/// `price_entry` carries a New price that needs materialization at apply time.
struct PendingAddOnInsert {
    add_on_id: AddOnId,
    product_id: ProductId,
    price_entry: Option<PriceEntry>,
    price_id: Option<PriceId>,
    name: String,
    period: SubscriptionFeeBillingPeriod,
    fee: SubscriptionFee,
    quantity: i32,
    /// Lineage root for an override insert (the closed add-on's lineage); `None` for
    /// a genuinely new add-on, which becomes its own root.
    lineage_id: Option<SubscriptionAddOnId>,
    /// Pre-generated id this add-on will be inserted with, for a genuine immediate
    /// add. The add-on analogue of `PendingComponentInsert::subscription_component_id`.
    subscription_add_on_id: Option<SubscriptionAddOnId>,
}

/// Result of resolving an amendment against the current subscription state.
struct ResolvedAmendment {
    preview: AmendmentPreview,
    component_close: Vec<SubscriptionPriceComponentId>,
    component_inserts: Vec<PendingComponentInsert>,
    addon_close: Vec<SubscriptionAddOnId>,
    addon_inserts: Vec<PendingAddOnInsert>,
}

impl Services {
    pub(in crate::services) async fn preview_amendment(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: SubscriptionAmendment,
    ) -> StoreResult<AmendmentPreviewExtended> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let mut conn = self.store.get_conn().await?;

            let sub_details = self
                .store
                .get_subscription_details_with_conn(&mut conn, tenant_id, subscription_id)
                .await?;

            validate_subscription_for_amendment(&sub_details.subscription.status)?;

            let is_immediate = matches!(amendment.apply_mode, PlanChangeMode::Immediate);
            let effective_date = amendment_effective_date(&sub_details, is_immediate);

            let resolved =
                resolve_amendment(&mut conn, &sub_details, &amendment, effective_date).await?;

            let precision = crate::constants::Currencies::resolve_currency_precision(
                &sub_details.subscription.currency,
            )
            .unwrap_or(2);

            let added = resolved.preview.all_added();
            let removed = resolved.preview.all_removed();
            if is_immediate {
                ensure_immediate_amendment_allowed(&sub_details, &added, &removed, precision)?;
            }

            let change_direction =
                crate::services::subscriptions::proration::detect_change_direction(
                    &[],
                    &added,
                    &removed,
                    precision,
                );

            let period_start = sub_details.subscription.current_period_start;
            let period_end = sub_details
                .subscription
                .current_period_end
                .unwrap_or(period_start);

            // Proration (immediate only) — drives both the summary and the adjustment invoice.
            let (proration_result, cadence) = if is_immediate {
                let (result, cadence) = prorate_amendment(
                    &mut conn,
                    &sub_details,
                    &resolved,
                    (&added, &removed),
                    (effective_date, period_end),
                    precision,
                )
                .await?;
                (Some(result), cadence)
            } else {
                (None, None)
            };

            let proration = proration_result.as_ref().map(|result| {
                let (period_start, period_end) = (result.period_start, result.period_end);
                let days_in_period = (period_end - period_start).num_days().max(0) as u32;
                let days_remaining = (period_end - effective_date).num_days().max(0) as u32;

                // Arrears components added by this amendment are NOT billed on the
                // immediate adjustment invoice — they land prorated on the next
                // renewal invoice. Track them separately so charges/net match the
                // adjustment invoice, while the UI can still surface the deferred
                // amount. Genuine adds only (`net_key.is_none()`); overrides keep
                // their existing handling via the credit/charge netting path.
                let arrears_charge_cents = {
                    use crate::domain::enums::BillingType;
                    let full_arrears: i64 = added
                        .iter()
                        .filter(|a| a.net_key.is_none())
                        .map(|a| {
                            match &a.fee {
                        crate::domain::subscription_components::SubscriptionFee::Recurring {
                            rate,
                            quantity,
                            billing_type: BillingType::Arrears,
                        } => {
                            let total = *rate * rust_decimal::Decimal::from(*quantity);
                            total.to_subunit_opt(precision).unwrap_or(0)
                        }
                        _ => 0,
                    }
                        })
                        .sum();
                    crate::services::subscriptions::proration::DayFraction::remaining(
                        period_start,
                        period_end,
                        effective_date,
                    )
                    .prorate(full_arrears)
                };

                ProrationSummary {
                    credits_total_cents: result
                        .lines
                        .iter()
                        .filter(|l| l.is_credit)
                        .map(|l| l.amount_cents)
                        .sum(),
                    charges_total_cents: result
                        .lines
                        .iter()
                        .filter(|l| !l.is_credit)
                        .map(|l| l.amount_cents)
                        .sum::<i64>(),
                    net_amount_cents: result.net_amount_cents,
                    // What would actually be credited (post-netting): an override's credit
                    // is netted into its charge, so an upgrade contributes nothing here.
                    net_credit_cents:
                        crate::services::subscriptions::proration::net_override_lines(&result.lines)
                            .iter()
                            .filter(|l| l.amount_cents < 0)
                            .map(|l| l.amount_cents)
                            .sum(),
                    arrears_charge_cents,
                    proration_factor: result.proration_factor,
                    days_remaining,
                    days_in_period,
                    pack_changes: Vec::new(),
                }
            });

            // MRR before/after: the whole component set before and after the amendment, each
            // floored by the minimum and netted as the apply path states it, the difference
            // applied to the stored figure.
            let minimum = sub_details
                .minimum
                .as_ref()
                .map(|min| (min.amount_subunit, &min.scope));
            let period = sub_details.subscription.period;
            let rate = sub_details.subscription.tax_inclusive_rate;
            let component_lines = calculate_components_mrr_with_slots(
                &mut conn,
                tenant_id,
                subscription_id,
                &sub_details.price_components,
                precision,
            )
            .await?;
            let before_lines = with_add_on_lines(&sub_details, component_lines.clone(), precision);
            let after_lines: Vec<MrrLine> = sub_details
                .price_components
                .iter()
                .zip(component_lines)
                .filter(|(c, _)| !resolved.component_close.contains(&c.id))
                .map(|(_, line)| line)
                .chain(resolved.component_inserts.iter().map(|c| {
                    let product_id = match &c.product_ref {
                        ProductRef::Existing(id) => Some(*id),
                        ProductRef::New { .. } => None,
                    };
                    MrrLine::new(
                        &c.fee,
                        calculate_mrr(&c.fee, &c.period, precision),
                        product_id,
                    )
                    .with_period_total(fee_period_total_cents(&c.fee, precision))
                }))
                .chain(
                    sub_details
                        .add_ons
                        .iter()
                        .filter(|a| !resolved.addon_close.contains(&a.id))
                        .map(|a| {
                            let quantity = i64::from(a.quantity);
                            MrrLine::new(
                                &a.fee,
                                calculate_mrr(&a.fee, &a.period, precision) * quantity,
                                a.product_id,
                            )
                            .with_period_total(fee_period_total_cents(&a.fee, precision) * quantity)
                        }),
                )
                .chain(resolved.addon_inserts.iter().map(|a| {
                    let quantity = i64::from(a.quantity);
                    MrrLine::new(
                        &a.fee,
                        calculate_mrr(&a.fee, &a.period, precision) * quantity,
                        Some(a.product_id),
                    )
                    .with_period_total(fee_period_total_cents(&a.fee, precision) * quantity)
                }))
                .collect();
            // The rate apply will store: re-resolved on the lines the amendment leaves.
            let rate_after = self
                .tax_inclusive_rate_for(&mut conn, &sub_details, &after_lines, effective_date)
                .await?;
            let mrr_before_cents = sub_details.subscription.mrr_cents as i64;
            let mrr_after_cents = mrr_before_cents
                + subscription_mrr(&after_lines, minimum, period, rate_after)
                - subscription_mrr(&before_lines, minimum, period, rate);

            // Adjustment invoice (immediate, non-trial): the charge side of the netted
            // proration, taxed — the same content the apply path issues.
            let adjustment_invoice = match (proration_result.as_ref(), is_free_trial(&sub_details))
            {
                (Some(result), false) => {
                    let charge_lines: Vec<_> =
                        crate::services::subscriptions::proration::net_override_lines(
                            &result.lines,
                        )
                        .into_iter()
                        .filter(|l| l.amount_cents > 0)
                        .collect();
                    if charge_lines.is_empty() {
                        None
                    } else {
                        let charge_proration =
                            crate::domain::subscription_changes::ProrationResult {
                                net_amount_cents: charge_lines.iter().map(|l| l.amount_cents).sum(),
                                lines: charge_lines,
                                change_date: effective_date,
                                period_start: result.period_start,
                                period_end: result.period_end,
                                proration_factor: result.proration_factor,
                            };
                        Some(
                            self.compute_adjustment_invoice_content(
                                &mut conn,
                                tenant_id,
                                &sub_details.subscription,
                                &sub_details.customer,
                                &charge_proration,
                            )
                            .await?
                            .computed,
                        )
                    }
                }
                _ => None,
            };

            // Credit note (immediate, non-trial): what reconcile takes back of the period's
            // lines, as the apply path issues it.
            let credit_note = match (proration_result.as_ref(), is_free_trial(&sub_details)) {
                (Some(_), false) => {
                    let grid = amendment_grid(
                        &sub_details.subscription,
                        cadence.as_ref(),
                        period_end,
                        effective_date,
                    );
                    let change = crate::services::subscriptions::fee_terms::AmendmentChange {
                        details: &sub_details,
                        added: &added,
                        removed: &removed,
                        change: effective_date,
                        own_period: &grid,
                        precision,
                    };
                    self.compute_amendment_credit_note_preview(
                        &mut conn,
                        tenant_id,
                        (period_start, period_end),
                        &change,
                        precision,
                    )
                    .await?
                }
                _ => None,
            };

            // Next renewal invoice under the amended subscription: simulate the
            // post-amendment component/add-on set in memory and compute the upcoming
            // invoice. Best-effort — a preview should never fail the whole request.
            let (next_invoice, subscription_after) = {
                let mut hypothetical = sub_details.clone();
                let closed_components: std::collections::HashSet<_> =
                    resolved.component_close.iter().copied().collect();
                hypothetical
                    .price_components
                    .retain(|c| !closed_components.contains(&c.id));
                for insert in &resolved.component_inserts {
                    hypothetical
                        .price_components
                        .push(pending_component_to_subscription(
                            insert,
                            subscription_id,
                            effective_date,
                        ));
                }
                let closed_addons: std::collections::HashSet<_> =
                    resolved.addon_close.iter().copied().collect();
                hypothetical
                    .add_ons
                    .retain(|a| !closed_addons.contains(&a.id));
                for insert in &resolved.addon_inserts {
                    hypothetical.add_ons.push(pending_addon_to_subscription(
                        insert,
                        subscription_id,
                        effective_date,
                    ));
                }
                if let Some(cadence) = &cadence {
                    let sub = &mut hypothetical.subscription;
                    sub.period = cadence.period;
                    sub.current_period_start = cadence.current.start;
                    sub.current_period_end = Some(
                        sub.end_date
                            .map_or(cadence.current.end, |end| end.min(cadence.current.end)),
                    );
                    sub.cycle_index = Some(cadence.cycle_index);
                }
                let invoice = self
                    .compute_upcoming_invoice(&mut conn, &hypothetical)
                    .await
                    .ok();
                (invoice, hypothetical.subscription)
            };

            let adjustment_period =
                adjustment_invoice
                    .as_ref()
                    .map(|invoice| crate::domain::Period {
                        start: effective_date,
                        end: invoice
                            .invoice_lines
                            .iter()
                            .map(|line| line.end_date)
                            .max()
                            .unwrap_or(effective_date),
                    });

            Ok(AmendmentPreviewExtended {
                adjustment_period,
                subscription_after,
                preview: resolved.preview,
                proration,
                change_direction,
                mrr_before_cents,
                mrr_after_cents,
                adjustment_invoice,
                credit_note,
                next_invoice,
            })
        })
        .await
    }

    pub(in crate::services) async fn apply_amendment_immediate(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: SubscriptionAmendment,
    ) -> StoreResult<ImmediateAmendmentResult> {
        let today = common_domain::clock::now().naive_utc().date();
        self.apply_amendment_immediate_at(actor, subscription_id, tenant_id, amendment, today)
            .await
    }

    pub(crate) async fn apply_amendment_immediate_at(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: SubscriptionAmendment,
        change_date: NaiveDate,
    ) -> StoreResult<ImmediateAmendmentResult> {
        let amendment = &amendment;
        let mut conn = self.store.get_conn().await?;
        self.transaction_reading_usage_first(&mut conn, |services, conn| {
            let actor = &actor;
            async move {
                let lock =
                    SubscriptionRow::lock_subscription_for_update(conn, subscription_id).await?;
                credit_inputs::bump_customer(conn, &lock).await?;
                crate::services::subscriptions::plan_change::refuse_while_a_paid_change_is_pending(
                    conn,
                    tenant_id,
                    subscription_id,
                    None,
                )
                .await?;

                let sub_details = self
                    .store
                    .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                    .await?;

                validate_subscription_for_amendment(&sub_details.subscription.status)?;
                reject_if_plan_change_pending(conn, subscription_id, tenant_id).await?;

                let period_start = sub_details.subscription.current_period_start;
                let period_end = sub_details.subscription.current_period_end.ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(
                        "Subscription has no current_period_end".to_string(),
                    ))
                })?;
                if change_date < period_start || change_date > period_end {
                    return Err(Report::new(StoreError::InvalidArgument(format!(
                        "Change date {} is outside current period [{}, {}]",
                        change_date, period_start, period_end
                    ))));
                }

                let resolved =
                    resolve_amendment(conn, &sub_details, amendment, change_date).await?;

                let precision = crate::constants::Currencies::resolve_currency_precision(
                    &sub_details.subscription.currency,
                )
                .unwrap_or(2);

                let added = resolved.preview.all_added();
                let removed = resolved.preview.all_removed();
                ensure_immediate_amendment_allowed(&sub_details, &added, &removed, precision)?;

                let (proration, cadence) = prorate_amendment(
                    conn,
                    &sub_details,
                    &resolved,
                    (&added, &removed),
                    (change_date, period_end),
                    precision,
                )
                .await?;

                let is_free_trial = is_free_trial(&sub_details);
                let grid = amendment_grid(
                    &sub_details.subscription,
                    cadence.as_ref(),
                    period_end,
                    change_date,
                );
                let change = crate::services::subscriptions::fee_terms::AmendmentChange {
                    details: &sub_details,
                    added: &added,
                    removed: &removed,
                    change: change_date,
                    own_period: &grid,
                    precision,
                };

                // The charges of the netted proration go on an adjustment invoice; what
                // reconcile takes back of the period's lines on credit notes against them.
                let (adjustment_invoice_id, credit_note_ids) = services
                    .settle_immediate_proration(
                        conn,
                        actor,
                        tenant_id,
                        &sub_details.customer,
                        (period_start, period_end),
                        &proration,
                        &change,
                        is_free_trial,
                        "Subscription amendment",
                    )
                    .await?;

                let (component_inserts, addon_inserts) = materialize_inserts(
                    conn,
                    tenant_id,
                    &sub_details,
                    resolved.component_inserts,
                    resolved.addon_inserts,
                )
                .await?;

                services
                    .execute_amendment_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                        change_date,
                        &resolved.component_close,
                        &component_inserts,
                        &resolved.addon_close,
                        &addon_inserts,
                        sub_details.subscription.mrr_cents as i64,
                        precision,
                    )
                    .await?;

                let activity = Activity::new(
                    ActivityType::SubscriptionAmended,
                    EntityType::Subscription,
                    subscription_id.as_uuid(),
                )
                .with_metadata(amendment_metadata(
                    &resolved.component_close,
                    &component_inserts,
                    &resolved.addon_close,
                    &addon_inserts,
                    change_date,
                    adjustment_invoice_id,
                ))
                .agg_customer(sub_details.customer.id);
                services
                    .store
                    .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;

                Ok(ImmediateAmendmentResult {
                    adjustment_invoice_id,
                    credit_note_ids,
                    effective_date: change_date,
                })
            }
            .scope_boxed()
        })
        .await
    }

    /// Settle the financial side of an immediate, mid-period amendment in the EU-correct way.
    ///
    /// The positive lines of the netted proration are *charges* that go onto a finalized
    /// adjustment invoice; what reconcile takes back of the period's lines (read before that
    /// invoice bills the change) becomes real credit notes against the invoice(s) that billed
    /// them (`CreditToBalance` when paid or a payment is in flight, otherwise
    /// `DebtCancellation`). This avoids mixing a gross charge and an untaxed credit on a single
    /// document. Free trials settle nothing (no money has changed hands yet). Returns the
    /// adjustment invoice id (if any charges) and the issued credit note ids (if any credits).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn settle_immediate_proration(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        customer: &crate::domain::Customer,
        (period_start, period_end): (NaiveDate, NaiveDate),
        proration: &crate::domain::subscription_changes::ProrationResult,
        change: &crate::services::subscriptions::fee_terms::AmendmentChange<'_>,
        is_free_trial: bool,
        reason: &str,
    ) -> StoreResult<(Option<InvoiceId>, Vec<CreditNoteId>)> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            if is_free_trial {
                return Ok((None, vec![]));
            }
            let subscription = &change.details.subscription;
            let ctx = gather_period_credit_context(
                conn,
                tenant_id,
                subscription.id,
                period_start,
                period_end,
            )
            .await?;
            let credits = change.credits(conn, &ctx).await?;

            let charge_lines: Vec<_> =
                crate::services::subscriptions::proration::net_override_lines(&proration.lines)
                    .into_iter()
                    .filter(|l| l.amount_cents > 0)
                    .collect();
            let adjustment_invoice_id = if !charge_lines.is_empty() {
                let charge_proration = crate::domain::subscription_changes::ProrationResult {
                    net_amount_cents: charge_lines.iter().map(|l| l.amount_cents).sum(),
                    lines: charge_lines,
                    change_date: proration.change_date,
                    period_start,
                    period_end,
                    proration_factor: proration.proration_factor,
                };
                let invoice = self
                    .create_adjustment_invoice(
                        conn,
                        tenant_id,
                        subscription,
                        customer,
                        &charge_proration,
                    )
                    .await?;
                if let Some(inv) = &invoice {
                    self.finalize_invoice_tx(conn, actor, inv.id, tenant_id, false, &None)
                        .await?;
                }
                invoice.map(|inv| inv.id)
            } else {
                None
            };

            let credit_note_ids = self
                .issue_period_credit_notes_tx(
                    conn,
                    actor,
                    tenant_id,
                    &ctx,
                    credits,
                    reason,
                    false,
                    Some(
                        proration
                            .change_date
                            .and_time(chrono::NaiveTime::MIN)
                            .and_utc(),
                    ),
                    crate::domain::enums::CreditNoteOrigin::Amendment,
                )
                .await?
                .into_iter()
                .map(|note| note.id)
                .collect();

            Ok((adjustment_invoice_id, credit_note_ids))
        })
        .await
    }

    /// Issues and finalizes one credit note per period invoice that billed `items`, oldest first:
    /// `CreditToBalance` where money was collected or is in flight, or nothing is left to collect
    /// (credit paid it), else `DebtCancellation`. Credit the notes return to the balance is dated
    /// `credit_at` (the change's business date), never later than now.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn issue_period_credit_notes_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        ctx: &PeriodCreditContext,
        items: Vec<CreditLineItem>,
        reason: &str,
        // The caller removes the credit the credited purchase lines funded.
        purchased_credit_removed: bool,
        credit_at: Option<chrono::DateTime<chrono::Utc>>,
        origin: crate::domain::enums::CreditNoteOrigin,
    ) -> StoreResult<Vec<crate::domain::credit_notes::CreditNote>> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            if items.is_empty() {
                return Ok(vec![]);
            }

            // Group the matched credit lines by the source invoice that billed them, so
            // each credit note reverses lines from a single invoice.
            let mut by_invoice: HashMap<InvoiceId, Vec<CreditLineItem>> = HashMap::new();
            for it in items {
                if let Some(invoice_id) = ctx.line_to_invoice.get(it.local_id()) {
                    by_invoice.entry(*invoice_id).or_default().push(it);
                }
            }

            let invoice_by_id: HashMap<InvoiceId, &Invoice> =
                ctx.invoices.iter().map(|i| (i.id, i)).collect();

            let mut created = Vec::new();
            // Deterministic order: oldest billing invoice first.
            let mut invoice_ids: Vec<InvoiceId> = by_invoice.keys().copied().collect();
            invoice_ids.sort_by_key(|id| {
                // `Option<NaiveDate>` orders None first; the id string is a stable
                // tiebreaker. (`get` is always `Some` here — line ids come from these
                // invoices — but stay total-order safe regardless.)
                (
                    invoice_by_id.get(id).map(|i| i.billing_period_start),
                    id.to_string(),
                )
            });

            for invoice_id in invoice_ids {
                let line_items = by_invoice.remove(&invoice_id).unwrap_or_default();
                if line_items.is_empty() {
                    continue;
                }
                let invoice = invoice_by_id[&invoice_id];
                // Money already collected OR in flight (accepted debit, pending charge)
                // lands in full, so credit the balance; only a genuinely uncollected
                // invoice can have its debt cancelled. (If the in-flight debit later
                // fails the invoice reopens for its full total, so the customer still
                // owes it and the credit nets against it — no money is lost either way.)
                // Nothing left to collect because credit paid it is paid too, whether or not the
                // invoice was marked paid yet.
                let collected_or_in_flight = matches!(
                    invoice.payment_status,
                    InvoicePaymentStatus::Paid | InvoicePaymentStatus::PartiallyPaid
                ) || invoice.amount_due <= 0
                    || crate::repositories::credit_notes::invoice_payment_in_flight(
                        conn, tenant_id, invoice,
                    )
                    .await?;
                let credit_type = if collected_or_in_flight {
                    CreditType::CreditToBalance
                } else {
                    CreditType::DebtCancellation
                };

                let credit_note = create_user_credit_note_tx(
                    &self.store,
                    conn,
                    tenant_id,
                    actor,
                    CreateCreditNoteParams {
                        invoice_id,
                        line_items,
                        reason: Some(reason.to_string()),
                        memo: None,
                        credit_type,
                        custom_properties: None,
                    },
                    // Amendment credits come from proration; `negate_line_items` flags each
                    // line prorated only where proration actually reduced it (factor < 1).
                    true,
                    purchased_credit_removed,
                    origin,
                )
                .await?;

                let finalized = crate::repositories::credit_notes::finalize_credit_note_at_tx(
                    &self.store,
                    conn,
                    tenant_id,
                    actor,
                    credit_note.id,
                    purchased_credit_removed,
                    credit_at,
                )
                .await?;
                created.push(finalized);
            }

            Ok(created)
        })
        .await
    }

    /// Read-only preview of the credit note(s) that `settle_immediate_proration`
    /// would issue: what reconcile takes back of each originally-billed line (across the
    /// recurring invoice and any in-period adjustment invoices), with its VAT
    /// reversed proportionally. Returns the negative-amount lines (a credit) as a
    /// single combined `ComputedInvoiceContent` so it renders with the same card as
    /// the adjustment invoice. `None` when there is nothing to credit.
    async fn compute_amendment_credit_note_preview(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        (period_start, period_end): (NaiveDate, NaiveDate),
        change: &crate::services::subscriptions::fee_terms::AmendmentChange<'_>,
        precision: u8,
    ) -> StoreResult<Option<ComputedInvoiceContent>> {
        let ctx = gather_period_credit_context(
            conn,
            tenant_id,
            change.details.subscription.id,
            period_start,
            period_end,
        )
        .await?;
        if ctx.invoices.is_empty() {
            return Ok(None);
        }

        // Which original lines get credited, and for what: the same reconcile the change runs.
        let credits = change.credits(conn, &ctx).await?;
        if credits.is_empty() {
            return Ok(None);
        }

        let line_by_id: HashMap<&str, &LineItem> = ctx
            .all_lines
            .iter()
            .map(|l| (l.local_id.as_str(), l))
            .collect();

        // Each credit as the issued note resolves it, so the preview nets the discount and
        // promotional credit the line took, and the tax it bore, exactly as the note does.
        let mut credited = Vec::new();
        for ci in &credits {
            let Some(orig) = line_by_id.get(ci.local_id()).copied() else {
                continue;
            };
            let already = ctx
                .already_credited
                .get(&orig.local_id)
                .copied()
                .unwrap_or(0);
            let remaining = (orig.amount_subtotal - already).max(0);
            // Resolved as the issued note resolves it, refusing what it would refuse.
            let amount = match ci {
                CreditLineItem::Line { quantity, .. } => {
                    if let Some(credit) = &orig.credit_unit {
                        return Err(Report::new(StoreError::InvalidArgument(format!(
                            "Line item '{}' is priced in {}: credit its money overage by amount; credits come back by voiding the invoice or a manual grant",
                            orig.local_id, credit.code
                        ))));
                    }
                    let amount = (orig.unit_price.unwrap_or(Decimal::ZERO) * *quantity)
                        .to_subunit_opt(precision)
                        .unwrap_or(0);
                    if amount > remaining {
                        return Err(Report::new(StoreError::InvalidArgument(format!(
                            "Credit amount {} for line item '{}' exceeds remaining {}",
                            amount, orig.local_id, remaining
                        ))));
                    }
                    amount
                }
                CreditLineItem::Amount { amount_cents, .. } => (*amount_cents).min(remaining),
                CreditLineItem::SubLines { .. } => continue,
            };
            if amount > 0 {
                credited.push((orig.local_id.clone(), amount));
            }
        }
        let lines = crate::repositories::credit_notes::credited_lines(
            &ctx.all_lines,
            &credited,
            &ctx.already_credited,
            &ctx.returned,
            precision,
        );
        if lines.is_empty() {
            return Ok(None);
        }

        let promotional_credits = crate::services::credit::promotional::net_total(&lines)?;
        let discount = lines
            .iter()
            .map(|l| l.amount_subtotal - l.taxable_amount)
            .sum::<i64>()
            - promotional_credits;
        let totals = crate::services::invoice_lines::invoice_lines::totals_of(&lines, discount);
        // The note states the tax position of the invoices it corrects, in their words.
        let mut tax_breakdown = crate::repositories::invoices::compute_tax_breakdown(&lines);
        crate::repositories::invoices::carry_over_tax_findings(
            &ctx.invoices
                .iter()
                .flat_map(|invoice| invoice.tax_breakdown.iter().cloned())
                .collect::<Vec<_>>(),
            &mut tax_breakdown,
        );
        Ok(Some(ComputedInvoiceContent {
            amount_due: 0,
            ..ComputedInvoiceContent::of(lines, totals, vec![], tax_breakdown, 0, 0)
        }))
    }

    pub(in crate::services) async fn schedule_amendment(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: SubscriptionAmendment,
    ) -> StoreResult<ScheduledEvent> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id).await?;
                    credit_inputs::bump_customer(conn, &lock).await?;
                    crate::services::subscriptions::plan_change::refuse_while_a_paid_change_is_pending(
                        conn,
                        tenant_id,
                        subscription_id,
                        None,
                    )
                    .await?;

                    let sub_details = self
                        .store
                        .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                        .await?;

                    validate_subscription_for_amendment(&sub_details.subscription.status)?;
                    reject_if_plan_change_pending(conn, subscription_id, tenant_id).await?;

                    let effective_date =
                        sub_details.subscription.current_period_end.ok_or_else(|| {
                            Report::new(StoreError::InvalidArgument(
                                "Subscription has no current_period_end".to_string(),
                            ))
                        })?;

                    let resolved =
                        resolve_amendment(conn, &sub_details, &amendment, effective_date).await?;

                    // Cancel any previously-scheduled amendment (but NOT plan changes).
                    cancel_pending_amendments(conn, subscription_id, tenant_id).await?;

                    let (component_inserts, addon_inserts) = materialize_inserts(
                        conn,
                        tenant_id,
                        &sub_details,
                        resolved.component_inserts,
                        resolved.addon_inserts,
                    )
                    .await?;

                    let metadata = amendment_metadata(
                        &resolved.component_close,
                        &component_inserts,
                        &resolved.addon_close,
                        &addon_inserts,
                        effective_date,
                        None,
                    );

                    let events = self
                        .store
                        .schedule_events(
                            conn,
                            vec![ScheduledEventNew {
                                subscription_id,
                                tenant_id,
                                scheduled_time: effective_date.and_time(NaiveTime::MIN),
                                event_data: ScheduledEventData::ApplyAmendment {
                                    component_close: resolved.component_close,
                                    component_insert: component_inserts,
                                    addon_close: resolved.addon_close,
                                    addon_insert: addon_inserts,
                                },
                                source: "api".to_string(),
                                created_by: actor.clone(),
                            }],
                        )
                        .await?;

                    let event = events
                        .into_iter()
                        .next()
                        .ok_or_else(|| Report::new(StoreError::InsertError))?;

                    let activity = Activity::new(
                        ActivityType::SubscriptionAmendmentScheduled,
                        EntityType::Subscription,
                        subscription_id.as_uuid(),
                    )
                    .with_metadata(metadata)
                    .agg_customer(sub_details.customer.id);
                    self.store
                        .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                        .await?;

                    Ok(event)
                }
                .scope_boxed()
            })
            .await
    }

    pub(in crate::services) async fn cancel_amendment(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id)
                        .await?;
                    credit_inputs::bump_customer(conn, &lock).await?;
                    let cancelled =
                        cancel_pending_amendments(conn, subscription_id, tenant_id).await?;
                    if cancelled == 0 {
                        return Err(Report::new(StoreError::ValueNotFound(
                            "No pending amendment found for this subscription".to_string(),
                        )));
                    }

                    let customer_id =
                        SubscriptionRow::get_customer_id(conn, &tenant_id, subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    let activity = Activity::new(
                        ActivityType::SubscriptionAmendmentCancelled,
                        EntityType::Subscription,
                        subscription_id.as_uuid(),
                    )
                    .agg_customer(customer_id);
                    self.store
                        .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                        .await?;

                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_amendment_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        change_date: NaiveDate,
        component_close: &[SubscriptionPriceComponentId],
        component_insert: &[ResolvedComponentInsert],
        addon_close: &[SubscriptionAddOnId],
        addon_insert: &[ResolvedAddOnInsert],
        old_mrr: i64,
        precision: u8,
    ) -> StoreResult<()> {
        SubscriptionComponentRow::close_components(conn, component_close, change_date)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        SubscriptionAddOnRow::close_addons(conn, addon_close, change_date)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let component_rows: Vec<SubscriptionComponentRowNew> = component_insert
            .iter()
            .map(|c| {
                let mut row: SubscriptionComponentRowNew = SubscriptionComponentNew {
                    subscription_id,
                    internal: SubscriptionComponentNewInternal {
                        price_component_id: c.price_component_id,
                        product_id: c.product_id,
                        name: c.name.clone(),
                        period: c.period,
                        fee: c.fee.clone(),
                        is_override: c.is_override,
                        price_id: c.price_id,
                        effective_from: change_date,
                    },
                }
                .try_into()?;
                // Inherit the overridden component's lineage so amendment credits stay
                // matched to the originally-billed invoice line; `None` leaves the new
                // row as its own root.
                row.lineage_id = c.lineage_id;
                // Use the pre-generated id (immediate adds and overrides) so it matches the
                // id already stamped onto the adjustment invoice's charge line.
                if let Some(id) = c.subscription_component_id {
                    row.id = id;
                }
                // Mark as amendment-added so a one-time fee bills on its effective period.
                row.added_by_amendment = true;
                Ok::<_, Report<StoreError>>(row)
            })
            .collect::<Result<Vec<_>, Report<StoreError>>>()?;

        if !component_rows.is_empty() {
            let refs: Vec<&SubscriptionComponentRowNew> = component_rows.iter().collect();
            SubscriptionComponentRow::insert_subscription_component_batch(conn, refs)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        }

        let addon_rows: Vec<SubscriptionAddOnRowNew> = addon_insert
            .iter()
            .map(|a| {
                let mut row: SubscriptionAddOnRowNew = SubscriptionAddOnNew {
                    subscription_id,
                    internal: crate::domain::subscription_add_ons::SubscriptionAddOnNewInternal {
                        add_on_id: a.add_on_id,
                        name: a.name.clone(),
                        period: a.period,
                        fee: a.fee.clone(),
                        product_id: a.product_id,
                        price_id: a.price_id,
                        quantity: a.quantity,
                        effective_from: change_date,
                    },
                }
                .try_into()?;
                row.lineage_id = a.lineage_id;
                if let Some(id) = a.subscription_add_on_id {
                    row.id = id;
                }
                row.added_by_amendment = true;
                Ok::<_, Report<StoreError>>(row)
            })
            .collect::<Result<Vec<_>, Report<StoreError>>>()?;

        if !addon_rows.is_empty() {
            let refs: Vec<&SubscriptionAddOnRowNew> = addon_rows.iter().collect();
            SubscriptionAddOnRow::insert_batch(conn, refs)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        }

        crate::services::subscriptions::cadence::realign_cadence_tx(
            conn,
            tenant_id,
            subscription_id,
            change_date,
        )
        .await?;

        // Overrides and edited add-ons keep their unit's ledger: a seed would reset the live count.
        for c in component_insert.iter().filter(|c| !c.is_override) {
            if let Some(tx) = crate::domain::slot_transactions::SlotTransactionNewInternal::from_fee(
                &c.fee,
                change_date,
            ) {
                tx.into_row(subscription_id)
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
            }
        }
        for a in addon_insert.iter().filter(|a| a.lineage_id.is_none()) {
            if let Some(tx) = crate::domain::slot_transactions::SlotTransactionNewInternal::from_fee(
                &a.fee,
                change_date,
            ) {
                tx.into_row(subscription_id)
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
            }
        }

        let sub_event = diesel_models::subscription_events::SubscriptionEventRow {
            id: uuid::Uuid::now_v7(),
            subscription_id,
            event_type: diesel_models::enums::SubscriptionEventType::Switch,
            details: Some(serde_json::json!({ "kind": "amendment", "mode": "immediate" })),
            created_at: common_domain::clock::now().naive_utc(),
            mrr_delta: None,
            bi_mrr_movement_log_id: None,
            applies_to: change_date,
        };
        sub_event
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Recalculate MRR from the resulting active set.
        let mut sub_details = self
            .store
            .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
            .await?;

        let component_lines = calculate_components_mrr_with_slots(
            conn,
            tenant_id,
            subscription_id,
            &sub_details.price_components,
            precision,
        )
        .await?;

        // Stated as stored, so the delta against `old_mrr` is like for like.
        let new_mrr = self
            .restate_subscription_mrr(
                conn,
                &mut sub_details,
                component_lines,
                precision,
                change_date,
            )
            .await?;
        let mrr_delta = new_mrr - old_mrr;
        if mrr_delta != 0 {
            SubscriptionRow::update_subscription_mrr_delta(conn, subscription_id, mrr_delta)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            self.create_movement_mrr_log(conn, tenant_id, subscription_id, change_date, mrr_delta)
                .await?;
        }

        log::info!(
            "Applied immediate amendment for subscription {}: closed_components={}, added_components={}, closed_addons={}, added_addons={}, mrr_delta={}",
            subscription_id,
            component_close.len(),
            component_insert.len(),
            addon_close.len(),
            addon_insert.len(),
            mrr_delta,
        );

        self.emit_subscription_updated_tx(
            conn,
            tenant_id,
            subscription_id,
            crate::domain::outbox_event::SubscriptionUpdateType::Amended,
        )
        .await?;

        Ok(())
    }
}

/// The cadences of the fees an amendment leaves the subscription billing.
fn cadences_after(
    sub_details: &SubscriptionDetails,
    resolved: &ResolvedAmendment,
) -> Vec<SubscriptionFeeBillingPeriod> {
    sub_details
        .price_components
        .iter()
        .filter(|c| !resolved.component_close.contains(&c.id))
        .map(|c| c.period)
        .chain(resolved.component_inserts.iter().map(|c| c.period))
        .chain(
            sub_details
                .add_ons
                .iter()
                .filter(|a| !resolved.addon_close.contains(&a.id))
                .map(|a| a.period),
        )
        .chain(resolved.addon_inserts.iter().map(|a| a.period))
        .collect()
}

/// An immediate amendment on `change_date` prorates each fee over what is left of its own period,
/// on the grid of the cadence the amendment leaves the subscription renewing on.
async fn prorate_amendment(
    conn: &mut PgConn,
    sub_details: &SubscriptionDetails,
    resolved: &ResolvedAmendment,
    (added, removed): (&[AddedComponent], &[RemovedComponent]),
    (change_date, period_end): (NaiveDate, NaiveDate),
    precision: u8,
) -> StoreResult<(
    crate::domain::subscription_changes::ProrationResult,
    Option<crate::services::subscriptions::cadence::Cadence>,
)> {
    use crate::services::subscriptions::proration::{calculate_proration_by, stub_on_own_grid};
    let cadence = crate::services::subscriptions::cadence::cadence_after_change(
        conn,
        sub_details,
        &cadences_after(sub_details, resolved),
        change_date,
    )
    .await?;
    let sub = &sub_details.subscription;
    let (current, renews_on) = amendment_period(sub, cadence.as_ref(), period_end);
    let proration = calculate_proration_by(
        &[],
        added,
        removed,
        current,
        change_date,
        precision,
        stub_on_own_grid(
            current,
            renews_on,
            sub.billing_start_date.unwrap_or(sub.current_period_start),
            u32::from(sub.billing_day_anchor),
            change_date,
        ),
    );
    Ok((proration, cadence))
}

/// The period an immediate amendment prorates over, and the cadence it renews on.
fn amendment_period(
    sub: &crate::domain::Subscription,
    cadence: Option<&crate::services::subscriptions::cadence::Cadence>,
    period_end: NaiveDate,
) -> (
    (NaiveDate, NaiveDate),
    crate::domain::enums::BillingPeriodEnum,
) {
    match cadence {
        Some(cadence) => ((cadence.current.start, cadence.current.end), cadence.period),
        None => ((sub.current_period_start, period_end), sub.period),
    }
}

/// The period each fee of an immediate amendment bills on, as its proration reads it.
fn amendment_grid(
    sub: &crate::domain::Subscription,
    cadence: Option<&crate::services::subscriptions::cadence::Cadence>,
    period_end: NaiveDate,
    change_date: NaiveDate,
) -> impl Fn(&SubscriptionFeeBillingPeriod) -> (NaiveDate, NaiveDate) + Sync + use<> {
    let (current, renews_on) = amendment_period(sub, cadence, period_end);
    crate::services::subscriptions::proration::own_period(
        current,
        renews_on,
        sub.billing_start_date.unwrap_or(sub.current_period_start),
        u32::from(sub.billing_day_anchor),
        change_date,
    )
}

/// Audit-trail metadata describing the shape of an amendment.
fn amendment_metadata(
    component_close: &[SubscriptionPriceComponentId],
    component_insert: &[ResolvedComponentInsert],
    addon_close: &[SubscriptionAddOnId],
    addon_insert: &[ResolvedAddOnInsert],
    effective_date: NaiveDate,
    adjustment_invoice_id: Option<InvoiceId>,
) -> serde_json::Value {
    serde_json::json!({
        "effective_date": effective_date.to_string(),
        "components_closed": component_close.len(),
        "components_added": component_insert.len(),
        "addons_closed": addon_close.len(),
        "addons_added": addon_insert.len(),
        "adjustment_invoice_id": adjustment_invoice_id.map(|id| id.to_string()),
    })
}

pub(crate) const MINIMUM_COMMITMENT_AMENDMENT_AT_PERIOD_END: &str = "This subscription has a minimum commitment; changes to it take effect at the end of the billing period.";

/// Under a minimum, an increase applies at once and counts toward the period's true-up. Anything
/// removed or decreased waits for the period's end: every removal must be the old side of an
/// override (same `net_key`) that the override raises, as `detect_change_direction` reads it.
pub(crate) fn ensure_immediate_amendment_allowed(
    sub_details: &SubscriptionDetails,
    added: &[AddedComponent],
    removed: &[RemovedComponent],
    precision: u8,
) -> StoreResult<()> {
    use crate::domain::subscription_changes::ChangeDirection;
    use crate::services::subscriptions::proration::detect_change_direction;
    if sub_details.minimum.is_none() {
        return Ok(());
    }
    let raised = |old: &RemovedComponent| {
        old.net_key.as_ref().is_some_and(|key| {
            let new: Vec<AddedComponent> = added
                .iter()
                .filter(|a| a.net_key.as_ref() == Some(key))
                .cloned()
                .collect();
            !new.is_empty()
                && detect_change_direction(&[], &new, std::slice::from_ref(old), precision)
                    == ChangeDirection::Upgrade
        })
    };
    let increase = !added.is_empty()
        && removed.iter().all(raised)
        && detect_change_direction(&[], added, removed, precision) != ChangeDirection::Downgrade;
    if !increase {
        return Err(Report::new(StoreError::FailedPrecondition(
            MINIMUM_COMMITMENT_AMENDMENT_AT_PERIOD_END.to_owned(),
        )));
    }
    Ok(())
}

fn validate_subscription_for_amendment(status: &SubscriptionStatusEnum) -> StoreResult<()> {
    match status {
        SubscriptionStatusEnum::Active | SubscriptionStatusEnum::TrialActive => Ok(()),
        _ => Err(Report::new(StoreError::InvalidArgument(format!(
            "Cannot amend subscription in {:?} status",
            status
        )))),
    }
}

fn is_free_trial(sub_details: &SubscriptionDetails) -> bool {
    sub_details.subscription.status == SubscriptionStatusEnum::TrialActive
        && sub_details.trial_config.as_ref().is_some_and(|t| t.is_free)
}

fn amendment_effective_date(sub_details: &SubscriptionDetails, is_immediate: bool) -> NaiveDate {
    if is_immediate {
        common_domain::clock::now().naive_utc().date()
    } else {
        sub_details
            .subscription
            .current_period_end
            .unwrap_or(sub_details.subscription.current_period_start)
    }
}

async fn reject_if_plan_change_pending(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    tenant_id: TenantId,
) -> StoreResult<()> {
    let pending =
        ScheduledEventRow::get_pending_events_for_subscription(conn, subscription_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    if pending
        .iter()
        .any(|e| e.event_type == diesel_models::enums::ScheduledEventTypeEnum::ApplyPlanChange)
    {
        return Err(Report::new(StoreError::InvalidArgument(
            "A plan change is already scheduled for this subscription; cancel it before amending"
                .to_string(),
        )));
    }
    Ok(())
}

/// Cancel pending ApplyAmendment events only. Returns the number cancelled.
async fn cancel_pending_amendments(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    tenant_id: TenantId,
) -> StoreResult<usize> {
    let pending =
        ScheduledEventRow::get_pending_events_for_subscription(conn, subscription_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

    let mut count = 0;
    for event in pending
        .iter()
        .filter(|e| e.event_type == diesel_models::enums::ScheduledEventTypeEnum::ApplyAmendment)
    {
        if ScheduledEventRow::cancel_event(conn, &event.id, "Cancelled by user")
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        {
            count += 1;
        }
    }
    Ok(count)
}

/// Patch a Slot fee's `initial_slots` with the live count from the slot_transaction ledger.
async fn resolve_live_slot_count(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    fee: &mut SubscriptionFee,
) -> StoreResult<()> {
    if let SubscriptionFee::Slot {
        unit,
        initial_slots,
        ..
    } = fee
    {
        let actual = SlotTransactionRow::fetch_active_slots(
            conn,
            tenant_id,
            subscription_id,
            unit.clone(),
            common_domain::clock::now().naive_utc(),
        )
        .await
        .map(|slots| slots.unwrap_or(0) as u32)
        .unwrap_or(*initial_slots);
        *initial_slots = actual;
    }
    Ok(())
}

/// `lines` as billing-core reads them: each with its lineage, quantity, what notes already took
/// off it, and what an earlier ending credited from it (`settled`).
pub(in crate::services) fn issued_lines<'a>(
    lines: impl IntoIterator<Item = &'a LineItem>,
    line_lineage: &HashMap<String, String>,
    already_credited: &HashMap<String, i64>,
    corrected: &HashMap<String, i64>,
    up_front: impl Fn(&LineItem) -> bool,
    settled: impl Fn(&str) -> Option<i64>,
) -> Vec<billing_core::IssuedLine> {
    use billing_core::{IssuedKind, IssuedLine};
    let lines: Vec<&LineItem> = lines.into_iter().collect();
    let lineages = crate::services::invoice_lines::charges::Lineages::of_lines(
        lines.iter().copied(),
        line_lineage,
    );
    lines
        .into_iter()
        .map(|line| IssuedLine {
            id: line.local_id.clone(),
            key: crate::services::invoice_lines::charges::charge_key(line, &lineages),
            kind: match (&line.kind, line.metric_id) {
                (LineItemKind::Regular, None) => IssuedKind::Fee {
                    price_component: line.price_component_id.map(|id| id.as_uuid()),
                    add_on: line.sub_add_on_id.is_some(),
                },
                (LineItemKind::MinimumShortfall, _) => IssuedKind::Shortfall {
                    up_front: up_front(line),
                },
                _ => IssuedKind::Other,
            },
            lineage: line_lineage.get(&line.local_id).cloned(),
            quantity: line.quantity,
            unit_priced: line.unit_price.is_some(),
            from: line.start_date,
            until: line.end_date,
            subtotal: line.amount_subtotal,
            quoted: line
                .tax_inclusive_quote
                .as_ref()
                .map(|quote| quote.amount_subtotal),
            already_credited: already_credited.get(&line.local_id).copied().unwrap_or(0),
            corrected: corrected.get(&line.local_id).copied().unwrap_or(0),
            settled: settled(&line.local_id),
        })
        .collect()
}

impl PeriodCreditContext {
    /// The period's lines as billing-core reads them (see [`issued_lines`]).
    pub(in crate::services) fn issued_lines(
        &self,
        settled: impl Fn(&str) -> Option<i64>,
    ) -> Vec<billing_core::IssuedLine> {
        issued_lines(
            &self.all_lines,
            &self.line_lineage,
            &self.already_credited,
            &self.corrected,
            |line| {
                self.line_to_invoice
                    .get(&line.local_id)
                    .and_then(|id| self.invoices.iter().find(|invoice| invoice.id == *id))
                    .is_some_and(|invoice| {
                        crate::services::invoice_lines::minimum::is_up_front(
                            line,
                            invoice.billing_period_start,
                        )
                    })
            },
            settled,
        )
    }
}

/// Credit-matching context spanning every invoice that advance-billed part of the
/// current period (the recurring invoice + any in-period adjustment invoices). The
/// per-line maps are merged across invoices by `local_id`, which a charge keeps on every
/// document billing it (R-075): two invoices sharing one fail the gathering rather than merge.
/// `line_to_invoice` groups matched credits back into a credit note per source invoice.
pub(in crate::services) struct PeriodCreditContext {
    pub invoices: Vec<Invoice>,
    pub all_lines: Vec<LineItem>,
    pub line_to_invoice: HashMap<String, InvoiceId>,
    pub line_lineage: HashMap<String, String>,
    pub already_credited: HashMap<String, i64>,
    /// What the engine's own notes took off each line: corrections of what it billed.
    pub corrected: HashMap<String, i64>,
    /// What the lines' finalized notes returned of their taxable amount and tax.
    pub returned: HashMap<String, crate::repositories::credit_notes::Returned>,
}

/// Gather the finalized recurring + in-period adjustment invoices for the
/// subscription and build the combined credit-matching context across them. A
/// component/add-on removed by an immediate amendment may have been billed either
/// on the period's recurring invoice (billed in advance at period start) or on an
/// adjustment invoice from an earlier same-period amendment (mid-period add) — both
/// must be searched so the credit lands on the invoice that actually charged it.
pub(in crate::services) async fn gather_period_credit_context(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> StoreResult<PeriodCreditContext> {
    let rows = InvoiceRow::list_creditable_period_invoices(
        conn,
        tenant_id,
        subscription_id,
        period_start,
        period_end,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;

    let mut ctx = PeriodCreditContext {
        invoices: Vec::new(),
        all_lines: Vec::new(),
        line_to_invoice: HashMap::new(),
        line_lineage: HashMap::new(),
        already_credited: HashMap::new(),
        corrected: HashMap::new(),
        returned: HashMap::new(),
    };

    for row in rows {
        let detailed = InvoiceRow::find_detailed_by_id(conn, tenant_id, row.id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let mut invoice: Invoice = detailed.invoice.try_into()?;
        // The query already filters to Finalized; guard defensively all the same.
        if invoice.status != InvoiceStatusEnum::Finalized {
            continue;
        }
        // A consolidated parent also bills the customer's other subscriptions: never theirs.
        if invoice.subscription_id.is_none() {
            let owners = crate::services::credit::scope::LineOwners::load_tx(
                conn,
                tenant_id,
                &invoice.line_items,
            )
            .await?;
            invoice.line_items.retain(|line| {
                crate::services::credit::scope::line_subscription(line, None, &owners)
                    == Some(subscription_id)
            });
        }

        if let Some(line) = invoice
            .line_items
            .iter()
            .find(|line| ctx.line_to_invoice.contains_key(&line.local_id))
        {
            return Err(
                Report::new(StoreError::InvoiceComputationError).attach(format!(
                    "line {} is billed by two invoices of the period being credited",
                    line.local_id
                )),
            );
        }
        let (line_lineage, already_credited, corrected, returned) =
            load_credit_matching_context(conn, tenant_id, &invoice).await?;
        ctx.line_lineage.extend(line_lineage);
        ctx.already_credited.extend(already_credited);
        ctx.corrected.extend(corrected);
        ctx.returned.extend(returned);
        for line in &invoice.line_items {
            ctx.line_to_invoice
                .insert(line.local_id.clone(), invoice.id);
            ctx.all_lines.push(line.clone());
        }
        ctx.invoices.push(invoice);
    }

    Ok(ctx)
}

/// Resolve, for an advance invoice, the data needed to match and size amendment
/// credits exactly:
///
/// 1. `line_lineage`: each invoice line's `local_id` → the lineage root of the
///    component / add-on that billed it, so a credit keyed by lineage matches the
///    originally-billed line across any number of overrides.
/// 2. `already_credited`: each line's `local_id` → the subtotal (in cents) already
///    reversed by earlier finalized credit notes (whose lines reuse the original
///    `local_id`), so the caller never credits more than the line's remaining value.
async fn load_credit_matching_context(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice: &Invoice,
) -> StoreResult<(
    HashMap<String, String>,
    HashMap<String, i64>,
    HashMap<String, i64>,
    HashMap<String, crate::repositories::credit_notes::Returned>,
)> {
    use std::collections::HashSet;

    let comp_ids: Vec<_> = invoice
        .line_items
        .iter()
        .filter_map(|l| l.sub_component_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let addon_ids: Vec<_> = invoice
        .line_items
        .iter()
        .filter_map(|l| l.sub_add_on_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let comp_lineage: HashMap<_, _> =
        SubscriptionComponentRow::find_lineage_by_ids(conn, &comp_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .collect();
    let addon_lineage: HashMap<_, _> = SubscriptionAddOnRow::find_lineage_by_ids(conn, &addon_ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .collect();

    let mut line_lineage = HashMap::new();
    for l in &invoice.line_items {
        // Resolve the line's lineage root (lineage_id, or the id itself when the row
        // is a root / was not found).
        let root = if let Some(cid) = l.sub_component_id {
            Some(
                comp_lineage
                    .get(&cid)
                    .copied()
                    .flatten()
                    .unwrap_or(cid)
                    .to_string(),
            )
        } else {
            l.sub_add_on_id.map(|aid| {
                addon_lineage
                    .get(&aid)
                    .copied()
                    .flatten()
                    .unwrap_or(aid)
                    .to_string()
            })
        };
        if let Some(root) = root {
            line_lineage.insert(l.local_id.clone(), root);
        }
    }

    let mut already_credited: HashMap<String, i64> = HashMap::new();
    let mut corrected: HashMap<String, i64> = HashMap::new();
    let mut returned: HashMap<String, crate::repositories::credit_notes::Returned> = HashMap::new();
    let credit_notes = CreditNoteRow::list_by_invoice_id(conn, tenant_id, invoice.id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    for cn in credit_notes {
        if cn.status != CreditNoteStatus::Finalized {
            continue;
        }
        let lines: Vec<LineItem> = serde_json::from_value(cn.line_items).map_err(|e| {
            Report::new(StoreError::SerdeError(
                "Failed to deserialize credit note line items".to_string(),
                e,
            ))
        })?;
        for l in lines {
            returned.entry(l.local_id.clone()).or_default().add(&l)?;
            // Credit-note lines carry negative subtotals; accumulate the magnitude.
            if l.amount_subtotal < 0 {
                if cn.origin != diesel_models::enums::CreditNoteOriginEnum::Manual {
                    *corrected.entry(l.local_id.clone()).or_insert(0) += -l.amount_subtotal;
                }
                *already_credited.entry(l.local_id).or_insert(0) += -l.amount_subtotal;
            }
        }
    }

    Ok((line_lineage, already_credited, corrected, returned))
}

/// Build an in-memory `SubscriptionComponent` from a resolved insert, for use in
/// the amendment preview's hypothetical post-amendment subscription state. The id
/// is synthetic (not persisted); only the fee/period/name drive invoice amounts.
fn pending_component_to_subscription(
    insert: &PendingComponentInsert,
    subscription_id: SubscriptionId,
    effective_from: NaiveDate,
) -> SubscriptionComponent {
    SubscriptionComponent {
        id: SubscriptionPriceComponentId::new(),
        price_component_id: insert.price_component_id,
        product_id: None,
        subscription_id,
        name: insert.name.clone(),
        period: insert.period,
        fee: insert.fee.clone(),
        price_id: None,
        effective_from,
        effective_to: None,
        lineage_id: None,
        added_by_amendment: true,
    }
}

/// Build an in-memory `SubscriptionAddOn` from a resolved insert (preview only).
fn pending_addon_to_subscription(
    insert: &PendingAddOnInsert,
    subscription_id: SubscriptionId,
    effective_from: NaiveDate,
) -> SubscriptionAddOn {
    SubscriptionAddOn {
        id: SubscriptionAddOnId::new(),
        subscription_id,
        add_on_id: insert.add_on_id,
        name: insert.name.clone(),
        period: insert.period,
        fee: insert.fee.clone(),
        created_at: common_domain::clock::now().naive_utc(),
        product_id: Some(insert.product_id),
        price_id: insert.price_id,
        quantity: insert.quantity,
        effective_from,
        effective_to: None,
        lineage_id: None,
        added_by_amendment: true,
    }
}

/// Resolve an amendment delta against the current subscription state.
/// Read-only: resolves all fees but does NOT materialize new prices.
async fn resolve_amendment(
    conn: &mut PgConn,
    sub_details: &SubscriptionDetails,
    amendment: &SubscriptionAmendment,
    effective_date: NaiveDate,
) -> StoreResult<ResolvedAmendment> {
    let tenant_id = sub_details.subscription.tenant_id;
    let subscription_id = sub_details.subscription.id;

    let mut component_added: Vec<AddedComponent> = Vec::new();
    let mut component_removed: Vec<RemovedComponent> = Vec::new();
    let mut addon_added: Vec<AddedComponent> = Vec::new();
    let mut addon_removed: Vec<RemovedComponent> = Vec::new();

    let mut component_close: Vec<SubscriptionPriceComponentId> = Vec::new();
    let mut component_inserts: Vec<PendingComponentInsert> = Vec::new();
    let mut addon_close: Vec<SubscriptionAddOnId> = Vec::new();
    let mut addon_inserts: Vec<PendingAddOnInsert> = Vec::new();

    let component_by_id: HashMap<SubscriptionPriceComponentId, &SubscriptionComponent> =
        sub_details
            .price_components
            .iter()
            .map(|c| (c.id, c))
            .collect();

    // --- Component edits (override): close old + insert new ---
    for edit in &amendment.component_changes.edited {
        let current = component_by_id
            .get(&edit.subscription_component_id)
            .ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(format!(
                    "Component {} not found on subscription",
                    edit.subscription_component_id
                )))
            })?;

        let product_id = current.product_id.ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "Cannot override a component without a product (legacy pricing). Create a new plan version and migrate this subscription first.".to_string(),
            ))
        })?;
        let fee_structure = load_product_fee_structure(conn, tenant_id, product_id).await?;
        let (mut new_fee, new_period) =
            resolve_fee_read_only(conn, &fee_structure, &edit.price_entry, tenant_id).await?;

        let mut current_fee = current.fee.clone();
        resolve_live_slot_count(conn, tenant_id, subscription_id, &mut current_fee).await?;
        // Repricing keeps the customer's seats: charge the new price on the live count, not min_slots.
        resolve_live_slot_count(conn, tenant_id, subscription_id, &mut new_fee).await?;

        let new_name = edit.name.clone().unwrap_or_else(|| current.name.clone());

        // Key the credit to the component's lineage root (not its current id): after
        // an override the new row has a fresh id, but the originally-billed invoice
        // line still references the lineage root, so this keeps them matched.
        let lineage = current.lineage();
        let net_key = Some(lineage.to_string());
        let billed_component_id = SubscriptionPriceComponentId::new();
        component_removed.push(RemovedComponent {
            name: current.name.clone(),
            current_fee,
            current_period: current.period,
            net_key: net_key.clone(),
        });
        component_added.push(AddedComponent {
            name: new_name.clone(),
            fee: new_fee.clone(),
            period: new_period,
            net_key,
            // The new row's id, stamped on the adjustment line: what it bills counts under
            // the lineage.
            billed_component_id: Some(billed_component_id),
            billed_add_on_id: None,
            instance_quantity: None,
            price_component_id: None,
        });

        component_close.push(current.id);
        component_inserts.push(PendingComponentInsert {
            price_component_id: current.price_component_id,
            product_ref: ProductRef::Existing(product_id),
            price_entry: edit.price_entry.clone(),
            name: new_name,
            period: new_period,
            fee: new_fee,
            is_override: true,
            lineage_id: Some(lineage),
            subscription_component_id: Some(billed_component_id),
        });
    }

    // --- Extra components (add) ---
    for extra in &amendment.component_changes.added {
        let fee_structure = match &extra.product {
            ProductRef::New { fee_structure, .. } => fee_structure.clone(),
            ProductRef::Existing(pid) => load_product_fee_structure(conn, tenant_id, *pid).await?,
        };
        let (fee, period) =
            resolve_fee_read_only(conn, &fee_structure, &extra.price_entry, tenant_id).await?;

        // Pre-generate the id so the immediate adjustment invoice can stamp it onto
        // the prorated charge line, letting a later removal credit it.
        let billed_component_id = SubscriptionPriceComponentId::new();
        component_added.push(AddedComponent {
            name: extra.name.clone(),
            fee: fee.clone(),
            period,
            net_key: None,
            billed_component_id: Some(billed_component_id),
            billed_add_on_id: None,
            // Unscaled catalog fee, so the instance count is recoverable from it —
            // lets the prorated adjustment line show "N × rate" instead of "1 × total".
            instance_quantity: Some(
                crate::services::subscriptions::proration::fee_instance_count(&fee),
            ),
            price_component_id: None,
        });
        component_inserts.push(PendingComponentInsert {
            price_component_id: None,
            product_ref: extra.product.clone(),
            price_entry: extra.price_entry.clone(),
            name: extra.name.clone(),
            period,
            fee,
            is_override: false,
            // A genuinely new component is its own lineage root.
            lineage_id: None,
            subscription_component_id: Some(billed_component_id),
        });
    }

    // --- Component removals ---
    for id in &amendment.component_changes.removed {
        let current = component_by_id.get(id).ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(format!(
                "Component {} not found on subscription",
                id
            )))
        })?;
        let mut current_fee = current.fee.clone();
        resolve_live_slot_count(conn, tenant_id, subscription_id, &mut current_fee).await?;

        component_removed.push(RemovedComponent {
            name: current.name.clone(),
            current_fee,
            current_period: current.period,
            // Carry the component's lineage root so the credit side of the amendment
            // can be matched to its line on the original invoice across overrides.
            net_key: Some(current.lineage().to_string()),
        });
        component_close.push(current.id);
    }

    // --- Add-on changes ---
    // Load catalog add-ons referenced by added/edited entries.
    let mut catalog_addon_ids: Vec<AddOnId> = amendment
        .add_on_changes
        .added
        .iter()
        .map(|a| a.add_on_id)
        .collect();
    let addon_by_sub_id: HashMap<SubscriptionAddOnId, &SubscriptionAddOn> =
        sub_details.add_ons.iter().map(|a| (a.id, a)).collect();
    for e in &amendment.add_on_changes.edited {
        if let Some(a) = addon_by_sub_id.get(&e.subscription_add_on_id) {
            catalog_addon_ids.push(a.add_on_id);
        }
    }

    let (addons, addon_products, addon_prices, addon_units) =
        load_addon_catalog(conn, tenant_id, &catalog_addon_ids, amendment).await?;

    // Added add-ons
    for cs_ao in &amendment.add_on_changes.added {
        if cs_ao.quantity < 1 {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "add-on {} quantity must be >= 1",
                cs_ao.add_on_id
            ))));
        }
        let addon = addons
            .iter()
            .find(|a| a.id == cs_ao.add_on_id)
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(format!(
                    "add-on {} not found",
                    cs_ao.add_on_id
                )))
            })?;
        if let Some(max) = addon.max_instances_per_subscription
            && cs_ao.quantity > max
        {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "add-on {} quantity {} exceeds max_instances_per_subscription {}",
                cs_ao.add_on_id, cs_ao.quantity, max
            ))));
        }
        let resolved = addon
            .resolve_customized(
                &addon_products,
                &addon_prices,
                &addon_units,
                &cs_ao.customization,
            )
            .map_err(Report::new)?;

        // Pre-generate the id so the immediate adjustment invoice can stamp it onto
        // the prorated charge line, letting a later removal credit it.
        let billed_add_on_id = SubscriptionAddOnId::new();
        addon_added.push(AddedComponent {
            name: resolved.name.clone(),
            fee: scale_fee(&resolved.fee, cs_ao.quantity),
            period: resolved.period,
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: Some(billed_add_on_id),
            instance_quantity: Some(Decimal::from(cs_ao.quantity)),
            price_component_id: None,
        });
        addon_inserts.push(PendingAddOnInsert {
            add_on_id: addon.id,
            product_id: addon.product_id,
            price_entry: resolved.price_entry.clone(),
            price_id: resolved.price_id,
            name: resolved.name,
            period: resolved.period,
            fee: resolved.fee,
            quantity: cs_ao.quantity,
            // A genuinely new add-on is its own lineage root.
            lineage_id: None,
            subscription_add_on_id: Some(billed_add_on_id),
        });
    }

    // Edited add-ons (quantity and/or customization): close old + insert new.
    for e in &amendment.add_on_changes.edited {
        let current = addon_by_sub_id
            .get(&e.subscription_add_on_id)
            .ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(format!(
                    "Add-on {} not found on subscription",
                    e.subscription_add_on_id
                )))
            })?;
        let new_quantity = e.quantity.map(|q| q as i32).unwrap_or(current.quantity);
        if new_quantity < 1 {
            return Err(Report::new(StoreError::InvalidArgument(
                "add-on quantity must be >= 1".to_string(),
            )));
        }
        let addon = addons.iter().find(|a| a.id == current.add_on_id);
        if let Some(max) = addon.and_then(|a| a.max_instances_per_subscription)
            && new_quantity > max
        {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "add-on quantity {} exceeds max_instances_per_subscription {}",
                new_quantity, max
            ))));
        }

        // New fee/period: either re-resolve from a new customization, or keep current.
        let (new_fee, new_period, new_name, price_entry, price_id, product_id) =
            if let (Some(addon), Some(cust)) = (addon, &e.customization) {
                let resolved = addon
                    .resolve_customized(&addon_products, &addon_prices, &addon_units, cust)
                    .map_err(Report::new)?;
                (
                    resolved.fee,
                    resolved.period,
                    resolved.name,
                    resolved.price_entry,
                    resolved.price_id,
                    addon.product_id,
                )
            } else {
                (
                    current.fee.clone(),
                    current.period,
                    current.name.clone(),
                    current.price_id.map(PriceEntry::Existing),
                    current.price_id,
                    current
                        .product_id
                        .or_else(|| addon.map(|a| a.product_id))
                        .unwrap_or_else(|| ProductId::from(uuid::Uuid::nil())),
                )
            };

        // Key the credit to the add-on's lineage root so it stays matched to the
        // originally-billed invoice line across overrides.
        let lineage = current.lineage();
        let net_key = Some(lineage.to_string());
        let billed_add_on_id = SubscriptionAddOnId::new();
        addon_removed.push(RemovedComponent {
            name: current.name.clone(),
            current_fee: scale_fee(&current.fee, current.quantity),
            current_period: current.period,
            net_key: net_key.clone(),
        });
        addon_added.push(AddedComponent {
            name: new_name.clone(),
            fee: scale_fee(&new_fee, new_quantity),
            period: new_period,
            net_key,
            // The new row's id, stamped on the adjustment line, as for a component override.
            billed_component_id: None,
            billed_add_on_id: Some(billed_add_on_id),
            instance_quantity: None,
            price_component_id: None,
        });

        addon_close.push(current.id);
        addon_inserts.push(PendingAddOnInsert {
            add_on_id: current.add_on_id,
            product_id,
            price_entry,
            price_id,
            name: new_name,
            period: new_period,
            fee: new_fee,
            quantity: new_quantity,
            lineage_id: Some(lineage),
            subscription_add_on_id: Some(billed_add_on_id),
        });
    }

    // Removed add-ons
    for id in &amendment.add_on_changes.removed {
        let current = addon_by_sub_id.get(id).ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(format!(
                "Add-on {} not found on subscription",
                id
            )))
        })?;
        addon_removed.push(RemovedComponent {
            name: current.name.clone(),
            current_fee: scale_fee(&current.fee, current.quantity),
            current_period: current.period,
            // Carry the add-on's lineage root so the credit side can be matched to
            // its line on the original invoice across overrides.
            net_key: Some(current.lineage().to_string()),
        });
        addon_close.push(current.id);
    }

    let preview = AmendmentPreview {
        component_added,
        component_removed,
        addon_added,
        addon_removed,
        effective_date,
    };

    Ok(ResolvedAmendment {
        preview,
        component_close,
        component_inserts,
        addon_close,
        addon_inserts,
    })
}

async fn load_product_fee_structure(
    conn: &mut PgConn,
    tenant_id: TenantId,
    product_id: ProductId,
) -> StoreResult<FeeStructure> {
    let (_, products) = fetch_prices_and_products(
        conn,
        tenant_id,
        std::iter::empty(),
        std::iter::once(product_id),
    )
    .await?;
    products
        .get(&product_id)
        .map(|p| p.fee_structure.clone())
        .ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(format!(
                "Product {} not found",
                product_id
            )))
        })
}

#[allow(clippy::type_complexity)]
async fn load_addon_catalog(
    conn: &mut PgConn,
    tenant_id: TenantId,
    add_on_ids: &[AddOnId],
    amendment: &SubscriptionAmendment,
) -> StoreResult<(
    Vec<crate::domain::add_ons::AddOn>,
    HashMap<ProductId, Product>,
    HashMap<PriceId, crate::domain::prices::Price>,
    crate::domain::credit_units::CreditUnitsById,
)> {
    if add_on_ids.is_empty() {
        return Ok((
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            crate::domain::credit_units::CreditUnitsById::new(),
        ));
    }

    let rows = diesel_models::add_ons::AddOnRow::list_by_ids(conn, add_on_ids, &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let addons = crate::repositories::add_ons::enrich_add_ons(conn, rows, tenant_id).await?;

    // Collect product/price ids: the add-on definitions + any Existing override prices.
    let product_ids: Vec<ProductId> = addons.iter().map(|a| a.product_id).collect();
    let mut price_ids: Vec<PriceId> = addons.iter().map(|a| a.price_id).collect();

    let collect_override =
        |price_ids: &mut Vec<PriceId>,
         cust: &crate::domain::subscription_add_ons::SubscriptionAddOnCustomization| {
            if let crate::domain::subscription_add_ons::SubscriptionAddOnCustomization::PriceOverride {
                price_entry: PriceEntry::Existing(pid),
                ..
            } = cust
            {
                price_ids.push(*pid);
            }
        };
    for a in &amendment.add_on_changes.added {
        collect_override(&mut price_ids, &a.customization);
    }
    for e in &amendment.add_on_changes.edited {
        if let Some(c) = &e.customization {
            collect_override(&mut price_ids, c);
        }
    }

    // fetch_prices_and_products dedups the ids internally.
    let (prices, products) = fetch_prices_and_products(
        conn,
        tenant_id,
        price_ids.into_iter(),
        product_ids.into_iter(),
    )
    .await?;
    let new_price_units = amendment
        .add_on_changes
        .added
        .iter()
        .map(|a| &a.customization)
        .chain(
            amendment
                .add_on_changes
                .edited
                .iter()
                .filter_map(|e| e.customization.as_ref()),
        )
        .flat_map(|c| c.new_credit_unit_ids());
    let units = crate::repositories::credit_units::load_credit_units(
        conn,
        tenant_id,
        prices
            .values()
            .flat_map(crate::domain::prices::Price::credit_unit_ids)
            .chain(new_price_units)
            .collect::<Vec<_>>(),
    )
    .await?;

    Ok((addons, products, prices, units))
}

/// Materialize any New prices/products in the pending inserts and produce the
/// fully-resolved inserts carried by the apply/execute path and the scheduled event.
async fn materialize_inserts(
    conn: &mut PgConn,
    tenant_id: TenantId,
    sub_details: &SubscriptionDetails,
    component_inserts: Vec<PendingComponentInsert>,
    addon_inserts: Vec<PendingAddOnInsert>,
) -> StoreResult<(Vec<ResolvedComponentInsert>, Vec<ResolvedAddOnInsert>)> {
    let currency = &sub_details.subscription.currency;
    let product_family_id = diesel_models::plan_versions::PlanVersionRow::get_product_family_id(
        conn,
        sub_details.subscription.plan_version_id,
        tenant_id,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;

    let mut resolved_components = Vec::with_capacity(component_inserts.len());
    for c in component_inserts {
        let internal = PriceComponentNewInternal {
            name: c.name.clone(),
            product_ref: c.product_ref.clone(),
            prices: vec![c.price_entry.clone()],
        };
        let (product_id, price_ids) = resolve_component_internal(
            conn,
            &internal,
            tenant_id,
            product_family_id,
            currency,
            false,
        )
        .await?;
        let price_id = price_ids.into_iter().next();

        resolved_components.push(ResolvedComponentInsert {
            price_component_id: c.price_component_id,
            product_id: Some(product_id),
            name: c.name,
            period: c.period,
            fee: c.fee,
            is_override: c.is_override,
            price_id,
            lineage_id: c.lineage_id,
            subscription_component_id: c.subscription_component_id,
        });
    }

    let mut resolved_addons = Vec::with_capacity(addon_inserts.len());
    for a in addon_inserts {
        // Materialize a New override price against the add-on's existing product.
        let (product_id, price_id) = if let Some(PriceEntry::New(_)) = &a.price_entry {
            let internal = PriceComponentNewInternal {
                name: a.name.clone(),
                product_ref: ProductRef::Existing(a.product_id),
                prices: vec![a.price_entry.clone().unwrap()],
            };
            let (pid, price_ids) = resolve_component_internal(
                conn,
                &internal,
                tenant_id,
                product_family_id,
                currency,
                false,
            )
            .await?;
            (Some(pid), price_ids.into_iter().next())
        } else {
            (Some(a.product_id), a.price_id)
        };

        resolved_addons.push(ResolvedAddOnInsert {
            add_on_id: a.add_on_id,
            name: a.name,
            period: a.period,
            fee: a.fee,
            product_id,
            price_id,
            quantity: a.quantity,
            lineage_id: a.lineage_id,
            subscription_add_on_id: a.subscription_add_on_id,
        });
    }

    Ok((resolved_components, resolved_addons))
}

#[cfg(test)]
mod tests {
    use super::scale_fee;
    use crate::domain::enums::SubscriptionFeeBillingPeriod;
    use crate::domain::subscription_changes::{AddedComponent, RemovedComponent};
    use crate::domain::subscription_components::SubscriptionFee;
    use crate::services::subscriptions::proration::calculate_proration;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    fn rate(n: i64) -> SubscriptionFee {
        SubscriptionFee::Rate {
            rate: Decimal::from(n),
        }
    }

    #[test]
    fn scale_rate_multiplies_by_quantity() {
        let scaled = scale_fee(&rate(10), 3);
        match scaled {
            SubscriptionFee::Rate { rate } => assert_eq!(rate, Decimal::from(30)),
            _ => panic!("expected rate"),
        }
    }

    #[test]
    fn scale_slot_multiplies_initial_slots() {
        let fee = SubscriptionFee::Slot {
            unit: "seat".into(),
            unit_rate: Decimal::from(5),
            min_slots: None,
            max_slots: None,
            initial_slots: 4,
        };
        match scale_fee(&fee, 2) {
            SubscriptionFee::Slot { initial_slots, .. } => assert_eq!(initial_slots, 8),
            _ => panic!("expected slot"),
        }
    }

    #[test]
    fn scale_quantity_one_is_identity() {
        match scale_fee(&rate(7), 1) {
            SubscriptionFee::Rate { rate } => assert_eq!(rate, Decimal::from(7)),
            _ => panic!("expected rate"),
        }
    }

    // An add-on added mid-cycle for half the period charges quantity * rate * factor.
    #[test]
    fn addon_add_prorates_by_quantity() {
        let period_start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let period_end = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let change_date = NaiveDate::from_ymd_opt(2026, 1, 16).unwrap(); // 15 of 30 days remaining

        // 2 units of a $100/mo add-on => $200 full period; half period => $100 charge.
        let added = vec![AddedComponent {
            name: "Seats".into(),
            fee: scale_fee(&rate(100), 2),
            period: SubscriptionFeeBillingPeriod::Monthly,
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: None,
            instance_quantity: Some(Decimal::from(2u32)),
            price_component_id: None,
        }];

        let result =
            calculate_proration(&[], &added, &[], period_start, period_end, change_date, 2);
        // full period = 20000 cents, factor 15/30 => 10000
        assert_eq!(result.net_amount_cents, 10000);
        assert!(result.lines.iter().all(|l| !l.is_credit));
    }

    // A quantity change (remove old qty + add new qty) nets the delta, prorated.
    #[test]
    fn addon_quantity_change_nets_delta() {
        let period_start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let period_end = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let change_date = NaiveDate::from_ymd_opt(2026, 1, 16).unwrap();

        // old: 1 unit @ $100; new: 3 units @ $100 => +$200 full period; half => +$100 net.
        let removed = vec![RemovedComponent {
            name: "Seats".into(),
            current_fee: scale_fee(&rate(100), 1),
            current_period: SubscriptionFeeBillingPeriod::Monthly,
            net_key: None,
        }];
        let added = vec![AddedComponent {
            name: "Seats".into(),
            fee: scale_fee(&rate(100), 3),
            period: SubscriptionFeeBillingPeriod::Monthly,
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: None,
            instance_quantity: Some(Decimal::from(3u32)),
            price_component_id: None,
        }];

        let result = calculate_proration(
            &[],
            &added,
            &removed,
            period_start,
            period_end,
            change_date,
            2,
        );
        // credit -5000 (1 unit half), charge +15000 (3 units half) => net +10000
        assert_eq!(result.net_amount_cents, 10000);
    }
}
