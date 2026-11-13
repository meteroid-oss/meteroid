use crate::StoreResult;
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::{InvoiceEvent, OutboxEvent};
use crate::domain::{
    CouponLineItem, Invoice, InvoiceLinesPatch, InvoiceNew, InvoicePaymentStatus,
    InvoiceStatusEnum, InvoiceType, LineItem, TaxBreakdownItem,
};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use crate::repositories::invoices::insert_invoice_tx;
use crate::services::Services;
use crate::services::credit::promotional::TaxedDocument;
use crate::services::credit::value_application::{EstimateContext, billed_periods};
use crate::services::invoice_lines::credit_units::CreditUnitCover;
use crate::services::invoice_lines::invoice_lines::{ComputedInvoiceContent, PricedInvoice};
use crate::store::PgConn;
use chrono::NaiveTime;
use common_domain::ids::{BaseId, CustomerId, InvoiceId, InvoicingEntityId, TenantId};
use diesel_models::invoices::InvoiceRow;
use diesel_models::invoices::InvoiceRowLinesPatch;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::slot_transactions::SlotTransactionRow;
use error_stack::Report;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

impl Services {
    /// Entry point used by the `FinalizeInvoice` scheduled event.
    ///
    /// Either finalizes the triggering recurring draft on its own, or — when it has eligible
    /// siblings for the same customer sharing the merge key (same invoice date, currency,
    /// payment method, auto-advance flag and invoicing entity) — merges them into a single
    /// consolidated invoice which becomes the one finalized and charged.
    ///
    /// The per-subscription drafts are retained as consolidated children (linked via
    /// `consolidated_into_invoice_id`) so their MRR movements and per-subscription idempotency
    /// stay intact; only the consolidated parent is finalized/charged/rendered.
    pub(in crate::services) async fn consolidate_and_finalize(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        trigger_invoice_id: InvoiceId,
    ) -> StoreResult<()> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let trigger: Invoice = InvoiceRow::find_by_id(conn, tenant_id, trigger_invoice_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .try_into()?;

            // Already merged into a parent: the parent handles finalization, nothing to do.
            if trigger.is_consolidated_child() {
                return Ok(());
            }
            // Only pending drafts participate; if it was finalized meanwhile, do nothing.
            if trigger.status != InvoiceStatusEnum::Draft {
                return Ok(());
            }
            // An operator edited this draft's billing content: the grace expiry finalizes exactly what
            // they previewed — no refresh (which would regenerate usage lines and drop the manual
            // discount from the total), no Close, and no merge burying the hand-tuned lines in a
            // parent. Mirrors the manual-finalize routing on `manually_edited_at`.
            if trigger.manually_edited_at.is_some() {
                return self
                    .finalize_invoice_tx(
                        conn,
                        &Actor::System,
                        trigger_invoice_id,
                        tenant_id,
                        false,
                        &None,
                    )
                    .await
                    .map(|_| ());
            }
            // A final invoice never merges: it settles alone, as a period does.
            if trigger.invoice_type == InvoiceType::Final
                && !trigger.manual
                && trigger.subscription_id.is_some()
            {
                return self
                    .settle_recurring_invoice_tx(
                        conn,
                        &Actor::System,
                        trigger_invoice_id,
                        tenant_id,
                        true,
                        &None,
                    )
                    .await
                    .map(|_| ());
            }
            // Anything that is not a plain per-subscription recurring draft is finalized as-is.
            if trigger.invoice_type != InvoiceType::Recurring
                || trigger.manual
                || trigger.parent_invoice_id.is_some()
                || trigger.subscription_id.is_none()
            {
                return self
                    .finalize_invoice_tx(
                        conn,
                        &Actor::System,
                        trigger_invoice_id,
                        tenant_id,
                        true,
                        &None,
                    )
                    .await
                    .map(|_| ());
            }

            // Opt-in: when the invoicing entity hasn't enabled consolidation, finalize singly.
            let invoicing_entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                conn,
                trigger.invoicing_entity_id,
                tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
            if !invoicing_entity.consolidate_recurring_invoices {
                return self
                    .settle_recurring_invoice_tx(
                        conn,
                        &Actor::System,
                        trigger_invoice_id,
                        tenant_id,
                        true,
                        &None,
                    )
                    .await
                    .map(|_| ());
            }

            // Customer first, before the candidate drafts: finalizing them takes it anyway.
            let lock = crate::repositories::credit::customer_credit::lock_customer_tx(
                conn,
                tenant_id,
                trigger.customer_id,
            )
            .await?;
            diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
            // Elect a single leader per merge partition (tenant, customer, date, currency, entity).
            let lock_key = consolidation_lock_key(
                tenant_id,
                trigger.customer_id,
                trigger.invoice_date,
                &trigger.currency,
                trigger.invoicing_entity_id,
            );
            InvoiceRow::advisory_xact_lock(conn, lock_key)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            // Gather eligible drafts sharing the static merge key (locked FOR UPDATE).
            let candidates = InvoiceRow::find_consolidation_candidates(
                conn,
                &lock,
                trigger.invoice_date,
                &trigger.currency,
                trigger.auto_advance,
                trigger.invoicing_entity_id,
                trigger.net_terms,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            // The trigger may have been consolidated by a leader that ran while we waited on the
            // advisory lock; if it is no longer a candidate, there is nothing for us to do.
            if !candidates.iter().any(|c| c.id == trigger_invoice_id) {
                return Ok(());
            }

            // Payment method + charge_automatically are the per-subscription parts of the merge
            // key: members must match the trigger's so the parent's charging behavior is well-defined.
            let trigger_subscription_id = trigger.subscription_id.ok_or_else(|| {
                StoreError::ValueNotFound("trigger subscription_id is null".into())
            })?;
            let trigger_pm = self
                .store
                .resolve_payment_method_for_subscription(tenant_id, trigger_subscription_id)
                .await?;
            let trigger_charge_auto = self
                .store
                .get_subscription(tenant_id, trigger_subscription_id)
                .await?
                .charge_automatically;

            let mut members: Vec<Invoice> = Vec::new();
            for row in candidates {
                let inv: Invoice = row.try_into()?;
                let Some(sub_id) = inv.subscription_id else {
                    continue;
                };
                // Hand-edited drafts settle on their own (see the trigger routing above): merging one
                // would refresh it and clobber the operator's lines.
                if inv.manually_edited_at.is_some() {
                    continue;
                }
                let pm = self
                    .store
                    .resolve_payment_method_for_subscription(tenant_id, sub_id)
                    .await?;
                if pm != trigger_pm {
                    continue;
                }
                let sub_charge_auto = self
                    .store
                    .get_subscription(tenant_id, sub_id)
                    .await?
                    .charge_automatically;
                if sub_charge_auto != trigger_charge_auto {
                    continue;
                }
                members.push(inv);
            }

            // A single member: nothing to merge, finalize on its own (existing behavior).
            if members.len() <= 1 {
                return self
                    .settle_recurring_invoice_tx(
                        conn,
                        &Actor::System,
                        trigger_invoice_id,
                        tenant_id,
                        true,
                        &None,
                    )
                    .await
                    .map(|_| ());
            }

            self.build_and_finalize_consolidated(conn, tenant_id, &trigger, members)
                .await
        })
        .await
    }

    async fn build_and_finalize_consolidated(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        trigger: &Invoice,
        members: Vec<Invoice>,
    ) -> StoreResult<()> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            // Refresh each member to capture usage accrued during the grace window, then re-read it.
            let mut refreshed: Vec<Invoice> = Vec::with_capacity(members.len());
            for m in &members {
                self.refresh_invoice_data(conn, m.id, tenant_id, &None, true)
                    .await?;
                let fresh: Invoice = InvoiceRow::find_by_id(conn, tenant_id, m.id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .try_into()?;
                refreshed.push(fresh);
            }

            let mut merged = merge_members(&refreshed);
            let applied_credits = self
                .merged_credit(
                    conn,
                    tenant_id,
                    trigger.customer_id,
                    &trigger.currency,
                    None,
                    &refreshed,
                    &mut merged,
                    refreshed.iter().all(|m| m.tax_inclusive),
                    common_domain::clock::now(),
                )
                .await?;
            let Merged {
                line_items,
                coupons,
                tax_breakdown,
                subtotal,
                subtotal_recurring,
                tax_amount,
                total,
                discount,
                promotional_credits,
            } = merged;
            let amount_due = billing_core::amount_due(total, 0, applied_credits);

            // The parent spans several plans, so it has no subscription_id/plan_version and
            // process_mrr is a no-op for it (members already logged their MRR at draft time).
            let due_at = (trigger.invoice_date
                + chrono::Duration::days(i64::from(trigger.net_terms)))
            .and_time(NaiveTime::MIN);

            let invoice_new = InvoiceNew {
                status: InvoiceStatusEnum::Draft,
                tenant_id,
                customer_id: trigger.customer_id,
                subscription_id: None,
                // Spans several subscriptions, so no single cycle.
                cycle_index: None,
                currency: trigger.currency.clone(),
                invoice_number: "draft".to_string(),
                line_items,
                coupons,
                data_updated_at: None,
                invoice_date: trigger.invoice_date,
                billing_period_start: trigger.invoice_date,
                // Members are already net, and the parent is never recomputed from prices.
                // It reads as tax-included only if every subscription it gathers does.
                tax_inclusive: refreshed.iter().all(|m| m.tax_inclusive),
                plan_version_id: None,
                invoice_type: InvoiceType::Recurring,
                finalized_at: None,
                subtotal,
                subtotal_recurring,
                discount,
                tax_amount,
                total,
                amount_due,
                applied_credits,
                promotional_credits,
                net_terms: trigger.net_terms,
                reference: None,
                purchase_order: trigger.purchase_order.clone(),
                memo: None,
                due_at: Some(due_at),
                plan_name: None,
                customer_details: trigger.customer_details.clone(),
                seller_details: trigger.seller_details.clone(),
                auto_advance: trigger.auto_advance,
                payment_status: InvoicePaymentStatus::Unpaid,
                tax_breakdown,
                manual: false,
                invoicing_entity_id: trigger.invoicing_entity_id,
                parent_invoice_id: None,
                consolidated_into_invoice_id: None,
                // Annexes live on the per-subscription child drafts; the consolidated parent does
                // not re-derive them. (Aggregating children's annexes is a future refinement.)
                annexes: vec![],
                custom_properties: serde_json::Value::Null,
            };

            let parent = insert_invoice_tx(&self.store, conn, invoice_new).await?;

            // Link children and move their pending slot transactions to the parent (activated on pay).
            let child_ids: Vec<InvoiceId> = refreshed.iter().map(|m| m.id).collect();
            InvoiceRow::mark_consolidated_into(conn, tenant_id, &child_ids, parent.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            SlotTransactionRow::repoint_invoice(conn, tenant_id, &child_ids, parent.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            // Retract each child's `invoice.created` so external consumers see a clean
            // created → consolidated lifecycle pointing at the parent.
            let consolidated_events: Vec<OutboxEvent> = refreshed
                .iter()
                .map(|child| {
                    let mut ev: InvoiceEvent = child.into();
                    ev.consolidated_into_invoice_id = Some(parent.id);
                    OutboxEvent::invoice_consolidated(ev)
                })
                .collect();
            self.store
                .internal
                .publish_outbox_events_topic_tx(conn, consolidated_events)
                .await?;

            log::info!(
                "Consolidated {} recurring drafts into invoice {} for customer {} on {}",
                child_ids.len(),
                parent.id,
                trigger.customer_id,
                trigger.invoice_date
            );

            // refresh=false: the parent's lines were combined fresh just above and there is no single
            // subscription to recompute from, which satisfies settle's freshness contract. An empty
            // result means every merged period was empty, so the parent is Closed.
            self.settle_recurring_invoice_tx(
                conn,
                &Actor::System,
                parent.id,
                tenant_id,
                false,
                &None,
            )
            .await
            .map(|_| ())
        })
        .await
    }
}

