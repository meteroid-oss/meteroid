use crate::StoreResult;
use crate::domain::enums::InvoiceType;
use crate::domain::{
    CouponLineItem, Customer, Invoice, InvoicingEntity, LineItem, SubscriptionDetails,
    TaxBreakdownItem, TaxResolverEnum, UsagePricingModel,
};
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{
    InvoiceId, PriceComponentId, SubscriptionId, SubscriptionPriceComponentId, TenantId,
};
use diesel_models::invoices::InvoiceRow;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};

use crate::errors::StoreError;
use crate::services::Services;
use crate::services::credit::forecast::CreditBooks;
use crate::services::credit::promotional::{self, TaxedDocument};
use crate::services::credit::value_application::{EstimateContext, billed_periods};
use crate::services::invoice_lines::credit_units::{CoverTarget, CreditUnitCover};
use crate::services::invoice_lines::tax_inputs::customer_for_tax;
use crate::store::PgConn;
use billing_core::assembly::AssemblyInputs;
use billing_core::credit::{Cash, TotalBasis};
use billing_core::{LineAmounts, Totals};
use error_stack::{Report, ResultExt};
use meteroid_tax::{ManualTaxEngine, MeteroidTaxEngine, TaxDetails, TaxEngine};

/// Builds the tax engine for an invoicing entity — the single extension point for
/// adding a tax provider.
///
/// Built-in resolvers (`Manual`, `MeteroidEuVat`) need no credentials and are chosen
/// by `tax_resolver`. An external provider is configured as a `Tax`-typed connector
/// referenced by `invoicing_entity.tax_provider_id`; to add one, load its connector
/// (via the store, making this async) and return your `TaxEngine` from its provider.
///
/// The engine uses an empty reference for taxes without an accounting code
/// (engine-computed VAT); only a real `tax_code` is stored on the breakdown.
fn non_empty(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

/// A charge a coupon or promotional credit took whole: priced, nothing left to tax.
fn takes_rate_probe(line: &LineItem) -> bool {
    let promotional = line.promotional_credit.unwrap_or(0);
    line.taxable_amount == 0
        && line.amount_subtotal > 0
        && (promotional > 0 || line.amount_subtotal - promotional > 0)
        && !crate::services::credit::prepaid_purchase_tax::taxed_at_redemption(line)
}

/// Currency credit sold on the invoice is taxed when it is used, so the tax pass leaves it out. Its
/// own row states it at nothing, so its line has a category (BR-CO-18); kept apart from the
/// supplies, the tax reports leave it out. The category stays the document's zero-rate one.
fn state_untaxed_purchases(lines: &[LineItem], breakdown: &mut Vec<TaxBreakdownItem>) {
    let sold: i64 = lines
        .iter()
        .filter(|line| line.states_prepaid_credit())
        .map(|line| line.taxable_amount)
        .sum();
    if sold == 0 {
        return;
    }
    let exempt = breakdown
        .iter()
        .find(|row| row.tax_rate.is_zero() && row.exemption_type.is_some())
        .cloned();
    // The levy the document's other rows name, when they all name the same one.
    let tax_kind = breakdown
        .first()
        .map(|row| row.tax_kind)
        .filter(|kind| breakdown.iter().all(|row| row.tax_kind == *kind))
        .flatten();
    breakdown.push(match exempt {
        Some(exempt) => TaxBreakdownItem {
            taxable_amount: sold,
            tax_amount: 0,
            tax_reference: None,
            overridden: false,
            prepaid_credit: true,
            ..exempt
        },
        None => TaxBreakdownItem {
            taxable_amount: sold,
            tax_amount: 0,
            tax_rate: Decimal::ZERO,
            name: "Tax".to_owned(),
            tax_kind,
            exemption_type: None,
            exemption_reason: None,
            tax_reference: None,
            overridden: false,
            supply_basis: None,
            prepaid_credit: true,
        },
    });
}

/// The taxes `details` levies, rates and names only.
pub(in crate::services) fn rates_of(details: &TaxDetails) -> Vec<crate::domain::TaxDetail> {
    let detail = |tax_rate: Decimal, tax_name: &str| crate::domain::TaxDetail {
        tax_rate,
        tax_name: tax_name.to_owned(),
        tax_amount: 0,
    };
    match details {
        TaxDetails::Tax {
            tax_rate, tax_name, ..
        } => vec![detail(*tax_rate, tax_name)],
        TaxDetails::MultipleTaxes { taxes, .. } => taxes
            .iter()
            .map(|t| detail(t.tax_rate, &t.tax_name))
            .collect(),
        TaxDetails::Exempt(_) => vec![],
    }
}

/// `Ok(None)` means "apply no tax" (`tax_resolver = None`).
pub(in crate::services) fn build_tax_engine(
    invoicing_entity: &InvoicingEntity,
) -> StoreResult<Option<Box<dyn TaxEngine + Send + Sync>>> {
    if invoicing_entity.tax_resolver == TaxResolverEnum::External
        || invoicing_entity.tax_provider_id.is_some()
    {
        // TODO(tax-provider): load the Tax connector and match on its provider to
        // return the corresponding TaxEngine (see the add-tax-provider skill).
        // No external tax engine is registered yet.
        return Err(Report::new(StoreError::InvalidArgument(
            "tax_provider_id is set but no external tax engine is registered".to_string(),
        )));
    }

    Ok(match invoicing_entity.tax_resolver {
        TaxResolverEnum::None => None,
        TaxResolverEnum::Manual => Some(Box::new(ManualTaxEngine {})),
        TaxResolverEnum::MeteroidEuVat => Some(Box::new(MeteroidTaxEngine {})),
        TaxResolverEnum::External => unreachable!("External is handled above"),
    })
}

#[derive(Debug, Clone)]
pub struct ComputedInvoiceContent {
    pub invoice_lines: Vec<LineItem>,
    pub subtotal: i64, // before discounts, coupons, credits, taxes
    pub applied_coupons: Vec<CouponLineItem>,
    pub discount: i64,
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    pub applied_credits: i64,
    /// Promotional credit taken off the lines before tax, net like `discount`.
    pub promotional_credits: i64,

    pub total: i64,
    pub tax_amount: i64,
    pub amount_due: i64,
    //
    pub subtotal_recurring: i64,
}

/// Assembles priced `lines` in place (`billing_core::assembly::assemble`): what an earlier estimate
/// took off them put back, the discount spread over their taxable amounts.
pub(in crate::services) fn assemble_lines(
    lines: &mut [LineItem],
    inputs: &AssemblyInputs,
) -> billing_core::assembly::Assembled {
    let assembled = billing_core::assembly::assemble(
        lines
            .iter()
            .map(|line| crate::services::credit::document::line_of(line, None))
            .collect(),
        inputs,
    );
    for (line, assembled) in lines.iter_mut().zip(&assembled.lines) {
        line.taxable_amount = assembled.taxable;
        line.promotional_credit = assembled.promotional;
    }
    assembled
}

/// The totals of a document's lines under its `discount` (R-076).
pub(crate) fn totals_of(lines: &[LineItem], discount: i64) -> Totals {
    Totals::of(
        lines.iter().map(|line| LineAmounts {
            subtotal: line.amount_subtotal,
            tax: line.tax_amount,
            promotional: promotional::net_share(line),
            recurring: line.metric_id.is_none(),
        }),
        discount,
    )
}

impl ComputedInvoiceContent {
    /// A document's content: its lines, their totals, and what is already settled against them.
    pub(crate) fn of(
        invoice_lines: Vec<LineItem>,
        totals: Totals,
        applied_coupons: Vec<CouponLineItem>,
        tax_breakdown: Vec<TaxBreakdownItem>,
        already_paid: i64,
        applied_credits: i64,
    ) -> Self {
        Self {
            invoice_lines,
            subtotal: totals.subtotal,
            applied_coupons,
            discount: totals.discount,
            tax_breakdown,
            applied_credits,
            promotional_credits: totals.promotional,
            total: totals.total,
            amount_due: totals.amount_due(already_paid, applied_credits),
            subtotal_recurring: totals.subtotal_recurring,
            tax_amount: totals.tax,
        }
    }

    /// An invoice that bills nothing.
    pub fn empty() -> Self {
        Self {
            invoice_lines: Vec::new(),
            subtotal: 0,
            applied_coupons: Vec::new(),
            discount: 0,
            tax_breakdown: Vec::new(),
            applied_credits: 0,
            promotional_credits: 0,
            total: 0,
            tax_amount: 0,
            amount_due: 0,
            subtotal_recurring: 0,
        }
    }
}

/// A subscription period's invoice once its lines are priced: either its stored content, kept as
/// is, or fresh lines still to be covered, discounted and taxed.
pub(in crate::services) enum PricedInvoice {
    Stored(ComputedInvoiceContent),
    Lines(PricedLines),
}

pub(in crate::services) struct PricedLines {
    pub lines: Vec<LineItem>,
    pub invoice_date: NaiveDate,
    pub currency_precision: u8,
    pub tax_inclusive: bool,
    pub invoice_type: InvoiceType,
    pub prepaid_amount: Option<u64>,
    pub invoice_id: Option<InvoiceId>,
    pub true_up: Option<super::minimum::PeriodTrueUp>,
    /// The true-ups of earlier periods an ending gave spend back in, billed on its document.
    pub reopened: Box<[super::minimum::PeriodTrueUp]>,
    pub span: BilledSpan,
}

impl PricedLines {
    pub(in crate::services) fn cover_target<'a>(
        &'a mut self,
        details: &'a SubscriptionDetails,
    ) -> CoverTarget<'a> {
        CoverTarget {
            details,
            invoice_date: self.invoice_date,
            span: self.span,
            currency_precision: self.currency_precision,
            lines: &mut self.lines,
        }
    }
}

