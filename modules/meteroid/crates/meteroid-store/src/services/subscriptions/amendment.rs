use crate::StoreResult;
use crate::domain::LineItemKind;
use crate::domain::SubscriptionDetails;
use crate::domain::SubscriptionStatusEnum;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::InvoiceStatusEnum;
use crate::domain::enums::SubscriptionFeeBillingPeriod;
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
    AddOnChanges, AmendmentPreview, AmendmentPreviewExtended, ComponentChanges,
    ImmediateAmendmentResult, SubscriptionAmendment,
};
use crate::domain::subscription_changes::{
    AddedComponent, PlanChangeMode, ProrationSummary, RemovedComponent,
};
use crate::domain::subscription_components::{
    SubscriptionComponent, SubscriptionComponentNew, SubscriptionComponentNewInternal,
    SubscriptionFee,
};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, create_user_credit_note_tx,
};
use crate::repositories::entity_activity::EntityActivityInterface;
use crate::repositories::price_components::resolve_component_internal;
use crate::repositories::subscriptions::fetch_prices;
use crate::services::Services;
use crate::services::evaluate::change::{Change, GiveBack};
use crate::services::subscriptions::insert::context::resolve_fee_read_only;
use crate::services::subscriptions::plan_change::calculate_components_mrr_with_slots;
use crate::services::subscriptions::timeline_rewrite;
use crate::services::subscriptions::utils::{
    MrrLine, calculate_mrr, fee_period_total_cents, resolve_add_on_sold_in, scale_fee,
    subscription_mrr, with_add_on_lines,
};
use crate::store::PgConn;
use chrono::{NaiveDate, NaiveTime};
use common_domain::ids::{
    AddOnId, BaseId, InvoiceId, PriceComponentId, PriceId, ProductId, SubscriptionAddOnId,
    SubscriptionId, SubscriptionPriceComponentId, TenantId,
};
use diesel_models::credit_notes::CreditNoteRow;
use diesel_models::enums::CreditNoteStatus;
use diesel_models::enums::SubscriptionChangeKindEnum;
use diesel_models::invoices::InvoiceRow;
use diesel_models::query::credit_inputs;
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
    /// How the charge bills its product, its new price checked against it.
    fee_structure: FeeStructure,
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
    /// The add-on's structure, a new override price checked against it.
    fee_structure: Option<FeeStructure>,
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

            // The period an immediate amendment lands in, on the cadence it leaves; its amounts
            // are the facts' (below), the period and the deferred arrears are shown with them.
            let (cadence, grid) = if is_immediate {
                let cadence = crate::services::subscriptions::cadence::cadence_after_change(
                    &mut conn,
                    &sub_details,
                    &cadences_after(&sub_details, &resolved),
                    effective_date,
                )
                .await?;
                let grid = amendment_grid(
                    &mut conn,
                    &sub_details.subscription,
                    cadence.as_ref(),
                    period_end,
                    effective_date,
                )
                .await?;
                (cadence, Some(grid))
            } else {
                (None, None)
            };

            let proration = grid.map(|grid| {
                let (period_start, period_end, factor) = grid.billed(effective_date);
                let days_in_period = (period_end - period_start).num_days().max(0) as u32;
                let days_remaining = (period_end - effective_date).num_days().max(0) as u32;

                // Arrears components added by this amendment are NOT billed on the
                // immediate adjustment invoice — they land prorated on the next
                // renewal invoice. Tracked apart so the UI can surface the deferred amount.
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
                                model,
                            } => crate::domain::price_components::fixed_quantity_minor(
                                *rate,
                                rust_decimal::Decimal::from(*quantity),
                                model.as_ref(),
                                precision,
                            ),
                            _ => 0,
                        }
                        })
                        .sum();
                    grid.left(grid.current, effective_date)
                        .prorate(full_arrears)
                };

                ProrationSummary {
                    document: Default::default(),
                    credits_total_cents: 0,
                    charges_total_cents: 0,
                    net_amount_cents: 0,
                    net_credit_cents: 0,
                    arrears_charge_cents,
                    proration_factor: factor.as_f64(),
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
            // A seat add-on bills its quantity's live count, kept or added (R-137, R-138).
            let live_add_ons = crate::services::subscriptions::plan_change::add_ons_at_live_seats(
                &mut conn,
                tenant_id,
                subscription_id,
                &sub_details.add_ons,
            )
            .await?;
            let added_quantities = resolved
                .addon_inserts
                .iter()
                .filter_map(|a| a.fee.seat_quantity())
                .collect();
            let added_counts = crate::services::subscriptions::plan_change::live_seat_counts(
                &mut conn,
                tenant_id,
                subscription_id,
                added_quantities,
            )
            .await?;
            let before_lines = with_add_on_lines(&live_add_ons, component_lines.clone(), precision);
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
                    live_add_ons
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
                    let mut fee = a.fee.clone();
                    crate::services::subscriptions::plan_change::at_live_count(
                        &mut fee,
                        &added_counts,
                    );
                    MrrLine::new(
                        &fee,
                        calculate_mrr(&fee, &a.period, precision) * quantity,
                        Some(a.product_id),
                    )
                    .with_period_total(fee_period_total_cents(&fee, precision) * quantity)
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

            // The amendment's rows laid over the facts at its date, billed as applying it bills:
            // its adjustment invoice, its credit notes and the renewal after it. Nothing written.
            let event = crate::services::evaluate::preview::change_event(
                tenant_id,
                subscription_id,
                effective_date,
                ScheduledEventData::ApplyAmendment {
                    component_close: resolved.component_close.clone(),
                    component_insert: resolved
                        .component_inserts
                        .iter()
                        .map(previewed_component)
                        .collect(),
                    addon_close: resolved.addon_close.clone(),
                    addon_insert: resolved
                        .addon_inserts
                        .iter()
                        .map(previewed_add_on)
                        .collect(),
                },
            );
            let overlay = self.overlay_of_change(&mut conn, &event).await?;
            let documents = match (is_immediate, is_free_trial(&sub_details)) {
                (true, false) => Some(
                    self.preview_change_documents(
                        &mut conn,
                        &sub_details,
                        &overlay,
                        Change::of_rows(effective_date, GiveBack::LatestFirst, "amendment")
                            .without_packs(),
                        Vec::new(),
                    )
                    .await?,
                ),
                _ => None,
            };
            let proration = proration.map(|summary| match &documents {
                Some(documents) => ProrationSummary {
                    credits_total_cents: -documents.credited,
                    charges_total_cents: documents.charged,
                    net_amount_cents: documents.charged - documents.credited,
                    net_credit_cents: -documents.credited,
                    ..summary
                },
                None => summary,
            });
            let adjustment_invoice = documents
                .as_ref()
                .and_then(|documents| documents.adjustment.clone());
            let credit_note = documents
                .as_ref()
                .and_then(|documents| documents.credit_note.clone());

            // The renewal under the amended subscription. Best-effort: a preview never fails on it,
            // it says what it left out.
            let mut missing = Vec::new();
            let (next_invoice, subscription_after) = {
                let mut hypothetical =
                    crate::services::evaluate::preview::details_after(&sub_details, &overlay)?;
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
                let invoice = match self
                    .preview_next_invoice(&mut conn, &hypothetical, &overlay, None)
                    .await
                {
                    Ok(invoice) => Some(invoice),
                    Err(error) => {
                        log::warn!(
                            "The renewal after an amendment of subscription {} was not previewed: {error:?}",
                            sub_details.subscription.id
                        );
                        missing.push(crate::domain::subscription_changes::PreviewPart::NextInvoice);
                        None
                    }
                };
                (invoice, hypothetical)
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
                subscription_after: subscription_after.subscription.clone(),
                details_after: subscription_after,
                preview: resolved.preview,
                proration,
                change_direction,
                mrr_before_cents,
                mrr_after_cents,
                adjustment_invoice,
                credit_note,
                next_invoice,
                missing,
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
        let today = common_clock::now().naive_utc().date();
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
        self.amend_immediately_at(
            actor,
            subscription_id,
            tenant_id,
            amendment,
            change_date,
            AmendmentOrigin::Manual,
        )
        .await
    }

    /// A level change applied today, refused under the subscription's lock while another change
    /// waits (R-106).
    pub(in crate::services) async fn apply_level_change_immediate(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: SubscriptionAmendment,
    ) -> StoreResult<ImmediateAmendmentResult> {
        let today = common_clock::now().naive_utc().date();
        self.amend_immediately_at(
            actor,
            subscription_id,
            tenant_id,
            amendment,
            today,
            AmendmentOrigin::LevelChange,
        )
        .await
    }

    async fn amend_immediately_at(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: SubscriptionAmendment,
        change_date: NaiveDate,
        origin: AmendmentOrigin,
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
                // R-106: a scheduled amendment stands, this one written under it.
                reject_if_plan_change_pending(conn, subscription_id, tenant_id).await?;
                origin
                    .refuse_while_a_change_is_pending(conn, tenant_id, subscription_id)
                    .await?;

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

                // The timeline from its day is written again over it (R-162).
                let lifted =
                    timeline_rewrite::lift_tx(conn, tenant_id, subscription_id, change_date)
                        .await?;
                let resolved =
                    resolve_amendment(conn, &sub_details, amendment, change_date).await?;

                let precision = crate::constants::Currencies::resolve_currency_precision(
                    &sub_details.subscription.currency,
                )
                .unwrap_or(2);

                let added = resolved.preview.all_added();
                let removed = resolved.preview.all_removed();
                ensure_immediate_amendment_allowed(&sub_details, &added, &removed, precision)?;

                let is_free_trial = is_free_trial(&sub_details);

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
                        !is_free_trial,
                    )
                    .await?;
                if !lifted.is_empty() {
                    timeline_rewrite::replay_tx(services, conn, actor, lifted).await?;
                    services
                        .derive_state_tx(
                            conn,
                            actor,
                            tenant_id,
                            subscription_id,
                            crate::services::evaluate::bill::midnight(change_date),
                        )
                        .await?;
                }

                // What the rows just written owe goes on an adjustment invoice; what reconcile
                // takes back of the lines billing them, latest first, on notes (R-078, R-080).
                let (adjustment_invoice_id, credit_note_ids) = if is_free_trial {
                    (None, vec![])
                } else {
                    services
                        .settle_change_tx(
                            conn,
                            actor,
                            &sub_details.subscription,
                            &sub_details.customer,
                            &Change::of_rows(change_date, GiveBack::LatestFirst, "amendment")
                                .without_packs(),
                            "Subscription amendment",
                            crate::domain::enums::CreditNoteOrigin::Amendment,
                        )
                        .await?
                };
                if !credit_note_ids.is_empty() {
                    diesel_models::subscription_events::SubscriptionEventRow::record_ending_notes(
                        conn,
                        diesel_models::subscription_events::SettlingEnding::PlanChange,
                        subscription_id,
                        change_date,
                        &credit_note_ids
                            .iter()
                            .map(|id| id.as_uuid())
                            .collect::<Vec<_>>(),
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                }

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
                // Partly settled by credit or cash, the note may exceed what is still due: to the
                // balance, which cancels the debt left first (I17).
                let credit_type =
                    crate::repositories::credit_notes::change_note_type(conn, tenant_id, invoice)
                        .await?;

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

    /// What an amendment does to the subscription's timeline, read in a transaction rolled back:
    /// `None` without a timeline after it (R-162).
    pub(in crate::services) async fn amendment_timeline_effect(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: SubscriptionAmendment,
    ) -> StoreResult<Option<crate::domain::subscription_phases::TimelineEffect>> {
        let mut conn = self.store.get_conn().await?;
        let details = self
            .store
            .get_subscription_details_with_conn(&mut conn, tenant_id, subscription_id)
            .await?;
        let is_immediate = matches!(amendment.apply_mode, PlanChangeMode::Immediate);
        let on = amendment_effective_date(&details, is_immediate);
        drop(conn);
        timeline_rewrite::effect_of(self, tenant_id, subscription_id, on, is_immediate, |conn| {
            let services = self;
            let actor = &actor;
            let amendment = &amendment;
            async move {
                if is_immediate {
                    services
                        .amend_rows_now_tx(conn, actor, subscription_id, tenant_id, amendment, on)
                        .await
                } else {
                    services
                        .schedule_amendment_tx(
                            conn,
                            actor,
                            subscription_id,
                            tenant_id,
                            amendment,
                            AmendmentOrigin::Manual,
                        )
                        .await
                        .map(|(_, timeline)| timeline)
                }
            }
            .scope_boxed()
        })
        .await
    }

    /// An immediate amendment's rows written on `on` and the timeline replayed over them, its
    /// documents aside: what its timeline preview writes.
    async fn amend_rows_now_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: &SubscriptionAmendment,
        on: NaiveDate,
    ) -> StoreResult<Vec<crate::domain::subscription_phases::RewriteEntry>> {
        let sub_details = self
            .store
            .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
            .await?;
        let lifted = timeline_rewrite::lift_tx(conn, tenant_id, subscription_id, on).await?;
        let resolved = resolve_amendment(conn, &sub_details, amendment, on).await?;
        let (component_inserts, addon_inserts) = materialize_inserts(
            conn,
            tenant_id,
            &sub_details,
            resolved.component_inserts,
            resolved.addon_inserts,
        )
        .await?;
        self.execute_amendment_tx(
            conn,
            tenant_id,
            subscription_id,
            on,
            &resolved.component_close,
            &component_inserts,
            &resolved.addon_close,
            &addon_inserts,
            false,
        )
        .await?;
        timeline_rewrite::replay_tx(self, conn, actor, lifted).await
    }

    pub(in crate::services) async fn schedule_amendment(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: SubscriptionAmendment,
    ) -> StoreResult<ScheduledEvent> {
        self.schedule_amendment_from(
            actor,
            subscription_id,
            tenant_id,
            amendment,
            AmendmentOrigin::Manual,
        )
        .await
    }

    /// A level change scheduled for the period's end, refused under the subscription's lock while
    /// another change waits (R-106) rather than replacing it as an amendment does.
    pub(in crate::services) async fn schedule_level_change(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: SubscriptionAmendment,
    ) -> StoreResult<ScheduledEvent> {
        self.schedule_amendment_from(
            actor,
            subscription_id,
            tenant_id,
            amendment,
            AmendmentOrigin::LevelChange,
        )
        .await
    }

    async fn schedule_amendment_from(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: SubscriptionAmendment,
        origin: AmendmentOrigin,
    ) -> StoreResult<ScheduledEvent> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                let amendment = &amendment;
                async move {
                    self.schedule_amendment_tx(
                        conn,
                        actor,
                        subscription_id,
                        tenant_id,
                        amendment,
                        origin,
                    )
                    .await
                    .map(|(event, _)| event)
                }
                .scope_boxed()
            })
            .await
    }

    /// [`Self::schedule_amendment_from`] inside the caller's transaction.
    async fn schedule_amendment_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: &SubscriptionAmendment,
        origin: AmendmentOrigin,
    ) -> StoreResult<(
        ScheduledEvent,
        Vec<crate::domain::subscription_phases::RewriteEntry>,
    )> {
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
        origin
            .refuse_while_a_change_is_pending(conn, tenant_id, subscription_id)
            .await?;

        let effective_date = sub_details.subscription.current_period_end.ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "Subscription has no current_period_end".to_string(),
            ))
        })?;

        // The timeline from its day is written again over it (R-162).
        let lifted =
            timeline_rewrite::lift_tx(conn, tenant_id, subscription_id, effective_date).await?;
        let resolved = resolve_amendment(conn, &sub_details, amendment, effective_date).await?;

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

        // Written as facts now, the amendment's rows dated from the period's end.
        let event = crate::services::facts::pending::schedule_as_facts_tx(
            &self.store,
            conn,
            ScheduledEventNew {
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
            },
        )
        .await?;
        let timeline = timeline_rewrite::replay_tx(self, conn, actor, lifted).await?;
        // A period's end already past (the controller behind): dated as a late change.
        if effective_date <= common_clock::now().date_naive() {
            crate::repositories::subscriptions::usage_attribution::settle_late_change(
                conn,
                tenant_id,
                subscription_id,
                effective_date,
                "The amendment",
            )
            .await?;
        }
        self.derive_state_tx(conn, actor, tenant_id, subscription_id, common_clock::now())
            .await?;

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

        Ok((event, timeline))
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
                    let cancelled = crate::services::subscriptions::phases::pending_one_offs_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                        &[SubscriptionChangeKindEnum::Amendment],
                    )
                    .await?
                    .len();
                    timeline_rewrite::withdraw_one_offs_under_timeline_tx(
                        self,
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        &[SubscriptionChangeKindEnum::Amendment],
                        "Cancelled by user",
                    )
                    .await?;
                    if cancelled == 0 {
                        return Err(Report::new(StoreError::ValueNotFound(
                            "No pending amendment found for this subscription".to_string(),
                        )));
                    }
                    self.derive_state_tx(
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        common_clock::now(),
                    )
                    .await?;

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
        settles: bool,
    ) -> StoreResult<()> {
        // The quantities held before the change: a new seat fee of one binds to its count.
        let held = {
            let components: Vec<SubscriptionComponentRow> =
                SubscriptionComponentRow::list_fact_rows(conn, subscription_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .into_iter()
                    .filter(|row| row.effective_to.is_none())
                    .collect();
            let add_ons: Vec<SubscriptionAddOnRow> =
                SubscriptionAddOnRow::list_fact_rows(conn, subscription_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .into_iter()
                    .filter(|row| row.effective_to.is_none())
                    .collect();
            crate::services::facts::pending::held_seat_quantities(
                conn,
                tenant_id,
                &components,
                &add_ons,
            )
            .await?
        };
        crate::services::facts::close_components_tx(conn, component_close, change_date).await?;
        crate::services::facts::close_add_ons_tx(conn, addon_close, change_date).await?;

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
        under_scheduled_rows(
            conn,
            subscription_id,
            change_date,
            &component_rows,
            &addon_rows,
        )
        .await?;

        // A fee of another cadence moves the subscription onto the shortest one's grid: a fact
        // from the change's date; the writer cuts the period it renews on.
        crate::services::facts::pending::record_amendment_cadence_tx(
            conn,
            tenant_id,
            subscription_id,
            change_date,
        )
        .await?;
        crate::repositories::subscriptions::usage_attribution::settle_late_change(
            conn,
            tenant_id,
            subscription_id,
            change_date,
            "The amendment",
        )
        .await?;

        // Overrides and edited add-ons keep their quantity's ledger, and a new seat fee of a
        // quantity held binds to its count (R-137): a seed would reset the live count.
        use crate::domain::slot_transactions::SlotTransactionNewInternal;
        let seeds: Vec<_> = component_insert
            .iter()
            .filter(|c| !c.is_override)
            .map(|c| &c.fee)
            .chain(
                addon_insert
                    .iter()
                    .filter(|a| a.lineage_id.is_none())
                    .map(|a| &a.fee),
            )
            .filter(|fee| !fee.seat_quantity().is_some_and(|q| held.contains(&q)))
            .filter_map(|fee| SlotTransactionNewInternal::from_fee(fee, change_date))
            .map(|tx| (subscription_id, tx))
            .collect();
        let rows =
            crate::repositories::subscriptions::slots::ledger_rows(conn, tenant_id, &seeds).await?;
        SlotTransactionRow::insert_batch(conn, rows.iter().collect())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let sub_event = diesel_models::subscription_events::SubscriptionEventRow {
            id: uuid::Uuid::now_v7(),
            subscription_id,
            event_type: diesel_models::enums::SubscriptionEventType::Switch,
            // Settling, it credits the lines of invoices still drafts once they are issued.
            details: Some(if settles {
                serde_json::json!({ "kind": "amendment", "mode": "immediate", "outgoing": "settled" })
            } else {
                serde_json::json!({ "kind": "amendment", "mode": "immediate" })
            }),
            created_at: common_clock::now().naive_utc(),
            mrr_delta: None,
            bi_mrr_movement_log_id: None,
            applies_to: change_date,
        };
        sub_event
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // The period the cadence cuts and the MRR: the writer's, from the rows just written.
        self.derive_state_tx(
            conn,
            &Actor::System,
            tenant_id,
            subscription_id,
            crate::services::evaluate::bill::midnight(change_date),
        )
        .await?;

        log::info!(
            "Applied immediate amendment for subscription {}: closed_components={}, added_components={}, closed_addons={}, added_addons={}",
            subscription_id,
            component_close.len(),
            component_insert.len(),
            addon_close.len(),
            addon_insert.len(),
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

/// The grids an immediate amendment reads each fee's period on, as its proration does: renewing
/// on the cadence it moves the subscription to, if any.
async fn amendment_grid(
    conn: &mut PgConn,
    sub: &crate::domain::Subscription,
    cadence: Option<&crate::services::subscriptions::cadence::Cadence>,
    period_end: NaiveDate,
    change_date: NaiveDate,
) -> StoreResult<crate::services::subscriptions::proration::OwnGrid> {
    let grid = crate::services::subscriptions::proration::OwnGrid::read(
        conn,
        sub,
        (sub.current_period_start, period_end),
        change_date,
    )
    .await?;
    Ok(match cadence {
        Some(cadence) => grid.renewing((cadence.current.start, cadence.current.end)),
        None => grid,
    })
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
        common_clock::now().naive_utc().date()
    } else {
        sub_details
            .subscription
            .current_period_end
            .unwrap_or(sub_details.subscription.current_period_start)
    }
}

/// What asked for an amendment: an operator's, which replaces a scheduled one, or a level change,
/// which stands back from any change still waiting.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AmendmentOrigin {
    Manual,
    LevelChange,
}

impl AmendmentOrigin {
    /// A level change re-reads the pending changes under the subscription's lock: two moves
    /// checked before it (a downgrade at the period's end, an upgrade now) would both pass.
    async fn refuse_while_a_change_is_pending(
        self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<()> {
        if self == AmendmentOrigin::LevelChange {
            crate::services::subscriptions::levels::refuse_while_a_change_is_pending(
                conn,
                tenant_id,
                subscription_id,
            )
            .await?;
        }
        Ok(())
    }
}

/// The add-on row an amendment adding `quantity` of `add_on_id` on `on` would write, resolved and
/// checked against the plan as one: a timeline item's start, or its paid successor.
pub(crate) async fn resolve_added_add_on(
    conn: &mut PgConn,
    sub_details: &SubscriptionDetails,
    add_on_id: common_domain::ids::AddOnId,
    quantity: i32,
    on: NaiveDate,
) -> StoreResult<ResolvedAddOnInsert> {
    let amendment =
        SubscriptionAmendment {
            apply_mode: PlanChangeMode::EndOfPeriod,
            component_changes: ComponentChanges::default(),
            add_on_changes: AddOnChanges {
                added: vec![crate::domain::subscription_add_ons::CreateSubscriptionAddOn {
                add_on_id,
                customization:
                    crate::domain::subscription_add_ons::SubscriptionAddOnCustomization::None,
                quantity,
            }],
                ..AddOnChanges::default()
            },
        };
    let resolved = resolve_amendment(conn, sub_details, &amendment, on).await?;
    let (_, mut add_ons) = materialize_inserts(
        conn,
        sub_details.subscription.tenant_id,
        sub_details,
        vec![],
        resolved.addon_inserts,
    )
    .await?;
    add_ons
        .pop()
        .ok_or_else(|| Report::new(StoreError::InvalidArgument("no add-on to add".to_string())))
}

async fn reject_if_plan_change_pending(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    tenant_id: TenantId,
) -> StoreResult<()> {
    let pending = crate::services::subscriptions::phases::pending_one_offs_tx(
        conn,
        tenant_id,
        subscription_id,
        &[SubscriptionChangeKindEnum::PlanChange],
    )
    .await?;
    if !pending.is_empty() {
        return Err(Report::new(StoreError::InvalidArgument(
            "A plan change is already scheduled for this subscription; cancel it before amending"
                .to_string(),
        )));
    }
    Ok(())
}

/// Withdraws the amendments still to take effect. Returns how many.
async fn cancel_pending_amendments(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    tenant_id: TenantId,
) -> StoreResult<usize> {
    crate::services::facts::pending::withdraw_pending_of_tx(
        conn,
        tenant_id,
        subscription_id,
        &[SubscriptionChangeKindEnum::Amendment],
        "Cancelled by user",
    )
    .await
}

/// Patch a Slot fee's `initial_slots` with the live count of its quantity from the
/// slot_transaction ledger.
async fn resolve_live_slot_count(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    fee: &mut SubscriptionFee,
) -> StoreResult<()> {
    if let SubscriptionFee::Slot {
        quantity_id: Some(quantity),
        initial_slots,
        ..
    } = fee
    {
        let actual = SlotTransactionRow::fetch_active_slots(
            conn,
            tenant_id,
            subscription_id,
            *quantity,
            common_clock::now().naive_utc(),
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
                    price_component: line.price_component_id,
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
            voided: false,
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
        // A true-up of an earlier period is never credited back; an ending's document may bill
        // that period's charge again, under the same id (R-022, R-075).
        let period_start = invoice.billing_period_start;
        invoice.line_items.retain(|line| {
            line.kind != LineItemKind::MinimumShortfall
                || crate::services::invoice_lines::minimum::is_up_front(line, period_start)
        });

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
pub(in crate::services) async fn load_credit_matching_context(
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
                // A void's own note forgives its invoice (R-096): no correction of its lines.
                if !matches!(
                    cn.origin,
                    diesel_models::enums::CreditNoteOriginEnum::Manual
                        | diesel_models::enums::CreditNoteOriginEnum::Void
                ) {
                    *corrected.entry(l.local_id.clone()).or_insert(0) += -l.amount_subtotal;
                }
                *already_credited.entry(l.local_id).or_insert(0) += -l.amount_subtotal;
            }
        }
    }

    Ok((line_lineage, already_credited, corrected, returned))
}

/// R-106: a row opened on a lineage a change scheduled for a later day re-rows ends where that
/// change's row starts.
async fn under_scheduled_rows(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    on: NaiveDate,
    components: &[SubscriptionComponentRowNew],
    add_ons: &[SubscriptionAddOnRowNew],
) -> StoreResult<()> {
    let db = |e| Into::<Report<StoreError>>::into(e);
    if !components.is_empty() {
        let rows = SubscriptionComponentRow::list_fact_rows(conn, subscription_id)
            .await
            .map_err(db)?;
        for new in components {
            let root = new.lineage_id.unwrap_or(new.id);
            let next = rows
                .iter()
                .filter(|row| row.lineage_id.unwrap_or(row.id) == root && row.effective_from > on)
                .map(|row| row.effective_from)
                .min();
            if let Some(next) = next {
                SubscriptionComponentRow::close_components(conn, &[new.id], next)
                    .await
                    .map_err(db)?;
            }
        }
    }
    if !add_ons.is_empty() {
        let rows = SubscriptionAddOnRow::list_fact_rows(conn, subscription_id)
            .await
            .map_err(db)?;
        for new in add_ons {
            let root = new.lineage_id.unwrap_or(new.id);
            let next = rows
                .iter()
                .filter(|row| row.lineage_id.unwrap_or(row.id) == root && row.effective_from > on)
                .map(|row| row.effective_from)
                .min();
            if let Some(next) = next {
                SubscriptionAddOnRow::close_addons(conn, &[new.id], next)
                    .await
                    .map_err(db)?;
            }
        }
    }
    Ok(())
}

/// A component insert as the event applying the amendment carries it; a price still to be
/// created is left to the fee it was resolved to, which bills the same.
fn previewed_component(insert: &PendingComponentInsert) -> ResolvedComponentInsert {
    ResolvedComponentInsert {
        price_component_id: insert.price_component_id,
        product_id: match &insert.product_ref {
            ProductRef::Existing(id) => Some(*id),
            ProductRef::New { .. } => None,
        },
        name: insert.name.clone(),
        period: insert.period,
        fee: insert.fee.clone(),
        is_override: insert.is_override,
        price_id: insert.price_entry.existing_price_id(),
        lineage_id: insert.lineage_id,
        subscription_component_id: insert.subscription_component_id,
    }
}

/// An add-on insert as the event applying the amendment carries it (see [`previewed_component`]).
fn previewed_add_on(insert: &PendingAddOnInsert) -> ResolvedAddOnInsert {
    ResolvedAddOnInsert {
        add_on_id: insert.add_on_id,
        name: insert.name.clone(),
        period: insert.period,
        fee: insert.fee.clone(),
        product_id: Some(insert.product_id),
        price_id: match &insert.price_entry {
            Some(PriceEntry::New(_)) => None,
            _ => insert.price_id,
        },
        quantity: insert.quantity,
        lineage_id: insert.lineage_id,
        subscription_add_on_id: insert.subscription_add_on_id,
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
        // A new price of the charge keeps its shape: the one its row was resolved from.
        let fee_structure = FeeStructure::of_fee(&current.fee);
        let (mut new_fee, new_period) = resolve_fee_read_only(
            conn,
            &fee_structure,
            &edit.price_entry,
            tenant_id,
            Some(product_id),
            Some(sub_details.subscription.currency.as_str()),
        )
        .await?;
        // A usage-levels charge of the plan moves to an existing price only as another level of
        // its cell; a lower one waits for the period's end (R-135).
        if let PriceEntry::Existing(price_id) = &edit.price_entry
            && current.price_component_id.is_some()
            && matches!(current.fee, SubscriptionFee::Capacity { .. })
        {
            use crate::services::subscriptions::levels;
            let cell = levels::levels_of(conn, &sub_details.subscription, current).await?;
            let (_, _, direction) = levels::level_direction(current, &cell, *price_id)?;
            let at_period_end = sub_details.subscription.current_period_end == Some(effective_date);
            if direction == crate::domain::subscription_changes::ChangeDirection::Downgrade
                && !at_period_end
            {
                return Err(Report::new(StoreError::FailedPrecondition(
                    levels::LOWER_LEVEL_AT_PERIOD_END.to_string(),
                )));
            }
        }

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
            fee_structure,
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
        let fee_structure = crate::repositories::price_components::extra_charge_structure(
            conn,
            tenant_id,
            &extra.product,
            &extra.price_entry,
            extra.fee_structure.as_ref(),
        )
        .await?;
        let (fee, period) = resolve_fee_read_only(
            conn,
            &fee_structure,
            &extra.price_entry,
            tenant_id,
            extra.product.existing_product_id(),
            Some(sub_details.subscription.currency.as_str()),
        )
        .await?;

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
            fee_structure,
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

    let (addons, addon_prices, addon_units) =
        load_addon_catalog(conn, tenant_id, &catalog_addon_ids, amendment).await?;
    let sold_in = crate::services::subscriptions::currency::SubscriptionCurrency::new(
        &sub_details.subscription.currency,
    );
    let term = Some(sub_details.subscription.period);

    // A seat add-on is held once (R-138): not added again while kept, nor twice here.
    let held: Vec<AddOnId> = sub_details
        .add_ons
        .iter()
        .filter(|a| !amendment.add_on_changes.removed.contains(&a.id))
        .map(|a| a.add_on_id)
        .collect();
    crate::services::subscriptions::utils::ensure_seat_add_ons_once(
        amendment
            .add_on_changes
            .added
            .iter()
            .filter_map(|cs_ao| addons.iter().find(|a| a.id == cs_ao.add_on_id)),
        &held,
    )?;

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
        crate::services::subscriptions::utils::ensure_seat_add_on_instances(addon, cs_ao.quantity)?;
        let resolved = resolve_add_on_sold_in(
            addon,
            &addon_prices,
            &addon_units,
            &cs_ao.customization,
            &sold_in,
            term,
        )?;

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
            fee_structure: addon.fee_structure.clone(),
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
        if let Some(addon) = addon {
            crate::services::subscriptions::utils::ensure_seat_add_on_instances(
                addon,
                new_quantity,
            )?;
        }

        // New fee/period: either re-resolve from a new customization, or keep current.
        let (new_fee, new_period, new_name, price_entry, price_id, product_id) =
            if let (Some(addon), Some(cust)) = (addon, &e.customization) {
                let resolved = resolve_add_on_sold_in(
                    addon,
                    &addon_prices,
                    &addon_units,
                    cust,
                    &sold_in,
                    term,
                )?;
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
            fee_structure: addon
                .and_then(|a| a.fee_structure.clone())
                .or_else(|| Some(FeeStructure::of_fee(&current.fee))),
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

#[allow(clippy::type_complexity)]
async fn load_addon_catalog(
    conn: &mut PgConn,
    tenant_id: TenantId,
    add_on_ids: &[AddOnId],
    amendment: &SubscriptionAmendment,
) -> StoreResult<(
    Vec<crate::domain::add_ons::AddOn>,
    HashMap<PriceId, crate::domain::prices::Price>,
    crate::domain::credit_units::CreditUnitsById,
)> {
    if add_on_ids.is_empty() {
        return Ok((
            Vec::new(),
            HashMap::new(),
            crate::domain::credit_units::CreditUnitsById::new(),
        ));
    }

    let rows = diesel_models::add_ons::AddOnRow::list_by_ids(conn, add_on_ids, &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let addons = crate::repositories::add_ons::enrich_add_ons(conn, rows, tenant_id).await?;

    // Collect price ids: the add-on definitions + any Existing override prices.
    let mut price_ids: Vec<PriceId> = addons.iter().flat_map(|a| a.price_ids()).collect();

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

    // fetch_prices dedups the ids internally.
    let prices = fetch_prices(conn, tenant_id, price_ids.into_iter()).await?;
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

    Ok((addons, prices, units))
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
            fee_structure: Some(c.fee_structure.clone()),
            prices: vec![c.price_entry.clone()],
        };
        let charge = resolve_component_internal(
            conn,
            &internal,
            tenant_id,
            product_family_id,
            currency,
            false,
        )
        .await?;
        let price_id = charge.price_ids.into_iter().next();

        resolved_components.push(ResolvedComponentInsert {
            price_component_id: c.price_component_id,
            product_id: Some(charge.product_id),
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
                fee_structure: a.fee_structure.clone(),
                prices: vec![a.price_entry.clone().unwrap()],
            };
            let charge = resolve_component_internal(
                conn,
                &internal,
                tenant_id,
                product_family_id,
                currency,
                false,
            )
            .await?;
            (Some(charge.product_id), charge.price_ids.into_iter().next())
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
    use crate::domain::subscription_components::SubscriptionFee;
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
            quantity_id: None,
            model: crate::domain::price_components::RatingModel::PerUnit {
                rate: Decimal::from(5),
            },
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
    fn scale_modelled_fixed_fees_costs_each_instance_what_the_model_prices() {
        use crate::domain::price_components::{RatingModel, fixed_quantity_amount};
        let package = RatingModel::Package {
            block_size: 10,
            rate: Decimal::from(30),
        };
        let one_time = SubscriptionFee::OneTime {
            rate: Decimal::ZERO,
            quantity: 25,
            model: Some(package.clone()),
        };
        match scale_fee(&one_time, 3) {
            SubscriptionFee::OneTime {
                rate,
                quantity,
                model,
            } => {
                assert_eq!(quantity, 25);
                assert_eq!(
                    fixed_quantity_amount(rate, Decimal::from(quantity), model.as_ref()),
                    Decimal::from(270)
                );
            }
            _ => panic!("expected one-time"),
        }
        let recurring = SubscriptionFee::Recurring {
            rate: Decimal::ZERO,
            quantity: 25,
            billing_type: crate::domain::enums::BillingType::Advance,
            model: Some(package),
        };
        match scale_fee(&recurring, 2) {
            SubscriptionFee::Recurring {
                rate,
                quantity,
                model,
                ..
            } => assert_eq!(
                fixed_quantity_amount(rate, Decimal::from(quantity), model.as_ref()),
                Decimal::from(180)
            ),
            _ => panic!("expected recurring"),
        }
    }

    #[test]
    fn scale_quantity_one_is_identity() {
        match scale_fee(&rate(7), 1) {
            SubscriptionFee::Rate { rate } => assert_eq!(rate, Decimal::from(7)),
            _ => panic!("expected rate"),
        }
    }
}