/// The content of a consolidated invoice, aggregated from its members. Credits are NOT summed:
/// each member computed them against the same balance, so they are recomputed once on the
/// combined total.
struct Merged {
    line_items: Vec<LineItem>,
    coupons: Vec<CouponLineItem>,
    tax_breakdown: Vec<TaxBreakdownItem>,
    subtotal: i64,
    subtotal_recurring: i64,
    tax_amount: i64,
    total: i64,
    discount: i64,
    promotional_credits: i64,
}

fn merge_members(members: &[Invoice]) -> Merged {
    let mut line_items = Vec::new();
    let mut coupons = Vec::new();
    let mut tax_breakdown = Vec::new();
    for m in members {
        line_items.extend(m.line_items.iter().cloned());
        coupons.extend(m.coupons.iter().cloned());
        merge_tax_breakdown(&mut tax_breakdown, &m.tax_breakdown);
    }
    let discount = members.iter().map(|m| m.discount).sum();
    let totals = crate::services::invoice_lines::invoice_lines::totals_of(&line_items, discount);
    Merged {
        line_items,
        coupons,
        tax_breakdown,
        subtotal: totals.subtotal,
        subtotal_recurring: totals.subtotal_recurring,
        tax_amount: totals.tax,
        total: totals.total,
        discount: totals.discount,
        promotional_credits: totals.promotional,
    }
}