/// Invoice lines once tax has been resolved and applied to them. Always net: lines priced
/// tax-included come back with the tax carved out (see `super::tax_inclusive`).
pub(in crate::services) struct TaxedInvoiceLines {
    pub lines: Vec<LineItem>,
    pub breakdown: Vec<TaxBreakdownItem>,
    /// The part of the lines' discount that turned out to be tax. A discount on a
    /// tax-inclusive price is taken off the quoted figure, so the invoice-level discount
    /// has to shrink by this much to stay the net counterpart of the lines. Zero unless
    /// the lines were priced tax-included, discounted and taxed.
    pub discount_tax: i64,
}

impl TaxedInvoiceLines {
    /// Lines no tax engine reaches still close at what they cost: their discount and promotional
    /// credit are off their taxable amount, and everything downstream reads `amount_total`.
    fn untaxed(mut lines: Vec<LineItem>) -> Self {
        for line in &mut lines {
            line.amount_total = line.taxable_amount + line.tax_amount;
        }
        Self {
            lines,
            breakdown: Vec::new(),
            discount_tax: 0,
        }
    }

    /// The invoice-level discount and its coupon rows, restated to match the net lines
    /// (`billing_core::assembly::net_discount`).
    pub fn net_discount(&self, discount: i64, coupons: &mut [CouponLineItem]) -> StoreResult<i64> {
        let lines = self
            .lines
            .iter()
            .map(|line| crate::services::credit::document::line_of(line, None))
            .collect::<Vec<_>>();
        let quoted = coupons
            .iter()
            .map(|coupon| coupon.value)
            .collect::<Vec<_>>();
        let (net_discount, shared) =
            billing_core::assembly::net_discount(discount, self.discount_tax, &lines, &quoted)
                .map_err(|_| {
                    Report::new(StoreError::InvalidArgument(
                        "promotional credit exceeds the supported range".to_string(),
                    ))
                })?;
        super::tax_inclusive::net_coupons(coupons, shared);
        Ok(net_discount)
    }
}

