use crate::StoreResult;
use crate::domain::entity_activity::Actor;
use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
use crate::domain::price_components::{PriceEntry, ProductRef};
use crate::domain::subscriptions::PaymentMethodsConfig;
use crate::domain::{
    CreateSubscriptionAddOns, CreateSubscriptionComponents, CreatedSubscription, PriceComponent,
    Subscription, SubscriptionAddOnNewInternal, SubscriptionComponentNewInternal,
    SubscriptionDetails, SubscriptionFee, SubscriptionNew,
};
use crate::errors::StoreError;
use crate::hooks::CreditControlHooks;
use crate::services::subscriptions::insert::context::ResolvedCustomComponents;
use crate::store::{PgConn, Store};
use chrono::NaiveDate;
use common_domain::ids::{ConnectorId, ProductFamilyId};
use common_utils::decimals::ToSubunit;
use diesel_models::errors::DatabaseError;
use error_stack::Report;
use itertools::Itertools;
use std::collections::HashMap;

use crate::domain::add_ons::AddOn;
use crate::domain::coupons::{Coupon, CouponCreditRedemption, CouponDiscount};
use crate::domain::outbox_event::{OutboxEvent, SubscriptionEvent, SubscriptionUpdateType};
use crate::repositories::historical_rates::HistoricalRatesInterface;
use diesel_models::DbResult;
use diesel_models::applied_coupons::{AppliedCouponRow, AppliedCouponRowNew};
use diesel_models::coupon_plans::CouponPlanRow;
use diesel_models::coupons::CouponRow;
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::subscriptions::{SubscriptionRow, SubscriptionRowNew};
use rust_decimal::prelude::*;

use crate::services::Services;
use crate::utils::periods::calculate_advance_period_range;
use common_domain::ids::{
    AppliedCouponId, BaseId, CouponId, PlanVersionId, SubscriptionId, TenantId,
};
use diesel_models::plans::PlanRow;

/// A product/price that needs to be created inside the persist transaction.
#[derive(Debug, Clone)]
pub struct PendingMaterialization {
    /// Index into the components vector this materialization belongs to.
    pub component_index: usize,
    pub name: String,
    pub product_ref: ProductRef,
    pub price_entry: PriceEntry,
    pub product_family_id: ProductFamilyId,
    pub currency: String,
}

/// Validates charge_automatically: requires Online config + at least one online provider.
pub fn validate_charge_automatically_with_provider_ids(
    charge_automatically: bool,
    payment_methods_config: Option<&PaymentMethodsConfig>,
    card_provider_id: Option<ConnectorId>,
    direct_debit_provider_id: Option<ConnectorId>,
) -> StoreResult<()> {
    if !charge_automatically {
        return Ok(());
    }

    let is_online = match payment_methods_config {
        None | Some(PaymentMethodsConfig::Online { .. }) => true,
        Some(PaymentMethodsConfig::BankTransfer { .. } | PaymentMethodsConfig::External) => false,
    };

    if !is_online {
        return Err(Report::new(StoreError::InvalidArgument(
            "charge_automatically requires payment_methods_config to be Online".to_string(),
        )));
    }

    if card_provider_id.is_none() && direct_debit_provider_id.is_none() {
        return Err(Report::new(StoreError::InvalidArgument(
            "charge_automatically requires the invoicing entity to have a payment provider configured (card or direct debit)".to_string(),
        )));
    }

    Ok(())
}

/// The fee's quoted total for one of its own billing periods, in subunits.
///
/// This is the MRR base before it is spread over the period's months, and it is also the
/// amount the component's invoice line carries — so a caller that needs to ask the tax
/// engine what a component is taxed at hands it this, not the monthly share. A credit pack
/// counts at its net price like any recurring fee; one-time and usage fees, and a pack granted
/// once at start, contribute nothing.
pub(crate) fn fee_period_total_cents(fee: &SubscriptionFee, precision: u8) -> i64 {
    let mut total_cents = 0;

    match fee {
        SubscriptionFee::Rate { rate } => {
            total_cents = rate.to_subunit_opt(precision).unwrap_or(0);
        }
        SubscriptionFee::Recurring { quantity, rate, .. }
        | SubscriptionFee::Credit {
            quantity,
            rate,
            once: false,
            ..
        } => {
            let total = rate * Decimal::from(*quantity);
            total_cents = total.to_subunit_opt(precision).unwrap_or(0);
        }
        SubscriptionFee::Capacity { rate, .. } => {
            total_cents = rate.to_subunit_opt(precision).unwrap_or(0);
        }
        SubscriptionFee::Slot {
            initial_slots,
            unit_rate,
            ..
        } => {
            total_cents =
                i64::from(*initial_slots) * unit_rate.to_subunit_opt(precision).unwrap_or(0);
        }
        SubscriptionFee::OneTime { .. }
        | SubscriptionFee::Usage { .. }
        | SubscriptionFee::Credit { once: true, .. } => {}
    }

    total_cents
}

/// A currency credit pack is a prepayment taxed when the credit is used, so its price carries no
/// tax to take out of MRR; a pack of credit units is sold, and taxed, like any other fee.
pub(crate) fn fee_taxed_at_redemption(fee: &SubscriptionFee) -> bool {
    matches!(
        fee,
        SubscriptionFee::Credit {
            credit_unit: None,
            ..
        }
    )
}

/// The product a fee's charge is taxed and booked under: a credit unit pack's is the unit's tax
/// product when the unit has one, as for the unit's direct sales.
pub(crate) fn fee_tax_product(
    fee: &SubscriptionFee,
    component_product: Option<common_domain::ids::ProductId>,
) -> Option<common_domain::ids::ProductId> {
    match fee {
        SubscriptionFee::Credit {
            credit_unit: Some(unit),
            ..
        } => unit.product_id.or(component_product),
        _ => component_product,
    }
}

pub fn calculate_mrr(
    fee: &SubscriptionFee,
    period: &SubscriptionFeeBillingPeriod,
    precision: u8,
) -> i64 {
    monthly_share(
        fee_period_total_cents(fee, precision),
        i64::from(period.as_months()),
    )
}

/// A per-period subunit amount as its monthly share.
///
/// Half-up at the subunit, through the one rounding the tax side already shares, so a
/// monthly MRR is not biased a cent low every time a period does not divide evenly. Every
/// MRR figure — creation, recompute, the slot-aware recompute in `plan_change` — goes
/// through here: a figure rounded one way at creation and the other on recompute would
/// book a phantom one-cent movement on the first amendment.
///
/// Deploy note: this replaced truncation, and subscriptions created before it keep the
/// truncated figure until something recomputes them — there is no SQL backfill, because the
/// figure cannot be re-derived from `mrr_cents` alone. A subscription on a period that does
/// not divide evenly will therefore book a one-time ±1 cent Expansion/Contraction into
/// `bi_mrr_movement_log` on its first amendment or plan change after the deploy. Expect a
/// burst of 1-cent movements, and tell whoever owns the analytics plane before it lands.
pub fn monthly_share(period_total_cents: i64, period_months: i64) -> i64 {
    let months = period_months.max(1);
    meteroid_tax::round_to_minor(Decimal::from(period_total_cents) / Decimal::from(months))
}