impl Services {
    /// The credit a consolidated invoice would draw at `at`: promotional credit once over all the
    /// merged lines (each draft estimated its own against the same grants), then paid credit.
    #[allow(clippy::too_many_arguments)]
    async fn merged_credit(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        currency: &str,
        invoice_id: Option<InvoiceId>,
        members: &[Invoice],
        merged: &mut Merged,
        tax_inclusive: bool,
        at: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<i64> {
        let scope = EstimateContext {
            tenant_id,
            customer_id,
            currency,
            invoice_id,
            subscription_id: None,
            periods: members
                .iter()
                .flat_map(|member| billed_periods(member.subscription_id, &member.line_items))
                .collect(),
            at,
            book: None,
        };
        self.reprice_promotional_estimate(
            conn,
            &scope,
            &mut TaxedDocument {
                lines: &mut merged.line_items,
                subtotal: &mut merged.subtotal,
                subtotal_recurring: &mut merged.subtotal_recurring,
                discount: merged.discount,
                tax_inclusive,
                tax_amount: &mut merged.tax_amount,
                tax_breakdown: &mut merged.tax_breakdown,
                promotional_credits: &mut merged.promotional_credits,
                total: &mut merged.total,
            },
        )
        .await?;
        self.preview_credit_for_lines(conn, &scope, &merged.line_items, merged.total, 0)
            .await
    }
}

impl Services {
    /// Finalization content of a consolidated invoice billing usage priced in credit units. Its
    /// drafts are priced again and the units drawn once over all of them, against the consolidated
    /// invoice, under the unit's grant locks; then each draft is discounted and taxed on what its
    /// wallets left uncovered, written back, and merged again.
    pub(in crate::services) async fn consolidated_content_drawing_units(
        &self,
        conn: &mut PgConn,
        parent: &Invoice,
        at: crate::repositories::credit::customer_credit::MovementTime,
    ) -> StoreResult<ComputedInvoiceContent> {
        let mut children: Vec<Invoice> =
            InvoiceRow::list_consolidated_children(conn, parent.tenant_id, parent.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(Invoice::try_from)
                .collect::<Result<Vec<_>, _>>()?;
        // Keep the merged lines in the order the consolidation first gave them.
        let position = |child: &Invoice| {
            child
                .line_items
                .first()
                .and_then(|first| {
                    parent
                        .line_items
                        .iter()
                        .position(|line| line.local_id == first.local_id)
                })
                .unwrap_or(usize::MAX)
        };
        children.sort_by_key(|child| position(child));

        let mut details = Vec::with_capacity(children.len());
        for child in &children {
            let subscription_id = child.subscription_id.ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(
                    "a consolidated draft bills a subscription".into(),
                ))
            })?;
            let current = self
                .store
                .get_subscription_details_with_conn(conn, parent.tenant_id, subscription_id)
                .await?;
            details.push(
                crate::services::invoices::refresh::minimum_of_its_period(conn, child, &current)
                    .await?
                    .unwrap_or(current),
            );
        }
        let mut priced = Vec::with_capacity(children.len());
        for (child, child_details) in children.iter().zip(&details) {
            let already_paid = child.total - child.amount_due - child.applied_credits;
            priced.push(
                self.price_invoice_lines(
                    conn,
                    &child.billing_period_start,
                    child_details,
                    (already_paid > 0).then_some(already_paid as u64),
                    Some(child),
                    child.invoice_type.clone(),
                    None,
                )
                .await?,
            );
        }
        let mut targets = priced
            .iter_mut()
            .zip(&details)
            .filter_map(|(priced, child_details)| match priced {
                PricedInvoice::Lines(lines) => Some(lines.cover_target(child_details)),
                PricedInvoice::Stored(_) => None,
            })
            .collect::<Vec<_>>();
        self.cover_credit_unit_lines(
            conn,
            CreditUnitCover::Draw {
                invoice_id: parent.id,
                at,
            },
            &mut targets,
        )
        .await?;
        drop(targets);

