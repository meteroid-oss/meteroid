use super::enums::{InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType};
use crate::domain::connectors::ConnectionMeta;
use crate::domain::einvoicing::{EInvoicingReport, EInvoicingStatus};
use crate::domain::invoice_lines::LineItem;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::{Address, CouponLineItem, Customer, InvoicingEntity, PlanVersionOverview};
use crate::errors::{StoreError, StoreErrorReport};
use crate::services::CustomerDetailsUpdate;
use chrono::{NaiveDate, NaiveDateTime, Utc};
use common_domain::ids::{
    BaseId, CustomerId, InvoiceId, InvoicingEntityId, PlanVersionId, StoredDocumentId,
    SubscriptionId, TenantId,
};
use diesel_models::invoices::DetailedInvoiceRow;
use diesel_models::invoices::InvoiceRow;
use diesel_models::invoices::InvoiceRowLinesPatch;
use diesel_models::invoices::InvoiceRowNew;
use diesel_models::invoices::InvoiceWithCustomerRow;
use diesel_models::payments::PaymentTransactionRow;
use error_stack::Report;
use o2o::o2o;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, o2o, PartialEq, Eq)]
#[try_from_owned(InvoiceRow, StoreErrorReport)]
pub struct Invoice {
    pub id: InvoiceId,
    #[from(~.into())]
    pub status: InvoiceStatusEnum,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: Option<SubscriptionId>,
    pub currency: String,
    pub invoice_number: String,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub plan_version_id: Option<PlanVersionId>,
    #[from(~.into())]
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub amount_due: i64,
    pub applied_credits: i64,
    pub net_terms: i32,
    pub reference: Option<String>,
    pub memo: Option<String>,
    pub due_at: Option<NaiveDateTime>,
    pub plan_name: Option<String>,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize customer_details".to_string(), e)
    }) ?)]
    pub customer_details: InlineCustomer,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize seller_details".to_string(), e)
    }) ?)]
    pub seller_details: InlineInvoicingEntity,
    pub pdf_document_id: Option<StoredDocumentId>,
    pub xml_document_id: Option<StoredDocumentId>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub conn_meta: Option<ConnectionMeta>,
    pub auto_advance: bool,
    pub issued_at: Option<NaiveDateTime>,
    #[from(~.into())]
    pub payment_status: InvoicePaymentStatus,
    pub paid_at: Option<NaiveDateTime>,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize coupons".to_string(), e)
    }) ?)]
    pub coupons: Vec<CouponLineItem>,
    pub discount: i64,
    pub purchase_order: Option<String>,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize tax_breakdown".to_string(), e)
    }) ?)]
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    pub manual: bool,
    pub voided_at: Option<NaiveDateTime>,
    pub marked_as_uncollectible_at: Option<NaiveDateTime>,
    pub invoicing_entity_id: InvoicingEntityId,
    pub parent_invoice_id: Option<InvoiceId>,
    /// Set on a per-subscription draft that has been merged into a consolidated invoice.
    /// Such a draft is superseded: it is never finalized or charged on its own.
    pub consolidated_into_invoice_id: Option<InvoiceId>,
    /// Resolved appendices (event-detail tables, static text, …) rendered after the invoice
    /// body. A presentation snapshot — resolved at draft time and frozen at finalization.
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize annexes".to_string(), e)
    }) ?)]
    pub annexes: Vec<InvoiceAnnex>,
    /// User-defined property values, keyed by definition `key`.
    pub custom_properties: serde_json::Value,
    /// Cycle these lines were computed for, so a refresh of a draft that outlived a renewal still
    /// bills its own period. Null on pre-column rows.
    #[from(~.map(|v| v.max(0) as u32))]
    pub cycle_index: Option<u32>,
    /// Set when an operator edited this draft's billing content (lines or discount). A manual
    /// finalize of such a draft keeps the stored lines instead of settling with a refresh.
    pub manually_edited_at: Option<NaiveDateTime>,
    /// Set when an operator edited this draft's billing identity. Null means `customer_details` is
    /// a cache that tracks the live customer until finalization freezes it.
    pub customer_details_overridden_at: Option<NaiveDateTime>,
    /// Start of the billing period this invoice settles — the stable period key (dedup,
    /// plan-change guards). `invoice_date` is the emission date, stamped at finalization.
    pub billing_period_start: NaiveDate,
    /// Whether the structured e-invoice was produced. `None` when the invoicing
    /// entity had not opted in when this invoice was issued.
    #[from(~.map(| v | v.into()))]
    pub einvoicing_status: Option<EInvoicingStatus>,
    /// Why it was not, in the standard's own vocabulary. Never blocks issuance.
    #[from(~.and_then(| v | serde_json::from_value(v).ok()))]
    pub einvoicing_report: Option<EInvoicingReport>,
    /// The prices billed here were quoted tax-included. Every amount on the invoice is net
    /// regardless — the tax was carved out of the quoted price rather than added to it — so
    /// nothing that reads amounts needs this. It tells a recompute which way to read fresh
    /// prices, and a renderer that the total is the price the customer was quoted.
    pub tax_inclusive: bool,
    /// When an automatic charge was requested at finalization; `None` = pay-link invoice.
    pub auto_charge_requested_at: Option<NaiveDateTime>,
    /// Promotional credit taken off the charges before tax, net like `discount`:
    /// `subtotal - discount - promotional_credits + tax_amount = total`.
    pub promotional_credits: i64,
    /// When a dispute first withdrew money still lost on its payments; the invoice stays paid.
    pub payment_disputed_at: Option<NaiveDateTime>,
}