/// Takes the tax back out of an MRR figure read off tax-inclusive prices.
///
/// A tax-inclusive subscription quotes prices that already contain the tax, and every MRR
/// figure derived from its fees therefore contains it too. MRR is revenue, so it is stated
/// net: `gross / (1 + rate)`, rounded the same way as every other subunit here. `None` — a
/// tax-exclusive subscription, or one whose customer bears no tax — is the identity.
pub fn net_of_inclusive_tax(cents: i64, rate: Option<Decimal>) -> i64 {
    let Some(rate) = rate else {
        return cents;
    };

    let gross_per_unit = Decimal::ONE + rate;
    if gross_per_unit <= Decimal::ZERO {
        // A rate at or below -100% leaves no net to recover; keep the figure as priced.
        return cents;
    }

    meteroid_tax::round_to_minor(Decimal::from(cents) / gross_per_unit)
}

/// The single rate a subscription's quoted prices carry, from the rate each of its lines
/// resolved to. `None` when nothing is baked in — every line exempt, reverse-charged,
/// exported or genuinely zero-rated: the customer pays what they were quoted, so there is
/// nothing to take out of the MRR either.
///
/// Each entry is `(the line's monthly share, that line's effective rate)`, the rate as the
/// tax engine resolved it — compounding already folded in, and before any rounding.
///
/// A subscription stores one rate, so lines that resolved differently (a reduced-rate
/// category beside a standard one, a product override beside the statutory rate) are
/// blended: what the subscription as a whole carries per unit of net. The weights are
/// monthly shares, not billing-period totals, because that is what the rate is applied to —
/// MRR is the sum of the lines' monthly shares, so weighting by anything else would leave
/// `net_of_inclusive_tax(Σ monthly share)` disagreeing with `Σ` of the lines' net as soon as
/// a subscription mixes billing periods (an annual component at 20% beside a monthly one at
/// 5.5% would net by the annual line's rate almost entirely). When every line landed on the
/// same rate — the ordinary case — that rate is taken as it stands rather than re-derived
/// through two divisions, so the stored figure reads as the rate it is (`0.2`, not
/// `0.1999…`), and the weights then do not matter at all.
pub(crate) fn blended_inclusive_rate(lines: &[(Decimal, Decimal)]) -> Option<Decimal> {
    let taxed: Vec<&(Decimal, Decimal)> = lines
        .iter()
        .filter(|(amount, _)| !amount.is_zero())
        .collect();

    let (_, first_rate) = taxed.first()?;

    let rate = if taxed.iter().all(|(_, rate)| rate == first_rate) {
        *first_rate
    } else {
        let mut gross = Decimal::ZERO;
        let mut net = Decimal::ZERO;
        for (amount, rate) in taxed {
            let gross_per_unit = Decimal::ONE + rate;
            if gross_per_unit <= Decimal::ZERO {
                // A rate at or below -100% leaves no net to recover.
                return None;
            }
            gross += amount;
            net += amount / gross_per_unit;
        }
        if net.is_zero() {
            return None;
        }
        gross / net - Decimal::ONE
    };

    (!rate.is_zero()).then_some(rate)
}

/// Raise an MRR figure to honor a period minimum-commitment floor.
///
/// The minimum is a per-billing-period floor; MRR is monthly, so the floor is converted to its
/// monthly equivalent through the same `monthly_share` every component MRR goes through — a floor
/// rounded differently from the spend it measures would top up (or fail to) by a cent on a period
/// that does not divide evenly. The
/// top-up mirrors the invoice shortfall line exactly: `max(0, floor_monthly - in_scope_mrr)` is
/// added to the base MRR, where `in_scope_mrr` is the monthly MRR of the lines the floor measures
/// (every line for `AllComponents`, product-matched lines for a scoped minimum). For the common
/// `AllComponents` case `in_scope_mrr == base_mrr`, so this reduces to `max(base_mrr, floor_monthly)`.
pub fn apply_minimum_mrr_floor(
    base_mrr: i64,
    in_scope_mrr: i64,
    minimum_amount_subunit: i64,
    period: BillingPeriodEnum,
) -> i64 {
    let floor_monthly = monthly_share(minimum_amount_subunit, i64::from(period.as_months()));
    base_mrr + (floor_monthly - in_scope_mrr).max(0)
}

/// True when a line's product is measured by the floor: every line under `AllComponents`, only
/// product-matched lines under `Products` (a line with no product can never be product-scoped).
fn scope_includes_product(
    scope: &crate::domain::plan_minimums::MinimumScope,
    product_id: Option<common_domain::ids::ProductId>,
) -> bool {
    match scope {
        crate::domain::plan_minimums::MinimumScope::AllComponents => true,
        crate::domain::plan_minimums::MinimumScope::Products(_) => {
            product_id.map(|p| scope.includes(p)).unwrap_or(false)
        }
    }
}

/// Use the same creation-time floor for MRR and its tax inputs. New-product extras whose
/// products have not been materialized yet stay out of product-scoped spend.
pub(crate) fn creation_minimum_for_mrr(
    components: &[SubscriptionComponentNewInternal],
    plan_minimum: Option<&crate::domain::plan_minimums::PlanMinimum>,
    minimum_override: Option<&(i64, crate::domain::plan_minimums::CreateMinimumScope)>,
    drop_minimum: bool,
) -> Option<(i64, crate::domain::plan_minimums::MinimumScope)> {
    use crate::domain::plan_minimums::{CreateMinimumScope, MinimumScope};

    if drop_minimum {
        return None;
    }
    if let Some((amount, create_scope)) = minimum_override {
        let scope = match create_scope {
            CreateMinimumScope::AllComponents => MinimumScope::AllComponents,
            CreateMinimumScope::Products {
                product_ids,
                extra_product_names,
            } => {
                let mut ids = product_ids.clone();
                for name in extra_product_names {
                    // A credit pack never counts toward a minimum, as on the invoice.
                    if let Some(pid) = components
                        .iter()
                        .filter(|c| !matches!(c.fee, SubscriptionFee::Credit { .. }))
                        .find(|c| &c.name == name)
                        .and_then(|c| c.product_id)
                    {
                        ids.push(pid);
                    }
                }
                MinimumScope::Products(ids)
            }
        };
        Some((*amount, scope))
    } else {
        plan_minimum.map(|pm| (pm.amount_subunit, pm.scope.clone()))
    }
}

/// What one fee contributes to MRR, as quoted, with how the minimum and included tax treat it.
#[derive(Debug, Clone, Copy)]
pub struct MrrLine {
    pub monthly: i64,
    /// The product its tax resolves on (`fee_tax_product`).
    pub product_id: Option<common_domain::ids::ProductId>,
    /// What the line's invoice carries per billing period; only the tax-rate resolution reads it.
    pub period_total: i64,
    credit_pack: bool,
    taxed_at_redemption: bool,
}

impl MrrLine {
    pub fn new(
        fee: &SubscriptionFee,
        monthly: i64,
        product_id: Option<common_domain::ids::ProductId>,
    ) -> Self {
        Self {
            monthly,
            product_id: fee_tax_product(fee, product_id),
            period_total: 0,
            credit_pack: matches!(fee, SubscriptionFee::Credit { .. }),
            taxed_at_redemption: fee_taxed_at_redemption(fee),
        }
    }

    pub(crate) fn with_period_total(self, period_total: i64) -> Self {
        Self {
            period_total,
            ..self
        }
    }

    pub(crate) fn taxed(&self) -> bool {
        !self.taxed_at_redemption
    }
}

