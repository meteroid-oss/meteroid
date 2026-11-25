//! Previews of what is not written yet: the subscription's facts with the rows a change would
//! write laid over them, evaluated at the change's instant and billed through the same pipeline
//! as the change itself (evaluate, reconcile, assemble, settle). Nothing is written.

use std::collections::HashMap;

use billing_core::charge::ChargeKey;
use billing_core::evaluate::{Charge, DocumentSlot, evaluate};
use chrono::{Datelike, NaiveDate, NaiveTime};
use common_domain::actor::ActorType;
use common_domain::ids::{
    BaseId, InvoiceId, PlanVersionId, PriceComponentId, ScheduledEventId, SubscriptionId, TenantId,
};
use common_utils::decimals::ToSubunit;
use error_stack::Report;
use rust_decimal::Decimal;

use super::bill::{
    Order, OrderKey, as_written, evaluated_at, lineages_of, midnight, nominals, render, row_of,
};
use super::change::{Change, GiveBack, change_documents, credit_context_of};
use super::load::{LoadedFacts, Overlay, facts_of_details, load_facts_on_rows, load_facts_with};
use super::pricer::StorePricer;
use super::usage::read_usage;
use crate::StoreResult;
use crate::constants::Currencies;
use crate::domain::enums::InvoiceType;
use crate::domain::scheduled_events::{ScheduledEvent, ScheduledEventData};
use crate::domain::slot_transactions::SlotUpdatePreview;
use crate::domain::subscription_add_ons::CreateSubscriptionAddOn;
use crate::domain::{LineItem, SubscriptionAddOn, SubscriptionDetails, SubscriptionFee};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::credit_notes::CreditLineItem;
use crate::repositories::subscriptions::SubscriptionSlotsInterface;
use crate::services::Services;
use crate::services::credit::forecast::CreditBooks;
use crate::services::facts::pending::{amendment_cadence_window_tx, resolve_change_tx};
use crate::services::invoice_lines::charges::{Lineages, disambiguate};
use crate::services::invoice_lines::credit_units::CreditUnitCover;
use crate::services::invoice_lines::fees::pricing_error;
use crate::services::invoice_lines::invoice_lines::{
    BilledSpan, ComputedInvoiceContent, PricedLines, apply_temporal_date_range_to_names, totals_of,
};
use crate::services::invoices::upcoming::upcoming_invoice_details_on;
use crate::services::subscriptions::amendment::PeriodCreditContext;
use crate::services::subscriptions::plan_change::PreparedPlanChange;
use crate::store::PgConn;

/// What a change not yet written issues on its date.
#[derive(Clone, Debug)]
pub(in crate::services) struct ChangeDocuments {
    /// The adjustment invoice, taxed, the credit it draws estimated, what the change's notes
    /// return to the balance among it.
    pub adjustment: Option<ComputedInvoiceContent>,
    /// What the notes take back of the finalized invoices, as one document.
    pub credit_note: Option<ComputedInvoiceContent>,
    /// What the adjustment bills, before tax.
    pub charged: i64,
    /// What reconcile takes back of issued lines, net.
    pub credited: i64,
}

impl ChangeDocuments {
    /// What paying for the change settles: its adjustment, nothing when it bills nothing.
    pub(in crate::services) fn quote(&self) -> ComputedInvoiceContent {
        self.adjustment
            .clone()
            .unwrap_or_else(ComputedInvoiceContent::empty)
    }
}

/// A change requested for `on`, as the event scheduling it would carry it: never stored.
pub(in crate::services) fn change_event(
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    on: NaiveDate,
    event_data: ScheduledEventData,
) -> ScheduledEvent {
    let now = common_clock::now().naive_utc();
    ScheduledEvent {
        id: ScheduledEventId::new(),
        subscription_id,
        tenant_id,
        event_type: event_data.to_event_type_enum(),
        scheduled_time: on.and_time(NaiveTime::MIN),
        event_data,
        created_at: now,
        updated_at: now,
        withdrawn_at: None,
        source: "preview".to_owned(),
        created_by_customer: false,
        actor_type: ActorType::System,
        actor_uuid: None,
        actor_alias: None,
    }
}