/// Per-subscription inputs for projecting a draft's emission/due dates. The default yields a
/// "today" projection.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmissionProjectionContext {
    pub backdated: bool,
    pub auto_advance: bool,
    pub grace_period_hours: i32,
    /// An immediate termination's instant: the grace of the invoice billing up to it starts there.
    pub terminated_at: Option<NaiveDateTime>,
}

/// The day a draft's emission/due dates project to. An `auto_advance` draft auto-finalizes at
/// grace-end (`billing_period_start + grace_period_hours`), clamped so it never precedes `today`
/// (a draft past its grace, or a lagging worker, finalizes ~now). Otherwise it waits for a manual
/// finalize, whose day is unknowable, so it projects `today`.
#[cfg(test)]
fn emission_anchor(
    today: NaiveDate,
    billing_period_start: NaiveDate,
    auto_advance: bool,
    grace_period_hours: i32,
) -> NaiveDate {
    emission_anchor_from(
        today,
        billing_period_start.and_time(chrono::NaiveTime::MIN),
        auto_advance,
        grace_period_hours,
    )
}

/// `emission_anchor` for a draft whose window ends at `window_end`.
fn emission_anchor_from(
    today: NaiveDate,
    window_end: NaiveDateTime,
    auto_advance: bool,
    grace_period_hours: i32,
) -> NaiveDate {
    if auto_advance {
        let grace_end =
            (window_end + chrono::Duration::hours(i64::from(grace_period_hours))).date();
        today.max(grace_end)
    } else {
        today
    }
}