/// The one rule behind every MRR figure. A minimum floors the spend it measures, which never
/// includes credit packs (as on the invoice), and the tax-inclusive rate, which describes taxed
/// lines only, is carved out of the taxed spend: a currency pack carries no tax until it is used.
pub(crate) fn subscription_mrr(
    lines: &[MrrLine],
    minimum: Option<(i64, &crate::domain::plan_minimums::MinimumScope)>,
    period: BillingPeriodEnum,
    tax_inclusive_rate: Option<Decimal>,
) -> i64 {
    let at_redemption: i64 = lines
        .iter()
        .filter(|l| l.taxed_at_redemption)
        .map(|l| l.monthly)
        .sum();
    let taxed: i64 = lines.iter().map(|l| l.monthly).sum::<i64>() - at_redemption;
    let floored = match minimum {
        None => taxed,
        Some((amount, scope)) => {
            let in_scope: i64 = lines
                .iter()
                .filter(|l| !l.credit_pack && scope_includes_product(scope, l.product_id))
                .map(|l| l.monthly)
                .sum();
            apply_minimum_mrr_floor(taxed, in_scope, amount, period)
        }
    };
    net_of_inclusive_tax(floored, tax_inclusive_rate) + at_redemption
}

/// Monthly MRR of new-subscription components + add-ons, floored by an optional minimum commitment.
/// Used at subscription creation so the persisted `mrr_cents` (and its BI movement log) reflect the
/// committed floor, not just the line spend.
pub fn calculate_subscription_mrr_with_floor(
    components: &[SubscriptionComponentNewInternal],
    add_ons: &[SubscriptionAddOnNewInternal],
    minimum: Option<&(i64, crate::domain::plan_minimums::MinimumScope)>,
    period: BillingPeriodEnum,
    precision: u8,
    tax_inclusive_rate: Option<Decimal>,
) -> i64 {
    let lines: Vec<MrrLine> = components
        .iter()
        .map(|c| {
            MrrLine::new(
                &c.fee,
                calculate_mrr(&c.fee, &c.period, precision),
                c.product_id,
            )
        })
        .chain(add_ons.iter().map(|a| {
            MrrLine::new(
                &a.fee,
                calculate_mrr(&a.fee, &a.period, precision) * i64::from(a.quantity),
                a.product_id,
            )
        }))
        .collect();
    subscription_mrr(
        &lines,
        minimum.map(|(amount, scope)| (*amount, scope)),
        period,
        tax_inclusive_rate,
    )
}

/// A subscription's MRR as stored, from its components' (slot-aware) lines and its add-ons,
/// floored by its minimum and net of included tax.
pub fn floor_subscription_mrr(
    sub_details: &SubscriptionDetails,
    component_lines: Vec<MrrLine>,
    precision: u8,
) -> i64 {
    subscription_mrr(
        &with_add_on_lines(sub_details, component_lines, precision),
        sub_details
            .minimum
            .as_ref()
            .map(|min| (min.amount_subunit, &min.scope)),
        sub_details.subscription.period,
        sub_details.subscription.tax_inclusive_rate,
    )
}

/// The subscription's MRR lines: its components' (slot-aware) ones, then its add-ons'.
pub(crate) fn with_add_on_lines(
    sub_details: &SubscriptionDetails,
    mut component_lines: Vec<MrrLine>,
    precision: u8,
) -> Vec<MrrLine> {
    component_lines.extend(sub_details.add_ons.iter().map(|a| {
        let quantity = i64::from(a.quantity);
        MrrLine::new(
            &a.fee,
            calculate_mrr(&a.fee, &a.period, precision) * quantity,
            a.product_id,
        )
        .with_period_total(fee_period_total_cents(&a.fee, precision) * quantity)
    }));
    component_lines
}

/// Scale a per-unit fee by `quantity`. Add-ons store their fee per-unit with the
/// instance count in a separate `quantity` column; this bakes that count into the
/// monetary fields so a single computed line reflects all instances. One-time fees
/// scale their quantity (rate stays per-unit); usage fees are left unchanged. This
/// is the canonical "apply add-on quantity" operation, shared by proration and
/// invoice computation so they always agree.
pub(crate) fn scale_fee(fee: &SubscriptionFee, quantity: i32) -> SubscriptionFee {
    let q = Decimal::from(quantity.max(0));
    match fee {
        SubscriptionFee::Rate { rate } => SubscriptionFee::Rate { rate: rate * q },
        SubscriptionFee::OneTime {
            rate,
            quantity: inner_qty,
        } => SubscriptionFee::OneTime {
            rate: *rate,
            quantity: inner_qty * quantity.max(0) as u32,
        },
        SubscriptionFee::Credit {
            rate,
            quantity: inner_qty,
            credit_amount,
            expiry,
            credit_unit,
            rollover,
            once,
        } => SubscriptionFee::Credit {
            rate: *rate,
            quantity: inner_qty * quantity.max(0) as u32,
            credit_amount: *credit_amount,
            expiry: *expiry,
            credit_unit: credit_unit.clone(),
            rollover: *rollover,
            once: *once,
        },
        SubscriptionFee::Recurring {
            rate,
            quantity: inner_qty,
            billing_type,
        } => SubscriptionFee::Recurring {
            rate: rate * q,
            quantity: *inner_qty,
            billing_type: billing_type.clone(),
        },
        SubscriptionFee::Capacity {
            rate,
            included,
            overage_rate,
            metric_id,
            credit_unit,
        } => SubscriptionFee::Capacity {
            rate: rate * q,
            included: *included,
            overage_rate: *overage_rate,
            metric_id: *metric_id,
            credit_unit: credit_unit.clone(),
        },
        SubscriptionFee::Slot {
            unit,
            unit_rate,
            min_slots,
            max_slots,
            initial_slots,
        } => SubscriptionFee::Slot {
            unit: unit.clone(),
            unit_rate: *unit_rate,
            min_slots: *min_slots,
            max_slots: *max_slots,
            initial_slots: initial_slots * quantity.max(0) as u32,
        },
        other => other.clone(),
    }
}

pub fn process_create_subscription_add_ons(
    create: &Option<CreateSubscriptionAddOns>,
    add_ons: &[AddOn],
    products: &HashMap<common_domain::ids::ProductId, crate::domain::Product>,
    prices: &HashMap<common_domain::ids::PriceId, crate::domain::prices::Price>,
    units: &crate::domain::credit_units::CreditUnitsById,
    product_family_id: ProductFamilyId,
    currency: &str,
    effective_from: chrono::NaiveDate,
) -> Result<
    (
        Vec<SubscriptionAddOnNewInternal>,
        Vec<PendingMaterialization>,
    ),
    Report<StoreError>,
> {
    let mut processed_add_ons = Vec::new();
    let mut pending_materializations = Vec::new();

    if let Some(create) = create {
        for cs_ao in &create.add_ons {
            let add_on = add_ons.iter().find(|x| x.id == cs_ao.add_on_id).ok_or(
                StoreError::ValueNotFound(format!("add-on {} not found", cs_ao.add_on_id)),
            )?;

            if cs_ao.quantity < 1 {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "add-on {} quantity must be >= 1",
                    cs_ao.add_on_id
                ))));
            }

            if let Some(max) = add_on.max_instances_per_subscription
                && cs_ao.quantity > max
            {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "add-on {} quantity {} exceeds max_instances_per_subscription {}",
                    cs_ao.add_on_id, cs_ao.quantity, max
                ))));
            }

            let resolved = add_on
                .resolve_customized(products, prices, units, &cs_ao.customization)
                .map_err(Report::new)?;

            let idx = processed_add_ons.len();

            // If price_id is None and the override uses PriceEntry::New, we need materialization
            if resolved.price_id.is_none()
                && let Some(PriceEntry::New(_)) = &resolved.price_entry
            {
                pending_materializations.push(PendingMaterialization {
                    component_index: idx,
                    name: resolved.name.clone(),
                    product_ref: ProductRef::Existing(add_on.product_id),
                    price_entry: resolved.price_entry.clone().unwrap(),
                    product_family_id,
                    currency: currency.to_string(),
                });
            }

            processed_add_ons.push(SubscriptionAddOnNewInternal {
                add_on_id: add_on.id,
                name: resolved.name,
                period: resolved.period,
                fee: resolved.fee,
                product_id: resolved.product_id,
                price_id: resolved.price_id,
                quantity: cs_ao.quantity,
                effective_from,
            });
        }
    }

    Ok((processed_add_ons, pending_materializations))
}