        let mut members = Vec::with_capacity(children.len());
        for ((child, child_details), priced) in children.into_iter().zip(&details).zip(priced) {
            let content = match priced {
                PricedInvoice::Stored(content) => content,
                PricedInvoice::Lines(lines) => {
                    self.finish_invoice_content(conn, child_details, lines, at.at, None)
                        .await?
                }
            };
            let row_patch: InvoiceRowLinesPatch = InvoiceLinesPatch {
                line_items: content.invoice_lines.clone(),
                amount_due: content.amount_due,
                subtotal: content.subtotal,
                subtotal_recurring: content.subtotal_recurring,
                total: content.total,
                tax_amount: content.tax_amount,
                applied_credits: content.applied_credits,
                promotional_credits: content.promotional_credits,
                applied_coupons: content.applied_coupons.clone(),
                tax_breakdown: content.tax_breakdown.clone(),
                annexes: child.annexes.clone(),
            }
            .try_into()?;
            row_patch
                .update_lines(child.id, parent.tenant_id, conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            members.push(Invoice {
                line_items: content.invoice_lines,
                coupons: content.applied_coupons,
                tax_breakdown: content.tax_breakdown,
                subtotal: content.subtotal,
                subtotal_recurring: content.subtotal_recurring,
                tax_amount: content.tax_amount,
                total: content.total,
                discount: content.discount,
                promotional_credits: content.promotional_credits,
                ..child
            });
        }

        let mut merged = merge_members(&members);
        let applied_credits = self
            .merged_credit(
                conn,
                parent.tenant_id,
                parent.customer_id,
                &parent.currency,
                Some(parent.id),
                &members,
                &mut merged,
                parent.tax_inclusive,
                at.at,
            )
            .await?;
        Ok(ComputedInvoiceContent {
            invoice_lines: merged.line_items,
            subtotal: merged.subtotal,
            applied_coupons: merged.coupons,
            discount: merged.discount,
            tax_breakdown: merged.tax_breakdown,
            applied_credits,
            promotional_credits: merged.promotional_credits,
            total: merged.total,
            tax_amount: merged.tax_amount,
            amount_due: billing_core::amount_due(merged.total, 0, applied_credits),
            subtotal_recurring: merged.subtotal_recurring,
        })
    }
}

