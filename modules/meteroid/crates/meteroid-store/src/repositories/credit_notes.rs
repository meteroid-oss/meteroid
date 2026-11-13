mod credit_note_effects;
mod financial;

pub(crate) use financial::{
    restore_invoice_credit_tx, reverse_missed_refund_credit_tx, take_back_invoice_granted_credit_tx,
};

use crate::StoreResult;
use crate::constants::Currencies;
use crate::domain::einvoicing::EInvoicingOutcome;
use crate::domain::entity_activity::Actor;
use crate::domain::invoice_lines::{LineItem, LineItemKind, TaxDetail};
use crate::domain::invoices::TaxBreakdownItem;
use crate::domain::{
    CreditNote, CreditNoteNew, Invoice, InvoicePaymentStatus, InvoiceStatusEnum,
    outbox_event::{InvoiceEvent, OutboxEvent},
};
use crate::errors::StoreError;
use crate::repositories::payment_transactions::{
    CreditNoteRefundSummary, recompute_invoice_payment_state,
};
use crate::services::payment_reminders::sweep::ReopenedReminders;
use crate::services::utils::format_document_number;
use crate::store::Store;
use crate::utils::money::format_money;
use chrono::{DateTime, NaiveDateTime, Utc};
use common_domain::ids::{BaseId, CreditNoteId, CustomerId, InvoiceId, TenantId};
use common_utils::decimals::{ToSubunit, ToUnit};
use diesel_models::PgConn;
use diesel_models::credit_notes::CreditNoteRow;
use diesel_models::invoices::InvoiceRow;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::lock::CustomerLock;
use diesel_models::payments::PaymentTransactionRow;
use error_stack::{Report, bail};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use scoped_futures::ScopedFutureExt;
use std::collections::HashMap;

pub use crate::domain::enums::CreditType;
use crate::domain::enums::StoredObjectKindEnum;
use crate::domain::stored_object::StoredObjectRef;
use diesel_models::stored_object::StoredObjectRowNew;

/// Specifies a line item to credit.
#[derive(Debug, Clone)]
pub enum CreditLineItem {
    /// Credit `quantity × line.unit_price`. The line must have a unit_price.
    Line { local_id: String, quantity: Decimal },
    /// Credit specific sub-lines; line subtotal is summed from their
    /// `quantity × unit_price`. Sub-lines not listed are dropped from the credited line.
    SubLines {
        local_id: String,
        sub_lines: Vec<CreditSubLineItem>,
    },
    /// Credit an explicit subtotal (excl. tax) against a line, capped at the line's
    /// remaining subtotal. For lines that carry no `unit_price` and can't be expressed
    /// as `quantity × price` — e.g. a synthesized minimum-commitment shortfall line.
    Amount { local_id: String, amount_cents: i64 },
}