pub fn process_create_subscription_coupons(
    subscription: &SubscriptionRowNew,
    coupons: &[Coupon],
) -> Result<Vec<AppliedCouponRowNew>, Report<StoreError>> {
    let processed_coupons = coupons
        .iter()
        .unique_by(|x| x.id)
        .map(|x| AppliedCouponRowNew {
            id: AppliedCouponId::new(),
            subscription_id: subscription.id,
            coupon_id: x.id,
            customer_id: subscription.customer_id,
            is_active: true,
            applied_amount: None,
            applied_count: None,
            last_applied_at: None,
        })
        .collect();

    Ok(processed_coupons)
}

pub async fn apply_coupons(
    tx_conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    subscription_coupons: &[&AppliedCouponRowNew],
    subscriptions: &[CreatedSubscription],
    tenant_id: TenantId,
) -> StoreResult<Vec<CouponCreditRedemption>> {
    apply_coupons_at(
        tx_conn,
        controls,
        subscription_coupons,
        subscriptions,
        tenant_id,
        chrono::Utc::now(),
    )
    .await
}

/// `apply_coupons` as of `at`: valid then, and redeemed then.
pub(crate) async fn apply_coupons_at(
    tx_conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    subscription_coupons: &[&AppliedCouponRowNew],
    subscriptions: &[CreatedSubscription],
    tenant_id: TenantId,
    at: chrono::DateTime<chrono::Utc>,
) -> StoreResult<Vec<CouponCreditRedemption>> {
    validate_coupons_at(
        tx_conn,
        subscription_coupons,
        subscriptions,
        tenant_id,
        at.naive_utc(),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    apply_coupons_internal(tx_conn, controls, subscription_coupons, tenant_id, at).await
}

/// Applies coupons without validation.
///
/// Use this for async payment flows where coupons were already validated before charging.
/// The customer already paid the discounted price, so we honor it even if the coupon
/// became invalid (e.g., hit redemption limit) during the async payment window.
/// TODO use a coupon_reservation table with cleanup linked checkouts to avoid any over-redemption due to async flows
pub async fn apply_coupons_without_validation(
    tx_conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    subscription_coupons: &[&AppliedCouponRowNew],
    tenant_id: TenantId,
) -> StoreResult<Vec<CouponCreditRedemption>> {
    apply_coupons_internal(
        tx_conn,
        controls,
        subscription_coupons,
        tenant_id,
        chrono::Utc::now(),
    )
    .await
}

/// `apply_coupons_without_validation` redeemed at `at`, in business time: the promo credit it
/// funds runs from then.
pub(crate) async fn apply_coupons_without_validation_at(
    tx_conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    subscription_coupons: &[&AppliedCouponRowNew],
    tenant_id: TenantId,
    at: chrono::DateTime<chrono::Utc>,
) -> StoreResult<Vec<CouponCreditRedemption>> {
    apply_coupons_internal(tx_conn, controls, subscription_coupons, tenant_id, at).await
}

/// Every applied coupon is created here, so a promotional credit coupon always funds its grant.
async fn apply_coupons_internal(
    tx_conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    subscription_coupons: &[&AppliedCouponRowNew],
    tenant_id: TenantId,
    at: chrono::DateTime<chrono::Utc>,
) -> StoreResult<Vec<CouponCreditRedemption>> {
    if subscription_coupons.is_empty() {
        return Ok(Vec::new());
    }
    record_coupon_redemptions(tx_conn, subscription_coupons, at.naive_utc())
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    crate::services::credit::coupon_grant::mint_coupon_credit_tx(
        tx_conn,
        controls,
        tenant_id,
        subscription_coupons,
        at,
    )
    .await
}

async fn record_coupon_redemptions(
    tx_conn: &mut PgConn,
    subscription_coupons: &[&AppliedCouponRowNew],
    redeemed_at: chrono::NaiveDateTime,
) -> DbResult<()> {
    AppliedCouponRow::insert_batch(tx_conn, subscription_coupons.to_vec()).await?;

    CouponRow::update_last_redemption_at(
        tx_conn,
        &subscription_coupons
            .iter()
            .map(|c| c.coupon_id)
            .unique()
            .collect::<Vec<_>>(),
        redeemed_at,
    )
    .await?;

    let subscriptions_by_coupon: HashMap<CouponId, usize> =
        subscription_coupons.iter().counts_by(|x| x.coupon_id);

    for (coupon_id, subscriptions_count) in subscriptions_by_coupon {
        CouponRow::inc_redemption_count(tx_conn, coupon_id, subscriptions_count as i32).await?;
    }

    Ok(())
}

/// validate coupons can be applied to subscriptions
/// must be inside tx to handle concurrent inserts
async fn validate_coupons_at(
    tx_conn: &mut PgConn,
    subscription_coupons: &[&AppliedCouponRowNew],
    subscriptions: &[CreatedSubscription],
    tenant_id: TenantId,
    now: chrono::NaiveDateTime,
) -> DbResult<()> {
    if subscription_coupons.is_empty() {
        return Ok(());
    }

    let coupons_ids = subscription_coupons
        .iter()
        .map(|x| x.coupon_id)
        .unique()
        .collect::<Vec<_>>();

    let coupons = CouponRow::list_by_ids_for_update(tx_conn, &coupons_ids, &tenant_id).await?;

    // Fetch plan restrictions for coupons
    let coupon_plan_restrictions = CouponPlanRow::list_by_coupon_ids(tx_conn, &coupons_ids).await?;

    // Fetch plan_ids for all plan_version_ids in subscriptions
    let plan_version_ids: Vec<PlanVersionId> =
        subscriptions.iter().map(|s| s.plan_version_id).collect();
    let plan_version_to_plan =
        PlanVersionRow::get_plan_ids_by_version_ids(tx_conn, &plan_version_ids).await?;

    for coupon in &coupons {
        let applied_coupon = subscription_coupons
            .iter()
            .find(|x| x.coupon_id == coupon.id)
            .ok_or(Report::new(DatabaseError::ValidationError(format!(
                "Applied coupon {} not found",
                coupon.code
            ))))?;

        let subscription = subscriptions
            .iter()
            .find(|x| x.id == applied_coupon.subscription_id)
            .ok_or(Report::new(DatabaseError::ValidationError(format!(
                "Subscription {} not found",
                applied_coupon.subscription_id
            ))))?;

        // check expired coupons
        if coupon.expires_at.is_some_and(|x| x <= now) {
            return Err(Report::new(DatabaseError::ValidationError(format!(
                "coupon {} is expired",
                coupon.code
            )))
            .into());
        }
        // check archived coupons
        if coupon.archived_at.is_some() {
            return Err(Report::new(DatabaseError::ValidationError(format!(
                "coupon {} is archived",
                coupon.code
            )))
            .into());
        }

        // check disabled coupons
        if coupon.disabled {
            return Err(Report::new(DatabaseError::ValidationError(format!(
                "coupon {} is disabled",
                coupon.code
            )))
            .into());
        }

        // TEMPORARY CHECK: currency is the same as in subscription
        let discount: CouponDiscount =
            serde_json::from_value(coupon.discount.clone()).map_err(|_| {
                Report::new(DatabaseError::ValidationError(format!(
                    "Discount serde error for coupon {}",
                    &coupon.code
                )))
            })?;

        if discount
            .currency()
            .is_some_and(|x| x != subscription.currency)
        {
            return Err(Report::new(DatabaseError::ValidationError(format!(
                "coupon {} currency does not match subscription currency",
                coupon.code
            )))
            .into());
        }

        // check plan restrictions
        if let Some(allowed_plans) = coupon_plan_restrictions.get(&coupon.id)
            && !allowed_plans.is_empty()
        {
            let plan_id = plan_version_to_plan
                .get(&subscription.plan_version_id)
                .ok_or(Report::new(DatabaseError::ValidationError(format!(
                    "Plan not found for subscription {}",
                    subscription.id
                ))))?;

            if !allowed_plans.contains(plan_id) {
                return Err(Report::new(DatabaseError::ValidationError(format!(
                    "coupon {} cannot be applied to this plan",
                    coupon.code
                )))
                .into());
            }
        }
    }

    // check if the coupon has reached its redemption limit
    let subscriptions_by_coupon: HashMap<CouponId, usize> =
        subscription_coupons.iter().counts_by(|x| x.coupon_id);
    for (coupon_id, subscriptions_count) in subscriptions_by_coupon {
        let coupon = coupons
            .iter()
            .find(|x| x.id == coupon_id)
            .ok_or(Report::new(DatabaseError::ValidationError(format!(
                "coupon {coupon_id} not found"
            ))))?;

        if let Some(redemption_limit) = coupon.redemption_limit
            && redemption_limit < subscriptions_count as i32 + coupon.redemption_count
        {
            return Err(Report::new(DatabaseError::ValidationError(format!(
                "coupon {} has reached its maximum redemptions",
                coupon.code
            )))
            .into());
        }
    }

    // check non-reusable coupons - these cannot be reused by the same customer. Promotional
    // credit is redeemed once per customer, whatever `reusable` says.
    let non_reusable_coupons = coupons
        .iter()
        .filter(|x| {
            !x.reusable
                || serde_json::from_value::<CouponDiscount>(x.discount.clone())
                    .is_ok_and(|discount| discount.is_credit())
        })
        .collect::<Vec<_>>();
    if !non_reusable_coupons.is_empty() {
        let in_batch = subscription_coupons
            .iter()
            .filter(|applied| {
                non_reusable_coupons
                    .iter()
                    .any(|c| c.id == applied.coupon_id)
            })
            .map(|applied| (applied.coupon_id, applied.customer_id))
            .counts();
        if let Some(((coupon_id, _), _)) = in_batch.into_iter().find(|(_, count)| *count > 1) {
            return Err(Report::new(DatabaseError::ValidationError(format!(
                "coupon {coupon_id} can be used only once per customer"
            )))
            .into());
        }

        // Build a list of (coupon_id, customer_id) pairs for non-reusable coupons
        let customer_coupon_pairs: Vec<(CouponId, common_domain::ids::CustomerId)> =
            subscription_coupons
                .iter()
                .filter_map(|applied| {
                    let coupon = non_reusable_coupons
                        .iter()
                        .find(|c| c.id == applied.coupon_id)?;
                    Some((coupon.id, applied.customer_id))
                })
                .collect();

        // Check if any of these pairs already exist in the database
        let existing_pairs =
            AppliedCouponRow::find_existing_customer_coupon_pairs(tx_conn, &customer_coupon_pairs)
                .await?;

        // If any customer is trying to reuse a non-reusable coupon, reject it
        for coupon in non_reusable_coupons {
            let reused_by_customer = subscription_coupons.iter().find(|applied| {
                applied.coupon_id == coupon.id
                    && existing_pairs.contains(&(coupon.id, applied.customer_id))
            });

            if let Some(applied) = reused_by_customer {
                return Err(Report::new(DatabaseError::ValidationError(format!(
                    "coupon {} is not reusable and has already been used by customer {}",
                    coupon.code, applied.customer_id
                )))
                .into());
            }
        }
    }

    Ok(())
}

// TODO check with the other calculate_coupons_discount
#[allow(dead_code)]
pub async fn calculate_coupons_discount(
    store: &Store,
    coupons: &[Coupon],
    subscription_currency: &String,
    amount: Decimal,
) -> StoreResult<Decimal> {
    if (amount == Decimal::ZERO) || coupons.is_empty() {
        return Ok(amount);
    }

    let mut total = amount;

    for coupon in coupons {
        if !coupon.is_infinite() {
            continue;
        }

        match &coupon.discount {
            CouponDiscount::Percentage(percentage) => {
                total = total * percentage / Decimal::new(100, 0);
            }
            CouponDiscount::Fixed {
                currency,
                amount: fixed_amount,
            } => {
                let discount_amount = if currency == subscription_currency {
                    *fixed_amount
                } else {
                    let rate = store
                        .get_historical_rate(
                            currency,
                            subscription_currency,
                            chrono::Utc::now().date_naive(),
                        )
                        .await?
                        .ok_or(StoreError::ValueNotFound(format!(
                            "historical rate from {currency} to {subscription_currency}"
                        )))?
                        .rate;

                    fixed_amount * Decimal::from_f64(rate).unwrap_or(Decimal::ZERO)
                };

                total = (total - discount_amount).max(Decimal::ZERO);
            }
            CouponDiscount::Credit { .. } => {}
        }
    }

    Ok(total)
}

pub fn extract_billing_period(
    components: &[SubscriptionComponentNewInternal],
    add_ons: &[SubscriptionAddOnNewInternal],
) -> BillingPeriodEnum {
    components
        .iter()
        .map(|x| &x.period)
        .chain(add_ons.iter().map(|x| &x.period))
        .filter_map(SubscriptionFeeBillingPeriod::as_billing_period_opt)
        .min()
        .unwrap_or(BillingPeriodEnum::Monthly)
}

pub fn process_create_subscription_components(
    param: &Option<CreateSubscriptionComponents>,
    map: &HashMap<PlanVersionId, Vec<PriceComponent>>,
    sub: &SubscriptionNew,
    products: &HashMap<common_domain::ids::ProductId, crate::domain::Product>,
    units: &crate::domain::credit_units::CreditUnitsById,
    resolved: &ResolvedCustomComponents,
    product_family_id: ProductFamilyId,
    currency: &str,
    effective_from: NaiveDate,
) -> Result<
    (
        Vec<SubscriptionComponentNewInternal>,
        Vec<PendingMaterialization>,
    ),
    Report<StoreError>,
> {
    let mut processed_components = Vec::new();
    let mut pending_materializations = Vec::new();

    let (parameterized_components, remove_components) = if let Some(p) = param {
        (&p.parameterized_components, &p.remove_components)
    } else {
        (&Vec::new(), &Vec::new())
    };

    let binding = vec![];
    let plan_price_components = map.get(&sub.plan_version_id).unwrap_or(&binding);

    for c in plan_price_components {
        let component_id = c.id;

        // Check overridden_components first (pre-resolved in gather phase).
        // If there's also a parameterization for this component, apply its parameters
        // (e.g. initial_slot_count) to the override's fee.
        if let Some(resolved_override) = resolved.overrides.get(&component_id) {
            let mut fee = resolved_override.fee.clone();
            if let Some(parameterized) = parameterized_components
                .iter()
                .find(|p| p.component_id == component_id)
            {
                fee.apply_parameters(&parameterized.parameters);
            }
            let idx = processed_components.len();
            processed_components.push(SubscriptionComponentNewInternal {
                price_component_id: resolved_override.price_component_id,
                product_id: resolved_override.existing_product_id(),
                name: resolved_override.name.clone(),
                period: resolved_override.period,
                fee,
                is_override: true,
                price_id: resolved_override.existing_price_id(),
                effective_from,
            });
            if resolved_override.needs_materialization() {
                pending_materializations.push(PendingMaterialization {
                    component_index: idx,
                    name: resolved_override.name.clone(),
                    product_ref: resolved_override.product_ref.clone(),
                    price_entry: resolved_override.price_entry.clone(),
                    product_family_id,
                    currency: currency.to_string(),
                });
            }
            continue;
        }

        // Check parameterized_components (without override)
        if let Some(parameterized) = parameterized_components
            .iter()
            .find(|p| p.component_id == component_id)
        {
            use crate::domain::price_components::ComponentParameters;
            let params = ComponentParameters {
                initial_slot_count: parameterized.parameters.initial_slot_count,
                billing_period: parameterized.parameters.billing_period,
                committed_capacity: parameterized.parameters.committed_capacity,
            };
            let resolved = c
                .resolve_fee(products, units, Some(&params))
                .map_err(Report::new)?;

            processed_components.push(SubscriptionComponentNewInternal {
                price_component_id: Some(c.id),
                product_id: c.product_id,
                name: c.name.clone(),
                period: resolved.period,
                fee: resolved.fee,
                is_override: false,
                price_id: resolved.price_id,
                effective_from,
            });
            continue;
        }

        // Check if the component is in remove_components
        if remove_components.contains(&component_id) {
            continue;
        }

        // Default: resolve via v2 path or legacy path
        let resolved = c.resolve_fee(products, units, None).map_err(Report::new)?;

        processed_components.push(SubscriptionComponentNewInternal {
            price_component_id: Some(c.id),
            product_id: c.product_id,
            name: c.name.clone(),
            period: resolved.period,
            fee: resolved.fee,
            is_override: false,
            price_id: resolved.price_id,
            effective_from,
        });
    }

    // Append pre-resolved extra components
    for extra in &resolved.extras {
        let idx = processed_components.len();
        processed_components.push(SubscriptionComponentNewInternal {
            price_component_id: None,
            product_id: extra.existing_product_id(),
            name: extra.name.clone(),
            period: extra.period,
            fee: extra.fee.clone(),
            is_override: false,
            price_id: extra.existing_price_id(),
            effective_from,
        });
        if extra.needs_materialization() {
            pending_materializations.push(PendingMaterialization {
                component_index: idx,
                name: extra.name.clone(),
                product_ref: extra.product_ref.clone(),
                price_entry: extra.price_entry.clone(),
                product_family_id,
                currency: currency.to_string(),
            });
        }
    }

    for (idx, component) in processed_components.iter().enumerate() {
        if let SubscriptionFee::Credit {
            rate,
            quantity,
            credit_amount,
            expiry,
            credit_unit,
            rollover,
            once,
        } = &component.fee
        {
            crate::domain::prices::validate_credit_once(*once, rollover.as_ref())?;
            // A one-time credit is granted once per product, which needs the product.
            if *once
                && component.product_id.is_none()
                && !pending_materializations
                    .iter()
                    .any(|pending| pending.component_index == idx)
            {
                return Err(Report::new(StoreError::InvalidArgument(
                    "a credit granted once must belong to a product".to_owned(),
                )));
            }
            // A price-backed pack's amount was checked against its denomination when the price
            // was saved.
            let unit_precision = credit_unit.as_ref().map(|unit| unit.precision).or(component
                .price_id
                .map(|_| crate::domain::credit_units::CREDIT_UNIT_MAX_PRECISION));
            let precision = crate::domain::prices::validate_credit_pack(
                *rate,
                *quantity,
                *credit_amount,
                *expiry,
                currency,
                unit_precision,
            )?;
            crate::domain::prices::validate_credit_rollover(
                *rate,
                *expiry,
                rollover.as_ref(),
                precision,
            )?;
        }
    }

    Ok((processed_components, pending_materializations))
}

impl SubscriptionDetails {
    /// Whether the subscription can accrue billable amounts in arrears — usage/metered or
    /// pure-arrears components/add-ons, or a minimum commitment. Such a subscription must be
    /// recomputed at finalization even when its current invoice lines are empty, because usage may
    /// have been imported since the draft was created; short-circuiting on empty lines would drop
    /// it. Advance-only subscriptions can safely reuse their stored lines.
    pub fn can_accrue_arrears(&self) -> bool {
        self.minimum.is_some()
            || self
                .price_components
                .iter()
                .any(|c| c.fee.is_pure_arrears() || c.fee.metric_id().is_some())
            || self
                .add_ons
                .iter()
                .any(|a| a.fee.is_pure_arrears() || a.fee.metric_id().is_some())
    }

    pub fn calculate_cancellable_end_of_period_date(&self, now: NaiveDate) -> Option<NaiveDate> {
        // to calculate last billing period :
        // if there is a commitment, use that commitment (currently no commitment so let's ignore)
        // else, we take the longest period from the main components (rate/slots/capacity), as that's what the user has already paid
        // else, that mean we're arrear and it's monthly. TODO
        let period = self
            .price_components
            .iter()
            .filter(|c| c.is_standard())
            .map(|c| c.period)
            .max_by(|a, b| a.as_months().cmp(&b.as_months()))
            .and_then(|p| p.as_billing_period_opt())
            .unwrap_or(BillingPeriodEnum::Monthly);

        let periods = calculate_advance_period_range(
            now,
            u32::from(self.subscription.billing_day_anchor),
            false,
            &period,
        );

        Some(periods.end)
    }
}

pub async fn is_paid_trial(
    conn: &mut PgConn,
    plan_version_id: PlanVersionId,
    tenant_id: TenantId,
    has_trial: bool,
) -> StoreResult<bool> {
    if !has_trial {
        return Ok(false);
    }

    let plan_with_version = PlanRow::get_with_version(conn, plan_version_id, tenant_id).await?;
    Ok(plan_with_version
        .version
        .map(|v| !v.trial_is_free)
        .unwrap_or(false))
}

impl Services {
    pub(super) async fn insert_created_outbox_events_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        created: &[CreatedSubscription],
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let ids = created.iter().map(|c| c.id).collect::<Vec<_>>();
        if ids.is_empty() {
            return Ok(());
        }
        let subscriptions: Vec<Subscription> =
            SubscriptionRow::list_subscriptions_by_ids(conn, &tenant_id, &ids)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(std::convert::TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?;
        let outbox_events: Vec<OutboxEvent> = subscriptions
            .into_iter()
            .map(|s| OutboxEvent::subscription_created(s.into()))
            .collect();
        self.store
            .internal
            .record_outbox_batch_tx(conn, tenant_id, actor, outbox_events)
            .await
    }

    /// Emit a `subscription.updated` webhook for an in-place mutation. The mutation site
    /// records its own audit activity, so this only publishes to the outbox topic
    /// (`OutboxEvent::SubscriptionUpdated` maps to no activity row).
    pub(in crate::services) async fn emit_subscription_updated_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        change_type: SubscriptionUpdateType,
    ) -> StoreResult<()> {
        let mut event = self
            .subscription_event_payload_tx(conn, tenant_id, subscription_id)
            .await?;
        event.change_type = Some(change_type);
        self.store
            .internal
            .record_outbox_batch_tx(
                conn,
                tenant_id,
                &Actor::System,
                vec![OutboxEvent::subscription_updated(event)],
            )
            .await
    }

    /// Emit a `subscription.cancelled` webhook when a subscription reaches a terminal
    /// cancelled/aborted state. `reason` is the cancellation reason if one was supplied.
    pub(in crate::services) async fn emit_subscription_cancelled_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        reason: Option<String>,
    ) -> StoreResult<()> {
        let mut event = self
            .subscription_event_payload_tx(conn, tenant_id, subscription_id)
            .await?;
        event.cancellation_reason = reason;
        self.store
            .internal
            .record_outbox_batch_tx(
                conn,
                tenant_id,
                &Actor::System,
                vec![OutboxEvent::subscription_cancelled(event)],
            )
            .await
    }

    /// Emit a `subscription.ended` webhook when a fixed-term subscription reaches its end
    /// date and completes. Terminal end-of-life, sibling of `subscription.cancelled`.
    pub(in crate::services) async fn emit_subscription_ended_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<()> {
        let event = self
            .subscription_event_payload_tx(conn, tenant_id, subscription_id)
            .await?;
        self.store
            .internal
            .record_outbox_batch_tx(
                conn,
                tenant_id,
                &Actor::System,
                vec![OutboxEvent::subscription_ended(event)],
            )
            .await
    }

    async fn subscription_event_payload_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<SubscriptionEvent> {
        let subscription: Subscription =
            SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .try_into()?;
        Ok(subscription.into())
    }
}