/// A resolved appendix carried by an invoice. This is the already-projected, already-grouped
/// presentation snapshot — it mirrors `meteroid_invoicing::model::Annex` and performs no
/// computation at render time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceAnnex {
    pub title: String,
    pub description: Option<String>,
    pub table: InvoiceAnnexTable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceAnnexTable {
    pub columns: Vec<String>,
    pub groups: Vec<InvoiceAnnexGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceAnnexGroup {
    pub heading: Option<String>,
    pub rows: Vec<InvoiceAnnexRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceAnnexRow {
    pub cells: Vec<InvoiceAnnexCell>,
    /// Emphasized rows (subtotals / grand total) are rendered bold with a fill.
    #[serde(default)]
    pub emphasis: bool,
}

/// A single annex cell, kept as a typed value rather than a pre-formatted string so the invoice
/// renderer can format it with the document's language and currency — consistently with the rest
/// of the invoice. `Money` is carried in minor currency units; dates carry their strftime pattern.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum InvoiceAnnexCell {
    Empty,
    Text(String),
    Number(rust_decimal::Decimal),
    Money(rust_decimal::Decimal),
    Date {
        ts: NaiveDateTime,
        fmt: String,
    },
    DateRange {
        min: NaiveDateTime,
        max: NaiveDateTime,
        fmt: String,
    },
}

impl Invoice {
    pub fn can_edit(&self) -> bool {
        self.status == InvoiceStatusEnum::Draft
    }

    /// Paid with nothing collected: credit notes cancelled the whole charge, as a Stripe invoice
    /// a credit note brings to zero is `paid`. `debt_cancelled` is what its finalized notes
    /// cancelled; `collected`, whether a payment ever settled on it (refunded since included).
    pub fn settled_by_credit_notes(&self, debt_cancelled: i64, collected: bool) -> bool {
        self.payment_status == InvoicePaymentStatus::Paid
            && self.total > 0
            && debt_cancelled >= self.total
            && !collected
    }

    /// True when this invoice reissues a fully credited parent (the credit-note correction
    /// flow). Its lines are a detached clone of an issued document, owned by the operator.
    pub fn is_correction(&self) -> bool {
        self.parent_invoice_id.is_some()
    }

    /// True when this draft has been merged into a consolidated invoice and must not be
    /// finalized or charged independently.
    pub fn is_consolidated_child(&self) -> bool {
        self.consolidated_into_invoice_id.is_some()
    }

    /// Rejects operating on a consolidated child independently of its parent. `operation` names the
    /// attempted action (e.g. "void"), used in the error message.
    pub fn ensure_not_consolidated_child(&self, operation: &str) -> Result<(), StoreError> {
        if self.is_consolidated_child() {
            return Err(StoreError::InvalidArgument(format!(
                "Cannot {operation} an invoice merged into a consolidated parent"
            )));
        }
        Ok(())
    }

    /// A draft is not an issued document yet, so it renders the *current* customer and seller —
    /// the identity that matters is the one at issuance, which finalization then freezes. Anything
    /// past draft keeps its stored snapshot so an issued document never changes under us.
    pub fn tracks_live_seller(&self) -> bool {
        self.status == InvoiceStatusEnum::Draft
    }

    /// As [`Self::tracks_live_seller`], but an operator who edited this draft's billing identity
    /// has taken ownership of it: their override must survive later customer-record changes.
    pub fn tracks_live_customer(&self) -> bool {
        self.tracks_live_seller() && self.customer_details_overridden_at.is_none()
    }

    /// The dates this invoice would carry if issued on `today`: `today` as the emission date and
    /// a due date rebased on it — unless an operator has taken ownership of the due date, which
    /// survives untouched. Ownership means anything other than the draft's computed default:
    /// an explicit date, or a cleared (`None`) due date. Finalization stamps with this; draft
    /// rendering projects with it. Callers gate on backdated subscriptions and manual invoices
    /// themselves, which keep their stored dates.
    pub fn dates_if_issued_on(&self, today: NaiveDate) -> (NaiveDate, Option<NaiveDateTime>) {
        let net_terms = chrono::Duration::days(i64::from(self.net_terms));
        let default_due = (self.invoice_date + net_terms).and_time(chrono::NaiveTime::MIN);
        // Only rebase the untouched default; an operator-set or cleared (None) date survives.
        let due_at = if self.due_at == Some(default_due) {
            Some((today + net_terms).and_time(chrono::NaiveTime::MIN))
        } else {
            self.due_at
        };
        (today, due_at)
    }

    /// Projects onto a draft the dates finalization will stamp, for display. No-op on finalized,
    /// manual, or backdated invoices. An `auto_advance` draft auto-finalizes at grace-end
    /// (`billing_period_start + grace_period_hours`), clamped to `today`; otherwise it waits for a
    /// manual finalize, whose day is unknowable, so it projects `today`.
    /// Returns whether the dates were actually projected (false when kept as stored — finalized,
    /// manual, or backdated), so callers can label the emission date honestly.
    pub fn project_emission_dates(
        &mut self,
        backdated: bool,
        auto_advance: bool,
        grace_period_hours: i32,
    ) -> bool {
        self.project_emission_dates_ending(backdated, auto_advance, grace_period_hours, None)
    }

    /// `project_emission_dates`; a draft created by the immediate termination at `terminated_at`
    /// bills up to that instant, and its grace starts there (`schedule_invoice_finalization`).
    fn project_emission_dates_ending(
        &mut self,
        backdated: bool,
        auto_advance: bool,
        grace_period_hours: i32,
        terminated_at: Option<NaiveDateTime>,
    ) -> bool {
        if self.can_edit() && !self.manual && !backdated {
            let opens = self.billing_period_start.and_time(chrono::NaiveTime::MIN);
            let window_end = terminated_at
                .filter(|at| at.date() == self.billing_period_start && self.created_at >= *at)
                .map_or(opens, |at| at.max(opens));
            let issued_on = emission_anchor_from(
                chrono::Utc::now().date_naive(),
                window_end,
                auto_advance,
                grace_period_hours,
            );
            let (invoice_date, due_at) = self.dates_if_issued_on(issued_on);
            self.invoice_date = invoice_date;
            self.due_at = due_at;
            true
        } else {
            false
        }
    }

    pub fn project_emission_dates_with(&mut self, ctx: Option<EmissionProjectionContext>) -> bool {
        let ctx = ctx.unwrap_or_default();
        self.project_emission_dates_ending(
            ctx.backdated,
            ctx.auto_advance,
            ctx.grace_period_hours,
            ctx.terminated_at,
        )
    }

    /// Overlays the live customer record onto a tracking draft. No-op once frozen or overridden,
    /// which is what makes this safe to call from any read path.
    pub fn overlay_live_customer(&mut self, customer: &Customer) {
        self.overlay_customer_details(customer.clone().into());
    }

    /// Overlays the live invoicing entity onto a tracking draft. See [`Self::overlay_live_customer`].
    pub fn overlay_live_seller(&mut self, seller: &InvoicingEntity) {
        self.overlay_seller_details(seller.clone().into());
    }

    /// The gate behind [`Self::overlay_live_customer`], split out so the tracking rule can be
    /// exercised without materializing a whole customer record.
    pub fn overlay_customer_details(&mut self, details: InlineCustomer) {
        if self.tracks_live_customer() {
            self.customer_details = details;
        }
    }

    /// The gate behind [`Self::overlay_live_seller`]. See [`Self::overlay_customer_details`].
    pub fn overlay_seller_details(&mut self, details: InlineInvoicingEntity) {
        if self.tracks_live_seller() {
            self.seller_details = details;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaxExemptionType {
    ReverseCharge,
    TaxExempt,
    /// No tax was applied and the engine could not say why: a country was missing, the
    /// tenant resolves tax outside Meteroid, or no rate matched the scenario. It is
    /// *not* "the seller is not VAT-registered" — nothing establishes that — so a
    /// document must not put a statutory exemption citation against it.
    ///
    /// Persisted in the `tax_breakdown` jsonb column, where rows written before the
    /// rename still carry the old name.
    #[serde(alias = "NotRegistered")]
    NotRegistered,
    Export,
    /// Taxable in a part of the seller's own VAT territory where VAT does not apply:
    /// Guyane and Mayotte (CGI art. 294-1). Out of scope like an export, but not one —
    /// the supply never leaves the territory, so it cites art. 294-1 rather than a
    /// place-of-supply article and reports apart from exports.
    ///
    /// Persisted in the `tax_breakdown` jsonb column; the variant name is part of the
    /// stored format.
    NoVatTerritory,
}

/// Whether the tax engine priced the supply as one to a taxable person.
///
/// Persisted alongside the exemption because the place-of-supply article a document
/// cites must match the basis the tax was computed on. Deriving it again at render
/// time reads a different field — the invoice's customer snapshot, which an operator
/// can edit without the engine re-running — and the two then disagree about whether
/// the customer is a taxable person, on a document that states it as fact.
///
/// `None` on rows written before this was recorded; renderers fall back to the
/// snapshot for those, which is what they did for every row previously.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupplyBasis {
    Business,
    Consumer,
}

/// What levy an engine-resolved breakdown row is (VAT, Canadian GST/HST/PST/QST,
/// US sales tax), recorded as a fact so document renderers can word the row label
/// in the presentation language instead of parroting the engine's English `name`
/// ("USt." not "VAT" on a German invoice).
///
/// `name` stays authoritative for merchant-authored custom taxes (which carry no
/// kind — the merchant chose that name) and for rows persisted before the kind was
/// recorded: those keep rendering the label they were issued with, because a
/// finalized document's snapshot must not be re-worded retroactively by a guess.
///
/// Persisted in the `tax_breakdown` jsonb column; variant names are part of the
/// stored format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaxKind {
    Vat,
    Gst,
    Hst,
    Pst,
    Qst,
    SalesTax,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaxBreakdownItem {
    // Signed: credit/proration lines net negative and must reconcile with the
    // invoice-level signed tax_amount (W4). Never clamp per-rate rows.
    pub taxable_amount: i64,
    #[serde(default)]
    pub tax_amount: i64,
    pub tax_rate: Decimal,
    pub name: String,
    pub exemption_type: Option<TaxExemptionType>,
    /// Free-text legal exemption mention (EU exempt/reverse-charge invoices).
    #[serde(default)]
    pub exemption_reason: Option<String>,
    /// Accounting/reporting code of the tax rate that produced this line (W1),
    /// for accounting exports. `None` for engine-computed VAT and exemptions.
    #[serde(default)]
    pub tax_reference: Option<String>,
    /// True when a merchant override replaced the built-in EU VAT engine rate on
    /// this line (C2 precedence ladder). Drives the "EU VAT not applied" hint.
    #[serde(default)]
    pub overridden: bool,
    /// The basis the engine priced this on. See [`SupplyBasis`].
    #[serde(default)]
    pub supply_basis: Option<SupplyBasis>,
    /// What the levy is, for engine-resolved rates. See [`TaxKind`]; `None` for
    /// merchant custom taxes, exemption rows, and rows written before this existed.
    #[serde(default)]
    pub tax_kind: Option<TaxKind>,
    /// Currency credit sold on the document, taxed when the credit is used: stated so its line
    /// has a category, never reported as a supply (VAT return, EC Sales List).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub prepaid_credit: bool,
}

#[derive(Debug, o2o)]
#[owned_try_into(InvoiceRowNew, StoreErrorReport)]
#[ghosts(id: {InvoiceId::new()})]
pub struct InvoiceNew {
    #[into(~.into())]
    pub status: InvoiceStatusEnum,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: Option<SubscriptionId>,
    pub currency: String,
    pub invoice_number: String,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize coupons".to_string(), e)
    }) ?)]
    pub coupons: Vec<CouponLineItem>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub plan_version_id: Option<PlanVersionId>,
    #[into(~.into())]
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub discount: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub amount_due: i64,
    pub applied_credits: i64,
    pub promotional_credits: i64,
    pub net_terms: i32,
    pub reference: Option<String>,
    pub purchase_order: Option<String>,
    pub memo: Option<String>,
    pub due_at: Option<NaiveDateTime>, // TODO due_date
    pub plan_name: Option<String>,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize customer_details".to_string(), e)
    }) ?)]
    pub customer_details: InlineCustomer,
    #[into(serde_json::to_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize seller_details".to_string(), e)
    }) ?)]
    pub seller_details: InlineInvoicingEntity,
    pub auto_advance: bool,
    #[into(~.into())]
    pub payment_status: InvoicePaymentStatus,
    #[into(serde_json::to_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize tax_breakdown".to_string(), e)
    }) ?)]
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    pub manual: bool,
    pub invoicing_entity_id: InvoicingEntityId,
    pub parent_invoice_id: Option<InvoiceId>,
    pub consolidated_into_invoice_id: Option<InvoiceId>,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize annexes".to_string(), e)
    }) ?)]
    pub annexes: Vec<InvoiceAnnex>,
    // JSON object of custom property values. Null means "not managed" (validation is skipped and
    // the row falls back to an empty object); validated against tenant definitions at insert time.
    pub custom_properties: serde_json::Value,
    /// Cycle these lines were computed for; see `Invoice::cycle_index`.
    #[into(~.map(|v| v as i32))]
    pub cycle_index: Option<u32>,
    /// See `Invoice::billing_period_start`.
    pub billing_period_start: NaiveDate,
    /// See `Invoice::tax_inclusive`.
    pub tax_inclusive: bool,
}

