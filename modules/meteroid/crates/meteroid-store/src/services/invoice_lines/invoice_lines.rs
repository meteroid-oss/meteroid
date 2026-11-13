use crate::StoreResult;
use crate::constants::{Currencies, Currency};
use crate::domain::enums::{InvoiceType, SubscriptionStatusEnum};
use crate::domain::{
    ComponentPeriods, CouponLineItem, Customer, Invoice, InvoicingEntity, LineItem, Period,
    SubscriptionAddOn, SubscriptionComponent, SubscriptionDetails, SubscriptionFee,
    SubscriptionFeeInterface, TaxBreakdownItem, TaxResolverEnum, UsagePricingModel,
};
use crate::services::subscriptions::proration::DayFraction;
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{
    BillableMetricId, InvoiceId, PriceComponentId, SubscriptionAddOnId,
    SubscriptionPriceComponentId,
};
use diesel_models::invoices::InvoiceRow;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use itertools::Itertools;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};

use crate::domain::BillableMetric;
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::credit::forecast::CreditBooks;
use crate::services::credit::promotional::{self, TaxedDocument};
use crate::services::credit::value_application::{EstimateContext, billed_periods};
use crate::services::invoice_lines::component::{ExistingLineKey, is_usage_based_line};
use crate::services::invoice_lines::credit_units::{CoverTarget, CreditUnitCover};
use crate::services::invoice_lines::discount::calculate_coupons_discount;
use crate::services::invoice_lines::tax_inputs::customer_for_tax;
use crate::store::PgConn;
use crate::utils::periods::calculate_component_period_for_invoice_date;
use billing_core::{LineAmounts, Totals};
use common_utils::integers::ToNonNegativeU64;
use diesel_models::billable_metrics::BillableMetricRow;
use error_stack::{Report, ResultExt};
use meteroid_tax::{ManualTaxEngine, MeteroidTaxEngine, TaxDetails, TaxEngine};

impl Services {}

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
    pub cycle_index: u32,
    pub currency_precision: u8,
    pub tax_inclusive: bool,
    pub invoice_type: InvoiceType,
    pub prepaid_amount: Option<u64>,
    pub invoice_id: Option<InvoiceId>,
    pub true_up: Option<super::minimum::PeriodTrueUp>,
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
            cycle_index: self.cycle_index,
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

    /// The invoice-level discount and its coupon rows, restated to match the net lines. The
    /// lines' reduction also holds their promotional credit, whose own tax stays with it.
    pub fn net_discount(&self, discount: i64, coupons: &mut [CouponLineItem]) -> StoreResult<i64> {
        let promotional_tax =
            promotional::quoted_total(&self.lines)? - promotional::net_total(&self.lines)?;
        let net_discount = discount - (self.discount_tax - promotional_tax);
        super::tax_inclusive::net_coupons(coupons, net_discount);
        Ok(net_discount)
    }
}

impl Services {
    /// Whether settling this invoice's period could produce arrears lines — so an empty draft must
    /// be recomputed (or anchored) rather than treated as nothing to bill. True when the
    /// subscription accrues arrears from its currently-active components, or when a *closed*
    /// pure-arrears component/add-on still bills a segment on this invoice (the mid-period plan
    /// change away from usage).
    ///
    /// `cycle_index` must be the cycle the invoice's lines are computed for — the invoice's pinned
    /// cycle on a refresh, the subscription's current one at draft creation — so the probe gates
    /// segments exactly like the biller will.
    pub(in crate::services) async fn can_accrue_arrears_in_period(
        &self,
        conn: &mut PgConn,
        subscription_details: &SubscriptionDetails,
        invoice_date: NaiveDate,
        cycle_index: u32,
    ) -> StoreResult<bool> {
        if subscription_details.can_accrue_arrears() {
            return Ok(true);
        }
        let Some(billing_start_date) = subscription_details.subscription.billing_start_date else {
            return Ok(false);
        };

        let subscription_id = &subscription_details.subscription.id;

        let active_component_ids: HashSet<SubscriptionPriceComponentId> = subscription_details
            .price_components
            .iter()
            .map(|c| c.id)
            .collect();
        let closed_components: Vec<SubscriptionComponent> =
            SubscriptionComponentRow::list_component_history_for_period(
                conn,
                subscription_id,
                billing_start_date,
                invoice_date,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .filter(|row| row.effective_to.is_some() && !active_component_ids.contains(&row.id))
            // Propagate rather than skip: this probe is the late-usage safety net, so a row that
            // fails to convert must not silently answer "no arrears" and drop the period's usage.
            .map(TryInto::<SubscriptionComponent>::try_into)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|comp| bills_closed_arrears(&comp.fee))
            .collect();
        let runs =
            super::cadence::load_cadence_runs(conn, subscription_details, invoice_date, true)
                .await?;
        if bills_arrears_segment(
            subscription_details,
            &closed_components,
            invoice_date,
            cycle_index,
            &runs,
        ) {
            return Ok(true);
        }

        let active_addon_ids: HashSet<SubscriptionAddOnId> =
            subscription_details.add_ons.iter().map(|a| a.id).collect();
        let closed_add_ons: Vec<SubscriptionAddOn> =
            SubscriptionAddOnRow::list_add_on_history_for_period(
                conn,
                subscription_id,
                billing_start_date,
                invoice_date,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .filter(|row| row.effective_to.is_some() && !active_addon_ids.contains(&row.id))
            .map(TryInto::<SubscriptionAddOn>::try_into)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|ao| bills_closed_arrears(&ao.fee))
            .collect();

        Ok(bills_arrears_segment(
            subscription_details,
            &closed_add_ons,
            invoice_date,
            cycle_index,
            &runs,
        ))
    }

    pub async fn compute_invoice(
        &self,
        conn: &mut PgConn,
        invoice_date: &NaiveDate,
        subscription_details: &SubscriptionDetails,
        prepaid_amount: Option<u64>,
        invoice: Option<&Invoice>, // for refresh purposes
        invoice_type: InvoiceType,
    ) -> StoreResult<ComputedInvoiceContent> {
        self.compute_invoice_covering(
            conn,
            invoice_date,
            subscription_details,
            prepaid_amount,
            invoice,
            invoice_type,
            CreditUnitCover::Estimate {
                invoice_id: invoice.map(|invoice| invoice.id),
            },
        )
        .await
    }