#[cfg(test)]
mod tax_inclusive_mrr_tests {
    use super::{blended_inclusive_rate, net_of_inclusive_tax};
    use rust_decimal_macros::dec;

    #[test]
    fn one_rate_across_every_line_is_that_rate_exactly() {
        assert_eq!(
            blended_inclusive_rate(&[(dec!(3500), dec!(0.2)), (dec!(1200), dec!(0.2))]),
            Some(dec!(0.2))
        );
    }

    #[test]
    fn lines_at_different_rates_blend_by_their_quoted_amounts() {
        // 120.00 at 20% is 100.00 net, 105.50 at 5.5% is 100.00 net: 225.50 quoted on
        // 200.00 of revenue, so the subscription carries 12.75%.
        assert_eq!(
            blended_inclusive_rate(&[(dec!(120), dec!(0.2)), (dec!(105.5), dec!(0.055))]),
            Some(dec!(0.1275))
        );
    }

    #[test]
    fn an_untaxed_line_pulls_the_blended_rate_down() {
        // A non-taxable component beside a standard-rated one: 120 + 100 quoted on
        // 100 + 100 net.
        assert_eq!(
            blended_inclusive_rate(&[(dec!(120), dec!(0.2)), (dec!(100), dec!(0))]),
            Some(dec!(0.1))
        );
    }

