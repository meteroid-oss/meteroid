use crate::StoreResult;
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::{InvoiceEvent, OutboxEvent};
use crate::domain::{
    CouponLineItem, Invoice, InvoiceNew, InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType,
    LineItem, TaxBreakdownItem,
};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::customer_balance::convert_currency;
use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use crate::repositories::invoices::insert_invoice_tx;
use crate::services::Services;
use crate::store::PgConn;
use chrono::NaiveTime;
use common_domain::ids::{BaseId, CustomerId, InvoiceId, InvoicingEntityId, TenantId};
use diesel_models::customers::CustomerRow;
use diesel_models::invoices::InvoiceRow;
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
            tenant_id,
            trigger.customer_id,
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
        let trigger_subscription_id = trigger
            .subscription_id
            .ok_or_else(|| StoreError::ValueNotFound("trigger subscription_id is null".into()))?;
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

        self.build_and_finalize_consolidated(conn, tenant_id, &trigger, members)
            .await
    }

    async fn build_and_finalize_consolidated(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        trigger: &Invoice,
        members: Vec<Invoice>,
    ) -> StoreResult<()> {
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

        // Aggregate lines/coupons/taxes/totals. Credits are NOT summed: each member computed
        // them against the same balance, so they are recomputed once on the combined total.
        let mut line_items: Vec<LineItem> = Vec::new();
        let mut coupons: Vec<CouponLineItem> = Vec::new();
        let mut tax_breakdown: Vec<TaxBreakdownItem> = Vec::new();
        let mut subtotal = 0i64;
        let mut subtotal_recurring = 0i64;
        let mut tax_amount = 0i64;
        let mut total = 0i64;
        let mut discount = 0i64;
        for m in &refreshed {
            line_items.extend(m.line_items.iter().cloned());
            coupons.extend(m.coupons.iter().cloned());
            merge_tax_breakdown(&mut tax_breakdown, &m.tax_breakdown);
            subtotal += m.subtotal;
            subtotal_recurring += m.subtotal_recurring;
            tax_amount += m.tax_amount;
            total += m.total;
            discount += m.discount;
        }

        let customer = CustomerRow::find_by_id(conn, &trigger.customer_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let applied_credits = if total > 0 {
            let balance_in_invoice_currency = convert_currency(
                conn,
                customer.balance_value_cents.max(0),
                &customer.currency,
                &trigger.currency,
            )
            .await?;
            std::cmp::min(total, balance_in_invoice_currency.max(0))
        } else {
            0
        };
        let amount_due = std::cmp::max(0, total - applied_credits);

        // The parent spans several plans, so it has no subscription_id/plan_version and
        // process_mrr is a no-op for it (members already logged their MRR at draft time).
        let due_at = (trigger.invoice_date + chrono::Duration::days(i64::from(trigger.net_terms)))
            .and_time(NaiveTime::MIN);

        let invoice_new = InvoiceNew {
            status: InvoiceStatusEnum::Draft,
            tenant_id,
            customer_id: trigger.customer_id,
            subscription_id: None,
            currency: trigger.currency.clone(),
            invoice_number: "draft".to_string(),
            line_items,
            coupons,
            data_updated_at: None,
            invoice_date: trigger.invoice_date,
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
            .insert_outbox_events_tx(conn, consolidated_events)
            .await?;

        log::info!(
            "Consolidated {} recurring drafts into invoice {} for customer {} on {}",
            child_ids.len(),
            parent.id,
            trigger.customer_id,
            trigger.invoice_date
        );

        // refresh=false: content is already combined and fresh, so no per-subscription recompute
        // (which would need a subscription_id) happens.
        self.finalize_invoice_tx(conn, &Actor::System, parent.id, tenant_id, false, &None)
            .await
            .map(|_| ())
    }
}

/// Merges tax breakdown items by (name, rate, exemption), summing taxable and tax amounts.
fn merge_tax_breakdown(acc: &mut Vec<TaxBreakdownItem>, items: &[TaxBreakdownItem]) {
    for item in items {
        if let Some(existing) = acc.iter_mut().find(|e| {
            e.tax_rate == item.tax_rate
                && e.name == item.name
                && e.exemption_type == item.exemption_type
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

    fn tax(name: &str, rate: i64, taxable: u64, amount: u64) -> TaxBreakdownItem {
        TaxBreakdownItem {
            taxable_amount: taxable,
            tax_amount: amount,
            tax_rate: Decimal::from(rate),
            name: name.to_string(),
            exemption_type: None,
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