impl Services {
    /// A stored subscription's period invoice, its lines from its facts (`price_from_facts`),
    /// the credit-priced lines covered as `cover` says, credit estimated on `book` when given.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn compute_invoice_from_facts(
        &self,
        conn: &mut PgConn,
        invoice_date: &NaiveDate,
        subscription_details: &SubscriptionDetails,
        prepaid_amount: Option<u64>,
        invoice: Option<&Invoice>,
        invoice_type: InvoiceType,
        cover: CreditUnitCover,
        book: Option<&CreditBooks>,
    ) -> StoreResult<ComputedInvoiceContent> {
        self.compute_invoice_from_facts_with(
            conn,
            invoice_date,
            subscription_details,
            prepaid_amount,
            invoice,
            invoice_type,
            cover,
            book,
            None,
        )
        .await
    }

    /// [`Self::compute_invoice_from_facts`] with a change not yet written laid over the facts:
    /// `subscription_details` are the subscription as that change leaves it.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn compute_invoice_from_facts_with(
        &self,
        conn: &mut PgConn,
        invoice_date: &NaiveDate,
        subscription_details: &SubscriptionDetails,
        prepaid_amount: Option<u64>,
        invoice: Option<&Invoice>,
        invoice_type: InvoiceType,
        cover: CreditUnitCover,
        book: Option<&CreditBooks>,
        overlay: Option<&crate::services::evaluate::load::Overlay>,
    ) -> StoreResult<ComputedInvoiceContent> {
        let mut priced = match self
            .price_from_facts_with(
                conn,
                invoice_date,
                subscription_details,
                prepaid_amount,
                invoice,
                invoice_type,
                book,
                overlay,
            )
            .await?
        {
            PricedInvoice::Stored(content) => return Ok(content),
            PricedInvoice::Lines(priced) => priced,
        };
        self.cover_credit_unit_lines_on(
            conn,
            cover,
            &mut [priced.cover_target(subscription_details)],
            book,
        )
        .await?;
        let credit_at = match cover {
            CreditUnitCover::Estimate { .. } => common_clock::now(),
            CreditUnitCover::Draw { at, .. } | CreditUnitCover::DrawAsShown { at, .. } => at.at,
        };
        self.finish_invoice_content(conn, subscription_details, priced, credit_at, book)
            .await
    }

    /// The content of an invoice whose taxed lines stand, with the credit it would draw at `at`:
    /// promotional credit re-estimated on its lines (re-taxing the ones it moves), then paid
    /// credit on the total.
    pub(in crate::services) async fn estimate_stored_credit(
        &self,
        conn: &mut PgConn,
        invoice: &Invoice,
        already_paid: i64,
        at: DateTime<Utc>,
        book: Option<&CreditBooks>,
    ) -> StoreResult<ComputedInvoiceContent> {
        let scope = EstimateContext {
            tenant_id: invoice.tenant_id,
            customer_id: invoice.customer_id,
            currency: &invoice.currency,
            invoice_id: Some(invoice.id),
            subscription_id: invoice.subscription_id,
            periods: billed_periods(invoice.subscription_id, &invoice.line_items),
            at,
            book,
        };
        let mut lines = invoice.line_items.clone();
        let mut tax_amount = invoice.tax_amount;
        let mut tax_breakdown = invoice.tax_breakdown.clone();
        let mut promotional_credits = invoice.promotional_credits;
        let mut total = invoice.total;
        let mut subtotal = invoice.subtotal;
        let mut subtotal_recurring = invoice.subtotal_recurring;
        let applied_credits = self
            .settle_estimate(
                conn,
                &scope,
                &mut TaxedDocument {
                    lines: &mut lines,
                    subtotal: &mut subtotal,
                    subtotal_recurring: &mut subtotal_recurring,
                    discount: invoice.discount,
                    tax_inclusive: invoice.tax_inclusive,
                    tax_amount: &mut tax_amount,
                    tax_breakdown: &mut tax_breakdown,
                    promotional_credits: &mut promotional_credits,
                    total: &mut total,
                },
                Cash {
                    already_paid,
                    ..Cash::none(TotalBasis::Folded)
                },
            )
            .await?;
        let totals = totals_of(&lines, invoice.discount);
        Ok(ComputedInvoiceContent::of(
            lines,
            totals,
            invoice.coupons.clone(),
            tax_breakdown,
            already_paid,
            applied_credits,
        ))
    }

    /// Discounts, minimum, tax and totals of priced (and covered) lines.
    pub(in crate::services) async fn finish_invoice_content(
        &self,
        conn: &mut PgConn,
        subscription_details: &SubscriptionDetails,
        priced: PricedLines,
        credit_at: DateTime<Utc>,
        book: Option<&CreditBooks>,
    ) -> StoreResult<ComputedInvoiceContent> {
        let PricedLines {
            lines: invoice_lines,
            invoice_date,
            tax_inclusive,
            invoice_type,
            prepaid_amount,
            invoice_id,
            true_up,
            reopened,
            ..
        } = priced;
        let mut invoice_lines = invoice_lines;

        // The true-up is measured on spend net of coupons and appended after them, so a coupon
        // never discounts it (R-022, R-023); it is taxed like any other line.
        super::minimum::drop_true_up_lines(
            &invoice_type,
            &mut invoice_lines,
            invoice_date,
            true_up.as_ref(),
        );
        let currency = &subscription_details.subscription.currency;
        let queue =
            super::discount::Queue::load(conn, subscription_details, invoice_id, invoice_date)
                .await?;
        let owed = queue.owed(&subscription_details.applied_coupons, currency);
        let applicable = super::discount::applicable_coupons(&owed);
        let assembled = assemble_lines(
            &mut invoice_lines,
            &AssemblyInputs {
                currency: subscription_details.subscription.currency.clone(),
                coupons: super::discount::coupon_inputs(&applicable),
                discount: None,
            },
        );
        let discount_total = assembled.discount;
        let consumed: Vec<_> = assembled
            .coupons
            .iter()
            .map(|applied| applicable[applied.coupon].applied_coupon.id)
            .collect();
        super::minimum::append_minimum_shortfall_if_any(
            &invoice_type,
            subscription_details,
            &mut invoice_lines,
            true_up.as_ref(),
            &consumed,
        )?;
        for earlier in &reopened {
            super::minimum::append_minimum_shortfall_if_any(
                &invoice_type,
                subscription_details,
                &mut invoice_lines,
                Some(earlier),
                &consumed,
            )?;
        }
        super::charges::disambiguate(
            &mut invoice_lines,
            &super::charges::Lineages::of(subscription_details),
        );

        let scope = EstimateContext {
            tenant_id: subscription_details.subscription.tenant_id,
            customer_id: subscription_details.customer.id,
            currency: &subscription_details.subscription.currency,
            invoice_id,
            subscription_id: Some(subscription_details.subscription.id),
            periods: billed_periods(Some(subscription_details.subscription.id), &invoice_lines),
            at: credit_at,
            book,
        };
        // we add taxes
        let taxed = self
            .process_invoice_lines_taxes(
                conn,
                invoice_lines,
                &subscription_details.invoicing_entity,
                &subscription_details.customer,
                subscription_details.subscription.currency.clone(),
                &invoice_date,
                tax_inclusive,
            )
            .await?;
        let mut applied_coupons = super::discount::coupon_rows(&applicable, &assembled.coupons);
        let discount_total = taxed.net_discount(discount_total as i64, &mut applied_coupons)?;
        let (mut invoice_lines, mut breakdown) = (taxed.lines, taxed.breakdown);

        let already_paid = prepaid_amount.unwrap_or(0) as i64;
        let applied_credits = self
            .settle_taxed_lines(
                conn,
                &scope,
                &mut invoice_lines,
                &mut breakdown,
                discount_total,
                tax_inclusive,
                already_paid,
            )
            .await?;
        let totals = totals_of(&invoice_lines, discount_total);

        Ok(ComputedInvoiceContent::of(
            invoice_lines,
            totals,
            applied_coupons,
            breakdown,
            already_paid,
            applied_credits,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn compute_oneoff_invoice(
        &self,
        conn: &mut PgConn,
        invoice_date: &NaiveDate,
        invoice_lines: Vec<LineItem>,
        invoicing_entity: &InvoicingEntity,
        customer: &Customer,
        currency: String,
        discount: Option<u64>,
        prepaid_amount: Option<u64>,
        tax_inclusive: bool,
    ) -> StoreResult<ComputedInvoiceContent> {
        let mut invoice_lines = invoice_lines;
        let discount_total = assemble_lines(
            &mut invoice_lines,
            &AssemblyInputs {
                currency: currency.clone(),
                coupons: Vec::new(),
                discount: Some(discount.unwrap_or(0)),
            },
        )
        .discount;

        let invoice_currency = currency.clone();
        let scope = EstimateContext {
            tenant_id: customer.tenant_id,
            customer_id: customer.id,
            currency: &invoice_currency,
            invoice_id: None,
            subscription_id: None,
            periods: Vec::new(),
            at: common_clock::now(),
            book: None,
        };
        // we add taxes
        let taxed = self
            .process_invoice_lines_taxes(
                conn,
                invoice_lines,
                invoicing_entity,
                customer,
                currency,
                invoice_date,
                tax_inclusive,
            )
            .await?;
        let discount_total = taxed.net_discount(discount_total as i64, &mut [])?;
        let (mut line_items, mut breakdown) = (taxed.lines, taxed.breakdown);
        let already_paid = prepaid_amount.unwrap_or(0) as i64;
        let applied_credits = self
            .settle_taxed_lines(
                conn,
                &scope,
                &mut line_items,
                &mut breakdown,
                discount_total,
                tax_inclusive,
                already_paid,
            )
            .await?;
        let totals = totals_of(&line_items, discount_total);

        Ok(ComputedInvoiceContent::of(
            line_items,
            totals,
            Vec::new(),
            breakdown,
            already_paid,
            applied_credits,
        ))
    }

    pub(in crate::services) async fn process_invoice_lines_taxes(
        &self,
        conn: &mut PgConn,
        invoice_lines: Vec<LineItem>,
        invoicing_entity: &InvoicingEntity,
        customer: &Customer,
        currency: String,
        invoice_date: &NaiveDate,
        tax_inclusive: bool,
    ) -> StoreResult<TaxedInvoiceLines> {
        let Some(inputs) = self
            .load_tax_inputs(conn, &invoice_lines, invoicing_entity, customer)
            .await?
        else {
            return Ok(TaxedInvoiceLines::untaxed(invoice_lines));
        };
        let resolved = inputs
            .resolve(&invoice_lines, currency, invoice_date, tax_inclusive)
            .await?;
        Ok(apply_resolved_taxes(
            invoice_lines,
            resolved,
            tax_inclusive,
            inputs.engine_is_eu_vat,
        ))
    }

    /// What resolving a document's taxes reads; None when no tax applies (no billing address, or
    /// `tax_resolver = None`).
    async fn load_tax_inputs(
        &self,
        conn: &mut PgConn,
        lines: &[LineItem],
        invoicing_entity: &InvoicingEntity,
        customer: &Customer,
    ) -> StoreResult<Option<TaxInputs>> {
        if invoicing_entity.tenant_id != customer.tenant_id {
            return Err(Report::new(StoreError::InvalidArgument(
                "Tenant mismatch: invoicing_entity and customer must belong to the same tenant"
                    .to_string(),
            )));
        }

        let customer_address = match &customer.billing_address {
            Some(address) => address.clone(),
            None => return Ok(None),
        };

        // Select the tax engine for this invoicing entity (built-in or external
        // provider). See `build_tax_engine` — the single place to add a provider.
        let tax_engine = match build_tax_engine(invoicing_entity)? {
            Some(engine) => engine,
            None => return Ok(None),
        };

        let customer = customer_for_tax(customer, invoicing_entity, customer_address);

        // The tax configuration every line resolves against — product-linked taxes and
        // the category each line falls into. Shared with the subscription MRR path so the
        // two classify a product identically.
        let tax_config = self
            .load_product_tax_config_with_conn(
                conn,
                invoicing_entity,
                &lines.iter().map(|line| line.product_id).collect::<Vec<_>>(),
            )
            .await?;

        Ok(Some(TaxInputs {
            engine: tax_engine,
            customer,
            entity_address: invoicing_entity.address().into(),
            tax_config,
            // A merchant override (a tax rate carrying an accounting reference)
            // replaces the built-in EU VAT engine rate only under that engine (C2).
            engine_is_eu_vat: invoicing_entity.tax_resolver == TaxResolverEnum::MeteroidEuVat,
        }))
    }
}

/// What the tax engine resolves a document's lines against, read once.
pub(in crate::services) struct TaxInputs {
    engine: Box<dyn TaxEngine + Send + Sync>,
    customer: meteroid_tax::CustomerForTax,
    entity_address: meteroid_tax::Address,
    tax_config: super::tax_inputs::ProductTaxConfig,
    engine_is_eu_vat: bool,
}

/// The engine's answer for a document: its lines' taxes, and the rates of the charges a coupon or
/// promotional credit took whole.
pub(in crate::services) struct ResolvedTaxes {
    res: meteroid_tax::CalculationResult,
    probe: Option<meteroid_tax::CalculationResult>,
}

impl TaxInputs {
    /// Calls the engine on the lines' taxable amounts (the store's side of the tax boundary).
    async fn resolve(
        &self,
        lines: &[LineItem],
        currency: String,
        invoice_date: &NaiveDate,
        tax_inclusive: bool,
    ) -> StoreResult<ResolvedTaxes> {
        let invoice_lines = lines;
        let invoice_lines_for_tax: Vec<meteroid_tax::LineItemForTax> = invoice_lines
            .iter()
            .filter_map(|line| {
                if crate::services::credit::prepaid_purchase_tax::taxed_at_redemption(line) {
                    return None;
                }
                // Credit/proration lines carry a negative taxable amount and must
                // reduce tax symmetrically (W4); only truly-zero lines are skipped.
                if line.taxable_amount == 0 {
                    // Zero-amount line: no tax to compute.
                    return None;
                }

                let (custom_taxes, tax_category) = self.tax_config.for_product(line.product_id);

                Some(meteroid_tax::LineItemForTax {
                    line_id: line.local_id.to_string(),
                    amount: line.taxable_amount,
                    custom_taxes,
                    tax_category,
                    tax_inclusive,
                })
            })
            .collect();

        // A charge a coupon or promotional credit took whole still needs its rates: a later
        // re-price that leaves part of it to pay is taxed by them. Resolved on the charge; any
        // other zero line (a proration credit) is left as it always was.
        let rate_probes: Vec<meteroid_tax::LineItemForTax> = invoice_lines
            .iter()
            .filter(|line| takes_rate_probe(line))
            .map(|line| {
                let (custom_taxes, tax_category) = self.tax_config.for_product(line.product_id);
                meteroid_tax::LineItemForTax {
                    line_id: line.local_id.to_string(),
                    amount: line.amount_subtotal,
                    custom_taxes,
                    tax_category,
                    tax_inclusive,
                }
            })
            .collect();
        let probe = if rate_probes.is_empty() {
            None
        } else {
            Some(
                self.engine
                    .calculate_line_items_tax(
                        currency.clone(),
                        self.customer.clone(),
                        self.entity_address.clone(),
                        rate_probes,
                        *invoice_date,
                    )
                    .await
                    .change_context(StoreError::TaxError)?,
            )
        };
        let res = self
            .engine
            .calculate_line_items_tax(
                currency,
                self.customer.clone(),
                self.entity_address.clone(),
                invoice_lines_for_tax,
                *invoice_date,
            )
            .await
            .change_context(StoreError::TaxError)?;
        Ok(ResolvedTaxes { res, probe })
    }
}

/// Writes what the engine resolved onto the lines, nets the tax-inclusive ones (R-031) and states
/// the breakdown. Pure.
fn apply_resolved_taxes(
    invoice_lines: Vec<LineItem>,
    resolved: ResolvedTaxes,
    tax_inclusive: bool,
    engine_is_eu_vat: bool,
) -> TaxedInvoiceLines {
    let ResolvedTaxes { res, probe } = resolved;
    let probed = probe
        .as_ref()
        .map_or(&[][..], |probe| probe.line_items.as_slice());
    let mut updated_invoice_lines = invoice_lines.clone();
    let mut discount_tax = 0;

    for line in &mut updated_invoice_lines {
        // we get the matching taxed line
        if let Some(taxed_line) = res.line_items.iter().find(|l| l.line_id == line.local_id) {
            // we update the line with the tax details
            use crate::domain::TaxDetail;

            match &taxed_line.tax_details {
                TaxDetails::Tax {
                    tax_rate,
                    tax_amount,
                    tax_name,
                    ..
                } => {
                    line.tax_amount = *tax_amount;
                    line.tax_rate = *tax_rate;
                    line.tax_details = vec![TaxDetail {
                        tax_rate: *tax_rate,
                        tax_name: tax_name.clone(),
                        tax_amount: *tax_amount,
                    }];
                }
                TaxDetails::MultipleTaxes {
                    taxes,
                    total_tax_amount,
                } => {
                    line.tax_amount = *total_tax_amount;
                    line.tax_rate = taxes.iter().map(|t| t.tax_rate).sum();
                    line.tax_details = taxes
                        .iter()
                        .map(|t| TaxDetail {
                            tax_rate: t.tax_rate,
                            tax_name: t.tax_name.clone(),
                            tax_amount: t.tax_amount,
                        })
                        .collect();
                }
                TaxDetails::Exempt(_) => {
                    line.tax_amount = 0;
                    line.tax_rate = rust_decimal::Decimal::ZERO;
                    line.tax_details = vec![];
                }
            }
            let priced_taxable = line.taxable_amount;
            line.taxable_amount = taxed_line.pre_tax_amount;
            if tax_inclusive {
                discount_tax += super::tax_inclusive::net_line(line, priced_taxable);
            }
        } else if let Some(probe) = probed.iter().find(|l| l.line_id == line.local_id) {
            line.tax_details = rates_of(&probe.tax_details);
            line.tax_rate = line.tax_details.iter().map(|d| d.tax_rate).sum();
            line.tax_amount = 0;
            if tax_inclusive {
                discount_tax += super::tax_inclusive::net_line(line, 0);
            }
        } else {
            // no tax details found
            line.tax_rate = rust_decimal::Decimal::ZERO;
            line.tax_amount = 0;
            line.tax_details = vec![];
        }
        // Recalculate amount_total after discount and tax have been applied
        line.amount_total = line.taxable_amount + line.tax_amount;
    }

    // Convert tax breakdown items, expanding any MultipleTaxes into separate items
    // The engine's finding of *what* the levy is, carried into the persisted row
    // so renderers can word the label in the document language. Structural map
    // only — `None` (merchant custom taxes) stays `None` and keeps the name.
    let map_kind = |kind: Option<meteroid_tax::TaxKind>| {
        use crate::domain::invoices::TaxKind as StoreKind;
        use meteroid_tax::TaxKind as EngineKind;
        kind.map(|k| match k {
            EngineKind::Vat => StoreKind::Vat,
            EngineKind::Gst => StoreKind::Gst,
            EngineKind::Hst => StoreKind::Hst,
            EngineKind::Pst => StoreKind::Pst,
            EngineKind::Qst => StoreKind::Qst,
            EngineKind::SalesTax => StoreKind::SalesTax,
        })
    };
    let to_rows = |items: Vec<meteroid_tax::TaxBreakdownItem>, exemption_reason: Option<String>| {
        items
            .into_iter()
            .flat_map(|item| {
                match item.details {
                    TaxDetails::Tax {
                        tax_rate,
                        tax_name,
                        tax_kind,
                        tax_amount,
                        tax_reference,
                    } => {
                        let tax_reference = non_empty(tax_reference);
                        // Signed per-rate aggregate: a mixed charge+credit group
                        // (or a pure credit note) may net negative and must be
                        // preserved so the breakdown reconciles with tax_amount.
                        vec![TaxBreakdownItem {
                            taxable_amount: item.taxable_amount,
                            tax_amount,
                            tax_rate,
                            name: tax_name,
                            tax_kind: map_kind(tax_kind),
                            prepaid_credit: false,
                            exemption_type: None,
                            exemption_reason: None,
                            overridden: engine_is_eu_vat && tax_reference.is_some(),
                            tax_reference,
                            // Rate rows carry no exemption, so no place-of-supply basis.
                            supply_basis: None,
                        }]
                    }
                    TaxDetails::MultipleTaxes { taxes, .. } => {
                        // Expand multiple taxes into separate breakdown items
                        taxes
                            .into_iter()
                            .map(|tax| {
                                let tax_reference = non_empty(tax.tax_reference);
                                TaxBreakdownItem {
                                    taxable_amount: item.taxable_amount,
                                    tax_amount: tax.tax_amount,
                                    tax_rate: tax.tax_rate,
                                    name: tax.tax_name,
                                    tax_kind: map_kind(tax.tax_kind),
                                    prepaid_credit: false,
                                    exemption_type: None,
                                    exemption_reason: None,
                                    overridden: engine_is_eu_vat && tax_reference.is_some(),
                                    tax_reference,
                                    supply_basis: None,
                                }
                            })
                            .collect()
                    }
                    TaxDetails::Exempt(reason) => {
                        use crate::domain::TaxExemptionType;
                        use meteroid_tax::VatExemptionReason;

                        use crate::domain::SupplyBasis;
                        use meteroid_tax::SupplyBasis as EngineSupplyBasis;

                        // The basis travels with the exemption: only an export needs a
                        // place-of-supply article, and only that article depends on it.
                        let (exemption_type, supply_basis) = match reason {
                            VatExemptionReason::ReverseCharge => {
                                (TaxExemptionType::ReverseCharge, None)
                            }
                            VatExemptionReason::TaxExempt => (TaxExemptionType::TaxExempt, None),
                            VatExemptionReason::NotRegistered => {
                                (TaxExemptionType::NotRegistered, None)
                            }
                            VatExemptionReason::NoVatTerritory => {
                                (TaxExemptionType::NoVatTerritory, None)
                            }
                            VatExemptionReason::Export(basis) => (
                                TaxExemptionType::Export,
                                Some(match basis {
                                    EngineSupplyBasis::Business => SupplyBasis::Business,
                                    EngineSupplyBasis::Consumer => SupplyBasis::Consumer,
                                }),
                            ),
                        };

                        vec![TaxBreakdownItem {
                            taxable_amount: item.taxable_amount,
                            tax_amount: 0,
                            tax_rate: rust_decimal::Decimal::ZERO,
                            name: "Exempt".to_string(),
                            tax_kind: None,
                            prepaid_credit: false,
                            exemption_type: Some(exemption_type),
                            exemption_reason: exemption_reason.clone(),
                            overridden: false,
                            tax_reference: None,
                            supply_basis,
                        }]
                    }
                }
            })
            .collect::<Vec<_>>()
    };
    let mut breakdown = to_rows(res.breakdown, res.exemption_reason.clone());
    // Taxes on charges taken whole are stated at nothing, so the document still names the
    // rate or exemption its lines carry (BR-S-01, BR-AE-01).
    if let Some(probe) = probe {
        for mut row in to_rows(probe.breakdown, probe.exemption_reason) {
            if breakdown.iter().any(|kept| {
                kept.name == row.name
                    && kept.tax_rate == row.tax_rate
                    && kept.exemption_type == row.exemption_type
            }) {
                continue;
            }
            row.taxable_amount = 0;
            row.tax_amount = 0;
            breakdown.push(row);
        }
    }
    state_untaxed_purchases(&updated_invoice_lines, &mut breakdown);

    TaxedInvoiceLines {
        lines: updated_invoice_lines,
        breakdown,
        discount_tax,
    }
}

/// Where the subscription's fees start billing (R-084): its billing start, or the end of its free
/// trial, which bills nothing.
pub(crate) fn billing_from(details: &SubscriptionDetails, billing_start: NaiveDate) -> NaiveDate {
    details
        .trial_config
        .as_ref()
        .filter(|trial| trial.is_free)
        .and_then(|trial| trial.ends_on)
        .map_or(billing_start, |end| end.max(billing_start))
}

/// What an invoice bills of its subscription's period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BilledSpan {
    /// The period's own invoice: the arrears up to its date, the next period in advance.
    Period,
    /// A final invoice: the usage from its date, which the period's own invoice billed up to, until
    /// the termination instant. Nothing in advance.
    Final,
}

impl BilledSpan {
    pub(crate) fn of(invoice_type: &InvoiceType) -> Self {
        match invoice_type {
            InvoiceType::Final => Self::Final,
            _ => Self::Period,
        }
    }
}

/// The day and span the windows of a subscription's invoice of `span` keyed `on` are read at. A
/// final invoice issued after the day its cancellation ended reads that day as its recurring
/// invoice would, unless that invoice is live and billed up to it (R-016).
pub(crate) async fn billed_at(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    span: BilledSpan,
    on: NaiveDate,
) -> StoreResult<(NaiveDate, BilledSpan)> {
    if span != BilledSpan::Final {
        return Ok((on, span));
    }
    let ended =
        diesel_models::subscription_facts::SubscriptionEndingRow::list_of(conn, subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .find(|row| row.kind == diesel_models::subscription_facts::ENDING_CANCEL)
            .map(|row| row.at.date())
            .filter(|ended| *ended < on);
    let Some(ended) = ended else {
        return Ok((on, span));
    };
    let live = Box::pin(InvoiceRow::find_existing_recurring_invoice(
        conn,
        tenant_id,
        subscription_id,
        ended,
    ))
    .await
    .map_err(Into::<Report<StoreError>>::into)?
    .is_some_and(|invoice| {
        !matches!(
            invoice.status,
            diesel_models::enums::InvoiceStatusEnum::Void
                | diesel_models::enums::InvoiceStatusEnum::Closed
        )
    });
    Ok((
        ended,
        if live {
            BilledSpan::Final
        } else {
            BilledSpan::Period
        },
    ))
}

/// What a metered fee bills a window of usage.
#[derive(Debug, Clone)]
pub enum MeteredFee {
    Usage(UsagePricingModel),
    CapacityOverage {
        /// The units included for the window: the fee's, prorated with it (R-002).
        included: Decimal,
        overage_rate: Decimal,
    },
}

impl MeteredFee {
    /// Whether no usage is ever charged by it.
    pub fn is_free(&self) -> bool {
        let zero = |rate: &Decimal| rate.is_zero();
        let tiers_free = |tiers: &[crate::domain::price_components::TierRow]| {
            tiers.iter().all(|tier| {
                tier.rate.is_zero() && tier.flat_fee.as_ref().is_none_or(Decimal::is_zero)
            })
        };
        match self {
            Self::Usage(UsagePricingModel::PerUnit { rate })
            | Self::Usage(UsagePricingModel::Package { rate, .. }) => zero(rate),
            Self::Usage(UsagePricingModel::Tiered { tiers, .. })
            | Self::Usage(UsagePricingModel::Volume { tiers, .. }) => tiers_free(tiers),
            Self::Usage(UsagePricingModel::Matrix { rates }) => {
                rates.iter().all(|row| row.per_unit_price.is_zero())
            }
            Self::CapacityOverage { overage_rate, .. } => overage_rate.is_zero(),
        }
    }
}

/// Detect temporal splits and append date range to disambiguate line item names.
/// A temporal split occurs when multiple sub_component_ids exist for the same price_component_id,
/// indicating a mid-period plan change with old and new component versions.
pub(crate) fn apply_temporal_date_range_to_names(lines: &mut [LineItem]) {
    let mut price_component_sub_ids: HashMap<
        PriceComponentId,
        HashSet<SubscriptionPriceComponentId>,
    > = HashMap::new();

    for line in lines.iter() {
        if let (Some(pc_id), Some(sc_id)) = (line.price_component_id, line.sub_component_id) {
            price_component_sub_ids
                .entry(pc_id)
                .or_default()
                .insert(sc_id);
        }
    }

    let split_price_components: HashSet<PriceComponentId> = price_component_sub_ids
        .into_iter()
        .filter(|(_, sub_ids)| sub_ids.len() > 1)
        .map(|(pc_id, _)| pc_id)
        .collect();

    if split_price_components.is_empty() {
        return;
    }

    for line in lines.iter_mut() {
        if let Some(pc_id) = line.price_component_id
            && split_price_components.contains(&pc_id)
        {
            line.name = format!("{} ({} - {})", line.name, line.start_date, line.end_date);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn priced(amount_subtotal: i64, taxable_amount: i64) -> LineItem {
        LineItem {
            local_id: "line".to_owned(),
            name: "line".to_owned(),
            charge_type: None,
            amount_subtotal,
            tax_rate: Decimal::ZERO,
            taxable_amount,
            tax_amount: 0,
            amount_total: taxable_amount,
            tax_details: vec![],
            quantity: None,
            unit_price: None,
            start_date: date(2026, 1, 1),
            end_date: date(2026, 2, 1),
            sub_lines: vec![],
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
            usage_breakdown: None,
        }
    }

    /// Only a charge a coupon or promotional credit took whole is probed for its rates; a
    /// proration credit, untaxed at nothing, is left exactly as it always was.
    #[test]
    fn only_charges_taken_whole_are_probed_for_their_rates() {
        assert!(!takes_rate_probe(&priced(-500, 0)), "a proration credit");
        assert!(
            !takes_rate_probe(&priced(1_000, 1_000)),
            "an ordinary charge"
        );
        assert!(!takes_rate_probe(&priced(0, 0)), "nothing priced");
        assert!(takes_rate_probe(&priced(1_000, 0)), "a 100% coupon");
        let mut promoted = priced(1_000, 0);
        promoted.promotional_credit = Some(1_000);
        assert!(takes_rate_probe(&promoted));
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn a_fee_is_free_only_when_no_usage_is_ever_charged() {
        use crate::domain::price_components::{MatrixDimension, MatrixRow, TierRow};
        use rust_decimal_macros::dec;

        let tier = |first_unit, rate, flat_fee| TierRow {
            first_unit,
            rate,
            flat_fee,
            flat_cap: None,
        };
        let usage = MeteredFee::Usage;
        assert!(usage(UsagePricingModel::PerUnit { rate: dec!(0) }).is_free());
        assert!(!usage(UsagePricingModel::PerUnit { rate: dec!(0.0001) }).is_free());
        assert!(
            usage(UsagePricingModel::Package {
                block_size: 10,
                rate: dec!(0)
            })
            .is_free()
        );
        assert!(
            usage(UsagePricingModel::Tiered {
                tiers: vec![tier(0, dec!(0), None), tier(100, dec!(0), Some(dec!(0)))],
                block_size: None,
            })
            .is_free()
        );
        assert!(
            !usage(UsagePricingModel::Volume {
                tiers: vec![tier(0, dec!(0), None), tier(100, dec!(0), Some(dec!(5)))],
                block_size: None,
            })
            .is_free(),
            "a flat fee charges"
        );
        let cell = |price| MatrixRow {
            dimension1: MatrixDimension {
                key: "region".to_owned(),
                value: "eu".to_owned(),
            },
            dimension2: None,
            per_unit_price: price,
        };
        assert!(
            usage(UsagePricingModel::Matrix {
                rates: vec![cell(dec!(0))]
            })
            .is_free()
        );
        assert!(
            !usage(UsagePricingModel::Matrix {
                rates: vec![cell(dec!(0)), cell(dec!(1))]
            })
            .is_free()
        );
        assert!(
            !MeteredFee::CapacityOverage {
                included: dec!(10),
                overage_rate: dec!(2)
            }
            .is_free()
        );
    }
}