    /// The invariant the weights exist for: netting the summed monthly gross by the blended
    /// rate has to land on the sum of the lines' own net, whatever mix of billing periods
    /// the subscription carries.
    ///
    /// An annual component of 1200.00 at 20% beside a monthly one of 100.00 at the 5.5%
    /// reduced rate: both contribute 100.00 a month, so the blend is over 10000/10000, not
    /// over the 120000/10000 the invoice lines carry. Weighted by the period totals the
    /// annual line would dominate (rate 0.18745) and the stored MRR would come out 16844
    /// against a true 17812 — 969 cents a month understated, inherited by the Created
    /// event's delta, the BI movement log and every later recompute.
    #[test]
    fn a_mix_of_billing_periods_blends_over_the_monthly_shares() {
        let annual_share = dec!(10000); // 1200.00 a year
        let monthly_share = dec!(10000); // 100.00 a month

        let rate =
            blended_inclusive_rate(&[(annual_share, dec!(0.2)), (monthly_share, dec!(0.055))])
                .expect("two taxed lines carry a rate");

        let gross_mrr = 20_000;
        let line_by_line = net_of_inclusive_tax(10_000, Some(dec!(0.2)))
            + net_of_inclusive_tax(10_000, Some(dec!(0.055)));
        assert_eq!(line_by_line, 17_812);
        assert_eq!(
            net_of_inclusive_tax(gross_mrr, Some(rate)),
            line_by_line,
            "the stored rate must net the summed MRR to the sum of the lines' net"
        );
    }