/// The subscription as a change leaves it from its date, in memory.
pub(in crate::services) fn details_after(
    details: &SubscriptionDetails,
    overlay: &Overlay,
) -> StoreResult<SubscriptionDetails> {
    let rows = &overlay.rows;
    let mut after = details.clone();
    after
        .price_components
        .retain(|c| !rows.closes_components.contains(&c.id));
    after
        .add_ons
        .retain(|a| !rows.closes_add_ons.contains(&a.id));
    for row in &rows.opens_components {
        after
            .price_components
            .push(super::load::component_row_of(row).try_into()?);
    }
    for row in &rows.opens_add_ons {
        after
            .add_ons
            .push(super::load::add_on_row_of(row).try_into()?);
    }
    if let Some(window) = &rows.plan_window {
        after.subscription.plan_version_id = window.plan_version_id;
        after.subscription.period = window.cadence;
    }
    Ok(after)
}

/// The keys of `charges`, in their order.
fn keys_in(charges: &[&Charge]) -> Vec<ChargeKey> {
    let mut keys: Vec<ChargeKey> = Vec::new();
    for charge in charges {
        if !keys.contains(&charge.key) {
            keys.push(charge.key.clone());
        }
    }
    keys
}

/// The lines of `charges` for the amounts `to_bill`, in the order a period invoice lists them.
fn rendered_lines(
    loaded: &LoadedFacts,
    details: &SubscriptionDetails,
    charges: &[&Charge],
    to_bill: &[(ChargeKey, i64)],
    precision: u8,
) -> StoreResult<Vec<LineItem>> {
    let sub = &details.subscription;
    let lineages = lineages_of(&loaded.facts);
    let order = Order::of(details, &loaded.facts);
    let mut rendered: Vec<(OrderKey, LineItem)> = Vec::new();
    for (key, amount) in to_bill {
        let of_key: Vec<&Charge> = charges
            .iter()
            .copied()
            .filter(|charge| charge.key == *key)
            .collect();
        let Some(first) = of_key.first().copied() else {
            continue;
        };
        let Some(row) = row_of(&loaded.facts, first) else {
            continue;
        };
        let line = render(
            &of_key,
            *amount,
            row,
            precision,
            sub.id,
            &HashMap::new(),
            &lineages,
        )?;
        rendered.push((order.key(row, first), line));
    }
    rendered.sort_by(|a, b| a.0.cmp(&b.0));
    let mut lines: Vec<LineItem> = rendered.into_iter().map(|(_, line)| line).collect();
    apply_temporal_date_range_to_names(&mut lines);
    disambiguate(&mut lines, &Lineages::of(details));
    Ok(lines)
}