/// Merges tax breakdown items by (name, rate, exemption), summing taxable and tax amounts.
fn merge_tax_breakdown(acc: &mut Vec<TaxBreakdownItem>, items: &[TaxBreakdownItem]) {
    for item in items {
        if let Some(existing) = acc.iter_mut().find(|e| {
            e.tax_rate == item.tax_rate
                && e.name == item.name
                && e.exemption_type == item.exemption_type
                && e.exemption_reason == item.exemption_reason
                && e.prepaid_credit == item.prepaid_credit
        }) {
            existing.taxable_amount += item.taxable_amount;
            existing.tax_amount += item.tax_amount;
        } else {
            acc.push(item.clone());
        }
    }
}

fn consolidation_lock_key(
    tenant_id: TenantId,
    customer_id: CustomerId,
    invoice_date: chrono::NaiveDate,
    currency: &str,
    invoicing_entity_id: InvoicingEntityId,
) -> i64 {
    // Currency and invoicing entity are part of the merge partition, so distinct-currency or
    // distinct-entity groups for the same customer on the same day are independent and need not
    // serialize against each other.
    let mut hasher = DefaultHasher::new();
    "invoice_consolidation".hash(&mut hasher);
    tenant_id.as_uuid().hash(&mut hasher);
    customer_id.as_uuid().hash(&mut hasher);
    invoice_date.hash(&mut hasher);
    currency.hash(&mut hasher);
    invoicing_entity_id.as_uuid().hash(&mut hasher);
    hasher.finish() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::invoices::TaxExemptionType;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    fn tax(name: &str, rate: i64, taxable: i64, amount: i64) -> TaxBreakdownItem {
        TaxBreakdownItem {
            taxable_amount: taxable,
            tax_amount: amount,
            tax_rate: Decimal::from(rate),
            name: name.to_string(),
            exemption_type: None,
            exemption_reason: None,
            tax_reference: None,
            overridden: false,
            supply_basis: None,
            tax_kind: None,
            prepaid_credit: false,
        }
    }

    #[test]
    fn merge_tax_breakdown_sums_matching_and_keeps_distinct() {
        let mut acc: Vec<TaxBreakdownItem> = Vec::new();
        merge_tax_breakdown(&mut acc, &[tax("VAT 20%", 20, 100, 20)]);
        merge_tax_breakdown(
            &mut acc,
            &[tax("VAT 20%", 20, 50, 10), tax("VAT 10%", 10, 200, 20)],
        );

        assert_eq!(acc.len(), 2);
        let vat20 = acc.iter().find(|t| t.name == "VAT 20%").unwrap();
        assert_eq!(vat20.taxable_amount, 150);
        assert_eq!(vat20.tax_amount, 30);
        let vat10 = acc.iter().find(|t| t.name == "VAT 10%").unwrap();
        assert_eq!(vat10.taxable_amount, 200);
        assert_eq!(vat10.tax_amount, 20);
    }

    #[test]
    fn merge_tax_breakdown_separates_distinct_exemptions() {
        let mut acc: Vec<TaxBreakdownItem> = Vec::new();
        let mut exempt = tax("VAT 20%", 20, 100, 0);
        exempt.exemption_type = Some(TaxExemptionType::ReverseCharge);
        merge_tax_breakdown(&mut acc, &[tax("VAT 20%", 20, 100, 20), exempt]);
        // Same name/rate but different exemption => two distinct rows.
        assert_eq!(acc.len(), 2);
    }

    #[test]
    fn consolidation_lock_key_is_deterministic_and_partition_sensitive() {
        let tenant = TenantId::new();
        let customer = CustomerId::new();
        let entity = InvoicingEntityId::new();
        let entity2 = InvoicingEntityId::new();
        let d1 = NaiveDate::from_ymd_opt(2026, 5, 26).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 5, 27).unwrap();

        let base = consolidation_lock_key(tenant, customer, d1, "EUR", entity);

        // Deterministic.
        assert_eq!(
            base,
            consolidation_lock_key(tenant, customer, d1, "EUR", entity)
        );
        // Sensitive to every partition dimension.
        assert_ne!(
            base,
            consolidation_lock_key(tenant, customer, d2, "EUR", entity)
        );
        assert_ne!(
            base,
            consolidation_lock_key(tenant, customer, d1, "USD", entity)
        );
        assert_ne!(
            base,
            consolidation_lock_key(tenant, customer, d1, "EUR", entity2)
        );
    }
}