    #[test]
    fn nothing_is_carved_out_when_no_tax_is_borne() {
        // Exempt, reverse-charged, exported or genuinely zero-rated: the engine resolves
        // every line to a zero rate and the quoted price is left whole.
        assert_eq!(
            blended_inclusive_rate(&[(dec!(3500), dec!(0)), (dec!(1200), dec!(0))]),
            None
        );
        // Nothing to resolve at all.
        assert_eq!(blended_inclusive_rate(&[]), None);
        // A zero-amount line carries no rate and is not allowed to define one.
        assert_eq!(blended_inclusive_rate(&[(dec!(0), dec!(0.2))]), None);
    }

    #[test]
    fn netting_divides_the_quoted_price_and_rounds_half_up() {
        // 35.00 quoted at 20% is 29.1666… net.
        assert_eq!(net_of_inclusive_tax(3500, Some(dec!(0.2))), 2917);
        // A tax-exclusive subscription is left exactly as it was.
        assert_eq!(net_of_inclusive_tax(3500, None), 3500);
        assert_eq!(net_of_inclusive_tax(0, Some(dec!(0.2))), 0);
        // 10.05 / 1.2 = 8.375 — the half goes up.
        assert_eq!(net_of_inclusive_tax(1005, Some(dec!(0.2))), 838);
        // A churn of the same figure nets to the same magnitude.
        assert_eq!(net_of_inclusive_tax(-3500, Some(dec!(0.2))), -2917);
    }
}

#[cfg(test)]
mod mrr_rounding_tests {
    use super::{calculate_mrr, monthly_share};
    use crate::domain::SubscriptionFee;
    use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
    use rust_decimal_macros::dec;

    #[test]
    fn a_period_that_does_not_divide_evenly_rounds_half_up() {
        // 5 slots at 100.05/year: 500.25 / 12 = 41.6875 → 41.69, not 41.68.
        assert_eq!(monthly_share(50_025, 12), 4169);
        assert_eq!(
            calculate_mrr(
                &SubscriptionFee::Slot {
                    unit: "seat".to_string(),
                    unit_rate: dec!(100.05),
                    min_slots: None,
                    max_slots: None,
                    initial_slots: 5,
                },
                &SubscriptionFeeBillingPeriod::Annual,
                2,
            ),
            4169,
            "the slot-aware recompute in plan_change must land on this same figure"
        );
    }

    #[test]
    fn a_monthly_period_is_its_own_share() {
        assert_eq!(monthly_share(3_500, 1), 3_500);
        // A period of zero months would divide by zero; treat it as monthly.
        assert_eq!(monthly_share(3_500, 0), 3_500);
    }

    #[test]
    fn the_floor_is_rounded_the_same_way_the_spend_is() {
        use super::apply_minimum_mrr_floor;
        // An annual floor of 500.25 is 41.69/month, the same figure a 500.25/year
        // component contributes — so a floor equal to the spend never tops it up.
        assert_eq!(
            apply_minimum_mrr_floor(4169, 4169, 50_025, BillingPeriodEnum::Annual),
            4169
        );
    }
}

#[cfg(test)]
mod mrr_floor_tests {
    use super::apply_minimum_mrr_floor;
    use crate::domain::enums::BillingPeriodEnum;