impl CreditLineItem {
    pub fn local_id(&self) -> &str {
        match self {
            CreditLineItem::Line { local_id, .. }
            | CreditLineItem::SubLines { local_id, .. }
            | CreditLineItem::Amount { local_id, .. } => local_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreditSubLineItem {
    pub local_id: String,
    pub quantity: Decimal,
}

/// Parameters for creating a credit note via the public API
#[derive(Debug, Clone)]
pub struct CreateCreditNoteParams {
    pub invoice_id: InvoiceId,
    /// Line items to credit. Empty means credit all line items with full amounts.
    pub line_items: Vec<CreditLineItem>,
    pub reason: Option<String>,
    pub memo: Option<String>,
    pub credit_type: CreditType,
    /// User-supplied custom property values, validated against the tenant's `CREDIT_NOTE`
    /// definitions at insert time. `None` means "not managed" (validation skipped).
    pub custom_properties: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct CreateCreditNoteTxParams {
    pub invoice: Invoice,
    /// Line items to credit. If None, credits all invoice line items with full amounts.
    pub line_items: Option<Vec<CreditLineItem>>,
    /// Initial status of the credit note
    pub status: crate::domain::enums::CreditNoteStatus,
    /// When the credit note was finalized (only set if status is Finalized)
    pub finalized_at: Option<NaiveDateTime>,
    /// Reason for the credit note
    pub reason: Option<String>,
    /// Optional memo
    pub memo: Option<String>,
    /// How to apply the credit
    pub credit_type: CreditType,
    /// User-supplied custom property values (validated at insert time). `None` = not managed.
    pub custom_properties: Option<serde_json::Value>,
    /// Whether these credits come from a proration calculation (e.g. amendments crediting
    /// unused time). A category, not a per-line value: each line is then flagged prorated
    /// only when proration actually reduced it below the billed amount (see `negate_line_items`).
    pub from_proration: bool,
    /// The caller removes the credit the credited purchase lines funded in the same transaction
    /// (an immediate plan change), so crediting their money to the balance grants nothing twice.
    pub purchased_credit_removed: bool,
    /// True on the invoice-void path, which returns the invoice's drawn credit itself. The note then
    /// converts only collected cash to credit, so the draw is never returned twice.
    pub applied_credits_restored_externally: bool,
    /// Issued by voiding the invoice, whose void reversal reports the revenue it reverses.
    pub voids_invoice: bool,
}

#[async_trait::async_trait]
pub trait CreditNoteInterface {
    async fn insert_credit_note(&self, credit_note: CreditNoteNew) -> StoreResult<CreditNote>;

    async fn create_credit_note(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        params: CreateCreditNoteParams,
    ) -> StoreResult<CreditNote>;

    /// Create and finalize a credit note in a single transaction. Used by interactive
    /// UI paths so the user never sees a Draft CN (which would be a footgun: actions
    /// like settling the parent invoice and offering reissue only fire on finalize).
    async fn create_and_finalize_credit_note(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        params: CreateCreditNoteParams,
    ) -> StoreResult<CreditNote>;

    async fn list_credit_notes(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        invoice_id: Option<InvoiceId>,
        status: Option<crate::domain::enums::CreditNoteStatus>,
        search: Option<String>,
        filters: Vec<crate::domain::filters::Filter>,
        order_by: Option<String>,
        pagination: crate::domain::PaginationRequest,
    ) -> StoreResult<crate::domain::PaginatedVec<CreditNote>>;

    async fn get_credit_note_by_id(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<CreditNote>;

    async fn list_credit_notes_by_invoice_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Vec<CreditNote>>;

    async fn list_credit_notes_by_customer_id(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Vec<CreditNote>>;

    async fn finalize_credit_note(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<CreditNote>;

    async fn void_credit_note(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<CreditNote>;

    async fn delete_draft_credit_note(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<()>;

    async fn list_credit_notes_by_ids(
        &self,
        credit_note_ids: Vec<CreditNoteId>,
    ) -> StoreResult<Vec<CreditNote>>;

    /// The promotional credit the note gives back: as the ledger booked it once finalized, the
    /// lines' estimate on a draft.
    async fn promotional_credit_restored(&self, credit_note: &CreditNote) -> StoreResult<i64>;

    /// Of `invoices`, those credit notes alone settled (see `Invoice::settled_by_credit_notes`).
    async fn invoices_settled_by_credit_notes(
        &self,
        tenant_id: TenantId,
        invoices: &[&Invoice],
    ) -> StoreResult<std::collections::HashSet<InvoiceId>>;

    async fn save_credit_note_documents(
        &self,
        credit_note_id: CreditNoteId,
        tenant_id: TenantId,
        pdf: StoredObjectRef,
        xml: Option<StoredObjectRef>,
        einvoicing: Option<EInvoicingOutcome>,
    ) -> StoreResult<()>;

    /// What a finalized Refund credit note has drawn and still can, with its refund rows.
    async fn get_credit_note_refunds(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<CreditNoteRefundSummary>;

    /// Merge custom property values onto a draft credit note (send a key with `null` to remove
    /// it). Validated against the tenant's `CREDIT_NOTE` definitions; only allowed while draft.
    async fn patch_credit_note_custom_properties(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
        custom_properties: serde_json::Value,
    ) -> StoreResult<CreditNote>;
}

/// What crediting `credited` more of a line billed `original` takes of its `amount`, after
/// `already` was credited: each cumulative share rounded half-up (OQ14), so notes completing the
/// line take back exactly its amount, a half cent never twice.
fn cumulative_share(amount: i64, already: i64, credited: i64, original: i64) -> i64 {
    if original == 0 {
        return 0;
    }
    let share = |credited: i64| {
        (Decimal::from(amount) * Decimal::from(credited) / Decimal::from(original))
            .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
            .to_i64()
            .unwrap_or(0)
    };
    share(already.saturating_add(credited)) - share(already)
}

/// `cumulative_share` less what the line's earlier notes actually `returned` of `amount` rather than
/// what this rule would have: a note issued under another rounding rule is completed exactly.
/// `None` trusts the earlier notes to have followed this rule.
fn share_net_of_returned(
    amount: i64,
    returned: Option<i64>,
    (already, credited, original): (i64, i64, i64),
) -> i64 {
    let Some(returned) = returned else {
        return cumulative_share(amount, already, credited, original);
    };
    let share = cumulative_share(amount, 0, already.saturating_add(credited), original) - returned;
    // Earlier notes that returned more leave nothing for this one, never a negative share.
    if (share >= 0) == (amount >= 0) {
        share
    } else {
        0
    }
}

/// What a line's earlier non-voided notes returned of its taxable amount and tax, positive.
#[derive(Debug, Clone, Default)]
pub(crate) struct Returned {
    taxable: i64,
    tax: i64,
    details: Vec<TaxDetail>,
}

impl Returned {
    pub(crate) fn add(&mut self, note_line: &LineItem) -> StoreResult<()> {
        let overflow = || StoreError::InvalidArgument("credited tax exceeds currency range".into());
        self.taxable = self
            .taxable
            .checked_sub(note_line.taxable_amount)
            .ok_or_else(overflow)?;
        self.tax = self
            .tax
            .checked_sub(note_line.tax_amount)
            .ok_or_else(overflow)?;
        for detail in &note_line.tax_details {
            match self.details.iter_mut().find(|returned| {
                returned.tax_name == detail.tax_name && returned.tax_rate == detail.tax_rate
            }) {
                Some(returned) => {
                    returned.tax_amount = returned
                        .tax_amount
                        .checked_sub(detail.tax_amount)
                        .ok_or_else(overflow)?;
                }
                None => self.details.push(TaxDetail {
                    tax_amount: detail.tax_amount.checked_neg().ok_or_else(overflow)?,
                    ..detail.clone()
                }),
            }
        }
        Ok(())
    }

    pub(crate) fn taxable(&self) -> i64 {
        self.taxable
    }

    pub(crate) fn tax(&self) -> i64 {
        self.tax
    }

    fn detail(&self, detail: &TaxDetail) -> i64 {
        self.details
            .iter()
            .filter(|returned| {
                returned.tax_name == detail.tax_name && returned.tax_rate == detail.tax_rate
            })
            .map(|returned| returned.tax_amount)
            .sum()
    }
}

/// Prorates each tax detail like the line's tax (`share_net_of_returned`), negates it, and spreads
/// the residual to the cent of `expected_sum` from the largest detail down, each within what its
/// earlier notes left of it: a detail never returns more than it bore.
fn prorate_and_negate_tax_details(
    details: &[TaxDetail],
    shares: (i64, i64, i64),
    returned: Option<&Returned>,
    expected_sum: i64,
) -> Vec<TaxDetail> {
    if details.is_empty() {
        return Vec::new();
    }
    let (already, _, original) = shares;
    // Details of one line share its sign; work in returned magnitudes.
    let sign = if details.iter().map(|d| d.tax_amount).sum::<i64>() < 0 {
        -1
    } else {
        1
    };
    let mut taken: Vec<i64> = Vec::with_capacity(details.len());
    let mut left: Vec<i64> = Vec::with_capacity(details.len());
    for d in details {
        let returned_before = returned
            .map(|returned| returned.detail(d))
            .unwrap_or_else(|| cumulative_share(d.tax_amount, 0, already, original));
        taken.push(
            sign * share_net_of_returned(
                d.tax_amount,
                returned.map(|returned| returned.detail(d)),
                shares,
            ),
        );
        left.push((sign * (d.tax_amount - returned_before)).max(0));
    }

    let mut order: Vec<usize> = (0..details.len()).collect();
    order.sort_by_key(|i| std::cmp::Reverse(details[*i].tax_amount.unsigned_abs()));
    let mut residual = -sign * expected_sum - taken.iter().sum::<i64>();
    for i in order {
        if residual == 0 {
            break;
        }
        let room = if residual > 0 {
            (left[i] - taken[i]).max(0)
        } else {
            taken[i].max(0)
        };
        let moved = residual.signum() * residual.abs().min(room);
        taken[i] += moved;
        residual -= moved;
    }
    // Unplaceable only if the line's own figures disagree: the largest detail keeps the cent.
    if residual != 0
        && let Some(i) = (0..details.len()).max_by_key(|i| details[*i].tax_amount.unsigned_abs())
    {
        taken[i] += residual;
    }

    details
        .iter()
        .zip(taken)
        .map(|(d, taken)| TaxDetail {
            tax_rate: d.tax_rate,
            tax_name: d.tax_name.clone(),
            tax_amount: -sign * taken,
        })
        .collect()
}

/// Builds negated sub_lines for a credited line.
/// - If `override_` is Some: keep only the listed sublines (preserving original order),
///   set their `quantity`/`total` to the requested values (negated), drop the rest.
/// - If None: simply negate the original sublines as-is.
fn negate_sub_lines(
    original: &[crate::domain::invoice_lines::SubLineItem],
    override_: Option<&[SubLineOverride]>,
) -> Vec<crate::domain::invoice_lines::SubLineItem> {
    use crate::domain::invoice_lines::SubLineItem;
    match override_ {
        None => original
            .iter()
            .map(|sl| SubLineItem {
                local_id: sl.local_id.clone(),
                name: sl.name.clone(),
                total: -sl.total,
                quantity: -sl.quantity,
                unit_price: sl.unit_price,
                attributes: sl.attributes.clone(),
            })
            .collect(),
        Some(ovs) => original
            .iter()
            .filter_map(|sl| {
                let ov = ovs.iter().find(|o| o.original_local_id == sl.local_id)?;
                Some(SubLineItem {
                    local_id: sl.local_id.clone(),
                    name: sl.name.clone(),
                    total: -ov.new_total_cents,
                    quantity: -ov.new_quantity,
                    unit_price: sl.unit_price,
                    attributes: sl.attributes.clone(),
                })
            })
            .collect(),
    }
}

/// Per-line resolved credit instruction used by `negate_line_items`.
#[derive(Debug, Clone)]
struct ResolvedCredit {
    /// Subtotal (excl. tax) to credit, in cents. `None` means credit the full line as-is.
    credit_subtotal: Option<i64>,
    /// When set, replaces the line's sub_lines with these (already non-negated; will be negated).
    /// Each entry is the subline's new (quantity, total_cents) and the original subline to copy from.
    sub_lines_override: Option<Vec<SubLineOverride>>,
    /// Subtotal earlier non-voided notes already credited on the line.
    already_credited: i64,
    /// What those notes returned of the line's taxable amount and tax; `None` assumes they
    /// followed the cumulative rule.
    returned: Option<Returned>,
}

#[derive(Debug, Clone)]
struct SubLineOverride {
    original_local_id: String,
    new_quantity: Decimal,
    new_total_cents: i64,
}

/// Creates negated line items for a credit note.
/// `resolved` maps line local_id → ResolvedCredit. Lines not present are skipped.
/// The lines a credit note crediting `credits` (line, subtotal credited) of `line_items` carries:
/// each line's taxable amount, tax and promotional credit prorated as an issued note prorates them.
pub(crate) fn credited_lines(
    line_items: &[LineItem],
    credits: &[(String, i64)],
    already_credited: &HashMap<String, i64>,
    returned: &HashMap<String, Returned>,
    precision: u8,
) -> Vec<LineItem> {
    let resolved = credits
        .iter()
        .map(|(local_id, credit_subtotal)| {
            (
                local_id.clone(),
                ResolvedCredit {
                    credit_subtotal: Some(*credit_subtotal),
                    sub_lines_override: None,
                    already_credited: already_credited.get(local_id).copied().unwrap_or(0),
                    returned: Some(returned.get(local_id).cloned().unwrap_or_default()),
                },
            )
        })
        .collect::<HashMap<_, _>>();
    negate_line_items(line_items, &resolved, precision, true)
}

fn negate_line_items(
    line_items: &[LineItem],
    resolved: &HashMap<String, ResolvedCredit>,
    precision: u8,
    from_proration: bool,
) -> Vec<LineItem> {
    line_items
        .iter()
        .filter_map(|item| {
            let r = resolved.get(&item.local_id)?;
            Some((item, r))
        })
        .map(|(item, r)| {
            let partial_subtotal = r.credit_subtotal;
            let sub_override = r.sub_lines_override.as_deref();
            let negated_sub_lines = negate_sub_lines(&item.sub_lines, sub_override);

            if let Some(credit_subtotal) = partial_subtotal {
                // Partial credit: amount is the subtotal (excl. tax)
                let original_subtotal = item.amount_subtotal;
                if original_subtotal == 0 {
                    // Avoid division by zero - return zero amounts
                    return LineItem {
                        local_id: item.local_id.clone(),
                        name: item.name.clone(),
                        charge_type: item.charge_type,
                        amount_subtotal: 0,
                        taxable_amount: 0,
                        tax_amount: 0,
                        amount_total: 0,
                        tax_rate: item.tax_rate,
                        tax_details: item.tax_details.clone(),
                        quantity: item.quantity,
                        unit_price: Some(Decimal::ZERO),
                        start_date: item.start_date,
                        end_date: item.end_date,
                        sub_lines: negated_sub_lines,
                        is_prorated: item.is_prorated,
                        price_component_id: item.price_component_id,
                        sub_component_id: item.sub_component_id,
                        sub_add_on_id: item.sub_add_on_id,
                        product_id: item.product_id,
                        metric_id: item.metric_id,
                        description: item.description.clone(),
                        group_by_dimensions: item.group_by_dimensions.clone(),
                        kind: item.kind,
                        credit_grant_amount: None,
                        credit_grant_expiry: None,
                        credit_grant_included: false,
                        taxed_at_purchase: false,
                        tax_inclusive_quote: None,
                        credit_unit: None,
                        credit_grant_unit: None,
                        credit_grant_rollover: None,
                        promotional_credit: None,
                        subscription_id: item.subscription_id,
                        usage_ends_at: None,
                    };
                }

                let credited_grant_amount = item.credit_grant_amount.and_then(|face| {
                    credit_grant_reversal(
                        face,
                        r.already_credited,
                        credit_subtotal,
                        original_subtotal,
                    )?
                    .checked_neg()
                });

                // Taxable amount and tax are prorated by the credited share, cumulatively over
                // the line's earlier notes, half-up.
                let shares = (
                    r.already_credited.max(0),
                    credit_subtotal,
                    original_subtotal,
                );
                let returned = r.returned.as_ref();
                let prorated_taxable = share_net_of_returned(
                    item.taxable_amount,
                    returned.map(|returned| returned.taxable),
                    shares,
                );
                let prorated_tax = share_net_of_returned(
                    item.tax_amount,
                    returned.map(|returned| returned.tax),
                    shares,
                );

                // So is the promotional credit the line took before tax, which goes back to its
                // grants: cumulatively, so the note completing the line returns all of it.
                let prorated_promotional = item.promotional_credit.and_then(|share| {
                    credit_grant_reversal(
                        share,
                        r.already_credited,
                        credit_subtotal,
                        original_subtotal,
                    )
                    .filter(|share| *share != 0)
                    .map(|share| -share)
                });

                // Prorate each tax detail so the breakdown reflects the credited portion,
                // then negate and apply a residual correction so the sum matches -prorated_tax
                // to the cent (largest-magnitude detail absorbs the rounding delta).
                let prorated_tax_details = prorate_and_negate_tax_details(
                    &item.tax_details,
                    shares,
                    returned,
                    -prorated_tax,
                );

                // Total = taxable + tax (uses taxable to account for discounts)
                let credit_total = prorated_taxable + prorated_tax;

                // Effective-rate display so qty × unit_price always reconciles to the credited
                // amount: quantity = the line's real unit count, unit_price = amount ÷ quantity
                // (the effective, possibly-prorated, per-unit rate). Never divide to fabricate a
                // fractional quantity (that produced the repeating decimal). Sub-line overrides
                // keep the parent blank — the negated sub-lines carry the detail.
                let (credited_quantity, credited_unit_price) = if sub_override.is_some() {
                    (None, None)
                } else {
                    effective_rate_display(item, credit_subtotal, precision)
                };
                // Per line: flagged prorated when the line was itself billed prorated, or —
                // for a proration-driven batch (`from_proration`, i.e. amendments) — when
                // proration actually reduced THIS line below what was billed. Within such a
                // batch the credit IS `billed × time_factor`, so `credit_subtotal < amount`
                // is exactly `factor < 1` (covers less than the full period). The
                // `from_proration` gate keeps manual partial credits (also a smaller amount,
                // but not a time-proration) from being mislabelled.
                let credited_is_prorated =
                    item.is_prorated || (from_proration && credit_subtotal < item.amount_subtotal);

                LineItem {
                    local_id: item.local_id.clone(),
                    name: item.name.clone(),
                    charge_type: item.charge_type,
                    amount_subtotal: -credit_subtotal,
                    taxable_amount: -prorated_taxable,
                    tax_amount: -prorated_tax,
                    amount_total: -credit_total,
                    tax_rate: item.tax_rate,
                    tax_details: prorated_tax_details,
                    quantity: credited_quantity,
                    unit_price: credited_unit_price,
                    start_date: item.start_date,
                    end_date: item.end_date,
                    sub_lines: negated_sub_lines,
                    is_prorated: credited_is_prorated,
                    price_component_id: item.price_component_id,
                    sub_component_id: item.sub_component_id,
                    sub_add_on_id: item.sub_add_on_id,
                    product_id: item.product_id,
                    metric_id: item.metric_id,
                    description: item.description.clone(),
                    group_by_dimensions: item.group_by_dimensions.clone(),
                    kind: item.kind,
                    credit_grant_amount: credited_grant_amount,
                    credit_grant_expiry: item.credit_grant_expiry,
                    credit_grant_included: item.credit_grant_included,
                    taxed_at_purchase: item.taxed_at_purchase,
                    tax_inclusive_quote: None,
                    credit_unit: None,
                    credit_grant_unit: item.credit_grant_unit.clone(),
                    credit_grant_rollover: None,
                    promotional_credit: prorated_promotional,
                    subscription_id: item.subscription_id,
                    usage_ends_at: None,
                }
            } else {
                // Full credit: negate the amounts, keep original quantity and positive
                // unit_price (the sign lives in the amounts).
                // Note: amount_total is recalculated as taxable_amount + tax_amount to properly
                // account for discounts (which reduce taxable_amount but not amount_subtotal)
                // A quantity without a price (usage priced in a credit unit, whose money is only
                // the overage) is stated at its effective rate, or qty × price ≠ amount (R120).
                let (quantity, unit_price) = match (item.quantity, item.unit_price) {
                    (Some(_), None) => {
                        effective_rate_display(item, item.amount_subtotal, precision)
                    }
                    shown => shown,
                };
                LineItem {
                    local_id: item.local_id.clone(),
                    name: item.name.clone(),
                    charge_type: item.charge_type,
                    amount_subtotal: -item.amount_subtotal,
                    taxable_amount: -item.taxable_amount,
                    tax_amount: -item.tax_amount,
                    amount_total: -(item.taxable_amount + item.tax_amount),
                    tax_rate: item.tax_rate,
                    tax_details: item
                        .tax_details
                        .iter()
                        .map(|d| TaxDetail {
                            tax_rate: d.tax_rate,
                            tax_name: d.tax_name.clone(),
                            tax_amount: -d.tax_amount,
                        })
                        .collect(),
                    quantity,
                    unit_price,
                    start_date: item.start_date,
                    end_date: item.end_date,
                    sub_lines: negated_sub_lines,
                    is_prorated: item.is_prorated,
                    price_component_id: item.price_component_id,
                    sub_component_id: item.sub_component_id,
                    sub_add_on_id: item.sub_add_on_id,
                    product_id: item.product_id,
                    metric_id: item.metric_id,
                    description: item.description.clone(),
                    group_by_dimensions: item.group_by_dimensions.clone(),
                    kind: item.kind,
                    credit_grant_amount: item.credit_grant_amount.and_then(i64::checked_neg),
                    credit_grant_expiry: item.credit_grant_expiry,
                    credit_grant_included: item.credit_grant_included,
                    taxed_at_purchase: item.taxed_at_purchase,
                    tax_inclusive_quote: None,
                    credit_unit: None,
                    credit_grant_unit: item.credit_grant_unit.clone(),
                    credit_grant_rollover: None,
                    promotional_credit: item.promotional_credit.and_then(i64::checked_neg),
                    subscription_id: item.subscription_id,
                    usage_ends_at: None,
                }
            }
        })
        .collect()
}

/// Quantity and unit price that reconcile to `credited` (minor units): the line's real unit
/// count and the effective rate, never a fabricated fractional quantity.
fn effective_rate_display(
    item: &LineItem,
    credited: i64,
    precision: u8,
) -> (Option<Decimal>, Option<Decimal>) {
    let qty = item
        .quantity
        .filter(|q| *q > Decimal::ZERO)
        .unwrap_or(Decimal::ONE);
    (Some(qty), Some(credited.to_unit(precision) / qty))
}

/// Funded credit a partial credit of a `Credit` line reverses, rounded cumulatively over the
/// line's notes so that crediting the whole subtotal always reverses the whole grant.
fn credit_grant_reversal(
    face: i64,
    already_credited: i64,
    credit_subtotal: i64,
    original_subtotal: i64,
) -> Option<i64> {
    if already_credited < 0 {
        return None;
    }
    let after = already_credited.checked_add(credit_subtotal)?;
    cumulative_grant_reversal(face, after, original_subtotal)?.checked_sub(
        cumulative_grant_reversal(face, already_credited, original_subtotal)?,
    )
}

/// The paid credit a note crediting `credited` of a pack line sold below face takes back, the
/// line's bonus credit taking the rest: paid credit follows the money, cumulatively over the line's
/// finalized notes (`already`), as the mint netted their cost.
pub(crate) fn paid_share_of_reversal(line: &LineItem, already: i64, credited: i64) -> Option<i64> {
    credit_grant_reversal(line.taxable_amount, already, credited, line.amount_subtotal)
}

/// Credit reversed once `credited` of the line's subtotal is credited: pro rata, half up, never
/// above the face.
pub(crate) fn cumulative_grant_reversal(
    face: i64,
    credited: i64,
    original_subtotal: i64,
) -> Option<i64> {
    if original_subtotal <= 0 || face < 0 || credited < 0 {
        return None;
    }
    let credited = credited.min(original_subtotal);
    let numerator = i128::from(face).checked_mul(i128::from(credited))?;
    let denominator = i128::from(original_subtotal);
    let (quotient, remainder) = (numerator / denominator, numerator % denominator);
    let prorated = if remainder * 2 >= denominator {
        quotient + 1
    } else {
        quotient
    };
    let prorated = i64::try_from(prorated).ok()?;
    Some(prorated.min(face))
}

fn compute_tax_breakdown(line_items: &[LineItem]) -> Vec<TaxBreakdownItem> {
    // Group by (tax_rate, tax_name) to preserve detailed breakdown
    let mut tax_groups: HashMap<(Decimal, String), u64> = HashMap::new();
    // Track taxable amounts separately per tax rate (since multiple taxes can apply to same base)
    let mut taxable_by_rate: HashMap<Decimal, u64> = HashMap::new();

    for item in line_items {
        let taxable = item.taxable_amount.unsigned_abs();

        if item.tax_details.is_empty() {
            // No detailed breakdown available, use combined rate
            if item.tax_amount != 0 || taxable > 0 {
                let key = (item.tax_rate, "Tax".to_string());
                *tax_groups.entry(key).or_insert(0) += item.tax_amount.unsigned_abs();
                *taxable_by_rate.entry(item.tax_rate).or_insert(0) += taxable;
            }
        } else {
            // Use detailed breakdown - each tax_detail has its own rate and amount
            for detail in &item.tax_details {
                let key = (detail.tax_rate, detail.tax_name.clone());
                *tax_groups.entry(key).or_insert(0) += detail.tax_amount.unsigned_abs();
                // Taxable amount is shared across all taxes on this line item
                // We track it per rate to avoid double-counting when multiple taxes share a rate
            }
            // Add taxable amount once per unique rate on this item
            let mut seen_rates: HashMap<Decimal, bool> = HashMap::new();
            for detail in &item.tax_details {
                seen_rates.entry(detail.tax_rate).or_insert_with(|| {
                    *taxable_by_rate.entry(detail.tax_rate).or_insert(0) += taxable;
                    true
                });
            }
        }
    }

    tax_groups
        .into_iter()
        .map(|((tax_rate, name), tax_amount)| {
            let taxable_amount = taxable_by_rate.get(&tax_rate).copied().unwrap_or(0);
            // Credit-note breakdown keeps its positive-magnitude convention; the
            // domain field is signed (W4) so cast the unsigned aggregates.
            TaxBreakdownItem {
                taxable_amount: taxable_amount as i64,
                tax_amount: tax_amount as i64,
                tax_rate,
                name,
                exemption_type: None, // Carried over from the source invoice by the caller.
                exemption_reason: None,
                tax_reference: None,
                overridden: false,
                supply_basis: None,
                // Rebuilt from line details, which carry no tax kind of their own;
                // the caller carries it over from the source invoice.
                tax_kind: None,
                prepaid_credit: false,
            }
        })
        .collect()
}

/// Whether credit notes alone settled `invoice`: see `Invoice::settled_by_credit_notes`.
async fn settled_by_credit_notes_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice: &Invoice,
) -> StoreResult<bool> {
    let cancelled = CreditNoteRow::sum_finalized_debt_cancelled(conn, invoice.id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let collected =
        !PaymentTransactionRow::invoices_with_collection(conn, tenant_id, &[invoice.id])
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .is_empty();
    Ok(invoice.settled_by_credit_notes(cancelled, collected))
}

/// Reduces `amount_due` by the debt the note cancelled (a Refund note by its settled refunds); at
/// zero the invoice becomes Paid and emits `invoice_paid`. Forgiven debt funds no purchased credit.
async fn apply_credit_note_to_invoice_tx(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    actor: &Actor,
    credit_note: &CreditNote,
) -> StoreResult<()> {
    // A refund note is settled by a separate refund action, so `apply_transaction` (an unclamped
    // delta) would drift: re-derive the balance from the settled payments and the note instead.
    if credit_note.credit_type == CreditType::Refund {
        let (_, recomputed, became_paid) = recompute_invoice_payment_state(
            conn,
            tenant_id,
            credit_note.invoice_id,
            credit_note.finalized_at,
        )
        .await?;

        if became_paid {
            let invoice: Invoice = recomputed.try_into()?;
            let event = InvoiceEvent {
                settled_by_credit_notes: settled_by_credit_notes_tx(conn, tenant_id, &invoice)
                    .await?,
                ..(&invoice).into()
            };
            store
                .internal
                .record_outbox_batch_tx(
                    conn,
                    tenant_id,
                    actor,
                    vec![OutboxEvent::invoice_paid(event)],
                )
                .await?;
        }
        return Ok(());
    }

    // The debt the note cancelled: all of a debt cancellation, the unpaid share of a
    // credit-to-balance note.
    let cn_amount = credit_note.debt_cancelled_amount_cents;
    if cn_amount == 0 {
        return Ok(());
    }

    let updated = InvoiceRow::apply_transaction(conn, credit_note.invoice_id, tenant_id, cn_amount)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    if updated.amount_due <= 0
        && updated.payment_status != diesel_models::enums::InvoicePaymentStatus::Paid
    {
        let updated_row = InvoiceRow::apply_payment_status(
            conn,
            credit_note.invoice_id,
            tenant_id,
            diesel_models::enums::InvoicePaymentStatus::Paid,
            credit_note.finalized_at,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let invoice: Invoice = updated_row.try_into()?;
        // What was paid before the rest was forgiven funds its share of the purchased credit,
        // from when that money landed.
        let paid_at = PaymentTransactionRow::last_settled_by_invoice_id(
            conn,
            credit_note.invoice_id,
            tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .and_then(|payment| payment.processed_at)
        .map_or_else(common_domain::clock::now, |at| at.and_utc());
        crate::services::credit::mint::mint_purchased_credit_tx(
            conn,
            &*store.hooks.credit,
            &invoice,
            paid_at,
            common_domain::clock::now(),
        )
        .await?;
        let event = InvoiceEvent {
            settled_by_credit_notes: settled_by_credit_notes_tx(conn, tenant_id, &invoice).await?,
            ..(&invoice).into()
        };
        store
            .internal
            .record_outbox_batch_tx(
                conn,
                tenant_id,
                actor,
                vec![OutboxEvent::invoice_paid(event)],
            )
            .await?;
    }

    Ok(())
}

/// Reverses the effect of `apply_credit_note_to_invoice_tx` when voiding a CN.
/// Adds the credit note total back to `amount_due`. If the invoice was Paid and now has outstanding balance, recomputes payment_status from
/// settled payment transactions (Unpaid if none, PartiallyPaid otherwise).
/// Returns whether the invoice was `Paid` before the revert (it is payable again afterwards).
async fn revert_credit_note_from_invoice_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    credit_note_row: &CreditNoteRow,
) -> StoreResult<bool> {
    // Voiding a refund note that money was already given back on would leave the refund
    // unbacked; the refund has to be reversed first.
    if credit_note_row.credit_type == diesel_models::enums::CreditTypeEnum::Refund {
        let drawn =
            PaymentTransactionRow::list_refunds_by_credit_note(conn, credit_note_row.id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        if drawn
            .iter()
            .any(|r| r.status != diesel_models::enums::PaymentStatusEnum::Failed)
        {
            bail!(StoreError::InvalidArgument(
                "This credit note already has refunds attached and cannot be voided".to_string()
            ));
        }

        let was_paid =
            InvoiceRow::select_for_update_by_id(conn, tenant_id, credit_note_row.invoice_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .invoice
                .payment_status
                == diesel_models::enums::InvoicePaymentStatus::Paid;
        let _ = recompute_invoice_payment_state(conn, tenant_id, credit_note_row.invoice_id, None)
            .await?;
        return Ok(was_paid);
    }

    let cn_amount = credit_note_row.debt_cancelled_amount_cents;
    if cn_amount == 0 {
        return Ok(false);
    }

    let updated =
        InvoiceRow::apply_transaction(conn, credit_note_row.invoice_id, tenant_id, -cn_amount)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

    if updated.payment_status == diesel_models::enums::InvoicePaymentStatus::Paid
        && updated.amount_due > 0
    {
        let txs =
            PaymentTransactionRow::list_by_invoice_id(conn, credit_note_row.invoice_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let amount_paid: i64 = txs
            .iter()
            .filter(|t| {
                t.transaction.payment_type.is_collection()
                    && t.transaction.status == diesel_models::enums::PaymentStatusEnum::Settled
            })
            .map(|t| t.transaction.amount - t.transaction.amount_reversed)
            .sum();

        let new_status = if amount_paid > 0 {
            diesel_models::enums::InvoicePaymentStatus::PartiallyPaid
        } else {
            diesel_models::enums::InvoicePaymentStatus::Unpaid
        };

        InvoiceRow::apply_payment_status(
            conn,
            credit_note_row.invoice_id,
            tenant_id,
            new_status,
            None,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        return Ok(true);
    }

    Ok(false)
}

/// True when money is already moving toward this invoice: an accepted async debit
/// (`Processing`) or any live Pending/Ready payment attempt. Such a charge collects
/// the full amount, so debt can no longer be cancelled — only credited back.
pub(crate) async fn invoice_payment_in_flight(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice: &Invoice,
) -> StoreResult<bool> {
    if invoice.payment_status == InvoicePaymentStatus::Processing {
        return Ok(true);
    }
    PaymentTransactionRow::exists_live_for_invoice(conn, invoice.id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)
}

const CREDITS_PURCHASE_TWICE: &str = "the credit purchased on this invoice is already the customer's; issue a Refund to reverse it, not a credit-to-balance note that would grant it twice";
/// Credit-to-balance on a line whose purchase already funded credit would grant it twice.
async fn credits_funded_purchase_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    invoice: &Invoice,
    line_items_to_credit: &[LineItem],
) -> StoreResult<bool> {
    let grants = financial::purchase_grants_tx(conn, lock, invoice).await?;
    Ok(line_items_to_credit.iter().any(|line| {
        grants
            .iter()
            .any(|grant| grant.line_local_id.as_deref() == Some(line.local_id.as_str()))
    }))
}

const PURCHASE_IN_FLIGHT: &str = "the credit sold on this invoice is not the customer's until its payment settles; wait until the payment settles, then issue the credit note";

/// Credit sold on an invoice whose payment is in flight is not funded yet: a note returning it to
/// the balance would stand if the payment failed, and the credit would be minted again on a late
/// capture.
async fn refuse_unfunded_purchase_credit_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    invoice: &Invoice,
    line_items_to_credit: &[LineItem],
) -> StoreResult<()> {
    let sold = invoice
        .line_items
        .iter()
        .filter(|line| {
            line.kind == crate::domain::LineItemKind::Credit
                && !line.credit_grant_included
                && line.credit_grant_amount.is_some_and(|amount| amount > 0)
                && line_items_to_credit
                    .iter()
                    .any(|credited| credited.local_id == line.local_id)
        })
        .collect::<Vec<_>>();
    if sold.is_empty() || !invoice_payment_in_flight(conn, lock.tenant_id(), invoice).await? {
        return Ok(());
    }
    let grants = financial::purchase_grants_tx(conn, lock, invoice).await?;
    let unfunded = sold.iter().any(|line| {
        !grants
            .iter()
            .any(|grant| grant.line_local_id.as_deref() == Some(line.local_id.as_str()))
    });
    if unfunded {
        bail!(StoreError::InvalidArgument(PURCHASE_IN_FLIGHT.to_owned()));
    }
    Ok(())
}

/// What is left of the credit a purchase line bought: credits of its unit, or money.
fn purchased_amount(minor: i64, line: &LineItem, currency: &str) -> String {
    match &line.credit_grant_unit {
        Some(unit) => format!(
            "{} {}",
            grouped_amount(Decimal::new(minor, u32::from(unit.precision))),
            unit.code
        ),
        None => format_money(minor, currency),
    }
}

/// "15,000" or "1,234.5": digits grouped like `format_money`'s.
fn grouped_amount(amount: Decimal) -> String {
    let text = amount.normalize().abs().to_string();
    let (whole, fraction) = text
        .split_once('.')
        .map_or((text.as_str(), None), |(w, f)| (w, Some(f)));
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let sign = if amount.is_sign_negative() && !amount.is_zero() {
        "-"
    } else {
        ""
    };
    match fraction {
        Some(fraction) => format!("{sign}{grouped}.{fraction}"),
        None => format!("{sign}{grouped}"),
    }
}

/// A refund can only take back purchased credit that is still unspent: each purchase line's
/// credited subtotal is reduced until the credit it reverses fits its grant's unspent value.
async fn clamp_refund_to_unspent_purchased_credit(
    conn: &mut PgConn,
    lock: &CustomerLock,
    invoice: &Invoice,
    line_items_to_credit: &[LineItem],
    resolved: &mut HashMap<String, ResolvedCredit>,
) -> StoreResult<()> {
    let now = common_domain::clock::now();
    let lines = financial::purchase_lines_tx(conn, lock, invoice).await?;
    let finalized = if lines.iter().any(|line| line.bonus.is_some()) {
        crate::services::credit::mint::credited_by_finalized_notes_tx(conn, invoice, None).await?
    } else {
        HashMap::new()
    };
    for financial::PurchaseLine { grant, bonus } in lines {
        // Every purchase is minted from a Credit line with a face: any other shape is refused.
        let Some(line_id) = grant.line_local_id.as_deref() else {
            if line_items_to_credit.iter().any(|l| l.amount_subtotal != 0) {
                bail!(StoreError::InvalidArgument(format!(
                    "credit purchase grant {} of this invoice names no invoice line; it cannot be refunded",
                    grant.id
                )));
            }
            continue;
        };
        let Some(line) = line_items_to_credit.iter().find(|l| l.local_id == line_id) else {
            continue;
        };
        let Some(rc) = resolved.get_mut(line_id) else {
            continue;
        };
        let Some(face) = line
            .credit_grant_amount
            .filter(|_| line.kind == LineItemKind::Credit)
        else {
            bail!(StoreError::InvalidArgument(format!(
                "the line \"{}\" of credit purchase grant {} sells no credit; it cannot be refunded",
                line.name, grant.id
            )));
        };
        // Lapsed credit is gone whether or not the expiry sweep has recorded it yet.
        let available = if grant.lapses_at.is_some_and(|at| at <= now) {
            0
        } else {
            grant.remaining
        };
        let already = rc.already_credited;
        let paid_before = finalized.get(line_id).copied().unwrap_or(0);
        let reversal = |subtotal: Option<i64>| -> Option<i64> {
            // Sold below face, only the paid credit must be unspent: used bonus is not clawed back.
            if bonus.is_some() {
                return paid_share_of_reversal(
                    line,
                    paid_before,
                    subtotal.unwrap_or(line.amount_subtotal),
                );
            }
            match subtotal {
                None => Some(face),
                Some(subtotal) => {
                    credit_grant_reversal(face, already, subtotal, line.amount_subtotal)
                }
            }
        };
        let requested = reversal(rc.credit_subtotal).unwrap_or(0);
        if requested <= available {
            continue;
        }
        if available == 0 {
            bail!(StoreError::InvalidArgument(
                "the credit purchased on this invoice is spent or expired; nothing remains to refund"
                    .to_owned()
            ));
        }
        if rc.sub_lines_override.is_some() {
            bail!(StoreError::InvalidArgument(format!(
                "only {} of the credit bought on \"{}\" remains refundable; credit that line by amount instead of by sub-lines",
                purchased_amount(available, line, &invoice.currency),
                line.name
            )));
        }
        let requested_subtotal = rc.credit_subtotal.unwrap_or(line.amount_subtotal);
        let scaled = i128::from(requested_subtotal) * i128::from(available) / i128::from(requested);
        let mut subtotal = i64::try_from(scaled).map_err(|_| {
            StoreError::InvalidArgument("refunded credit exceeds the supported range".into())
        })?;
        // Rounding of the pro-rata reversal can land one unit above what remains.
        while subtotal > 0 && reversal(Some(subtotal)).is_none_or(|amount| amount > available) {
            subtotal -= 1;
        }
        rc.credit_subtotal = Some(subtotal.min(requested_subtotal));
    }
    Ok(())
}

/// Applied invoice credit a refund returns: the credited share of the invoice, cumulatively
/// across refund notes, never clawing back what earlier notes already returned.
fn refund_applied_credit_share(
    invoice: &Invoice,
    credit_total: i64,
    existing: &[CreditNoteRow],
) -> StoreResult<i64> {
    if invoice.total <= 0 || invoice.applied_credits <= 0 || credit_total <= 0 {
        return Ok(0);
    }
    let overflow = || StoreError::InvalidArgument("prior refunds exceed currency range".into());
    let (mut prior_total, mut prior_credited) = (0_i64, 0_i64);
    for note in existing.iter().filter(|note| {
        note.status != diesel_models::enums::CreditNoteStatus::Voided
            && note.credit_type == diesel_models::enums::CreditTypeEnum::Refund
    }) {
        prior_total = prior_total
            .checked_add(credit_note_effects::total(note.total)?)
            .ok_or_else(overflow)?;
        prior_credited = prior_credited
            .checked_add(note.credited_amount_cents)
            .ok_or_else(overflow)?;
    }
    let cumulative = credit_note_effects::cumulative_return(
        invoice.applied_credits,
        invoice.total,
        prior_total,
        0,
        credit_total,
    )?;
    Ok(cumulative
        .saturating_sub(prior_credited)
        .clamp(0, credit_total))
}

/// User-facing create path: loads the invoice, validates status + credit type invariants,
/// and delegates to the shared `create_credit_note_tx`. Used by both `create_credit_note`
/// (draft) and `create_and_finalize_credit_note` (interactive).
pub(crate) async fn create_user_credit_note_tx(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    actor: &Actor,
    params: CreateCreditNoteParams,
    from_proration: bool,
    purchased_credit_removed: bool,
) -> StoreResult<CreditNote> {
    // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
    Box::pin(async move {
    let (_, invoice) = lock_credit_invoice(conn, tenant_id, params.invoice_id).await?;

    // A merged draft is never issued: its consolidated parent bills its lines and takes the notes.
    invoice.ensure_not_consolidated_child("credit")?;
    if invoice.status != InvoiceStatusEnum::Finalized {
        bail!(StoreError::InvalidArgument(
            "Credit notes can only be created for finalized invoices".to_string()
        ));
    }

    let payment_in_flight = invoice_payment_in_flight(conn, tenant_id, &invoice).await?;

    match params.credit_type {
        CreditType::DebtCancellation => {
            if invoice.amount_due <= 0 {
                bail!(StoreError::InvalidArgument(
                    "DebtCancellation credit notes can only be created for invoices with an outstanding balance".to_string()
                ));
            }
            // A submitted debit/charge collects the full amount regardless of
            // what we cancel; the overpayment would then be silently clamped away.
            if payment_in_flight {
                bail!(StoreError::InvalidArgument(
                    "DebtCancellation credit notes cannot be created while a payment is in flight; use CreditToBalance".to_string()
                ));
            }
        }
        CreditType::Refund => {
            if !matches!(
                invoice.payment_status,
                InvoicePaymentStatus::Paid | InvoicePaymentStatus::PartiallyPaid
            ) {
                bail!(StoreError::InvalidArgument(
                    "Refund-type credit notes can only be created for paid or partially paid invoices".to_string()
                ));
            }
        }
        CreditType::CreditToBalance => {
            // Credit that paid the whole invoice makes it paid, marked so yet or not.
            if !matches!(
                invoice.payment_status,
                InvoicePaymentStatus::Paid | InvoicePaymentStatus::PartiallyPaid
            ) && !payment_in_flight
                && invoice.amount_due > 0
            {
                bail!(StoreError::InvalidArgument(
                    "CreditToBalance credit notes can only be created for paid or partially paid invoices, or while a payment is in flight".to_string()
                ));
            }
        }
    }

    let line_items = if params.line_items.is_empty() {
        None
    } else {
        Some(params.line_items)
    };

    create_credit_note_tx(
        store,
        conn,
        tenant_id,
        actor,
        CreateCreditNoteTxParams {
            invoice,
            line_items,
            status: crate::domain::enums::CreditNoteStatus::Draft,
            finalized_at: None,
            reason: params.reason,
            memo: params.memo,
            credit_type: params.credit_type,
            custom_properties: params.custom_properties,
            from_proration,
            purchased_credit_removed,
            applied_credits_restored_externally: false,
            voids_invoice: false,
        },
    )
    .await
}).await
}

/// Locks the invoice a credit note is issued against, serializing every note and ledger movement
/// of that invoice.
pub(crate) async fn lock_credit_invoice(
    conn: &mut PgConn,
    tenant: TenantId,
    invoice: InvoiceId,
) -> StoreResult<(CustomerLock, Invoice)> {
    let locked = InvoiceRow::select_for_update_by_id(conn, tenant, invoice).await?;
    Ok((locked.lock, locked.invoice.try_into()?))
}

async fn lock_mutable_credit_note_tx(
    conn: &mut PgConn,
    tenant: TenantId,
    note: CreditNoteId,
) -> StoreResult<(CustomerLock, CreditNoteRow)> {
    let observed = CreditNoteRow::find_by_id(conn, tenant, note).await?;
    let (lock, _) = lock_credit_invoice(conn, tenant, observed.invoice_id).await?;
    let row = CreditNoteRow::find_by_id_for_update(conn, &lock, note).await?;
    Ok((lock, row))
}

/// A payment may land after the draft was created: keep the reviewed credit total, but split it
/// again between cancelled debt and returned value under the invoice lock.
async fn refresh_credit_note_balance_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice: &Invoice,
    credit_note_row: &CreditNoteRow,
    external: bool,
) -> StoreResult<()> {
    if credit_note_row.credit_type == diesel_models::enums::CreditTypeEnum::CreditToBalance {
        let mut prior = CreditNoteRow::list_by_invoice_id(conn, tenant_id, invoice.id).await?;
        prior.retain(|note| note.id != credit_note_row.id);
        let payment_in_flight = invoice_payment_in_flight(conn, tenant_id, invoice).await?;
        let (credited, debt) = credit_note_effects::balance_effects_in_flight(
            invoice,
            credit_note_effects::total(credit_note_row.total)?,
            &prior,
            external,
            payment_in_flight,
        )?;
        CreditNoteRow::set_balance_effects(conn, tenant_id, credit_note_row.id, credited, debt)
            .await?;
    } else if credit_note_row.debt_cancelled_amount_cents > invoice.amount_due.max(0) {
        bail!(StoreError::InvalidArgument(
            "invoice payment changed the debt available for cancellation".into()
        ));
    }
    Ok(())
}

pub(crate) async fn finalize_credit_note_tx(
    store: &Store,
    conn: &mut PgConn,
    tenant: TenantId,
    actor: &Actor,
    id: CreditNoteId,
) -> StoreResult<CreditNote> {
    financial::finalize_tx(store, conn, tenant, actor, id, false, false, None).await
}

/// `finalize_credit_note_tx` for a note a change issues: credit it returns to the balance is dated
/// `credit_at`, the change's business date. With `purchased_credit_removed`, the caller removes,
/// in the same transaction, the credit its purchase lines funded (an immediate plan change):
/// crediting their money grants nothing twice.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn finalize_credit_note_at_tx(
    store: &Store,
    conn: &mut PgConn,
    tenant: TenantId,
    actor: &Actor,
    id: CreditNoteId,
    purchased_credit_removed: bool,
    credit_at: Option<DateTime<Utc>>,
) -> StoreResult<CreditNote> {
    financial::finalize_tx(
        store,
        conn,
        tenant,
        actor,
        id,
        false,
        purchased_credit_removed,
        credit_at,
    )
    .await
}

/// The invoice-void note: the void returns the drawn credit itself, so the note converts only
/// collected cash.
pub(crate) async fn finalize_void_credit_note_tx(
    store: &Store,
    conn: &mut PgConn,
    tenant: TenantId,
    actor: &Actor,
    id: CreditNoteId,
) -> StoreResult<CreditNote> {
    financial::finalize_tx(store, conn, tenant, actor, id, true, false, None).await
}

#[allow(clippy::too_many_arguments)]
async fn finalize_credit_note_decision_tx(
    store: &Store,
    conn: &mut PgConn,
    lock: &CustomerLock,
    actor: &Actor,
    invoice: &Invoice,
    credit_note_id: CreditNoteId,
    applied_credits_restored_externally: bool,
    purchased_credit_removed: bool,
) -> StoreResult<CreditNote> {
    let tenant_id = lock.tenant_id();
    let credit_note_row = CreditNoteRow::find_by_id_for_update(conn, lock, credit_note_id).await?;

    if credit_note_row.status != diesel_models::enums::CreditNoteStatus::Draft {
        bail!(StoreError::InvalidArgument(
            "Only draft credit notes can be finalized".to_string()
        ));
    }

    // The purchase may have been paid since the draft: its credit is then already granted.
    if credit_note_row.credit_type == diesel_models::enums::CreditTypeEnum::CreditToBalance {
        let lines: Vec<LineItem> = serde_json::from_value(credit_note_row.line_items.clone())
            .map_err(|error| StoreError::SerdeError("credit-note lines".into(), error))?;
        if !purchased_credit_removed {
            refuse_unfunded_purchase_credit_tx(conn, lock, invoice, &lines).await?;
        }
        if !purchased_credit_removed
            && credits_funded_purchase_tx(conn, lock, invoice, &lines).await?
        {
            return Err(crate::services::credit::precondition(
                CREDITS_PURCHASE_TWICE,
            ));
        }
    }

    refresh_credit_note_balance_tx(
        conn,
        tenant_id,
        invoice,
        &credit_note_row,
        applied_credits_restored_externally,
    )
    .await?;

    let invoicing_entity = InvoicingEntityRow::select_for_update_by_id_and_tenant(
        conn,
        credit_note_row.invoicing_entity_id,
        tenant_id,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;

    let credit_note_number_value = invoicing_entity.next_credit_note_number;
    let credit_note_number = format_document_number(
        credit_note_number_value,
        &invoicing_entity.credit_note_number_pattern,
        common_domain::clock::now().date_naive(),
    );

    CreditNoteRow::finalize_with_number(conn, credit_note_id, tenant_id, &credit_note_number)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    InvoicingEntityRow::update_credit_note_number(
        conn,
        credit_note_row.invoicing_entity_id,
        tenant_id,
        credit_note_number_value,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;

    let credit_note: CreditNote = CreditNoteRow::find_by_id(conn, tenant_id, credit_note_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .try_into()?;

    apply_credit_note_to_invoice_tx(store, conn, tenant_id, actor, &credit_note).await?;

    store
        .internal
        .record_outbox_batch_tx(
            conn,
            tenant_id,
            actor,
            vec![OutboxEvent::credit_note_finalized((&credit_note).into())],
        )
        .await?;

    Ok(credit_note)
}

// shared implementation used by both `create_credit_note` and `void_invoice`.
pub(crate) async fn create_credit_note_tx(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    actor: &Actor,
    params: CreateCreditNoteTxParams,
) -> StoreResult<CreditNote> {
    // Financial effects apply only on finalization, where the ledger moves with the note.
    if params.status != crate::domain::enums::CreditNoteStatus::Draft {
        bail!(StoreError::InvalidArgument(
            "a credit note is created as a draft and finalized separately".to_string()
        ));
    }
    // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
    Box::pin(async move {
    // 1. Lock the invoice row so concurrent credit notes (and ledger movements) serialize.
    let (lock, invoice) = lock_credit_invoice(conn, tenant_id, params.invoice.id).await?;
    let lock = &lock;
    invoice.ensure_not_consolidated_child("credit")?;

    // Get existing credit notes for this invoice to track already credited amounts
    let existing_credit_notes = CreditNoteRow::list_by_invoice_id(conn, tenant_id, invoice.id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    // Calculate total credited amount per line item (excluding voided credit notes)
    // We use the absolute value of amount_subtotal since credit note amounts are negative
    let mut already_credited_amounts: HashMap<String, i64> = HashMap::new();
    let mut returned_by_line: HashMap<String, Returned> = HashMap::new();
    for cn in existing_credit_notes
        .iter()
        .filter(|cn| cn.status != diesel_models::enums::CreditNoteStatus::Voided)
    {
        let line_items: Vec<LineItem> =
            serde_json::from_value(cn.line_items.clone()).map_err(|error| {
                StoreError::SerdeError("credit-note financial history".into(), error)
            })?;
        for item in line_items {
            returned_by_line
                .entry(item.local_id.clone())
                .or_default()
                .add(&item)?;
            let previous = already_credited_amounts.entry(item.local_id).or_insert(0);
            *previous = previous
                .checked_add(credit_note_effects::total(item.amount_subtotal)?)
                .ok_or_else(|| {
                    StoreError::InvalidArgument("credited subtotal exceeds currency range".into())
                })?;
        }
    }

    let precision = Currencies::resolve_currency_precision(&invoice.currency).ok_or_else(|| {
        Report::from(StoreError::InvalidArgument(format!(
            "Unknown currency '{}'",
            invoice.currency
        )))
    })?;

    // 2. Resolve which lines are being credited and how (full / partial qty / per-subline)
    let requested: Vec<CreditLineItem> = params.line_items.unwrap_or_default();

    // Map original invoice lines by local_id for lookup
    let invoice_lines_by_id: HashMap<&str, &LineItem> = invoice
        .line_items
        .iter()
        .map(|l| (l.local_id.as_str(), l))
        .collect();

    let mut resolved: HashMap<String, ResolvedCredit> = HashMap::new();
    let line_items_to_credit: Vec<LineItem>;

    if requested.is_empty() {
        // Credit all lines with their remaining amounts (full credit)
        line_items_to_credit = invoice
            .line_items
            .iter()
            .filter(|item| {
                let already = already_credited_amounts
                    .get(&item.local_id)
                    .copied()
                    .unwrap_or(0);
                item.amount_subtotal > already
            })
            .cloned()
            .collect();
        for item in &line_items_to_credit {
            let already = already_credited_amounts
                .get(&item.local_id)
                .copied()
                .unwrap_or(0);
            let remaining = item.amount_subtotal - already;
            // Treat as partial when there's already been a credit; otherwise full negation
            let credit_subtotal = if already == 0 { None } else { Some(remaining) };
            resolved.insert(
                item.local_id.clone(),
                ResolvedCredit {
                    credit_subtotal,
                    sub_lines_override: None,
                    already_credited: already,
                    returned: Some(returned_by_line.get(&item.local_id).cloned().unwrap_or_default()),
                },
            );
        }
    } else {
        let mut items: Vec<LineItem> = Vec::with_capacity(requested.len());
        for ci in requested {
            let local_id = ci.local_id().to_string();
            let item = *invoice_lines_by_id.get(local_id.as_str()).ok_or_else(|| {
                Report::from(StoreError::InvalidArgument(format!(
                    "Line item '{}' not found on invoice",
                    local_id
                )))
            })?;
            let already = already_credited_amounts
                .get(&item.local_id)
                .copied()
                .unwrap_or(0);
            let remaining = item.amount_subtotal - already;
            if remaining <= 0 {
                bail!(StoreError::InvalidArgument(format!(
                    "\"{}\" has already been fully credited",
                    item.name
                )));
            }

            let rc = match ci {
                CreditLineItem::SubLines { sub_lines, .. } => {
                    if item.sub_lines.is_empty() {
                        bail!(StoreError::InvalidArgument(format!(
                            "\"{}\" has no sub-lines to credit",
                            item.name
                        )));
                    }
                    let mut overrides: Vec<SubLineOverride> = Vec::new();
                    let mut total_cents: i64 = 0;
                    for csl in &sub_lines {
                        let original_sl = item
                            .sub_lines
                            .iter()
                            .find(|s| s.local_id == csl.local_id)
                            .ok_or_else(|| {
                                Report::from(StoreError::InvalidArgument(format!(
                                    "\"{}\" has no sub-line '{}'",
                                    item.name, csl.local_id
                                )))
                            })?;
                        if csl.quantity <= Decimal::ZERO {
                            bail!(StoreError::InvalidArgument(format!(
                                "the quantity to credit on \"{}\" must be positive",
                                original_sl.name
                            )));
                        }
                        if original_sl.quantity <= Decimal::ZERO {
                            bail!(StoreError::InvalidArgument(format!(
                                "\"{}\" has no quantity to credit",
                                original_sl.name
                            )));
                        }
                        if csl.quantity > original_sl.quantity {
                            bail!(StoreError::InvalidArgument(format!(
                                "cannot credit {} of \"{}\": it was invoiced for {}",
                                csl.quantity.normalize(),
                                original_sl.name,
                                original_sl.quantity.normalize()
                            )));
                        }
                        let new_total = (original_sl.unit_price * csl.quantity)
                            .to_subunit_opt(precision)
                            .ok_or_else(|| {
                                Report::from(StoreError::InvalidArgument(format!(
                                    "the amount to credit on \"{}\" is too large",
                                    original_sl.name
                                )))
                            })?;
                        total_cents += new_total;
                        overrides.push(SubLineOverride {
                            original_local_id: csl.local_id.clone(),
                            new_quantity: csl.quantity,
                            new_total_cents: new_total,
                        });
                    }
                    if overrides.is_empty() {
                        bail!(StoreError::InvalidArgument(format!(
                            "choose at least one sub-line of \"{}\" to credit",
                            item.name
                        )));
                    }
                    if total_cents > remaining {
                        bail!(StoreError::InvalidArgument(format!(
                            "cannot credit {} on \"{}\": only {} of it is left to credit",
                            format_money(total_cents, &invoice.currency),
                            item.name,
                            format_money(remaining, &invoice.currency)
                        )));
                    }
                    ResolvedCredit {
                        credit_subtotal: Some(total_cents),
                        sub_lines_override: Some(overrides),
                        already_credited: already,
                        returned: Some(returned_by_line.get(&item.local_id).cloned().unwrap_or_default()),
                    }
                }
                CreditLineItem::Line { quantity: qty, .. } => {
                    if qty <= Decimal::ZERO {
                        bail!(StoreError::InvalidArgument(format!(
                            "the quantity to credit on \"{}\" must be positive",
                            item.name
                        )));
                    }
                    // Usage priced in credits: only its money overage is credited, by amount.
                    if let Some(credit) = &item.credit_unit {
                        bail!(StoreError::InvalidArgument(format!(
                            "\"{}\" is priced in {}: credit its money overage by amount; credits come back by voiding the invoice or by adding credit by hand",
                            item.name, credit.code
                        )));
                    }
                    let unit_price = item.unit_price.ok_or_else(|| {
                        Report::from(StoreError::InvalidArgument(format!(
                            "\"{}\" has no unit price: credit it by sub-line or by amount",
                            item.name
                        )))
                    })?;
                    let original_qty = item.quantity.unwrap_or(Decimal::ONE);
                    if original_qty <= Decimal::ZERO {
                        bail!(StoreError::InvalidArgument(format!(
                            "\"{}\" has no quantity to credit",
                            item.name
                        )));
                    }
                    if qty > original_qty {
                        bail!(StoreError::InvalidArgument(format!(
                            "cannot credit {} of \"{}\": it was invoiced for {}",
                            qty.normalize(),
                            item.name,
                            original_qty.normalize()
                        )));
                    }
                    let credit_cents =
                        (unit_price * qty)
                            .to_subunit_opt(precision)
                            .ok_or_else(|| {
                                Report::from(StoreError::InvalidArgument(format!(
                                    "the amount to credit on \"{}\" is too large",
                                    item.name
                                )))
                            })?;
                    if credit_cents > remaining {
                        bail!(StoreError::InvalidArgument(format!(
                            "cannot credit {} on \"{}\": only {} of it is left to credit",
                            format_money(credit_cents, &invoice.currency),
                            item.name,
                            format_money(remaining, &invoice.currency)
                        )));
                    }
                    ResolvedCredit {
                        credit_subtotal: Some(credit_cents),
                        sub_lines_override: None,
                        already_credited: already,
                        returned: Some(returned_by_line.get(&item.local_id).cloned().unwrap_or_default()),
                    }
                }
                CreditLineItem::Amount { amount_cents, .. } => {
                    if amount_cents <= 0 {
                        bail!(StoreError::InvalidArgument(format!(
                            "the amount to credit on \"{}\" must be positive",
                            item.name
                        )));
                    }
                    // Cap at the line's remaining (not-yet-credited) subtotal.
                    let credit = amount_cents.min(remaining);
                    ResolvedCredit {
                        credit_subtotal: Some(credit),
                        sub_lines_override: None,
                        already_credited: already,
                        returned: Some(returned_by_line.get(&item.local_id).cloned().unwrap_or_default()),
                    }
                }
            };

            resolved.insert(item.local_id.clone(), rc);
            items.push(item.clone());
        }
        line_items_to_credit = items;
    }

    if line_items_to_credit.is_empty() {
        bail!(StoreError::InvalidArgument(
            "No line items to credit".to_string()
        ));
    }

    match params.credit_type {
        CreditType::Refund => {
            clamp_refund_to_unspent_purchased_credit(
                conn, lock,
                &invoice,
                &line_items_to_credit,
                &mut resolved,
            )
            .await?;
        }
        CreditType::CreditToBalance => {
            if !params.purchased_credit_removed {
                refuse_unfunded_purchase_credit_tx(conn, lock, &invoice, &line_items_to_credit)
                    .await?;
            }
            if !params.purchased_credit_removed
                && credits_funded_purchase_tx(conn, lock, &invoice, &line_items_to_credit).await?
            {
                bail!(StoreError::InvalidArgument(CREDITS_PURCHASE_TWICE.to_owned()));
            }
        }
        CreditType::DebtCancellation => {}
    }

    // 3. Create negated line items for the credit note
    let negated_line_items = negate_line_items(
        &line_items_to_credit,
        &resolved,
        precision,
        params.from_proration,
    );

    // 6. Totals of the negated line items (negative), as an invoice's (R-076).
    let promotional = credit_note_effects::sum(negated_line_items.iter().map(
        crate::services::credit::promotional::net_share,
    ))?;
    let discount = credit_note_effects::sum(
        negated_line_items
            .iter()
            .map(|item| item.amount_subtotal - item.taxable_amount),
    )? - promotional;
    let billing_core::Totals {
        subtotal,
        tax: tax_amount,
        total,
        ..
    } = crate::services::invoice_lines::invoice_lines::totals_of(&negated_line_items, discount);
    if total > 0 {
        bail!(StoreError::InvalidArgument(
            "a credit note cannot increase the invoice charge".into()
        ));
    }

    if params.credit_type == CreditType::DebtCancellation
        && credit_note_effects::total(total)? > invoice.amount_due
    {
        bail!(StoreError::InvalidArgument(format!(
            "a debt cancellation cannot cancel more than the invoice's amount due: {} requested, {} due",
            format_money(i64::try_from(total.unsigned_abs()).unwrap_or(i64::MAX), &invoice.currency),
            format_money(invoice.amount_due, &invoice.currency)
        )));
    }

    // 7. Compute tax breakdown from negated line items (uses actual credited amounts).
    // Exemptions, their mention, and each rate's tax kind live only on the source
    // invoice's breakdown, so carry them over: the credit note must state the same
    // tax position, in the same words, as the invoice it corrects.
    let mut tax_breakdown = compute_tax_breakdown(&negated_line_items);
    crate::repositories::invoices::carry_over_tax_findings(
        &invoice.tax_breakdown,
        &mut tax_breakdown,
    );

    // 8. A draft carries a placeholder number until it is finalized.
    let credit_note_number = "draft".to_string();

    // 9. Split the credited amount between returned value, cash refund and cancelled debt
    // (positive values).
    let credit_total = credit_note_effects::total(total)?;

    let (credited_amount_cents, refunded_amount_cents, debt_cancelled_amount_cents) =
        match params.credit_type {
            CreditType::CreditToBalance => {
                let payment_in_flight =
                    invoice_payment_in_flight(conn, tenant_id, &invoice).await?;
                let (credited, debt) = credit_note_effects::balance_effects_in_flight(
                    &invoice,
                    credit_total,
                    &existing_credit_notes,
                    params.applied_credits_restored_externally,
                    payment_in_flight,
                )?;
                (credited, 0, debt)
            }
            CreditType::DebtCancellation => (0, 0, credit_total),
            CreditType::Refund => {
                // Applied invoice credit returns to the customer in proportion to the credited
                // share, cumulatively across refunds, so rounding never returns more than applied.
                let credited =
                    refund_applied_credit_share(&invoice, credit_total, &existing_credit_notes)?;
                let refunded = credit_total.checked_sub(credited).ok_or_else(|| {
                    StoreError::InvalidArgument("refund amount exceeds currency range".into())
                })?;
                (credited, refunded, 0)
            }
        };

    // 10. Validate any user-supplied custom properties against the tenant's credit-note
    // definitions. A null bag (system-triggered creates, e.g. void) is a no-op → empty object.
    let custom_properties = store
        .internal
        .validate_and_normalize_custom_properties(
            conn,
            tenant_id,
            crate::domain::enums::CustomPropertyEntityType::CreditNote,
            &params
                .custom_properties
                .clone()
                .unwrap_or(serde_json::Value::Null),
            &serde_json::json!({}),
            crate::domain::custom_property_definitions::CustomPropertyMode::Replace,
        )
        .await?;

    // 11. Build the credit note
    let credit_note_new = CreditNoteNew {
        credit_note_number: credit_note_number.clone(),
        status: params.status.clone(),
        credit_type: params.credit_type,
        tenant_id,
        customer_id: invoice.customer_id,
        invoice_id: invoice.id,
        invoice_number: invoice.invoice_number.clone(),
        plan_version_id: invoice.plan_version_id,
        subscription_id: invoice.subscription_id,
        currency: invoice.currency.clone(),
        subtotal,
        tax_amount,
        total,
        refunded_amount_cents,
        credited_amount_cents,
        debt_cancelled_amount_cents,
        line_items: negated_line_items,
        tax_breakdown,
        reason: params.reason,
        memo: params.memo,
        customer_details: invoice.customer_details.clone(),
        seller_details: invoice.seller_details.clone(),
        invoicing_entity_id: invoice.invoicing_entity_id,
        custom_properties,
        finalized_at: params.finalized_at,
    };

    // 11. Insert the credit note
    let mut insertable: diesel_models::credit_notes::CreditNoteRowNew =
        credit_note_new.try_into()?;
    insertable.voids_invoice = params.voids_invoice;
    let inserted_credit_note = insertable
        .insert(conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    // 12. Convert to domain model
    let credit_note: CreditNote = inserted_credit_note.try_into()?;

    let events = vec![OutboxEvent::credit_note_created((&credit_note).into())];
    store
        .internal
        .record_outbox_batch_tx(conn, tenant_id, actor, events)
        .await?;

    Ok(credit_note)
}).await
}

#[async_trait::async_trait]
impl CreditNoteInterface for Store {
    async fn insert_credit_note(&self, credit_note: CreditNoteNew) -> StoreResult<CreditNote> {
        let mut conn = self.get_conn().await?;

        let insertable: diesel_models::credit_notes::CreditNoteRowNew = credit_note.try_into()?;

        insertable
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .and_then(std::convert::TryInto::try_into)
    }

    async fn create_credit_note(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        params: CreateCreditNoteParams,
    ) -> StoreResult<CreditNote> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                create_user_credit_note_tx(self, conn, tenant_id, actor, params, false, false).await
            }
            .scope_boxed()
        })
        .await
    }

    async fn create_and_finalize_credit_note(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        params: CreateCreditNoteParams,
    ) -> StoreResult<CreditNote> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let draft =
                    create_user_credit_note_tx(self, conn, tenant_id, actor, params, false, false)
                        .await?;
                finalize_credit_note_tx(self, conn, tenant_id, actor, draft.id).await
            }
            .scope_boxed()
        })
        .await
    }

    async fn list_credit_notes(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        invoice_id: Option<InvoiceId>,
        status: Option<crate::domain::enums::CreditNoteStatus>,
        search: Option<String>,
        filters: Vec<crate::domain::filters::Filter>,
        order_by: Option<String>,
        pagination: crate::domain::PaginationRequest,
    ) -> StoreResult<crate::domain::PaginatedVec<CreditNote>> {
        let mut conn = self.get_conn().await?;

        let cp_filters = if filters.is_empty() {
            vec![]
        } else {
            let definitions = self
                .internal
                .custom_property_definitions
                .load_active(
                    &mut conn,
                    tenant_id,
                    crate::domain::enums::CustomPropertyEntityType::CreditNote,
                )
                .await?;
            crate::repositories::filter_resolution::resolve_list_filters(
                crate::repositories::filter_resolution::ListEntity::CreditNote,
                definitions.as_slice(),
                filters,
            )?
        };

        let rows = CreditNoteRow::list(
            &mut conn,
            tenant_id,
            customer_id,
            invoice_id,
            status.map(Into::into),
            search,
            cp_filters,
            order_by.as_deref(),
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let items: Vec<CreditNote> = rows
            .items
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(crate::domain::PaginatedVec {
            items,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        })
    }

    async fn get_credit_note_by_id(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<CreditNote> {
        let mut conn = self.get_conn().await?;

        CreditNoteRow::find_by_id(&mut conn, tenant_id, credit_note_id)
            .await
            .map_err(Into::into)
            .and_then(std::convert::TryInto::try_into)
    }

    async fn list_credit_notes_by_invoice_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Vec<CreditNote>> {
        let mut conn = self.get_conn().await?;

        let rows = CreditNoteRow::list_by_invoice_id(&mut conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn list_credit_notes_by_customer_id(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Vec<CreditNote>> {
        let mut conn = self.get_conn().await?;

        let rows = CreditNoteRow::list_by_customer_id(&mut conn, tenant_id, customer_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn finalize_credit_note(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<CreditNote> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                finalize_credit_note_tx(self, conn, tenant_id, actor, credit_note_id).await
            }
            .scope_boxed()
        })
        .await
    }

    async fn void_credit_note(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<CreditNote> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                // 1. Customer, invoice, then the note, as finalization takes them: its lock
                // waits out a concurrent finalization.
                let (lock, credit_note_row) =
                    lock_mutable_credit_note_tx(conn, tenant_id, credit_note_id).await?;
                diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;

                // 2. Validate it's finalized (only finalized credit notes can be voided)
                if credit_note_row.status != diesel_models::enums::CreditNoteStatus::Finalized {
                    bail!(StoreError::InvalidArgument(
                        "Only finalized credit notes can be voided".to_string()
                    ));
                }

                // 3. Take back the credit it returned, then void the credit note
                financial::take_back_returned_credit_tx(
                    conn,
                    &*self.hooks.credit,
                    &lock,
                    &credit_note_row,
                )
                .await?;
                CreditNoteRow::void(conn, credit_note_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // 3b. Reverse the invoice settlement (DebtCancellation only)
                let was_paid =
                    revert_credit_note_from_invoice_tx(conn, tenant_id, &credit_note_row).await?;
                crate::services::credit::reconcile_purchased_credit_tx(
                    conn,
                    &*self.hooks.credit,
                    &lock,
                    credit_note_row.invoice_id,
                    None,
                )
                .await?;
                // A settled invoice reopened by the void is no longer auto-collected.
                self.internal
                    .on_invoice_reopened(
                        conn,
                        tenant_id,
                        credit_note_row.invoice_id,
                        was_paid,
                        was_paid,
                        ReopenedReminders::Restart,
                        common_domain::clock::now().naive_utc(),
                    )
                    .await?;

                // 5. Get the voided credit note
                let credit_note: CreditNote =
                    CreditNoteRow::find_by_id(conn, tenant_id, credit_note_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .try_into()?;

                // 6. Emit outbox event for voided credit note
                self.internal
                    .record_outbox_batch_tx(
                        conn,
                        tenant_id,
                        actor,
                        vec![OutboxEvent::credit_note_voided((&credit_note).into())],
                    )
                    .await?;

                Ok(credit_note)
            }
            .scope_boxed()
        })
        .await
    }

    async fn delete_draft_credit_note(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<()> {
        // A draft reserves debt other notes of the invoice are split against: delete it under the
        // invoice lock.
        self.transaction(|conn| {
            async move {
                lock_mutable_credit_note_tx(conn, tenant_id, credit_note_id).await?;
                let rows_affected = CreditNoteRow::delete_draft(conn, credit_note_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if rows_affected == 0 {
                    bail!(StoreError::InvalidArgument(
                        "Credit note not found or is not a draft".to_string()
                    ));
                }
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn promotional_credit_restored(&self, credit_note: &CreditNote) -> StoreResult<i64> {
        if credit_note.status == crate::domain::enums::CreditNoteStatus::Draft {
            return Ok(credit_note.promotional_credit_restored());
        }
        let mut conn = self.get_conn().await?;
        crate::repositories::credit::customer_credit::returned_by_note(
            &mut conn,
            credit_note.tenant_id,
            credit_note.invoice_id,
            &credit_note.currency,
            credit_note.id,
            crate::repositories::credit::allocation::GrantPool::Promotional,
        )
        .await
    }

    async fn invoices_settled_by_credit_notes(
        &self,
        tenant_id: TenantId,
        invoices: &[&Invoice],
    ) -> StoreResult<std::collections::HashSet<InvoiceId>> {
        // Only a paid invoice nothing was collected on can have been settled by notes alone.
        let candidates: Vec<InvoiceId> = invoices
            .iter()
            .filter(|invoice| invoice.settled_by_credit_notes(invoice.total, false))
            .map(|invoice| invoice.id)
            .collect();
        if candidates.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        let mut conn = self.get_conn().await?;
        let cancelled = CreditNoteRow::sum_finalized_debt_cancelled_by_invoices(
            &mut conn,
            tenant_id,
            &candidates,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let collected: std::collections::HashSet<InvoiceId> =
            diesel_models::payments::PaymentTransactionRow::invoices_with_collection(
                &mut conn,
                tenant_id,
                &candidates,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .collect();
        Ok(invoices
            .iter()
            .filter(|invoice| {
                invoice.settled_by_credit_notes(
                    cancelled.get(&invoice.id).copied().unwrap_or(0),
                    collected.contains(&invoice.id),
                )
            })
            .map(|invoice| invoice.id)
            .collect())
    }

    async fn list_credit_notes_by_ids(
        &self,
        credit_note_ids: Vec<CreditNoteId>,
    ) -> StoreResult<Vec<CreditNote>> {
        let mut conn = self.get_conn().await?;

        let rows = CreditNoteRow::list_by_ids(&mut conn, credit_note_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn save_credit_note_documents(
        &self,
        credit_note_id: CreditNoteId,
        tenant_id: TenantId,
        pdf: StoredObjectRef,
        xml: Option<StoredObjectRef>,
        einvoicing: Option<EInvoicingOutcome>,
    ) -> StoreResult<()> {
        self.transaction(|conn| {
            async move {
                let (status, report) = match einvoicing {
                    Some(outcome) => (
                        Some(outcome.status.into()),
                        Some(serde_json::to_value(&outcome.report).map_err(|e| {
                            StoreError::SerdeError(
                                "Failed to serialize einvoicing_report".to_string(),
                                e,
                            )
                        })?),
                    ),
                    None => (None, None),
                };

                CreditNoteRow::save_credit_note_documents(
                    conn,
                    credit_note_id,
                    tenant_id,
                    pdf.id,
                    xml.as_ref().map(|o| o.id),
                    status,
                    report,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                let mut docs = vec![StoredObjectRowNew {
                    id: pdf.id,
                    tenant_id,
                    kind: StoredObjectKindEnum::CreditNotePdf.into(),
                    key: pdf.key,
                }];
                if let Some(xml) = xml {
                    docs.push(StoredObjectRowNew {
                        id: xml.id,
                        tenant_id,
                        kind: StoredObjectKindEnum::InvoiceXml.into(),
                        key: xml.key,
                    });
                }
                StoredObjectRowNew::insert_batch(conn, &docs)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn get_credit_note_refunds(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<CreditNoteRefundSummary> {
        use diesel_models::enums::PaymentStatusEnum as PaymentStatus;

        let mut conn = self.get_conn().await?;

        let credit_note = CreditNoteRow::find_by_id(&mut conn, tenant_id, credit_note_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let rows = PaymentTransactionRow::list_refunds_by_credit_note(
            &mut conn,
            credit_note_id,
            tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let sum_of = |statuses: &[PaymentStatus]| -> i64 {
            rows.iter()
                .filter(|r| statuses.contains(&r.status))
                .map(|r| r.amount)
                .sum()
        };

        let refunded_cents = sum_of(&[PaymentStatus::Settled]);
        let pending_cents = sum_of(&[PaymentStatus::Ready, PaymentStatus::Pending]);

        Ok(CreditNoteRefundSummary {
            refundable_cents: (credit_note.refunded_amount_cents - refunded_cents - pending_cents)
                .max(0),
            refunded_cents,
            pending_cents,
            refunds: rows.into_iter().map(Into::into).collect(),
        })
    }

    async fn patch_credit_note_custom_properties(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
        custom_properties: serde_json::Value,
    ) -> StoreResult<CreditNote> {
        let mut conn = self.get_conn().await?;

        let existing = CreditNoteRow::find_by_id(&mut conn, tenant_id, credit_note_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let normalized = self
            .internal
            .validate_and_normalize_custom_properties(
                &mut conn,
                tenant_id,
                crate::domain::enums::CustomPropertyEntityType::CreditNote,
                &custom_properties,
                &existing.custom_properties,
                crate::domain::custom_property_definitions::CustomPropertyMode::Merge,
            )
            .await?;

        CreditNoteRow::update_custom_properties(&mut conn, tenant_id, credit_note_id, normalized)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| StoreError::InvalidArgument("Credit note not found".to_string()).into())
            .and_then(std::convert::TryInto::try_into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::invoice_lines::{SubLineItem, TaxDetail};
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    #[test]
    fn unit_amounts_group_their_digits_like_money() {
        assert_eq!(grouped_amount(dec!(15000)), "15,000");
        assert_eq!(grouped_amount(dec!(1234567.50)), "1,234,567.5");
        assert_eq!(grouped_amount(dec!(999)), "999");
        assert_eq!(grouped_amount(dec!(-2500)), "-2,500");
    }

    fn resolved_map(entries: &[(&str, Option<i64>)]) -> HashMap<String, ResolvedCredit> {
        entries
            .iter()
            .map(|(id, credit_subtotal)| {
                (
                    (*id).to_string(),
                    ResolvedCredit {
                        credit_subtotal: *credit_subtotal,
                        sub_lines_override: None,
                        already_credited: 0,
                        returned: None,
                    },
                )
            })
            .collect()
    }

    fn make_line(
        local_id: &str,
        amount_subtotal: i64,
        taxable_amount: i64,
        tax_amount: i64,
        tax_details: Vec<TaxDetail>,
    ) -> LineItem {
        LineItem {
            local_id: local_id.to_string(),
            name: "test".to_string(),
            charge_type: None,
            amount_subtotal,
            tax_rate: dec!(0.2),
            taxable_amount,
            tax_amount,
            amount_total: taxable_amount + tax_amount,
            tax_details,
            quantity: Some(Decimal::ONE),
            unit_price: Some(Decimal::from(amount_subtotal) / Decimal::from(100)),
            start_date: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            end_date: NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            sub_lines: Vec::<SubLineItem>::new(),
            is_prorated: false,
            price_component_id: None,
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: None,
            metric_id: None,
            description: None,
            group_by_dimensions: None,
            kind: crate::domain::LineItemKind::Regular,
            credit_grant_amount: None,
            credit_grant_expiry: None,
            credit_grant_included: false,
            taxed_at_purchase: false,
            tax_inclusive_quote: None,
            credit_unit: None,
            credit_grant_unit: None,
            credit_grant_rollover: None,
            promotional_credit: None,
            subscription_id: None,
            usage_ends_at: None,
        }
    }

    fn assert_breakdown_matches(items: &[LineItem], expected_tax_amount: i64, case: &str) {
        let breakdown = compute_tax_breakdown(items);
        let sum: i64 = breakdown.iter().map(|b| b.tax_amount).sum();
        assert_eq!(
            sum,
            expected_tax_amount.unsigned_abs() as i64,
            "[{}] Σ breakdown.tax_amount ({}) != |credit_note.tax_amount| ({})",
            case,
            sum,
            expected_tax_amount.unsigned_abs()
        );
    }

    #[test]
    fn full_credit_breakdown_matches_tax_amount() {
        let line = make_line(
            "l1",
            10_000,
            10_000,
            2_000,
            vec![
                TaxDetail {
                    tax_rate: dec!(0.15),
                    tax_name: "VAT".into(),
                    tax_amount: 1_500,
                },
                TaxDetail {
                    tax_rate: dec!(0.05),
                    tax_name: "City".into(),
                    tax_amount: 500,
                },
            ],
        );
        let amounts = resolved_map(&[("l1", None)]);
        let negated = negate_line_items(&[line], &amounts, 2, false);
        let tax: i64 = negated.iter().map(|i| i.tax_amount).sum();
        assert_eq!(tax, -2_000);
        assert_breakdown_matches(&negated, tax, "full");
    }

    /// A credit-priced line bills its usage quantity with no money price: a full note states it
    /// at its effective rate, as a partial one does, so quantity × price is the amount.
    #[test]
    fn a_full_credit_of_a_priceless_quantity_states_its_effective_rate() {
        let mut line = make_line("l1", 400, 400, 80, vec![]);
        line.quantity = Some(dec!(300));
        line.unit_price = None;
        let negated = negate_line_items(&[line], &resolved_map(&[("l1", None)]), 2, false);
        let (quantity, unit_price) = (negated[0].quantity, negated[0].unit_price);
        assert_eq!(quantity, Some(dec!(300)));
        let unit_price = unit_price.expect("an effective rate");
        assert_eq!((dec!(300) * unit_price).round_dp(2), dec!(4.00));
        assert_eq!(negated[0].amount_subtotal, -400);
    }

    #[test]
    fn partial_credit_breakdown_matches_tax_amount_and_values() {
        // 3333 / 10000 = 0.3333 proportion
        // VAT: 1500 * 0.3333 = 499.95 → 500
        // City: 500 * 0.3333 = 166.65 → 167
        // Total tax: 2000 * 0.3333 = 666.6 → 667 (equals 500 + 167)
        let line = make_line(
            "l1",
            10_000,
            10_000,
            2_000,
            vec![
                TaxDetail {
                    tax_rate: dec!(0.15),
                    tax_name: "VAT".into(),
                    tax_amount: 1_500,
                },
                TaxDetail {
                    tax_rate: dec!(0.05),
                    tax_name: "City".into(),
                    tax_amount: 500,
                },
            ],
        );
        let amounts = resolved_map(&[("l1", Some(3_333))]);
        let negated = negate_line_items(&[line], &amounts, 2, false);
        let tax: i64 = negated.iter().map(|i| i.tax_amount).sum();
        assert_eq!(tax, -667, "prorated tax_amount on negated line");
        assert_breakdown_matches(&negated, tax, "partial");

        // Value-level: each detail must reflect the credited portion, not the invoice
        let details = &negated[0].tax_details;
        let vat = details.iter().find(|d| d.tax_name == "VAT").unwrap();
        let city = details.iter().find(|d| d.tax_name == "City").unwrap();
        assert_eq!(
            vat.tax_amount, -500,
            "VAT detail should be prorated 1500→500"
        );
        assert_eq!(
            city.tax_amount, -167,
            "City detail should be prorated 500→167"
        );
    }

    #[test]
    fn two_partials_of_same_line_sum_exactly() {
        // Helper-level test: exercises proration math. Does NOT go through
        // create_credit_note_tx's already-credited tracking — see integration tests
        // for the full sequential-credit flow.
        //
        // VAT 1501 + City 500 = 2001 total tax.
        // a1 (40%): tax = round(800.4) = 800; VAT = round(600.4) = 600; City = 200
        // a2 (60%): tax = round(1200.6) = 1201; VAT = round(900.6) = 901; City = 300
        // Σ = 800 + 1201 = 2001 exactly.
        let line = make_line(
            "l1",
            10_000,
            10_000,
            2_001,
            vec![
                TaxDetail {
                    tax_rate: dec!(0.15),
                    tax_name: "VAT".into(),
                    tax_amount: 1_501,
                },
                TaxDetail {
                    tax_rate: dec!(0.05),
                    tax_name: "City".into(),
                    tax_amount: 500,
                },
            ],
        );
        let a1 = resolved_map(&[("l1", Some(4_000))]);
        let a2 = resolved_map(&[("l1", Some(6_000))]);
        let n1 = negate_line_items(std::slice::from_ref(&line), &a1, 2, false);
        let n2 = negate_line_items(&[line], &a2, 2, false);

        let tax1: i64 = n1.iter().map(|i| i.tax_amount).sum();
        let tax2: i64 = n2.iter().map(|i| i.tax_amount).sum();
        assert_breakdown_matches(&n1, tax1, "partial-1");
        assert_breakdown_matches(&n2, tax2, "partial-2");
        // Exactly equal — residual correction must guarantee no drift
        assert_eq!(
            tax1.unsigned_abs() + tax2.unsigned_abs(),
            2_001,
            "Σ partial tax should exactly equal original tax"
        );
    }

    /// A 10.00 line taxed 0.25 credited in two halves: 0.13 then 0.12, so the notes give back
    /// exactly what the invoice charged. Half-even proration gave 0.12 twice (OQ14).
    #[test]
    fn halves_of_a_half_cent_tax_mirror_the_invoice() {
        let line = make_line(
            "l1",
            1_000,
            1_000,
            25,
            vec![TaxDetail {
                tax_rate: dec!(0.025),
                tax_name: "VAT".into(),
                tax_amount: 25,
            }],
        );
        let first = negate_line_items(
            std::slice::from_ref(&line),
            &resolved_map(&[("l1", Some(500))]),
            2,
            false,
        );
        let mut rest = resolved_map(&[("l1", Some(500))]);
        rest.get_mut("l1").expect("line").already_credited = 500;
        let second = negate_line_items(&[line], &rest, 2, false);
        let figures = |items: &[LineItem]| {
            (
                items[0].taxable_amount,
                items[0].tax_amount,
                items[0].tax_details[0].tax_amount,
                items[0].amount_total,
            )
        };
        assert_eq!(figures(&first), (-500, -13, -13, -513));
        assert_eq!(figures(&second), (-500, -12, -12, -512));
    }

    #[test]
    fn partial_credit_with_empty_tax_details_uses_fallback() {
        // No tax_details: compute_tax_breakdown must use the scalar tax_rate/tax_amount
        // path and still produce a correct breakdown.
        let line = make_line("l1", 10_000, 10_000, 2_000, vec![]);
        let amounts = resolved_map(&[("l1", Some(3_333))]);
        let negated = negate_line_items(&[line], &amounts, 2, false);
        let tax: i64 = negated.iter().map(|i| i.tax_amount).sum();
        assert_eq!(tax, -667);
        assert!(
            negated[0].tax_details.is_empty(),
            "tax_details should remain empty"
        );

        let breakdown = compute_tax_breakdown(&negated);
        assert_eq!(breakdown.len(), 1);
        assert_eq!(breakdown[0].name, "Tax");
        assert_eq!(breakdown[0].tax_rate, dec!(0.2));
        assert_eq!(breakdown[0].tax_amount, 667);
    }

    #[test]
    fn partial_credit_uses_effective_rate_and_reconciles() {
        // The classic repeating-decimal case: credit 900 of a 2900 line. The old code
        // showed quantity = 900/2900 = 0.31034482…; the effective-rate model shows the
        // integer unit count with unit_price = amount / qty, which reconciles cleanly.
        let line = make_line("l1", 2_900, 2_900, 0, vec![]);
        let resolved = resolved_map(&[("l1", Some(900))]);
        let negated = negate_line_items(&[line], &resolved, 2, false);
        let cn = &negated[0];
        assert_eq!(cn.amount_subtotal, -900, "amount is authoritative");
        assert_eq!(
            cn.quantity,
            Some(dec!(1)),
            "quantity stays the integer unit count, never the 900/2900 fraction"
        );
        let q = cn.quantity.unwrap();
        let p = cn.unit_price.unwrap();
        assert_eq!(
            (q * p).round_dp(2),
            dec!(9.00),
            "qty × unit_price reconciles to the credited amount"
        );
    }

    #[test]
    fn partial_credit_multi_instance_keeps_integer_quantity() {
        // 2/3 of a 3-unit line: the effective rate (6400/3) repeats, but the displayed
        // quantity stays the integer 3 and the amount stays authoritative.
        let mut line = make_line("l1", 9_600, 9_600, 0, vec![]);
        line.quantity = Some(dec!(3));
        line.unit_price = Some(dec!(32));
        let resolved = resolved_map(&[("l1", Some(6_400))]);
        let negated = negate_line_items(&[line], &resolved, 2, false);
        let cn = &negated[0];
        assert_eq!(cn.amount_subtotal, -6_400);
        assert_eq!(
            cn.quantity,
            Some(dec!(3)),
            "multi-instance credit shows the integer count, not a fraction"
        );
        assert_eq!(
            (cn.quantity.unwrap() * cn.unit_price.unwrap()).round_dp(2),
            dec!(64.00),
            "qty × unit_price reconciles to the credited amount"
        );
    }

    #[test]
    fn prorated_is_decided_per_line_within_a_proration_batch() {
        let line = make_line("l1", 1_000, 1_000, 0, vec![]); // is_prorated: false
        let partial = resolved_map(&[("l1", Some(420))]);
        let full = resolved_map(&[("l1", Some(1_000))]);

        // Proration batch (prorated = true): a reduced credit IS flagged…
        assert!(
            negate_line_items(std::slice::from_ref(&line), &partial, 2, true)[0].is_prorated,
            "a reduced credit in a proration batch is prorated"
        );
        // …but a 100% credit (factor 1.0) of the same line is NOT — proration didn't reduce it.
        assert!(
            !negate_line_items(std::slice::from_ref(&line), &full, 2, true)[0].is_prorated,
            "a full (100%) credit is not prorated even in a proration batch"
        );
        // Manual partial credit (prorated = false) is never inferred from the amount alone.
        assert!(
            !negate_line_items(&[line], &partial, 2, false)[0].is_prorated,
            "a manual partial credit is not mislabelled prorated"
        );
    }

    #[test]
    fn partial_credits_of_a_credit_line_reverse_its_whole_grant_cumulatively() {
        let reversals = [0, 10, 20].map(|before| credit_grant_reversal(10, before, 10, 30));
        assert_eq!(reversals, [Some(3), Some(4), Some(3)]);
        assert_eq!(credit_grant_reversal(10, 0, 30, 30), Some(10));
        assert_eq!(credit_grant_reversal(10, 30, 1, 30), Some(0));
    }

    fn pack_line(face: i64, subtotal: i64) -> LineItem {
        LineItem {
            kind: LineItemKind::Credit,
            credit_grant_amount: Some(face),
            taxed_at_purchase: true,
            credit_grant_unit: Some(crate::domain::credit_units::CreditUnitRef {
                unit_id: common_domain::ids::CreditUnitId::new(),
                code: "CREDITS".to_owned(),
                precision: 0,
                product_id: None,
            }),
            ..make_line("pack", subtotal, subtotal, 0, vec![])
        }
    }

    /// 100 credits sold for 10.00: refunding 5.00 takes back 50 credits, never the 500 minor
    /// units of money credited.
    #[test]
    fn a_partial_refund_of_a_unit_pack_reverses_its_credits_pro_rata() {
        let reversed = |line: LineItem, credited: i64| {
            negate_line_items(
                &[line],
                &resolved_map(&[("pack", Some(credited))]),
                2,
                false,
            )[0]
            .credit_grant_amount
        };
        assert_eq!(reversed(pack_line(100, 1_000), 500), Some(-50));
        assert_eq!(reversed(pack_line(100, 1_000), 1_000), Some(-100));
    }

    /// 120.00 quoted at 20% with 60.00 of promotional credit is 100.00 net less 50.00: a note
    /// states the net 50.00 (pro rata on a partial note), not the quoted 60.00 it gives back.
    #[test]
    fn a_tax_inclusive_note_states_the_net_promotional_credit() {
        let source = LineItem {
            promotional_credit: Some(6_000),
            tax_inclusive_quote: Some(crate::domain::TaxInclusiveQuote {
                amount_subtotal: 12_000,
                unit_price: None,
                sub_lines: Vec::new(),
            }),
            ..make_line("seats", 10_000, 5_000, 1_000, vec![])
        };
        let note = |credited: Option<i64>| {
            negate_line_items(
                std::slice::from_ref(&source),
                &resolved_map(&[("seats", credited)]),
                2,
                false,
            )
            .remove(0)
        };
        let full = note(None);
        assert_eq!(
            full.promotional_credit,
            Some(-6_000),
            "the grants get 60.00"
        );
        assert_eq!(full.note_net_promotional_credit(Some(&source)), -5_000);
        assert_eq!(
            full.note_net_promotional_credit(None),
            -5_000,
            "without the source, the drawn 60.00 is capped at the note's 50.00 reduction"
        );
        assert_eq!(
            note(Some(5_000)).note_net_promotional_credit(Some(&source)),
            -2_500
        );
    }

    /// 100.00 less 3.33 of promotional credit, taxed 20% on 96.67; a note for half the line
    /// credits 50.00 down to 48.34, a reduction of 1.66, all of it promotional credit (no phantom
    /// 0.01 discount), while the grants get back the cumulative 1.67.
    #[test]
    fn a_partial_note_splits_its_reduction_without_a_phantom_discount() {
        let source = LineItem {
            promotional_credit: Some(333),
            ..make_line("seats", 10_000, 9_667, 1_933, vec![])
        };
        let half = negate_line_items(
            std::slice::from_ref(&source),
            &resolved_map(&[("seats", Some(5_000))]),
            2,
            false,
        )
        .remove(0);
        assert_eq!(half.amount_subtotal, -5_000);
        assert_eq!(half.taxable_amount, -4_834);
        assert_eq!(half.promotional_credit, Some(-167), "the grants get 1.67");
        assert_eq!(half.note_net_promotional_credit(Some(&source)), -166);
        let reduction = half.amount_subtotal - half.taxable_amount;
        assert_eq!(
            reduction - half.note_net_promotional_credit(Some(&source)),
            0,
            "no discount"
        );
    }

    /// 0.67 of tax: a note of an earlier release returned 0.33 of it for half the line. The note
    /// for the rest returns the 0.34 left, not the cumulative rule's 0.33 that assumes 0.34 went.
    #[test]
    fn the_last_note_returns_the_tax_earlier_notes_left_whatever_their_rounding() {
        let vat = |tax_amount: i64| TaxDetail {
            tax_rate: dec!(0.067),
            tax_name: "VAT".into(),
            tax_amount,
        };
        let line = make_line("l1", 1_000, 1_000, 67, vec![vat(67)]);
        let rest = |returned: Option<Returned>| {
            let resolved = HashMap::from([(
                "l1".to_owned(),
                ResolvedCredit {
                    credit_subtotal: Some(500),
                    sub_lines_override: None,
                    already_credited: 500,
                    returned,
                },
            )]);
            negate_line_items(std::slice::from_ref(&line), &resolved, 2, false).remove(0)
        };
        let mut earlier = Returned::default();
        earlier
            .add(&make_line("l1", -500, -500, -33, vec![vat(-33)]))
            .unwrap();
        let note = rest(Some(earlier));
        assert_eq!((note.taxable_amount, note.tax_amount), (-500, -34));
        assert_eq!(note.tax_details[0].tax_amount, -34);
        assert_eq!(rest(None).tax_amount, -33, "the rule alone leaves 0.01");

        // The preview of the same note reads the same history.
        let mut earlier = Returned::default();
        earlier
            .add(&make_line("l1", -500, -500, -33, vec![vat(-33)]))
            .unwrap();
        let previewed = credited_lines(
            std::slice::from_ref(&line),
            &[("l1".to_owned(), 500)],
            &HashMap::from([("l1".to_owned(), 500)]),
            &HashMap::from([("l1".to_owned(), earlier)]),
            2,
        );
        assert_eq!(previewed[0].tax_amount, -34);
    }

    /// A line taxed 0.50 + 0.17 at two rates; a note of an earlier release returned 0.25 and 0.09
    /// for half of it. The note for the rest returns 0.25 and 0.08: the cent the rounding leaves
    /// never lands on the detail already returned beyond its share.
    #[test]
    fn the_residual_never_returns_more_of_a_detail_than_it_bore() {
        let detail = |name: &str, rate: Decimal, tax_amount: i64| TaxDetail {
            tax_rate: rate,
            tax_name: name.into(),
            tax_amount,
        };
        let line = make_line(
            "l1",
            1_000,
            1_000,
            67,
            vec![
                detail("VAT", dec!(0.05), 50),
                detail("City", dec!(0.017), 17),
            ],
        );
        let mut earlier = Returned::default();
        earlier
            .add(&make_line(
                "l1",
                -500,
                -500,
                -34,
                vec![
                    detail("VAT", dec!(0.05), -25),
                    detail("City", dec!(0.017), -9),
                ],
            ))
            .unwrap();
        let resolved = HashMap::from([(
            "l1".to_owned(),
            ResolvedCredit {
                credit_subtotal: Some(500),
                sub_lines_override: None,
                already_credited: 500,
                returned: Some(earlier),
            },
        )]);
        let note = negate_line_items(&[line], &resolved, 2, false).remove(0);
        assert_eq!(note.tax_amount, -33);
        let amounts: Vec<i64> = note.tax_details.iter().map(|d| d.tax_amount).collect();
        assert_eq!(amounts, vec![-25, -8], "0.50 and 0.17 returned in all");
    }

    /// The residual is spread within each detail's room: two details of 0.01 each with a line
    /// expecting nothing both return 0, where the largest alone would have charged 0.01 of tax.
    #[test]
    fn the_residual_spills_over_details_within_their_room() {
        let detail = |name: &str, rate: Decimal, tax_amount: i64| TaxDetail {
            tax_rate: rate,
            tax_name: name.into(),
            tax_amount,
        };
        let details = [
            detail("VAT", dec!(0.05), 40),
            detail("City", dec!(0.035), 28),
        ];
        let mut earlier = Returned::default();
        earlier
            .add(&make_line(
                "l1",
                -500,
                -500,
                -35,
                vec![
                    detail("VAT", dec!(0.05), -21),
                    detail("City", dec!(0.035), -14),
                ],
            ))
            .unwrap();
        let out = prorate_and_negate_tax_details(&details, (500, 50, 1_000), Some(&earlier), 0);
        let amounts: Vec<i64> = out.iter().map(|d| d.tax_amount).collect();
        assert_eq!(amounts, vec![0, 0]);
    }

    /// A line's promotional share of 333, credited in two halves, goes back whole: 167 then 166.
    #[test]
    fn two_half_notes_return_the_whole_promotional_share() {
        let line = LineItem {
            promotional_credit: Some(333),
            ..make_line("seats", 1_000, 667, 0, vec![])
        };
        let returned = |already: i64| {
            let resolved = HashMap::from([(
                "seats".to_owned(),
                ResolvedCredit {
                    credit_subtotal: Some(500),
                    sub_lines_override: None,
                    already_credited: already,
                    returned: None,
                },
            )]);
            negate_line_items(std::slice::from_ref(&line), &resolved, 2, false)[0]
                .promotional_credit
        };
        assert_eq!(returned(0), Some(-167));
        assert_eq!(returned(500), Some(-166));
    }

    /// A note drafted by an older release drops what its line recorded; finalization reads the
    /// subscription from the invoice line it credits, so scoped credit goes back to its grant.
    #[test]
    fn a_note_line_bills_the_subscription_of_the_invoice_line_it_credits() {
        use common_domain::ids::{BaseId, SubscriptionId};
        let (a, invoice_subscription) = (SubscriptionId::new(), SubscriptionId::new());
        let shortfall = LineItem {
            subscription_id: Some(a),
            ..make_line("minimum", 10_000, 10_000, 0, vec![])
        };
        let fee = make_line("fee", 5_000, 5_000, 0, vec![]);
        let note = [
            make_line("minimum", -10_000, -10_000, 0, vec![]),
            make_line("fee", -5_000, -5_000, 0, vec![]),
            LineItem {
                subscription_id: Some(a),
                ..make_line("gone", -100, -100, 0, vec![])
            },
        ];
        let owned = super::financial::owned_note_lines(
            &note,
            &[shortfall, fee],
            Some(invoice_subscription),
            &crate::services::credit::scope::LineOwners::default(),
        );
        assert_eq!(
            owned
                .iter()
                .map(|line| line.subscription_id)
                .collect::<Vec<_>>(),
            vec![Some(a), Some(invoice_subscription), Some(a)]
        );
    }
}