#[derive(Debug, o2o)]
#[owned_try_into(InvoiceRowLinesPatch, StoreErrorReport)]
pub struct InvoiceLinesPatch {
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    pub amount_due: i64,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub total: i64,
    pub tax_amount: i64,
    pub applied_credits: i64,
    pub promotional_credits: i64,
    #[ghost({vec![]})]
    pub applied_coupons: Vec<CouponLineItem>,
    #[into(serde_json::to_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize tax_breakdown".to_string(), e)
    }) ?)]
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize annexes".to_string(), e)
    }) ?)]
    pub annexes: Vec<InvoiceAnnex>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct InlineCustomer {
    pub id: CustomerId,
    pub name: String,
    pub email: Option<String>,
    pub alias: Option<String>,
    pub vat_number: Option<String>,
    /// BT-47 at snapshot time. Absent on documents issued before the column
    /// existed, which is what they were issued with.
    #[serde(default)]
    pub legal_number: Option<String>,
    /// BT-10 at snapshot time.
    #[serde(default)]
    pub buyer_reference: Option<String>,
    pub billing_address: Option<Address>,
    pub snapshot_at: NaiveDateTime,
    /// Customer's preferred document languages captured at snapshot time, most-preferred
    /// first; empty inherits from the invoicing entity at render time.
    #[serde(
        default,
        alias = "invoicing_language",
        deserialize_with = "preferred_locales_compat"
    )]
    pub preferred_locales: Vec<String>,
}