    /// `compute_invoice`, with the credit-priced lines covered as `cover` says.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn compute_invoice_covering(
        &self,
        conn: &mut PgConn,
        invoice_date: &NaiveDate,
        subscription_details: &SubscriptionDetails,
        prepaid_amount: Option<u64>,
        invoice: Option<&Invoice>,
        invoice_type: InvoiceType,
        cover: CreditUnitCover,
    ) -> StoreResult<ComputedInvoiceContent> {
        self.compute_invoice_on(
            conn,
            invoice_date,
            subscription_details,
            prepaid_amount,
            invoice,
            invoice_type,
            cover,
            None,
        )
        .await
    }

    /// `compute_invoice_covering`, estimating credit on `book` when given (a forecast's).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn compute_invoice_on(
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
        let mut priced = match self
            .price_invoice_lines(
                conn,
                invoice_date,
                subscription_details,
                prepaid_amount,
                invoice,
                invoice_type,
                book,
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
            CreditUnitCover::Estimate { .. } => common_domain::clock::now(),
            CreditUnitCover::Draw { at, .. } | CreditUnitCover::DrawAsShown { at, .. } => at.at,
        };
        self.finish_invoice_content(conn, subscription_details, priced, credit_at, book)
            .await
    }

    /// The lines of a subscription period's invoice, priced but neither covered by credit units,
    /// discounted nor taxed; or the stored content when the invoice keeps its lines.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn price_invoice_lines(
        &self,
        conn: &mut PgConn,
        invoice_date: &NaiveDate,
        subscription_details: &SubscriptionDetails,
        prepaid_amount: Option<u64>,
        invoice: Option<&Invoice>, // for refresh purposes
        invoice_type: InvoiceType,
        book: Option<&CreditBooks>,
    ) -> StoreResult<PricedInvoice> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(self.price_invoice_lines_unboxed(
            conn,
            invoice_date,
            subscription_details,
            prepaid_amount,
            invoice,
            invoice_type,
            book,
        ))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn price_invoice_lines_unboxed(
        &self,
        conn: &mut PgConn,
        invoice_date: &NaiveDate,
        subscription_details: &SubscriptionDetails,
        prepaid_amount: Option<u64>,
        invoice: Option<&Invoice>,
        invoice_type: InvoiceType,
        book: Option<&CreditBooks>,
    ) -> StoreResult<PricedInvoice> {
        let span = BilledSpan::of(&invoice_type);
        let reading = details_for_span(conn, subscription_details, span).await?;
        let subscription_details: &SubscriptionDetails =
            reading.as_deref().unwrap_or(subscription_details);

        // Gate on the cycle the invoice belongs to, not the one the subscription has since reached:
        // a draft outliving a renewal would otherwise bill another period's components (or lose its
        // own). Falls back to the current cycle for invoices predating the column.
        let cycle_index = invoice
            .and_then(|i| i.cycle_index)
            .or(subscription_details.subscription.cycle_index)
            .unwrap_or(0);

        // Reuse the stored lines instead of recomputing when either holds:
        //
        // - The invoice is neither `Recurring` nor `Final`. Only those bill a subscription's usage;
        //   a OneOff, Adjustment or corrected draft carries bespoke lines (and a subscription_id),
        //   so re-deriving the period over them would replace them with a period-to-date recompute
        //   the next renewal bills again — a double charge.
        // - It is recurring but static: no usage-based lines, and no arrears can accrue for this
        //   period. Otherwise the amount still depends on usage that may have landed since the
        //   draft was created, and freezing it would drop that usage — including the
        //   empty/shortfall-only case, where there is no usage line to detect staleness from.
        if let Some(invoice) = invoice
            && (!invoice_type.bills_subscription_usage()
                || (!invoice.line_items.iter().any(is_usage_based_line)
                    && !self
                        .can_accrue_arrears_in_period(
                            conn,
                            subscription_details,
                            *invoice_date,
                            cycle_index,
                        )
                        .await?))
        {
            // Reused lines still estimate the customer credit they would draw.
            let already_paid = prepaid_amount.unwrap_or(0) as i64;
            let content = self
                .estimate_stored_credit(
                    conn,
                    invoice,
                    already_paid,
                    common_domain::clock::now(),
                    book,
                )
                .await?;
            return Ok(PricedInvoice::Stored(content));
        }

        let billing_start_date = subscription_details
            .subscription
            .billing_start_date
            // TODO should we return empty ?
            .ok_or(Report::new(StoreError::BillingError))
            .attach("No billing_start_date is present")?;

        let currency = Currencies::resolve_currency(&subscription_details.subscription.currency)
            .ok_or(Report::new(StoreError::ValueNotFound(format!(
                "Currency {} not found",
                subscription_details.subscription.currency
            ))))?;

        let invoice_date = *invoice_date;

        // An invoice keeps the basis it was first computed on; a new one takes the
        // subscription's.
        let tax_inclusive = invoice
            .map_or(subscription_details.subscription.tax_inclusive, |invoice| {
                invoice.tax_inclusive
            });

        // Stored lines are net, but everything below prices, discounts and taxes lines in
        // the figures they were quoted in. Put the stored ones back into those figures so
        // the ones carried over (and any unit price an operator pinned) re-enter the
        // pipeline on the same footing as the ones computed fresh.
        let repriced_invoice = invoice.filter(|_| tax_inclusive).map(|invoice| Invoice {
            line_items: invoice
                .line_items
                .iter()
                .cloned()
                .map(LineItem::into_tax_inclusive_quote)
                .collect(),
            ..invoice.clone()
        });
        let invoice = repriced_invoice.as_ref().or(invoice);

        // Build map of existing line items for refresh (only usage-based lines)
        let existing_lines: HashMap<ExistingLineKey, &LineItem> = if let Some(invoice) = invoice {
            invoice
                .line_items
                .iter()
                .filter(|line| is_usage_based_line(line))
                .filter_map(|line| ExistingLineKey::from_line_item(line).map(|key| (key, line)))
                .collect()
        } else {
            HashMap::new()
        };

        let runs =
            super::cadence::load_cadence_runs(conn, subscription_details, invoice_date, true)
                .await?;
        let price_components_lines = self
            .process_fee_records(
                conn,
                subscription_details,
                &subscription_details.price_components,
                invoice_date,
                billing_start_date,
                cycle_index,
                &currency,
                &existing_lines,
                invoice.map(|invoice| invoice.id),
                &runs,
                span,
            )
            .await?;

        // Add-ons store their fee per-unit with an instance count in `quantity`.
        // compute_component reads instance_count() from the trait and multiplies
        // the fee's own quantity, so no pre-scaling is needed here.
        let add_ons_lines = self
            .process_fee_records(
                conn,
                subscription_details,
                &subscription_details.add_ons,
                invoice_date,
                billing_start_date,
                cycle_index,
                &currency,
                &existing_lines,
                invoice.map(|invoice| invoice.id),
                &runs,
                span,
            )
            .await?;

        // Closed pure-arrears components/add-ons still bill their temporal segment
        // [effective_from, effective_to] after a mid-period plan change or amendment.
        let closed = load_closed_arrears(conn, subscription_details, invoice_date).await?;
        let closed_details = closed.details_for(subscription_details);

        let historical_lines = self
            .process_fee_records(
                conn,
                &closed_details,
                &closed.components,
                invoice_date,
                billing_start_date,
                cycle_index,
                &currency,
                &existing_lines,
                invoice.map(|invoice| invoice.id),
                &runs,
                span,
            )
            .await?;

        let historical_addon_lines = self
            .process_fee_records(
                conn,
                &closed_details,
                &closed.add_ons,
                invoice_date,
                billing_start_date,
                cycle_index,
                &currency,
                &existing_lines,
                invoice.map(|invoice| invoice.id),
                &runs,
                span,
            )
            .await?;

        // Merge non-usage-based lines from existing invoice (if refreshing)
        let mut invoice_lines = if let Some(invoice) = invoice {
            // TODO quick fix, do that part in process_component instead
            let computed_lines = price_components_lines
                .into_iter()
                .chain(add_ons_lines)
                .chain(historical_lines)
                .chain(historical_addon_lines)
                .filter(&is_usage_based_line)
                .collect_vec();

            // A true-up line is recomputed by `append_minimum_shortfall_if_any`, which also decides
            // on a shortfall charged up front before the true-up.
            let non_usage_lines = invoice
                .line_items
                .iter()
                .filter(|line| !is_usage_based_line(line))
                .filter(|line| !super::minimum::is_true_up(line, invoice_date, None))
                .cloned();

            computed_lines
                .into_iter()
                .chain(non_usage_lines)
                .collect_vec()
        } else {
            price_components_lines
                .into_iter()
                .chain(add_ons_lines)
                .chain(historical_lines)
                .chain(historical_addon_lines)
                .collect_vec()
        };

        // Append date range to line item names when a temporal split occurred
        // (multiple sub_component_ids for the same price_component_id)
        apply_temporal_date_range_to_names(&mut invoice_lines);
        super::charges::disambiguate(
            &mut invoice_lines,
            &super::charges::Lineages::of(subscription_details),
        );

        let true_up = if invoice_type == InvoiceType::Recurring {
            super::minimum::load_period_true_up(
                conn,
                subscription_details,
                invoice_date,
                cycle_index,
                invoice,
            )
            .await?
        } else {
            None
        };

        Ok(PricedInvoice::Lines(PricedLines {
            lines: invoice_lines,
            invoice_date,
            cycle_index,
            currency_precision: currency.precision,
            tax_inclusive,
            invoice_type,
            prepaid_amount,
            invoice_id: invoice.map(|invoice| invoice.id),
            true_up,
            span,
        }))
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
        self.reprice_promotional_estimate(
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
        )
        .await?;
        let totals = totals_of(&lines, invoice.discount);
        let applied_credits = self
            .preview_credit_for_lines(conn, &scope, &lines, totals.total, already_paid)
            .await?;
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
            ..
        } = priced;
        // Kept lines come back with what an earlier estimate took off them.
        let mut invoice_lines = invoice_lines;
        promotional::restore_before_tax(&mut invoice_lines);

        // The true-up is appended before coupons and tax: measured on GROSS spend, it is then
        // discounted and taxed like any other line (a 5.00 coupon on a 9.00 shortfall leaves 4.00).
        super::minimum::append_minimum_shortfall_if_any(
            &invoice_type,
            subscription_details,
            &mut invoice_lines,
            invoice_date,
            true_up.as_ref(),
        )?;
        super::charges::disambiguate(
            &mut invoice_lines,
            &super::charges::Lineages::of(subscription_details),
        );

        let subtotal = invoice_lines
            .iter()
            .fold(0, |acc, x| acc + x.amount_subtotal);

        let coupons_discount = calculate_coupons_discount(
            subtotal,
            &subscription_details.subscription.currency,
            &subscription_details.applied_coupons,
        );

        let discount_total = coupons_discount.discount_subunit.to_non_negative_u64(); // TODO we need to define the rules for negatives, same below with taxes & subtotal
        let mut invoice_lines = super::discount::distribute_discount(invoice_lines, discount_total);

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
        self.take_promotional_estimate(conn, &scope, &mut invoice_lines)
            .await?;

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
        let mut applied_coupons = coupons_discount.applied_coupons;
        let discount_total = taxed.net_discount(discount_total as i64, &mut applied_coupons)?;
        let (mut invoice_lines, mut breakdown) = (taxed.lines, taxed.breakdown);

        let Totals {
            mut subtotal,
            mut subtotal_recurring,
            promotional: mut promotional_credits,
            tax: mut tax_amount,
            mut total,
            ..
        } = totals_of(&invoice_lines, discount_total);
        // Paid settlements of the periods were placed at face value before tax; placed again by
        // the usage lines' rates, as finalizing places them.
        let taxed_usage = invoice_lines
            .iter()
            .any(|line| line.metric_id.is_some() && !line.tax_details.is_empty());
        if !scope.periods.is_empty() && !tax_inclusive && taxed_usage {
            self.reprice_promotional_estimate(
                conn,
                &scope,
                &mut TaxedDocument {
                    lines: &mut invoice_lines,
                    subtotal: &mut subtotal,
                    subtotal_recurring: &mut subtotal_recurring,
                    discount: discount_total,
                    tax_inclusive,
                    tax_amount: &mut tax_amount,
                    tax_breakdown: &mut breakdown,
                    promotional_credits: &mut promotional_credits,
                    total: &mut total,
                },
            )
            .await?;
        }
        let totals = totals_of(&invoice_lines, discount_total);
        let already_paid = prepaid_amount.unwrap_or(0) as i64;
        let applied_credits = self
            .preview_credit_for_lines(conn, &scope, &invoice_lines, totals.total, already_paid)
            .await?;

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
        let discount_total = discount.unwrap_or(0);
        let mut invoice_lines = invoice_lines;
        promotional::restore_before_tax(&mut invoice_lines);
        let mut invoice_lines = super::discount::distribute_discount(invoice_lines, discount_total);

        let invoice_currency = currency.clone();
        let scope = EstimateContext {
            tenant_id: customer.tenant_id,
            customer_id: customer.id,
            currency: &invoice_currency,
            invoice_id: None,
            subscription_id: None,
            periods: Vec::new(),
            at: common_domain::clock::now(),
            book: None,
        };
        self.take_promotional_estimate(conn, &scope, &mut invoice_lines)
            .await?;

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
        let (line_items, breakdown) = (taxed.lines, taxed.breakdown);

        let totals = totals_of(&line_items, discount_total);
        let already_paid = prepaid_amount.unwrap_or(0) as i64;
        let applied_credits = self
            .preview_credit_for_lines(conn, &scope, &line_items, totals.total, already_paid)
            .await?;

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
        if invoicing_entity.tenant_id != customer.tenant_id {
            return Err(Report::new(StoreError::InvalidArgument(
                "Tenant mismatch: invoicing_entity and customer must belong to the same tenant"
                    .to_string(),
            )));
        }

        let customer_address = match &customer.billing_address {
            Some(address) => address.clone(),
            None => return Ok(TaxedInvoiceLines::untaxed(invoice_lines)),
        };

        // Select the tax engine for this invoicing entity (built-in or external
        // provider). See `build_tax_engine` — the single place to add a provider.
        let tax_engine = match build_tax_engine(invoicing_entity)? {
            Some(engine) => engine,
            None => return Ok(TaxedInvoiceLines::untaxed(invoice_lines)),
        };

        let customer = customer_for_tax(customer, invoicing_entity, customer_address);

        // The tax configuration every line resolves against — product-linked taxes and
        // the category each line falls into. Shared with the subscription MRR path so the
        // two classify a product identically.
        let tax_config = self
            .load_product_tax_config_with_conn(
                conn,
                invoicing_entity,
                &invoice_lines
                    .iter()
                    .map(|line| line.product_id)
                    .collect::<Vec<_>>(),
            )
            .await?;

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

                let (custom_taxes, tax_category) = tax_config.for_product(line.product_id);

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
                let (custom_taxes, tax_category) = tax_config.for_product(line.product_id);
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
                tax_engine
                    .calculate_line_items_tax(
                        currency.clone(),
                        customer.clone(),
                        invoicing_entity.address().into(),
                        rate_probes,
                        *invoice_date,
                    )
                    .await
                    .change_context(StoreError::TaxError)?,
            )
        };
        let probed = probe
            .as_ref()
            .map_or(&[][..], |probe| probe.line_items.as_slice());

        let res = tax_engine
            .calculate_line_items_tax(
                currency,
                customer,
                invoicing_entity.address().into(),
                invoice_lines_for_tax,
                *invoice_date,
            )
            .await
            .change_context(StoreError::TaxError)?;

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

        // A merchant override (a tax rate carrying an accounting reference)
        // replaces the built-in EU VAT engine rate only under that engine (C2).
        let engine_is_eu_vat = invoicing_entity.tax_resolver == TaxResolverEnum::MeteroidEuVat;

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
        let to_rows = |items: Vec<meteroid_tax::TaxBreakdownItem>,
                       exemption_reason: Option<String>| {
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
                                VatExemptionReason::TaxExempt => {
                                    (TaxExemptionType::TaxExempt, None)
                                }
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

        Ok(TaxedInvoiceLines {
            lines: updated_invoice_lines,
            breakdown,
            discount_tax,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_fee_records<T: SubscriptionFeeInterface>(
        &self,
        conn: &mut PgConn,
        subscription_details: &SubscriptionDetails,
        fee_records: &[T],
        invoice_date: NaiveDate,
        billing_start_or_resume_date: NaiveDate,
        cycle_index: u32,
        currency: &Currency,
        existing_lines: &HashMap<ExistingLineKey, &LineItem>,
        invoice_id: Option<InvoiceId>,
        runs: &[crate::utils::periods::CadenceRun],
        span: BilledSpan,
    ) -> StoreResult<Vec<LineItem>> {
        // One-time fees are billed exactly once and follow a different rule than the
        // cadence-based components below: `applies_this_period` only admits one-time
        // fees on cycle 0, which would silently drop a one-time fee added by a later
        // amendment. Handle them separately so they bill on the invoice generated on
        // the date they become effective.
        let (one_time_records, recurring_records): (Vec<&T>, Vec<&T>) = fee_records
            .iter()
            .partition(|c| matches!(c.fee_ref(), SubscriptionFee::OneTime { .. }));

        // TODO case when invoiced early via threshold (that's for usage-based only)
        // can be quite easy => we need some last_invoice_threshold date in the subscription, to reduce the usage periods if that date is within the period

        let component_period_components = fee_periods(
            subscription_details,
            recurring_records,
            invoice_date,
            billing_start_or_resume_date,
            cycle_index,
            runs,
            span,
        );

        // we can now compute all the components for each period
        let mut invoice_lines = Vec::new();
        for (period, components) in component_period_components {
            for component in components {
                if is_once_credit(component.fee_ref())
                    && !self
                        .once_credit_due(
                            conn,
                            subscription_details,
                            component,
                            period.arrear.as_ref().map(|arrear| arrear.start),
                            cycle_index,
                            invoice_id,
                        )
                        .await?
                {
                    continue;
                }
                // Apply temporal bounds to arrear period for usage-based billing split
                let mut adjusted_period = restrict_arrear_period_by_temporal_bounds(
                    period.clone(),
                    component.effective_from(),
                    component.effective_to(),
                );
                // Ended by then, it bills nothing in advance (a capacity's overage still trails).
                if let Some(to) = component.effective_to()
                    && adjusted_period
                        .advance
                        .as_ref()
                        .is_some_and(|advance| advance.start >= to)
                {
                    adjusted_period.advance = None;
                }

                let lines = self
                    .compute_component(
                        conn,
                        subscription_details,
                        component,
                        adjusted_period,
                        currency.precision,
                        existing_lines,
                    )
                    .await?;

                invoice_lines.extend(lines);
            }
        }

        // The period's own invoice billed them.
        if span == BilledSpan::Final {
            return Ok(invoice_lines);
        }

        // One-time fees: billed in full, never prorated, exactly once. Billing point:
        //  - plan-level one-time fees bill on the subscription's first invoice
        //    (`cycle_index == 0`, not the closing invoice of a subscription ending in
        //    cycle 0); they are not re-billed on renewals or carried over
        //    by a plan change (which is why `applies_this_period` excludes them);
        //  - a one-time fee added by a manual amendment bills on the invoice for the
        //    period it becomes effective (`effective_from == invoice_date`), which for
        //    an immediate amendment is its adjustment invoice (handled elsewhere) and
        //    for an end-of-period amendment is the upcoming renewal.
        let sub = &subscription_details.subscription;
        let closes_first_period = sub.current_period_end.is_none()
            && !sub.pending_checkout
            && invoice_date > billing_start_or_resume_date;
        for component in one_time_records {
            let bill_now = (cycle_index == 0 && !closes_first_period)
                || (component.added_by_amendment()
                    && component.effective_from() == Some(invoice_date));
            if !bill_now {
                continue;
            }

            let periods = ComponentPeriods {
                proration_factor: None,
                proration_days: None,
                arrear_proration_factor: None,
                advance: Some(Period {
                    start: invoice_date,
                    end: invoice_date,
                }),
                arrear: None,
            };

            let lines = self
                .compute_component(
                    conn,
                    subscription_details,
                    component,
                    periods,
                    currency.precision,
                    existing_lines,
                )
                .await?;

            invoice_lines.extend(lines);
        }

        Ok(invoice_lines)
    }
}

impl Services {
    /// A `once` credit bills on the first invoice that carries its component, unless another
    /// invoice of the subscription already billed a `once` credit of the same product, so a plan
    /// change or amendment never grants a product's one-time credit twice.
    async fn once_credit_due<T: SubscriptionFeeInterface>(
        &self,
        conn: &mut PgConn,
        subscription_details: &SubscriptionDetails,
        component: &T,
        previous_billing_date: Option<NaiveDate>,
        cycle_index: u32,
        invoice_id: Option<InvoiceId>,
    ) -> StoreResult<bool> {
        // Refused on creation; a legacy product-less one keeps the first-invoice-only rule.
        let Some(product_id) = component.product_id() else {
            return Ok(cycle_index == 0);
        };
        let tenant_id = subscription_details.subscription.tenant_id;
        let subscription_id = subscription_details.subscription.id;
        // From cycle 2 on, a component the previous renewal already carried was settled there,
        // unless an invoice that carried it was voided: the void may have taken it back whole.
        if cycle_index >= 2
            && let (Some(from), Some(previous)) =
                (component.effective_from(), previous_billing_date)
            && from <= previous
            && !InvoiceRow::subscription_voided_once_credit(
                conn,
                tenant_id,
                subscription_id,
                product_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        {
            return Ok(false);
        }
        let billed = InvoiceRow::subscription_billed_once_credit(
            conn,
            tenant_id,
            subscription_id,
            product_id,
            invoice_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        Ok(!billed)
    }
}

fn is_once_credit(fee: &SubscriptionFee) -> bool {
    matches!(fee, SubscriptionFee::Credit { once: true, .. })
}

/// Closed pure-arrears components and add-ons of a subscription that can still bill a usage
/// segment on an invoice, with the metrics they reference that the details do not carry.
pub(crate) struct ClosedArrears {
    pub components: Vec<SubscriptionComponent>,
    pub add_ons: Vec<SubscriptionAddOn>,
    extra_metrics: Vec<BillableMetric>,
}

impl ClosedArrears {
    pub(crate) fn details_for<'a>(
        &self,
        details: &'a SubscriptionDetails,
    ) -> std::borrow::Cow<'a, SubscriptionDetails> {
        if self.extra_metrics.is_empty() {
            return std::borrow::Cow::Borrowed(details);
        }
        let mut extended = details.clone();
        extended.metrics.extend(self.extra_metrics.iter().cloned());
        std::borrow::Cow::Owned(extended)
    }
}

pub(crate) async fn load_closed_arrears(
    conn: &mut PgConn,
    subscription_details: &SubscriptionDetails,
    invoice_date: NaiveDate,
) -> StoreResult<ClosedArrears> {
    let Some(billing_start_date) = subscription_details.subscription.billing_start_date else {
        return Ok(ClosedArrears {
            components: Vec::new(),
            add_ons: Vec::new(),
            extra_metrics: Vec::new(),
        });
    };
    let subscription_id = &subscription_details.subscription.id;

    let active_component_ids: HashSet<SubscriptionPriceComponentId> = subscription_details
        .price_components
        .iter()
        .map(|c| c.id)
        .collect();
    let mut components: Vec<SubscriptionComponent> =
        SubscriptionComponentRow::list_component_history_for_period(
            conn,
            subscription_id,
            billing_start_date,
            invoice_date,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .filter(|row| row.effective_to.is_some() && !active_component_ids.contains(&row.id))
        .filter_map(|row| TryInto::<SubscriptionComponent>::try_into(row).ok())
        .filter(|comp| bills_closed_arrears(&comp.fee))
        .collect();

    let active_addon_ids: HashSet<SubscriptionAddOnId> =
        subscription_details.add_ons.iter().map(|a| a.id).collect();
    let mut add_ons: Vec<SubscriptionAddOn> = SubscriptionAddOnRow::list_add_on_history_for_period(
        conn,
        subscription_id,
        billing_start_date,
        invoice_date,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?
    .into_iter()
    .filter(|row| row.effective_to.is_some() && !active_addon_ids.contains(&row.id))
    .filter_map(|row| TryInto::<SubscriptionAddOn>::try_into(row).ok())
    .filter(|ao| bills_closed_arrears(&ao.fee))
    .collect();
    crate::repositories::subscriptions::restore_stored_fee_credit_units(
        conn,
        subscription_details.subscription.tenant_id,
        components
            .iter_mut()
            .map(|c| (c.price_id, &mut c.fee))
            .chain(add_ons.iter_mut().map(|a| (a.price_id, &mut a.fee)))
            .collect(),
    )
    .await?;

    // A closed fee may reference a metric no active fee carries (plan change away from usage).
    let known_metric_ids: HashSet<_> = subscription_details.metrics.iter().map(|m| m.id).collect();
    let missing_metric_ids: Vec<_> = components
        .iter()
        .filter_map(|c| c.metric_id())
        .chain(add_ons.iter().filter_map(|a| a.fee.metric_id()))
        .filter(|id| !known_metric_ids.contains(id))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let extra_metrics = if missing_metric_ids.is_empty() {
        Vec::new()
    } else {
        BillableMetricRow::get_by_ids(
            conn,
            &missing_metric_ids,
            &subscription_details.subscription.tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(|row| {
            TryInto::<BillableMetric>::try_into(row)
                .map(|metric| subscription_details.usage_scopes.apply(metric))
        })
        .collect::<Result<Vec<_>, Report<_>>>()?
    };

    Ok(ClosedArrears {
        components,
        add_ons,
        extra_metrics,
    })
}

/// The periods each group of same-cadence recurring fees bills on `invoice_date`; groups the
/// invoice has nothing to bill for are dropped. Shared by the invoice and the live usage windows.
/// The periods each record bills on `invoice_date`'s invoice. A record runs on the grid of the
/// latest change of cadence before it ended (`runs`, latest first), so what a change closed settles
/// on the grid it ran on; before any change, on the billing start's grid.
pub(crate) fn fee_periods<'a, T: SubscriptionFeeInterface>(
    subscription_details: &SubscriptionDetails,
    recurring_records: Vec<&'a T>,
    invoice_date: NaiveDate,
    billing_start_or_resume_date: NaiveDate,
    cycle_index: u32,
    runs: &[crate::utils::periods::CadenceRun],
    span: BilledSpan,
) -> Vec<(ComponentPeriods, Vec<&'a T>)> {
    let governing = |record: &T| {
        runs.iter()
            .find(|run| record.effective_to().is_none_or(|to| to > run.switched_on))
    };
    let (in_run, before_run): (Vec<&T>, Vec<&T>) = recurring_records
        .into_iter()
        .partition(|record| governing(record).is_some());
    let mut groups = fee_period_groups(
        subscription_details,
        before_run,
        invoice_date,
        billing_start_or_resume_date,
        cycle_index,
        !runs.is_empty(),
    );
    let sub = &subscription_details.subscription;
    let is_completed = sub.current_period_end.is_none() && !sub.pending_checkout;
    for record in in_run {
        let Some(run) = governing(record) else {
            continue;
        };
        for periods in crate::utils::periods::component_periods_in_run(
            run,
            invoice_date,
            record.period_ref(),
            u32::from(sub.billing_day_anchor),
            record.effective_from(),
            record.effective_to(),
            is_completed,
        ) {
            groups.push((periods, vec![record]));
        }
    }
    match span {
        BilledSpan::Period => groups,
        BilledSpan::Final => final_window(groups, invoice_date),
    }
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

/// The records whose arrears the period's own invoice settled up to `on`, on the final invoice's
/// window opening that day: `usage_window` stretches it to the termination instant. A record closed
/// by `on` consumed nothing after it; the in-run grid lists a record once per window.
fn final_window<'a, T: SubscriptionFeeInterface>(
    groups: Vec<(ComponentPeriods, Vec<&'a T>)>,
    on: NaiveDate,
) -> Vec<(ComponentPeriods, Vec<&'a T>)> {
    let mut records: Vec<&'a T> = Vec::new();
    for record in groups
        .into_iter()
        .filter(|(periods, _)| periods.arrear.is_some())
        .flat_map(|(_, group)| group)
    {
        if record.effective_to().is_none_or(|to| to > on)
            && !records.iter().any(|known| std::ptr::eq(*known, record))
        {
            records.push(record);
        }
    }
    if records.is_empty() {
        return Vec::new();
    }
    vec![(
        ComponentPeriods {
            proration_factor: None,
            proration_days: None,
            arrear_proration_factor: None,
            advance: None,
            arrear: Some(Period { start: on, end: on }),
        },
        records,
    )]
}

/// The subscription as an invoice of `span` reads its usage. Once a final invoice bills what it
/// consumed after its last period began, the period's own invoices read up to that period's start
/// only, not to the termination instant (`usage_window`). A voided final invoice keeps that window:
/// only its regeneration bills it again, never the period's invoice issued meanwhile.
/// None when the subscription reads as it stands; boxed, so callers' state machines stay small.
pub(crate) async fn details_for_span(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    span: BilledSpan,
) -> StoreResult<Option<Box<SubscriptionDetails>>> {
    let sub = &details.subscription;
    if span == BilledSpan::Final
        || sub.terminated_at.is_none()
        || !Box::pin(InvoiceRow::final_invoice_owns_last_window(
            conn,
            sub.tenant_id,
            sub.id,
        ))
        .await
        .map_err(Into::<Report<StoreError>>::into)?
    {
        return Ok(None);
    }
    let mut period_only = Box::new(details.clone());
    period_only.subscription.terminated_at = None;
    Ok(Some(period_only))
}

/// With `own_cadence`, a component is gated on its own cadence, not the subscription's: what a
/// change of cadence closed settles on the grid it ran on.
fn fee_period_groups<'a, T: SubscriptionFeeInterface>(
    subscription_details: &SubscriptionDetails,
    recurring_records: Vec<&'a T>,
    invoice_date: NaiveDate,
    billing_start_or_resume_date: NaiveDate,
    cycle_index: u32,
    own_cadence: bool,
) -> Vec<(ComponentPeriods, Vec<&'a T>)> {
    recurring_records
        .into_iter()
        .into_group_map_by(|c| c.period_ref())
        .into_iter()
        .filter_map(|(billing_period, components)| {
            let is_completed = subscription_details
                .subscription
                .current_period_end
                .is_none()
                && !subscription_details.subscription.pending_checkout;

            let is_cancellation =
                subscription_details.subscription.status == SubscriptionStatusEnum::Cancelled;

            // we calculate the periods range, for each billing_period. There can be advance, arrears, or both
            let subscription_period = billing_period
                .as_billing_period_opt()
                .filter(|_| own_cadence)
                .unwrap_or(subscription_details.subscription.period);
            let period = calculate_component_period_for_invoice_date(
                invoice_date,
                &subscription_period,
                billing_period,
                billing_start_or_resume_date,
                cycle_index,
                u32::from(subscription_details.subscription.billing_day_anchor),
                is_completed,
                is_cancellation,
            );

            // if period is None - the components are not relevant for this invoice
            period.map(|period| (period, components))
        })
        .collect()
}

/// What a metered fee bills a window of usage.
#[derive(Debug, Clone)]
pub enum MeteredFee {
    Usage(UsagePricingModel),
    CapacityOverage {
        included: u64,
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

/// One metric's arrears window on an invoice: which fee prices it, and over which dates.
#[derive(Debug, Clone)]
pub(crate) struct MeteredWindow {
    pub metric_id: BillableMetricId,
    pub sub_component_id: Option<SubscriptionPriceComponentId>,
    pub sub_add_on_id: Option<SubscriptionAddOnId>,
    pub fee: MeteredFee,
    pub period: Period,
    /// The unit the fee prices usage in; absent prices it in the subscription currency.
    pub credit_unit: Option<crate::domain::credit_units::FeeCreditUnit>,
}

/// The usage windows `invoice_date`'s invoice would bill for these records, computed with the
/// same period math and temporal restriction as the invoice lines.
pub(crate) fn metered_windows<T: SubscriptionFeeInterface>(
    subscription_details: &SubscriptionDetails,
    records: &[T],
    invoice_date: NaiveDate,
    billing_start_or_resume_date: NaiveDate,
    cycle_index: u32,
    runs: &[crate::utils::periods::CadenceRun],
    span: BilledSpan,
) -> Vec<MeteredWindow> {
    let recurring = records
        .iter()
        .filter(|record| !matches!(record.fee_ref(), SubscriptionFee::OneTime { .. }))
        .collect();
    fee_periods(
        subscription_details,
        recurring,
        invoice_date,
        billing_start_or_resume_date,
        cycle_index,
        runs,
        span,
    )
    .into_iter()
    .flat_map(|(periods, components)| {
        components.into_iter().filter_map(move |component| {
            let period = restrict_arrear_period_by_temporal_bounds(
                periods.clone(),
                component.effective_from(),
                component.effective_to(),
            )
            .arrear?;
            match component.fee_ref() {
                SubscriptionFee::Usage {
                    metric_id,
                    model,
                    credit_unit,
                } => Some(MeteredWindow {
                    metric_id: *metric_id,
                    sub_component_id: component.sub_component_id(),
                    sub_add_on_id: component.sub_add_on_id(),
                    fee: MeteredFee::Usage(model.clone()),
                    period,
                    credit_unit: credit_unit.clone(),
                }),
                SubscriptionFee::Capacity {
                    included,
                    overage_rate,
                    metric_id,
                    credit_unit,
                    ..
                } if overage_rate > &Decimal::ZERO => Some(MeteredWindow {
                    metric_id: *metric_id,
                    sub_component_id: component.sub_component_id(),
                    sub_add_on_id: component.sub_add_on_id(),
                    fee: MeteredFee::CapacityOverage {
                        included: *included,
                        overage_rate: *overage_rate,
                    },
                    period,
                    credit_unit: credit_unit.clone(),
                }),
                _ => None,
            }
        })
    })
    .collect()
}

/// Restrict the arrear period of a ComponentPeriods based on a component's temporal bounds.
/// - If effective_from > arrear.start: restrict arrear start to effective_from
/// - If effective_to < arrear.end: restrict arrear end to effective_to
/// - Returns the period unchanged if there are no temporal bounds or no arrear period.
///
/// When the arrear window is shrunk by a temporal bound (e.g. an arrears component
/// added or removed mid-period via an amendment), a fixed-rate arrears fee must be
/// prorated to the fraction of the period it was actually active — otherwise it
/// would bill a full period's rate for a partial window. A proration factor is set
/// for that case. Usage-based arrears ignore the factor (their amount derives from
/// usage queried over the restricted window), so this is a no-op for them.
/// Whether any of these closed pure-arrears records still bills an arrear segment on
/// `invoice_date`. Existing in the subscription's history is not enough: `process_fee_records`
/// clips every segment to this invoice's arrear window and drops it once elapsed, so asking
/// "does a closed segment exist" rather than "would one bill" answers true for every period
/// after the change — anchoring an empty draft forever. Runs the same period math the biller
/// does, so the two agree by construction.
fn bills_arrears_segment<T: SubscriptionFeeInterface>(
    subscription_details: &SubscriptionDetails,
    records: &[T],
    invoice_date: NaiveDate,
    cycle_index: u32,
    runs: &[crate::utils::periods::CadenceRun],
) -> bool {
    let Some(billing_start_date) = subscription_details.subscription.billing_start_date else {
        return false;
    };
    fee_periods(
        subscription_details,
        records.iter().collect(),
        invoice_date,
        billing_start_date,
        cycle_index,
        runs,
        BilledSpan::Period,
    )
    .into_iter()
    .any(|(periods, records)| {
        records.into_iter().any(|record| {
            restrict_arrear_period_by_temporal_bounds(
                periods.clone(),
                record.effective_from(),
                record.effective_to(),
            )
            .arrear
            .is_some()
        })
    })
}

/// A closed fee still owes what trails it: its arrears, or a capacity's overage.
fn bills_closed_arrears(fee: &SubscriptionFee) -> bool {
    fee.is_pure_arrears()
        || matches!(fee, SubscriptionFee::Capacity { overage_rate, .. } if *overage_rate > Decimal::ZERO)
}

fn restrict_arrear_period_by_temporal_bounds(
    mut periods: ComponentPeriods,
    effective_from: Option<NaiveDate>,
    effective_to: Option<NaiveDate>,
) -> ComponentPeriods {
    if let Some(ref mut arrear) = periods.arrear {
        let full_days = (arrear.end - arrear.start).num_days();

        if let Some(from) = effective_from
            && from > arrear.start
        {
            arrear.start = from;
        }
        if let Some(to) = effective_to
            && to < arrear.end
        {
            arrear.end = to;
        }
        // Keep only an exactly-zero-width window that was zero-width on arrival — the terminal
        // same-day settlement window [billing_start, termination_date] on the cancellation day,
        // whose usage is billed against the exact termination instant downstream (see
        // `arrear_usage_period`); a fixed-rate arrears fee on it bills nothing (compute_component
        // skips a zero-width arrears line). Drop everything else invalid: a period restriction
        // collapsed (a component closed at/before the period start), a negative-width arrival (an
        // immediate cancel before a future-dated billing start), and a zero-width arrival that a
        // temporal bound then clamped into an inverted window — so `Some(arrear)` is always a valid
        // window for any consumer.
        if (full_days != 0 && arrear.start >= arrear.end) || arrear.end < arrear.start {
            periods.arrear = None;
            periods.arrear_proration_factor = None;
            return periods;
        }

        // Prorate fixed-rate arrears down to the active fraction of the period. This
        // is set on the arrear-only factor so it never affects the advance line of a
        // component billed on the same period (e.g. an advance Rate add-on added
        // mid-cycle must still bill its full next period at renewal). Multiply with any existing
        // factor so a cancellation's proration composes rather than clobbers.
        let restricted_days = (arrear.end - arrear.start).num_days();
        if full_days > 0 && restricted_days < full_days {
            let restriction = DayFraction::new(restricted_days, full_days);
            periods.arrear_proration_factor = Some(
                periods
                    .arrear_proration_factor
                    .map_or(restriction, |factor| factor.of(restriction)),
            );
        }
    }
    periods
}

/// Detect temporal splits and append date range to disambiguate line item names.
/// A temporal split occurs when multiple sub_component_ids exist for the same price_component_id,
/// indicating a mid-period plan change with old and new component versions.
fn apply_temporal_date_range_to_names(lines: &mut [LineItem]) {
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

    fn arrear_periods(start: NaiveDate, end: NaiveDate) -> ComponentPeriods {
        ComponentPeriods {
            arrear: Some(Period { start, end }),
            advance: None,
            proration_factor: None,
            proration_days: None,
            arrear_proration_factor: None,
        }
    }

    #[test]
    fn arrear_added_mid_period_is_prorated() {
        // An arrears component effective from Jan 16 in a [Jan 1, Feb 1] period
        // should be prorated to the remaining 16 of 31 days.
        let periods = restrict_arrear_period_by_temporal_bounds(
            arrear_periods(date(2025, 1, 1), date(2025, 2, 1)),
            Some(date(2025, 1, 16)),
            None,
        );

        let arrear = periods.arrear.expect("arrear retained");
        assert_eq!(arrear.start, date(2025, 1, 16));
        assert_eq!(arrear.end, date(2025, 2, 1));
        let factor = periods.arrear_proration_factor.expect("arrear prorated");
        assert_eq!(factor, DayFraction::new(16, 31));
        // The advance factor must stay untouched (advance lines bill in full).
        assert_eq!(periods.proration_factor, None);
    }

    #[test]
    fn arrear_removed_mid_period_is_prorated() {
        // Closed at Jan 16: only [Jan 1, Jan 16] (15/31) should be billed.
        let periods = restrict_arrear_period_by_temporal_bounds(
            arrear_periods(date(2025, 1, 1), date(2025, 2, 1)),
            None,
            Some(date(2025, 1, 16)),
        );

        let arrear = periods.arrear.expect("arrear retained");
        assert_eq!(arrear.start, date(2025, 1, 1));
        assert_eq!(arrear.end, date(2025, 1, 16));
        let factor = periods.arrear_proration_factor.expect("arrear prorated");
        assert_eq!(factor, DayFraction::new(15, 31));
        assert_eq!(periods.proration_factor, None);
    }

    #[test]
    fn arrear_full_period_is_not_prorated() {
        // A component active for the whole period (effective_from == arrear.start)
        // keeps the full rate — no proration factor.
        let periods = restrict_arrear_period_by_temporal_bounds(
            arrear_periods(date(2025, 1, 1), date(2025, 2, 1)),
            Some(date(2025, 1, 1)),
            None,
        );

        assert_eq!(periods.arrear_proration_factor, None);
        assert_eq!(periods.proration_factor, None);
        let arrear = periods.arrear.expect("arrear retained");
        assert_eq!(arrear.start, date(2025, 1, 1));
        assert_eq!(arrear.end, date(2025, 2, 1));
    }

    #[test]
    fn arrear_restricted_to_empty_is_dropped() {
        // effective_from at/after the period end removes the arrear entirely.
        let periods = restrict_arrear_period_by_temporal_bounds(
            arrear_periods(date(2025, 1, 1), date(2025, 2, 1)),
            Some(date(2025, 2, 1)),
            None,
        );

        assert!(periods.arrear.is_none());
    }

    #[test]
    fn zero_width_arrear_on_arrival_is_kept() {
        // The terminal same-day settlement window [today, today] arrives zero-width and must be
        // kept — its usage is billed against the exact termination instant downstream. It carries
        // no proration factor (a fixed-rate arrears line on it is skipped by compute_component).
        let periods = restrict_arrear_period_by_temporal_bounds(
            arrear_periods(date(2025, 3, 10), date(2025, 3, 10)),
            None,
            None,
        );
        let arrear = periods.arrear.expect("zero-width terminal window retained");
        assert_eq!(arrear.start, date(2025, 3, 10));
        assert_eq!(arrear.end, date(2025, 3, 10));
        assert_eq!(periods.arrear_proration_factor, None);
    }

    #[test]
    fn negative_width_arrear_on_arrival_is_dropped() {
        // An immediate cancel before a future-dated billing start yields a negative-width arrear
        // ([future_start, today]); it must be dropped so a fixed-rate arrears fee is not billed a
        // full period and the usage query is not sent an inverted window.
        let periods = restrict_arrear_period_by_temporal_bounds(
            arrear_periods(date(2025, 3, 20), date(2025, 3, 10)),
            None,
            None,
        );
        assert!(periods.arrear.is_none());
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
                included: 10,
                overage_rate: dec!(2)
            }
            .is_free()
        );
    }

    struct Record {
        name: String,
        period: crate::domain::enums::SubscriptionFeeBillingPeriod,
        fee: SubscriptionFee,
        effective_to: Option<NaiveDate>,
    }

    impl SubscriptionFeeInterface for Record {
        fn price_component_id(&self) -> Option<PriceComponentId> {
            None
        }
        fn product_id(&self) -> Option<common_domain::ids::ProductId> {
            None
        }
        fn subscription_id(&self) -> common_domain::ids::SubscriptionId {
            common_domain::ids::SubscriptionId::default()
        }
        fn name_ref(&self) -> &String {
            &self.name
        }
        fn period_ref(&self) -> &crate::domain::enums::SubscriptionFeeBillingPeriod {
            &self.period
        }
        fn fee_ref(&self) -> &SubscriptionFee {
            &self.fee
        }
        fn sub_component_id(&self) -> Option<SubscriptionPriceComponentId> {
            None
        }
        fn sub_add_on_id(&self) -> Option<SubscriptionAddOnId> {
            None
        }
        fn effective_to(&self) -> Option<NaiveDate> {
            self.effective_to
        }
    }

    fn record(effective_to: Option<NaiveDate>) -> Record {
        Record {
            name: "Usage".to_owned(),
            period: crate::domain::enums::SubscriptionFeeBillingPeriod::Monthly,
            fee: SubscriptionFee::Rate { rate: Decimal::ONE },
            effective_to,
        }
    }

    /// A final invoice opens on the day the period's own invoice billed up to, for each record that
    /// invoice settled the arrears of, once: none closed by then, none billed in advance only.
    #[test]
    fn a_final_window_opens_on_its_day_for_the_records_settled_up_to_it() {
        let on = date(2026, 3, 10);
        let (open, in_run, closed, advance_only) =
            (record(None), record(None), record(Some(on)), record(None));
        let advance = ComponentPeriods {
            arrear: None,
            advance: Some(Period {
                start: on,
                end: date(2026, 4, 10),
            }),
            proration_factor: None,
            proration_days: None,
            arrear_proration_factor: None,
        };
        let groups = vec![
            (arrear_periods(date(2026, 2, 10), on), vec![&open, &closed]),
            (
                arrear_periods(date(2026, 2, 10), date(2026, 3, 1)),
                vec![&in_run],
            ),
            (arrear_periods(date(2026, 3, 1), on), vec![&in_run]),
            (advance, vec![&advance_only]),
        ];
        let window = final_window(groups, on);
        let [(periods, records)] = &window[..] else {
            panic!("one window, not {}", window.len());
        };
        assert_eq!(periods.arrear, Some(Period { start: on, end: on }));
        assert!(periods.advance.is_none());
        assert!(periods.arrear_proration_factor.is_none());
        assert_eq!(records.len(), 2);
        assert!(std::ptr::eq(records[0], &open));
        assert!(std::ptr::eq(records[1], &in_run));
    }

    #[test]
    fn a_final_window_is_empty_without_a_settled_record() {
        let on = date(2026, 3, 10);
        let closed = record(Some(on));
        let groups = vec![(arrear_periods(date(2026, 2, 10), on), vec![&closed])];
        assert!(final_window(groups, on).is_empty());
    }
}