impl Services {
    /// The rows `event` would write on its date, laid over the stored ones: resolved as a
    /// scheduled change is written, read and never written.
    pub(in crate::services) async fn overlay_of_change(
        &self,
        conn: &mut PgConn,
        event: &ScheduledEvent,
    ) -> StoreResult<Overlay> {
        let rows = match resolve_change_tx(conn, event).await? {
            Ok(rows) => rows,
            Err(why) => {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "the change cannot be previewed: {why:?}"
                ))));
            }
        };
        let mut overlay = Overlay::of(rows);
        if overlay.rows.plan_window.is_none()
            && matches!(event.event_data, ScheduledEventData::ApplyAmendment { .. })
        {
            overlay.rows.plan_window = amendment_cadence_window_tx(
                conn,
                event.tenant_id,
                event.subscription_id,
                overlay.rows.on,
                Some(&overlay.rows),
            )
            .await?;
        }
        Ok(overlay)
    }

    /// What `change` issues on its date, its rows laid over the facts: what the change itself
    /// computes (`change_documents`), its adjustment taxed and its notes as one document, with
    /// `also` credited on them (the old packs' fees). `details` are the subscription before it.
    pub(in crate::services) async fn preview_change_documents(
        &self,
        conn: &mut PgConn,
        details: &SubscriptionDetails,
        overlay: &Overlay,
        change: Change,
        also: Vec<(InvoiceId, CreditLineItem)>,
    ) -> StoreResult<ChangeDocuments> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let sub = &details.subscription;
            let documents = change_documents(
                conn,
                sub.tenant_id,
                sub.id,
                &change,
                Some(overlay),
                Some(self.usage_client.as_ref()),
            )
            .await?;
            self.preview_documents(conn, details, change.on, documents, also)
                .await
        })
        .await
    }

    /// What `documents`, those of a change on `on` not written yet, issue: the adjustment taxed
    /// and its credit estimated, the notes as one document with `also` credited on them. A change
    /// and a seat purchase preview through it.
    pub(in crate::services) async fn preview_documents(
        &self,
        conn: &mut PgConn,
        details: &SubscriptionDetails,
        on: NaiveDate,
        documents: super::change::ChangeDocuments,
        also: Vec<(InvoiceId, CreditLineItem)>,
    ) -> StoreResult<ChangeDocuments> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let sub = &details.subscription;
            let (tenant_id, subscription_id) = (sub.tenant_id, sub.id);
            let precision = Currencies::resolve_currency_precision(&sub.currency).unwrap_or(2);
            let credits: Vec<(InvoiceId, CreditLineItem)> =
                documents.credits.into_iter().chain(also).collect();
            let mut invoices: Vec<InvoiceId> = credits.iter().map(|(id, _)| *id).collect();
            invoices.sort_by_key(|id| id.as_uuid());
            invoices.dedup();
            let ctx = credit_context_of(conn, tenant_id, subscription_id, &invoices).await?;
            let items: Vec<CreditLineItem> = credits.into_iter().map(|(_, item)| item).collect();
            let (credit_note, to_balance) =
                credit_note_preview(conn, tenant_id, &ctx, &items, precision).await?;

            let mut adjustment = if documents.bills.is_empty() {
                None
            } else {
                Some(
                    self.change_adjustment_content(
                        conn,
                        sub,
                        &details.customer,
                        on,
                        &documents.bills,
                    )
                    .await?
                    .computed,
                )
            };
            // The notes go first: what they return to the balance pays the adjustment.
            if let Some(adjustment) = adjustment.as_mut() {
                let drawn = to_balance.min(adjustment.amount_due).max(0);
                adjustment.applied_credits += drawn;
                adjustment.amount_due -= drawn;
            }
            Ok(ChangeDocuments {
                adjustment,
                credit_note,
                charged: documents.charged,
                credited: documents.credited,
            })
        })
        .await
    }

    /// The renewal once a change is in force: `after` renewed at the end of its period, its rows
    /// laid over the facts.
    pub(in crate::services) async fn preview_next_invoice(
        &self,
        conn: &mut PgConn,
        after: &SubscriptionDetails,
        overlay: &Overlay,
        book: Option<&CreditBooks>,
    ) -> StoreResult<ComputedInvoiceContent> {
        let (renewal, invoice_date) = upcoming_invoice_details_on(conn, after).await?;
        self.compute_invoice_from_facts_with(
            conn,
            &invoice_date,
            &renewal,
            None,
            None,
            InvoiceType::Recurring,
            CreditUnitCover::Estimate { invoice_id: None },
            book,
            Some(overlay),
        )
        .await
    }

    /// The rows an immediate plan change prepared as `prepared` writes on `change_date`: those a
    /// scheduled one would, a component billed as before continuing its lineage, and a change
    /// ending a free trial starting the grid on its day.
    pub(in crate::services) async fn plan_change_overlay(
        &self,
        conn: &mut PgConn,
        prepared: &PreparedPlanChange,
        new_plan_version_id: PlanVersionId,
        change_date: NaiveDate,
    ) -> StoreResult<Overlay> {
        let sub = &prepared.subscription_details.subscription;
        let event = change_event(
            sub.tenant_id,
            sub.id,
            change_date,
            ScheduledEventData::ApplyPlanChange {
                source_plan_version_id: Some(sub.plan_version_id),
                new_plan_version_id,
                new_plan_option_id: prepared.target_option.as_ref().map(|o| o.id),
                component_mappings: prepared.component_mappings().to_vec(),
            },
        );
        let mut overlay = self.overlay_of_change(conn, &event).await?;
        for row in &mut overlay.rows.opens_components {
            if let Some(root) = row
                .price_component_id
                .and_then(|target: PriceComponentId| prepared.carried.get(&target))
            {
                row.lineage_id = Some(*root);
            }
        }
        if prepared.is_trial() && prepared.is_free_trial() {
            if let Some(window) = overlay.rows.plan_window.as_mut() {
                window.cadence_origin = window.from_date;
            }
            overlay.billing_day_anchor = Some(change_date.day());
        }
        Ok(overlay)
    }

    /// What paying for an immediate plan change settles: the first invoice of the new plan when
    /// it ends a free trial, else its adjustment, after what its notes return to the balance.
    pub(in crate::services) async fn plan_change_quote(
        &self,
        conn: &mut PgConn,
        prepared: &PreparedPlanChange,
        new_plan_version_id: PlanVersionId,
        change_date: NaiveDate,
    ) -> StoreResult<ComputedInvoiceContent> {
        let overlay = self
            .plan_change_overlay(conn, prepared, new_plan_version_id, change_date)
            .await?;
        if prepared.is_free_trial() {
            let after = details_after(&prepared.subscription_details, &overlay)?;
            return self
                .compute_invoice_from_facts_with(
                    conn,
                    &change_date,
                    &after,
                    None,
                    None,
                    InvoiceType::Recurring,
                    CreditUnitCover::Estimate { invoice_id: None },
                    None,
                    Some(&overlay),
                )
                .await;
        }
        let packs = self
            .outgoing_pack_credits_tx(conn, prepared, change_date)
            .await?;
        Ok(self
            .preview_change_documents(
                conn,
                &prepared.subscription_details,
                &overlay,
                Change::of_rows(change_date, GiveBack::Unused, "plan change"),
                packs,
            )
            .await?
            .quote())
    }

    /// What buying `add_ons` on `on` bills, as the purchase's completion prices it; and the
    /// add-ons as the purchase stores them.
    pub(in crate::services) async fn add_on_purchase_quote(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        add_ons: &[CreateSubscriptionAddOn],
        on: NaiveDate,
    ) -> StoreResult<(ComputedInvoiceContent, Vec<SubscriptionAddOn>)> {
        let rows = crate::repositories::subscription_add_ons::checkout_add_on_rows(
            conn,
            tenant_id,
            subscription_id,
            add_ons,
            on,
        )
        .await?;
        let bought = rows
            .iter()
            .map(|row| super::load::add_on_row_of(row).try_into())
            .collect::<StoreResult<Vec<SubscriptionAddOn>>>()?;
        let quote = self
            .compute_addon_purchase_invoice(conn, tenant_id, subscription_id, &rows, on)
            .await?
            .invoice_content;
        Ok((quote, bought))
    }

    /// A change of the seats of the slot fee of `charge` (a plan charge or an add-on), by a
    /// number or to a count, priced today: an increase recorded today laid over the facts and
    /// billed as the change bills it, by every fee counting its quantity; a decrease taking
    /// effect at the end of the fee's period, billing nothing now. Its delta is counted as the
    /// change counts it (`seats_counted`): a count holds the seats still paying as held.
    pub(in crate::services) async fn preview_seat_change(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        charge: impl Into<crate::domain::slot_transactions::SeatCharge>,
        change: crate::domain::managed_quantities::SeatChange,
    ) -> StoreResult<SlotUpdatePreview> {
        let charge = charge.into();
        if change == crate::domain::managed_quantities::SeatChange::By(0) {
            return Err(StoreError::InvalidArgument(
                "Delta cannot be zero - no slot change requested".to_string(),
            )
            .into());
        }
        let mut conn = self.store.get_conn().await?;
        let details = self
            .store
            .get_subscription_details_with_conn(&mut conn, tenant_id, subscription_id)
            .await?;
        let sub = &details.subscription;
        let (lineage, fee) =
            crate::services::subscriptions::slots::seat_charge_of(&details, charge)?;
        let SubscriptionFee::Slot { unit, model, .. } = fee else {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "{charge:?} does not charge per seat"
            ))));
        };
        let slot = self.extract_slot(&details, charge)?;
        let current = self
            .store
            .get_active_slots_value_with_conn(
                &mut conn,
                tenant_id,
                subscription_id,
                slot.quantity_id,
                None,
            )
            .await? as i32;
        let (held, delta) = crate::services::subscriptions::slots::seats_counted(
            &mut conn,
            tenant_id,
            subscription_id,
            slot.quantity_id,
            change,
            current,
        )
        .await?;
        let delta = delta.ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(format!(
                "the subscription already holds {held} {}",
                slot.unit
            )))
        })?;
        // Seats bought and still paying count against the maximum, as the change counts them.
        let paying = if delta > 0 {
            diesel_models::slot_transactions::SlotTransactionRow::pending_upgrades(
                &mut conn,
                tenant_id,
                subscription_id,
                slot.quantity_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        } else {
            0
        };
        crate::repositories::subscriptions::slots::validate_slot_limits(
            &slot,
            delta,
            current + paying,
        )?;
        let precision = Currencies::resolve_currency_precision(&sub.currency).unwrap_or(2);
        let today = common_clock::now().date_naive();
        let new_slots = current + delta;
        let period_total = |count: i32| {
            Decimal::new(
                model.period_total_minor(u64::try_from(count).unwrap_or(0), precision),
                u32::from(precision),
            )
        };
        let full_period_amount = period_total(new_slots) - period_total(current);
        let unit_rate = model
            .per_unit_rate()
            .unwrap_or_else(|| full_period_amount / Decimal::from(delta));

        // The fee's period holding today, from its charge there.
        let loaded =
            load_facts_with(&mut conn, tenant_id, subscription_id, today, None, None).await?;
        let evaluation =
            evaluate(&loaded.facts, midnight(today), None, &StorePricer).map_err(pricing_error)?;
        let (period_start, period_end) = evaluation
            .charges
            .iter()
            .filter(|charge| charge.key.fee == lineage)
            .find(|charge| charge.key.from <= today && today < charge.key.until)
            .map(|charge| (charge.key.from, charge.key.until))
            .or_else(|| {
                sub.current_period_end
                    .map(|end| (sub.current_period_start, end))
            })
            .ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(
                    "Cannot modify slots for subscription without active billing period"
                        .to_string(),
                ))
            })?;

        let deferred = crate::services::subscriptions::slots::deferred_to_period_end(
            &mut conn,
            tenant_id,
            slot.quantity_id,
            delta,
        )
        .await?;
        let (prorated_amount, adjustment_invoice) = if deferred {
            (Decimal::ZERO, None)
        } else if delta < 0 {
            // Removed at once, what their unused time gives back.
            let given_back = self
                .seat_purchase_documents(&mut conn, &details, slot.quantity_id, delta, today)
                .await?;
            let given_back = Decimal::new(
                given_back.map_or(0, |given_back| given_back.purchase.amount.min(0)),
                u32::from(precision),
            );
            (given_back, None)
        } else if delta > 0 {
            // What the purchase bills, from the facts, and the adjustment it issues for it.
            let bought = self
                .seat_purchase_documents(&mut conn, &details, slot.quantity_id, delta, today)
                .await?;
            let prorated = Decimal::new(
                bought
                    .as_ref()
                    .map_or(0, |bought| bought.purchase.amount.max(0)),
                u32::from(precision),
            );
            let documents = bought.map(|bought| bought.documents).unwrap_or_default();
            let previewed = self
                .preview_documents(&mut conn, &details, today, documents, Vec::new())
                .await?;
            (prorated, previewed.adjustment)
        } else {
            (Decimal::ZERO, None)
        };

        // The MRR once the count is held: every fee counting the quantity bills it (R-137).
        let mrr_after_cents = match self
            .mrr_after_change(
                &mut conn,
                &details,
                &details,
                today,
                HashMap::from([(slot.quantity_id, i64::from(new_slots))]),
            )
            .await
        {
            Ok(mrr) => Some(mrr),
            Err(error) => {
                log::warn!(
                    "The MRR after a seat change of subscription {subscription_id} was not previewed: {error:?}"
                );
                None
            }
        };

        Ok(SlotUpdatePreview {
            mrr_after_cents,
            current_slots: current,
            new_slots,
            delta,
            unit: unit.clone(),
            unit_rate,
            prorated_amount,
            full_period_amount,
            days_remaining: (period_end - today).num_days() as i32,
            days_total: (period_end - period_start).num_days() as i32,
            effective_at: if deferred { period_end } else { today },
            current_period_end: period_end,
            next_invoice_delta: prorated_amount,
            adjustment_invoice,
        })
    }

    /// The first invoice of a subscription not created yet, from the facts `details` would
    /// record: its opening charges on its billing start, rendered, discounted, taxed and its
    /// credit estimated as the period invoices are.
    pub(in crate::services) async fn preview_new_subscription_invoice(
        &self,
        conn: &mut PgConn,
        details: &SubscriptionDetails,
    ) -> StoreResult<ComputedInvoiceContent> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let sub = &details.subscription;
            let currency = Currencies::resolve_currency(&sub.currency).ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(format!(
                    "Currency {} not found",
                    sub.currency
                )))
            })?;
            let loaded = facts_of_details(conn, details).await?;
            let start = loaded.facts.start.date;
            let opening = DocumentSlot::Recurring {
                period_start: start,
            };
            let evaluation = evaluate(&loaded.facts, midnight(start), None, &StorePricer)
                .map_err(pricing_error)?;
            let charges: Vec<&Charge> = evaluation
                .charges
                .iter()
                .filter(|charge| charge.document == opening && !charge.is_commitment())
                .collect();
            let to_bill: Vec<(ChargeKey, i64)> = keys_in(&charges)
                .into_iter()
                .map(|key| {
                    let amount = charges
                        .iter()
                        .filter(|charge| charge.key == key)
                        .map(|charge| charge.amount)
                        .sum();
                    (key, amount)
                })
                .collect();
            let lines = rendered_lines(&loaded, details, &charges, &to_bill, currency.precision)?;
            let mut priced = PricedLines {
                lines,
                invoice_date: start,
                currency_precision: currency.precision,
                tax_inclusive: sub.tax_inclusive,
                invoice_type: InvoiceType::Recurring,
                prepaid_amount: None,
                invoice_id: None,
                true_up: None,
                reopened: Box::default(),
                span: BilledSpan::of(&InvoiceType::Recurring),
            };
            let cover = CreditUnitCover::Estimate { invoice_id: None };
            self.cover_credit_unit_lines_on(conn, cover, &mut [priced.cover_target(details)], None)
                .await?;
            self.finish_invoice_content(conn, details, priced, common_clock::now(), None)
                .await
        })
        .await
    }

    /// The stored subscription's recurring invoice of `period_start` billed on the rows of
    /// `details` instead of its own (`load_facts_on_rows`): every charge of the period with its
    /// usage, reconciled against no issued invoice, the minimum trued up as on `details`.
    pub(in crate::services) async fn preview_period_invoice_on_rows(
        &self,
        conn: &mut PgConn,
        details: &SubscriptionDetails,
        period_start: NaiveDate,
    ) -> StoreResult<ComputedInvoiceContent> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let sub = &details.subscription;
            let currency = Currencies::resolve_currency(&sub.currency).ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(format!(
                    "Currency {} not found",
                    sub.currency
                )))
            })?;
            let mut loaded = load_facts_on_rows(conn, details, period_start).await?;
            let now = evaluated_at(&loaded.facts, period_start);
            as_written(&mut loaded.facts, now);
            let slot = DocumentSlot::Recurring { period_start };
            let planned =
                evaluate(&loaded.facts, now, None, &StorePricer).map_err(pricing_error)?;
            let needs: Vec<_> = planned
                .usage_needed
                .iter()
                .filter(|need| need.document == slot && need.read.window.until <= now)
                .cloned()
                .collect();
            let reads = read_usage(self.usage_client.as_ref(), &loaded, &needs).await?;
            let evaluation =
                evaluate(&loaded.facts, now, Some(&reads), &StorePricer).map_err(pricing_error)?;
            let charges: Vec<&Charge> = evaluation
                .charges
                .iter()
                .filter(|charge| charge.document == slot && !charge.is_commitment())
                .collect();
            let to_bill: Vec<(ChargeKey, i64)> = keys_in(&charges)
                .into_iter()
                .map(|key| {
                    let amount = charges
                        .iter()
                        .filter(|charge| charge.key == key)
                        .map(|charge| charge.amount)
                        .sum();
                    (key, amount)
                })
                .collect();
            let lines = rendered_lines(&loaded, details, &charges, &to_bill, currency.precision)?;
            let nominals = nominals(details, &loaded.facts, &planned, currency.precision);
            let true_up = crate::services::invoice_lines::minimum::load_period_true_up_priced(
                conn,
                details,
                period_start,
                sub.cycle_index.unwrap_or(0),
                None,
                &nominals,
            )
            .await?;
            let mut priced = PricedLines {
                lines,
                invoice_date: period_start,
                currency_precision: currency.precision,
                tax_inclusive: sub.tax_inclusive,
                invoice_type: InvoiceType::Recurring,
                prepaid_amount: None,
                invoice_id: None,
                true_up,
                reopened: Box::default(),
                span: BilledSpan::of(&InvoiceType::Recurring),
            };
            let cover = CreditUnitCover::Estimate { invoice_id: None };
            self.cover_credit_unit_lines_on(conn, cover, &mut [priced.cover_target(details)], None)
                .await?;
            self.finish_invoice_content(conn, details, priced, common_clock::now(), None)
                .await
        })
        .await
    }
}