/// Accepts the legacy scalar `invoicing_language` as well as the list, so documents
/// snapshotted before the switch keep rendering in the language they were issued with.
fn preferred_locales_compat<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }

    Ok(match Option::<OneOrMany>::deserialize(deserializer)? {
        Some(OneOrMany::One(tag)) if !tag.trim().is_empty() => vec![tag],
        Some(OneOrMany::Many(tags)) => tags,
        _ => Vec::new(),
    })
}

impl InlineCustomer {
    /// Field-wise equality ignoring `snapshot_at`, which always differs. Used to tell a genuine
    /// per-invoice override from a client that simply echoed back the values it was shown.
    pub fn matches_record(&self, other: &InlineCustomer) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.email == other.email
            && self.alias == other.alias
            && self.vat_number == other.vat_number
            && self.billing_address == other.billing_address
            && self.preferred_locales == other.preferred_locales
    }
}

impl From<Customer> for InlineCustomer {
    fn from(value: Customer) -> Self {
        InlineCustomer {
            id: value.id,
            name: value.name.clone(),
            email: value.billing_email.clone(),
            alias: value.alias.clone(),
            vat_number: value.vat_number.clone(),
            legal_number: value.legal_number.clone(),
            buyer_reference: value.buyer_reference.clone(),
            billing_address: value.billing_address.clone(),
            snapshot_at: chrono::Utc::now().naive_utc(),
            preferred_locales: value.preferred_locales.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct InlineInvoicingEntity {
    pub id: InvoicingEntityId,
    pub legal_name: String,
    pub vat_number: Option<String>,
    pub address: Address,
    pub snapshot_at: NaiveDateTime,
    /// Entity default document language captured at snapshot time, so a document's
    /// statutory mentions keep the wording they were issued with. Absent on
    /// documents snapshotted before the column existed; those fall back to the
    /// entity country's locale, which is what they rendered with originally.
    #[serde(default)]
    pub invoicing_language: Option<String>,
    /// BT-30 at snapshot time. Absent on documents issued before the column
    /// existed, which is what they were issued with.
    #[serde(default)]
    pub legal_number: Option<String>,
    /// BT-41 / BT-42 / BT-43 at snapshot time.
    #[serde(default)]
    pub contact_name: Option<String>,
    #[serde(default)]
    pub contact_phone: Option<String>,
    #[serde(default)]
    pub contact_email: Option<String>,
}

impl From<InvoicingEntity> for InlineInvoicingEntity {
    fn from(value: InvoicingEntity) -> Self {
        InlineInvoicingEntity {
            id: value.id,
            legal_name: value.legal_name.clone(),
            vat_number: value.vat_number.clone(),
            address: value.address(),
            snapshot_at: chrono::Utc::now().naive_utc(),
            invoicing_language: value.invoicing_language.clone(),
            legal_number: value.legal_number.clone(),
            contact_name: value.contact_name.clone(),
            contact_phone: value.contact_phone.clone(),
            contact_email: value.contact_email.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceWithCustomer {
    pub invoice: Invoice,
    pub customer: Customer,
}

impl TryFrom<InvoiceWithCustomerRow> for InvoiceWithCustomer {
    type Error = Report<StoreError>;

    fn try_from(value: InvoiceWithCustomerRow) -> Result<Self, Self::Error> {
        let mut invoice: Invoice = value.invoice.try_into()?;
        let customer: Customer = value.customer.try_into()?;
        // The join already carried the live record here, so a tracking draft never escapes this
        // boundary showing a stale name or address.
        invoice.overlay_live_customer(&customer);

        Ok(InvoiceWithCustomer { invoice, customer })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetailedInvoice {
    pub invoice: Invoice,
    pub customer: Customer,
    pub plan: Option<PlanVersionOverview>,
    pub transactions: Vec<PaymentTransaction>,
}

impl DetailedInvoice {
    pub fn with_transactions(mut self, transactions: Vec<PaymentTransaction>) -> Self {
        self.transactions = transactions;
        self
    }

    pub fn with_transaction_rows(mut self, transactions: Vec<PaymentTransactionRow>) -> Self {
        self.transactions = transactions
            .into_iter()
            .map(std::convert::Into::into)
            .collect();
        self
    }
}

impl TryFrom<DetailedInvoiceRow> for DetailedInvoice {
    type Error = Report<StoreError>;

    fn try_from(value: DetailedInvoiceRow) -> Result<Self, Self::Error> {
        let mut invoice: Invoice = value.invoice.try_into()?;
        let customer: Customer = value.customer.try_into()?;
        invoice.overlay_live_customer(&customer);

        Ok(DetailedInvoice {
            invoice,
            customer,
            plan: value.plan.map(std::convert::Into::into),
            transactions: vec![],
        })
    }
}

impl From<InvoiceNew> for Invoice {
    fn from(value: InvoiceNew) -> Self {
        Invoice {
            id: InvoiceId::new(),
            status: value.status,
            created_at: Utc::now().naive_utc(),
            billing_period_start: value.billing_period_start,
            updated_at: None,
            tenant_id: value.tenant_id,
            customer_id: value.customer_id,
            subscription_id: value.subscription_id,
            currency: value.currency,
            invoice_number: value.invoice_number,
            line_items: value.line_items,
            data_updated_at: None,
            invoice_date: value.invoice_date,
            plan_version_id: value.plan_version_id,
            invoice_type: value.invoice_type,
            finalized_at: None,
            subtotal: value.subtotal,
            subtotal_recurring: value.subtotal_recurring,
            tax_amount: value.tax_amount,
            total: value.total,
            amount_due: value.amount_due,
            applied_credits: 0,
            promotional_credits: value.promotional_credits,
            net_terms: value.net_terms,
            reference: value.reference,
            memo: value.memo,
            due_at: value.due_at,
            plan_name: value.plan_name,
            customer_details: value.customer_details,
            customer_details_overridden_at: None,
            seller_details: value.seller_details,
            pdf_document_id: None,
            xml_document_id: None,
            einvoicing_status: None,
            einvoicing_report: None,
            conn_meta: None,
            auto_advance: value.auto_advance,
            issued_at: None,
            payment_status: value.payment_status,
            paid_at: None,
            coupons: value.coupons,
            discount: value.discount,
            purchase_order: value.purchase_order,
            tax_breakdown: value.tax_breakdown,
            manual: value.manual,
            voided_at: None,
            marked_as_uncollectible_at: None,
            payment_disputed_at: None,
            invoicing_entity_id: value.invoicing_entity_id,
            parent_invoice_id: value.parent_invoice_id,
            consolidated_into_invoice_id: value.consolidated_into_invoice_id,
            annexes: value.annexes,
            custom_properties: if value.custom_properties.is_null() {
                serde_json::json!({})
            } else {
                value.custom_properties
            },
            cycle_index: value.cycle_index,
            manually_edited_at: None,
            tax_inclusive: value.tax_inclusive,
            auto_charge_requested_at: None,
        }
    }
}

#[derive(Default)]
pub struct UpdateInvoiceParams {
    pub memo: Option<Option<String>>,
    pub reference: Option<Option<String>>,
    pub purchase_order: Option<Option<String>>,
    pub due_date: Option<Option<NaiveDate>>,
    pub line_items: Option<Vec<UpdateLineItemParams>>,
    pub discount: Option<String>,
    pub customer_details: Option<CustomerDetailsUpdate>,
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    /// Replacement annexes. `None` leaves the stored snapshot untouched. Only editable on drafts
    /// that no longer re-derive their appendix (corrections, hand-edited drafts).
    pub annexes: Option<Vec<InvoiceAnnex>>,
    /// Custom property values to merge (a key with a null value removes it). `None` leaves the
    /// existing bag untouched.
    pub custom_properties: Option<serde_json::Value>,
}

pub struct UpdateLineItemParams {
    pub id: Option<String>,
    pub name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub quantity: Option<Decimal>, // can be None for items with sublines
    pub unit_price: Option<Decimal>, // same
    pub tax_rate: Decimal,
    pub description: Option<String>,
    pub sub_lines: Vec<crate::domain::invoice_lines::SubLineItem>,
}

#[cfg(test)]
mod emission_anchor_tests {
    use super::{emission_anchor, emission_anchor_from};
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn manual_draft_projects_today() {
        let today = d(2026, 8, 25);
        // auto_advance = false: period start and grace are irrelevant.
        assert_eq!(emission_anchor(today, d(2026, 5, 1), false, 240), today);
    }

    #[test]
    fn auto_advance_projects_grace_end_when_in_the_future() {
        let today = d(2026, 8, 25);
        // Period starts today; 48h grace lands two days out.
        assert_eq!(emission_anchor(today, today, true, 48), d(2026, 8, 27));
    }

    #[test]
    fn auto_advance_clamps_to_today_when_grace_already_elapsed() {
        let today = d(2026, 8, 25);
        // A period from months ago whose grace is long past finalizes ~now, not in the past.
        assert_eq!(emission_anchor(today, d(2026, 5, 1), true, 24), today);
    }

    #[test]
    fn grace_end_uses_whole_days_from_midnight() {
        let today = d(2026, 1, 1);
        // 25h past midnight is still the next calendar day; the consolidation 1h floor can't
        // change a date-granularity result.
        assert_eq!(
            emission_anchor(today, d(2026, 6, 1), true, 25),
            d(2026, 6, 2)
        );
    }

    /// An immediate termination's invoice waits its grace from the cancellation: cancelled at
    /// 20:00 with 12h of grace, it is issued the next day.
    #[test]
    fn a_termination_s_grace_starts_at_its_instant() {
        let today = d(2026, 6, 1);
        let cancelled = today.and_hms_opt(20, 0, 0).unwrap();
        assert_eq!(
            emission_anchor_from(today, cancelled, true, 12),
            d(2026, 6, 2)
        );
        assert_eq!(emission_anchor(today, today, true, 12), today);
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use crate::domain::Address;
    use common_domain::country::CountryCode;

    /// `tax_breakdown` is a jsonb column: rows written before `tax_kind` existed
    /// carry no such key, and must keep loading (and keep their stored label — the
    /// renderer falls back to `name` when the kind is absent).
    #[test]
    fn tax_breakdown_rows_without_a_kind_still_deserialize() {
        let old_row = serde_json::json!({
            "taxable_amount": 1000,
            "tax_amount": 200,
            "tax_rate": "0.2",
            "name": "VAT",
            "exemption_type": null
        });
        let item: TaxBreakdownItem = serde_json::from_value(old_row).unwrap();
        assert_eq!(item.tax_kind, None);
        assert_eq!(item.name, "VAT");
    }

    /// `tax_breakdown` is a jsonb column, so rows written before the `NotRegistered`
    /// -> `NotRegistered` rename still carry the old name. The serde alias keeps them
    /// loadable; this pins it, because losing the alias would make every pre-rename
    /// invoice fail to deserialize and nothing else would catch it.
    #[test]
    fn tax_exemption_type_reads_the_pre_rename_name() {
        let from_old: TaxExemptionType =
            serde_json::from_value(serde_json::json!("NotRegistered")).unwrap();
        assert_eq!(from_old, TaxExemptionType::NotRegistered);
        let from_new: TaxExemptionType =
            serde_json::from_value(serde_json::json!("NotRegistered")).unwrap();
        assert_eq!(from_new, TaxExemptionType::NotRegistered);
        // New writes use the new name.
        assert_eq!(
            serde_json::to_value(TaxExemptionType::NotRegistered).unwrap(),
            serde_json::json!("NotRegistered")
        );
    }

    fn at(day: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 8, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    fn customer_details(name: &str) -> InlineCustomer {
        InlineCustomer {
            id: CustomerId::new(),
            name: name.to_string(),
            email: None,
            alias: None,
            vat_number: None,
            legal_number: None,
            buyer_reference: None,
            billing_address: None,
            snapshot_at: at(1),
            preferred_locales: Vec::new(),
        }
    }

    fn seller_details(legal_name: &str) -> InlineInvoicingEntity {
        InlineInvoicingEntity {
            id: InvoicingEntityId::new(),
            legal_name: legal_name.to_string(),
            vat_number: None,
            address: Address {
                line1: None,
                line2: None,
                city: None,
                country: CountryCode::parse_as_opt("FR"),
                state: None,
                zip_code: None,
            },
            snapshot_at: at(1),
            invoicing_language: None,
            legal_number: None,
            contact_name: None,
            contact_phone: None,
            contact_email: None,
        }
    }

    fn invoice(status: InvoiceStatusEnum) -> Invoice {
        Invoice {
            id: InvoiceId::new(),
            status,
            created_at: at(1),
            billing_period_start: chrono::NaiveDate::from_ymd_opt(2030, 1, 1).unwrap(),
            updated_at: None,
            tenant_id: TenantId::new(),
            customer_id: CustomerId::new(),
            subscription_id: None,
            currency: "EUR".to_string(),
            invoice_number: "INV-1".to_string(),
            line_items: vec![],
            data_updated_at: None,
            invoice_date: NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
            plan_version_id: None,
            invoice_type: super::super::enums::InvoiceType::Recurring,
            finalized_at: None,
            subtotal: 0,
            subtotal_recurring: 0,
            tax_amount: 0,
            total: 0,
            amount_due: 0,
            applied_credits: 0,
            net_terms: 0,
            reference: None,
            memo: None,
            due_at: None,
            plan_name: None,
            customer_details: customer_details("Stored Customer"),
            seller_details: seller_details("Stored Seller"),
            pdf_document_id: None,
            xml_document_id: None,
            einvoicing_status: None,
            einvoicing_report: None,
            conn_meta: None,
            auto_advance: false,
            issued_at: None,
            payment_status: super::super::enums::InvoicePaymentStatus::Unpaid,
            paid_at: None,
            coupons: vec![],
            discount: 0,
            purchase_order: None,
            tax_breakdown: vec![],
            manual: false,
            cycle_index: None,
            manually_edited_at: None,
            tax_inclusive: false,
            auto_charge_requested_at: None,
            promotional_credits: 0,
            customer_details_overridden_at: None,
            voided_at: None,
            marked_as_uncollectible_at: None,
            payment_disputed_at: None,
            invoicing_entity_id: InvoicingEntityId::new(),
            parent_invoice_id: None,
            consolidated_into_invoice_id: None,
            annexes: vec![],
            custom_properties: serde_json::Value::Null,
        }
    }

    #[test]
    fn draft_tracks_both_sides() {
        let mut inv = invoice(InvoiceStatusEnum::Draft);
        inv.overlay_customer_details(customer_details("Live Customer"));
        inv.overlay_seller_details(seller_details("Live Seller"));

        assert_eq!(inv.customer_details.name, "Live Customer");
        assert_eq!(inv.seller_details.legal_name, "Live Seller");
    }

    /// The whole point of the freeze: an issued document must not change under us.
    #[test]
    fn issued_invoice_keeps_its_snapshot() {
        for status in [
            InvoiceStatusEnum::Finalized,
            InvoiceStatusEnum::Void,
            InvoiceStatusEnum::Uncollectible,
            InvoiceStatusEnum::Closed,
        ] {
            let mut inv = invoice(status);
            inv.overlay_customer_details(customer_details("Live Customer"));
            inv.overlay_seller_details(seller_details("Live Seller"));

            assert_eq!(inv.customer_details.name, "Stored Customer");
            assert_eq!(inv.seller_details.legal_name, "Stored Seller");
        }
    }

    /// An operator's deliberate edit outranks the customer record.
    #[test]
    fn overridden_draft_keeps_customer_but_still_tracks_seller() {
        let mut inv = invoice(InvoiceStatusEnum::Draft);
        inv.customer_details_overridden_at = Some(at(2));

        inv.overlay_customer_details(customer_details("Live Customer"));
        inv.overlay_seller_details(seller_details("Live Seller"));

        assert_eq!(inv.customer_details.name, "Stored Customer");
        assert_eq!(inv.seller_details.legal_name, "Live Seller");
    }

    fn full_address() -> Address {
        Address {
            line1: Some("1 Market St".to_string()),
            line2: Some("Suite 400".to_string()),
            city: Some("San Francisco".to_string()),
            country: CountryCode::parse_as_opt("US"),
            state: Some("CA".to_string()),
            zip_code: Some("94105".to_string()),
        }
    }

    /// The console round-trip: the form shows the resolved identity, the operator edits nothing,
    /// and the values come back. That must not read as an override — otherwise an unrelated save
    /// (a memo edit) pins the draft forever. A dropped address component is enough to break it,
    /// which is exactly how a missing `state` field would surface.
    #[test]
    fn echoed_back_full_address_still_matches() {
        let mut live = customer_details("Acme");
        live.billing_address = Some(full_address());
        live.vat_number = Some("US123".to_string());
        live.email = Some("billing@acme.test".to_string());

        let mut echoed = live.clone();
        echoed.snapshot_at = at(9);

        assert!(
            live.matches_record(&echoed),
            "a faithful round-trip must not count as an override"
        );
    }

    /// Every address component has to survive the round-trip, or the invoice both loses the value
    /// and stops tracking the customer.
    #[test]
    fn dropping_any_address_component_diverges() {
        let mut live = customer_details("Acme");
        live.billing_address = Some(full_address());

        for drop in ["line1", "line2", "city", "state", "zip_code", "country"] {
            let mut partial = live.clone();
            let mut address = full_address();
            match drop {
                "line1" => address.line1 = None,
                "line2" => address.line2 = None,
                "city" => address.city = None,
                "state" => address.state = None,
                "zip_code" => address.zip_code = None,
                "country" => address.country = None,
                _ => unreachable!(),
            }
            partial.billing_address = Some(address);

            assert!(
                !live.matches_record(&partial),
                "dropping {drop} must count as a divergence"
            );
        }
    }

    /// Documents snapshotted before the switch hold a single `invoicing_language`
    /// string; they have to keep rendering in the language they were issued with.
    #[test]
    fn legacy_snapshots_deserialize_into_the_preference_list() {
        let mut json = serde_json::to_value(customer_details("Acme")).unwrap();
        json.as_object_mut().unwrap().remove("preferred_locales");

        for (stored, expected) in [
            (serde_json::json!("fr-FR"), vec!["fr-FR".to_string()]),
            (serde_json::json!(""), vec![]),
            (serde_json::Value::Null, vec![]),
        ] {
            json.as_object_mut()
                .unwrap()
                .insert("invoicing_language".to_string(), stored.clone());
            let parsed: InlineCustomer = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(parsed.preferred_locales, expected, "stored: {stored}");
        }

        // Older still: snapshotted before the column existed at all.
        json.as_object_mut().unwrap().remove("invoicing_language");
        let parsed: InlineCustomer = serde_json::from_value(json).unwrap();
        assert!(parsed.preferred_locales.is_empty());
    }

    #[test]
    fn matches_record_ignores_snapshot_time() {
        let a = customer_details("Acme");
        let mut b = a.clone();
        b.snapshot_at = at(9);

        assert!(a.matches_record(&b));
    }

    /// Each field that shows up on the document has to count as a divergence, or an override
    /// would be dropped and the edit silently reverted on the next read.
    #[test]
    fn matches_record_detects_document_fields() {
        let base = customer_details("Acme");

        let mut name = base.clone();
        name.name = "Acme Subsidiary".to_string();
        assert!(!base.matches_record(&name));

        let mut vat = base.clone();
        vat.vat_number = Some("FR123".to_string());
        assert!(!base.matches_record(&vat));

        let mut lang = base.clone();
        lang.preferred_locales = vec!["fr-FR".to_string()];
        assert!(!base.matches_record(&lang));

        let mut email = base.clone();
        email.email = Some("billing@acme.test".to_string());
        assert!(!base.matches_record(&email));

        let mut address = base.clone();
        address.billing_address = Some(Address {
            line1: Some("1 rue de la Paix".to_string()),
            line2: None,
            city: None,
            country: CountryCode::parse_as_opt("FR"),
            state: None,
            zip_code: None,
        });
        assert!(!base.matches_record(&address));
    }
}