    #[test]
    fn creation_minimum_honors_override_scope_and_drop() {
        use super::creation_minimum_for_mrr;
        use crate::domain::plan_minimums::{CreateMinimumScope, MinimumScope, PlanMinimum};
        use common_domain::ids::{BaseId, PlanVersionId, ProductId};

        let product_id = ProductId::new();
        let plan = PlanMinimum {
            plan_version_id: PlanVersionId::new(),
            amount_subunit: 12_000,
            scope: MinimumScope::AllComponents,
        };
        let override_minimum = (
            5_000,
            CreateMinimumScope::Products {
                product_ids: vec![product_id],
                extra_product_names: vec![],
            },
        );
        assert_eq!(
            creation_minimum_for_mrr(&[], Some(&plan), None, false),
            Some((12_000, MinimumScope::AllComponents)),
        );
        assert_eq!(
            creation_minimum_for_mrr(&[], Some(&plan), Some(&override_minimum), false),
            Some((5_000, MinimumScope::Products(vec![product_id]))),
        );
        assert_eq!(creation_minimum_for_mrr(&[], Some(&plan), None, true), None);
    }

    #[test]
    fn all_components_floor_raises_mrr_to_monthly_minimum() {
        // Rate $0.80, AllComponents floor $0.90 (in_scope == base for AllComponents):
        // the displayed MRR must be the 90 floor, not the 80 spend.
        assert_eq!(
            apply_minimum_mrr_floor(80, 80, 90, BillingPeriodEnum::Monthly),
            90
        );
    }

    #[test]
    fn floor_below_spend_leaves_mrr_untouched() {
        // Spend already above the floor → no top-up.
        assert_eq!(
            apply_minimum_mrr_floor(120, 120, 90, BillingPeriodEnum::Monthly),
            120
        );
    }

    #[test]
    fn annual_minimum_is_converted_to_monthly() {
        // An annual floor of 1200 is 100/month; base 80 → floored to 100.
        assert_eq!(
            apply_minimum_mrr_floor(80, 80, 1200, BillingPeriodEnum::Annual),
            100
        );
    }

    #[test]
    fn scoped_floor_only_tops_up_the_in_scope_shortfall() {
        // base 80 (50 in-scope + 30 out-of-scope), floor 90 measured on the in-scope 50:
        // top-up = 90 - 50 = 40, so MRR = 80 + 40 = 120 (out-of-scope spend is preserved).
        assert_eq!(
            apply_minimum_mrr_floor(80, 50, 90, BillingPeriodEnum::Monthly),
            120
        );
    }
}

#[cfg(test)]
mod credit_pack_mrr_tests {
    use super::{
        MrrLine, calculate_mrr, fee_tax_product, fee_taxed_at_redemption, subscription_mrr,
    };
    use crate::domain::SubscriptionFee;
    use crate::domain::credit_units::CreditUnitRef;
    use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
    use crate::domain::plan_minimums::MinimumScope;
    use common_domain::ids::{BaseId, CreditUnitId};
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    const MONTHLY: SubscriptionFeeBillingPeriod = SubscriptionFeeBillingPeriod::Monthly;

    fn pack(rate: Decimal, quantity: u32, credit_unit: Option<CreditUnitRef>) -> SubscriptionFee {
        SubscriptionFee::Credit {
            rate,
            quantity,
            credit_amount: dec!(550),
            expiry: None,
            credit_unit,
            rollover: None,
            once: false,
        }
    }

    fn unit() -> CreditUnitRef {
        CreditUnitRef {
            unit_id: CreditUnitId::new(),
            code: "credits".to_owned(),
            precision: 0,
            product_id: None,
        }
    }

    fn line(fee: &SubscriptionFee) -> MrrLine {
        MrrLine::new(fee, calculate_mrr(fee, &MONTHLY, 2), None)
    }

    #[test]
    fn a_unit_pack_is_taxed_under_the_unit_s_product_when_it_has_one() {
        use common_domain::ids::ProductId;
        let (component, unit_product) = (ProductId::new(), ProductId::new());
        let taxed = CreditUnitRef {
            product_id: Some(unit_product),
            ..unit()
        };
        assert_eq!(
            fee_tax_product(&pack(dec!(40), 1, Some(taxed)), Some(component)),
            Some(unit_product)
        );
        assert_eq!(
            fee_tax_product(&pack(dec!(40), 1, Some(unit())), Some(component)),
            Some(component),
            "a unit without a product keeps the component's"
        );
        assert_eq!(
            fee_tax_product(&pack(dec!(40), 1, None), Some(component)),
            Some(component),
            "a currency pack is the component's"
        );
    }

    #[test]
    fn a_recurring_pack_counts_at_its_net_price_per_month() {
        assert_eq!(
            calculate_mrr(&pack(dec!(500), 1, None), &MONTHLY, 2),
            50_000
        );
        assert_eq!(calculate_mrr(&pack(dec!(49), 2, None), &MONTHLY, 2), 9_800);
        assert_eq!(
            calculate_mrr(
                &pack(dec!(6000), 1, None),
                &SubscriptionFeeBillingPeriod::Annual,
                2
            ),
            50_000
        );
    }

    #[test]
    fn included_credit_and_one_time_fees_add_nothing() {
        assert_eq!(calculate_mrr(&pack(Decimal::ZERO, 1, None), &MONTHLY, 2), 0);
        assert_eq!(
            calculate_mrr(
                &SubscriptionFee::OneTime {
                    rate: dec!(100),
                    quantity: 1
                },
                &SubscriptionFeeBillingPeriod::OneTime,
                2
            ),
            0
        );
    }

    #[test]
    fn a_pack_granted_once_at_start_is_not_recurring_revenue() {
        let once = SubscriptionFee::Credit {
            rate: dec!(500),
            quantity: 1,
            credit_amount: dec!(550),
            expiry: None,
            credit_unit: None,
            rollover: None,
            once: true,
        };
        assert_eq!(calculate_mrr(&once, &MONTHLY, 2), 0);
        assert_eq!(
            subscription_mrr(
                &[line(&once), line(&pack(dec!(49), 1, Some(unit())))],
                None,
                BillingPeriodEnum::Monthly,
                None
            ),
            4_900
        );
    }

    #[test]
    fn only_a_currency_pack_is_taxed_at_redemption() {
        assert!(fee_taxed_at_redemption(&pack(dec!(500), 1, None)));
        assert!(!fee_taxed_at_redemption(&pack(dec!(49), 1, Some(unit()))));
    }

    /// The stored rate describes taxed lines only, so a pack added or removed later never
    /// shifts what the fees net to.
    #[test]
    fn a_currency_pack_is_never_netted_by_the_inclusive_rate() {
        let fee = SubscriptionFee::Rate { rate: dec!(120) };
        let rate = Some(dec!(0.2));
        let period = BillingPeriodEnum::Monthly;
        let fee_only = subscription_mrr(&[line(&fee)], None, period, rate);
        let with_pack = subscription_mrr(
            &[line(&fee), line(&pack(dec!(500), 1, None))],
            None,
            period,
            rate,
        );
        assert_eq!(fee_only, 10_000);
        assert_eq!(with_pack, 60_000);
        assert_eq!(
            subscription_mrr(&[line(&pack(dec!(500), 1, None))], None, period, rate),
            50_000
        );
        // A credit-unit pack is sold taxed, so its quoted price carries the tax.
        assert_eq!(
            subscription_mrr(
                &[line(&pack(dec!(60), 1, Some(unit())))],
                None,
                period,
                rate
            ),
            5_000
        );
    }

    /// $5k minimum + $1k fee + $500 pack: the invoice bills the $4k shortfall beside the pack.
    #[test]
    fn a_pack_never_counts_toward_the_minimum() {
        let fee = SubscriptionFee::Rate { rate: dec!(1000) };
        let lines = [line(&fee), line(&pack(dec!(500), 1, None))];
        assert_eq!(
            subscription_mrr(
                &lines,
                Some((500_000, &MinimumScope::AllComponents)),
                BillingPeriodEnum::Monthly,
                None
            ),
            550_000
        );
    }
}