/// What the notes of `credits` take back, as one document, and what they return to the balance:
/// a note on an invoice paid, being paid or with nothing left to collect credits the balance.
async fn credit_note_preview(
    conn: &mut PgConn,
    tenant_id: TenantId,
    ctx: &PeriodCreditContext,
    credits: &[CreditLineItem],
    precision: u8,
) -> StoreResult<(Option<ComputedInvoiceContent>, i64)> {
    if ctx.invoices.is_empty() || credits.is_empty() {
        return Ok((None, 0));
    }
    let line_by_id: HashMap<&str, &LineItem> = ctx
        .all_lines
        .iter()
        .map(|l| (l.local_id.as_str(), l))
        .collect();
    // Each credit as the issued note resolves it, refusing what it would refuse.
    let mut credited = Vec::new();
    for item in credits {
        let Some(orig) = line_by_id.get(item.local_id()).copied() else {
            continue;
        };
        let already = ctx
            .already_credited
            .get(&orig.local_id)
            .copied()
            .unwrap_or(0);
        let remaining = (orig.amount_subtotal - already).max(0);
        let amount = match item {
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
        return Ok((None, 0));
    }

    let mut to_balance = 0;
    for invoice in &ctx.invoices {
        let returned: i64 = lines
            .iter()
            .filter(|line| ctx.line_to_invoice.get(&line.local_id) == Some(&invoice.id))
            .map(|line| line.amount_total)
            .sum();
        if returned == 0 {
            continue;
        }
        to_balance += crate::repositories::credit_notes::change_note_to_balance(
            conn, tenant_id, invoice, returned,
        )
        .await?;
    }

    let promotional_credits = crate::services::credit::promotional::net_total(&lines)?;
    let discount = lines
        .iter()
        .map(|l| l.amount_subtotal - l.taxable_amount)
        .sum::<i64>()
        - promotional_credits;
    let totals = totals_of(&lines, discount);
    // The note states the tax position of the invoices it corrects, in their words.
    let mut tax_breakdown = crate::repositories::invoices::compute_tax_breakdown(&lines);
    crate::repositories::invoices::carry_over_tax_findings(
        &ctx.invoices
            .iter()
            .flat_map(|invoice| invoice.tax_breakdown.iter().cloned())
            .collect::<Vec<_>>(),
        &mut tax_breakdown,
    );
    Ok((
        Some(ComputedInvoiceContent {
            amount_due: 0,
            ..ComputedInvoiceContent::of(lines, totals, vec![], tax_breakdown, 0, 0)
        }),
        to_balance,
    ))
}
