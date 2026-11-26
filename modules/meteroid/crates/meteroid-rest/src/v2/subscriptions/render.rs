//! A subscription as v2 reads it: its row, and on a single one its charges (each fee read back as
//! the canonical composition a plan's charge reads as), quantities, add-ons, coupons, schedule
//! and minimum. And the documents a change previews.
use super::model::{
    AddOnRef, AmendmentSummary, AppliedCoupon, Cancellation, ChargeOrigin, CouponRef,
    CurrentPeriod, CustomerRef, Dates, Document, DocumentType, Invoicing, Money, PendingQuantity,
    PlanRef, QuantityState, ResourceRef, ScheduledChange, ScheduledChangeType, SubscribedAddOn,
    Subscription, SubscriptionCharge, SubscriptionStatus, TrialState,
};
use crate::AppState;
use crate::v2::charges::{OptionSet, StoredCharge, StoredPrice, render};
use crate::v2::customers::resources::resource_refs;
use crate::v2::documents::{self, ProductCodes};
use crate::v2::entitlements::model::{Codes, Entitlement};
use crate::v2::plans::Minimum;
use crate::v2::problem::Problem;
use crate::v2::references::CatalogNames;
use chrono::{DateTime, Utc};
use common_domain::ids::{
    AddOnId, BaseId, EntitlementEntityId, ManagedQuantityId, PlanId, PlanOptionId, PlanVersionId,
    ProductId, ResourceId, SubscriptionId, TenantId,
};
use meteroid_store::domain::credit_units::PriceCreditUnit;
use meteroid_store::domain::enums::{
    BillingPeriodEnum, ScheduledEventTypeEnum, SubscriptionFeeBillingPeriod,
};
use meteroid_store::domain::plan_minimums::MinimumScope;
use meteroid_store::domain::price_components::{DowngradePolicy, UpgradePolicy};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::slot_transactions::SlotTransaction;
use meteroid_store::domain::subscription_amendment::AmendmentPreviewExtended;
use meteroid_store::domain::subscription_changes::{CancellationPreview, PlanChangeOutcome};
use meteroid_store::domain::subscriptions::PendingScheduledEvent;
use meteroid_store::domain::{
    SlotTransactionStatusEnum, Subscription as DomainSubscription, SubscriptionDetails,
    SubscriptionFee,
};
use meteroid_store::repositories::add_ons::AddOnInterfaceAuto;
use meteroid_store::repositories::entitlements::EntitlementsInterface;
use meteroid_store::repositories::plan_options::PlanOptionInterface;
use meteroid_store::repositories::plans::PlansInterfaceAuto;
use meteroid_store::repositories::products::ProductInterface;
use meteroid_store::repositories::subscription_pages::ListedSubscription;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;
use meteroid_store::repositories::{
    CatalogCodesInterface, ManagedQuantitiesInterface, SubscriptionInterface,
    SubscriptionPagesInterface,
};
use meteroid_store::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashMap, HashSet};

// ── Fees as charges ─────────────────────────────────────────────────

/// How `fee` bills, as the charge structure and price a plan's charge would hold.
fn structure_of(fee: &SubscriptionFee) -> (FeeStructure, Pricing, Option<PriceCreditUnit>) {
    match fee {
        SubscriptionFee::Rate { rate } => {
            (FeeStructure::Rate {}, Pricing::Rate { rate: *rate }, None)
        }
        SubscriptionFee::OneTime {
            rate,
            quantity,
            model,
        } => (
            FeeStructure::OneTime {},
            Pricing::OneTime {
                unit_price: *rate,
                quantity: *quantity,
                model: model.clone(),
            },
            None,
        ),
        SubscriptionFee::Recurring {
            rate,
            quantity,
            billing_type,
            model,
        } => (
            FeeStructure::ExtraRecurring {
                billing_type: billing_type.clone(),
            },
            Pricing::ExtraRecurring {
                unit_price: *rate,
                quantity: *quantity,
                model: model.clone(),
            },
            None,
        ),
        SubscriptionFee::Credit {
            rate,
            quantity,
            credit_amount,
            expiry,
            credit_unit,
            rollover,
            once,
        } => (
            FeeStructure::Credit {},
            Pricing::Credit {
                unit_price: *rate,
                quantity: *quantity,
                credit_amount: *credit_amount,
                expiry: *expiry,
                credit_unit_id: credit_unit.as_ref().map(|unit| unit.unit_id),
                rollover: *rollover,
                once: *once,
            },
            None,
        ),
        SubscriptionFee::Capacity {
            rate,
            included,
            overage_rate,
            metric_id,
            credit_unit,
        } => (
            FeeStructure::Capacity {
                metric_id: *metric_id,
            },
            Pricing::Capacity {
                rate: *rate,
                included: *included,
                overage_rate: *overage_rate,
            },
            credit_unit.as_ref().map(|unit| PriceCreditUnit {
                unit_id: unit.unit_id,
                rate: unit.rate,
            }),
        ),
        SubscriptionFee::Slot {
            unit,
            quantity_id,
            model,
            min_slots,
            max_slots,
            ..
        } => (
            FeeStructure::Slot {
                unit_name: unit.clone(),
                quantity_id: *quantity_id,
                upgrade_policy: UpgradePolicy::Prorated,
                downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
            },
            Pricing::Slot {
                model: model.clone(),
                min_slots: *min_slots,
                max_slots: *max_slots,
            },
            None,
        ),
        SubscriptionFee::Usage {
            metric_id,
            model,
            credit_unit,
        } => (
            FeeStructure::Usage {
                metric_id: *metric_id,
                model: UsageModel::from(model),
            },
            Pricing::Usage(model.clone()),
            credit_unit.as_ref().map(|unit| PriceCreditUnit {
                unit_id: unit.unit_id,
                rate: unit.rate,
            }),
        ),
    }
}

/// A subscription's fee as the stored charge `charges::render` reads: its one price in every
/// option (a subscription has none), in the subscription's currency, on the fee's cadence (the
/// subscription's for a one-time fee).
pub fn fee_charge(
    name: &str,
    product_id: ProductId,
    fee: &SubscriptionFee,
    period: SubscriptionFeeBillingPeriod,
    subscription_period: BillingPeriodEnum,
    currency: &str,
) -> StoredCharge {
    let (structure, pricing, credit_unit) = structure_of(fee);
    StoredCharge {
        name: name.to_owned(),
        product_id,
        structure: Some(structure),
        prices: vec![StoredPrice {
            cadence: period
                .as_billing_period_opt()
                .unwrap_or(subscription_period),
            currency: currency.to_owned(),
            pricing,
            credit_unit,
            options: OptionSet::Every,
        }],
    }
}

/// A charge of the subscription before it is rendered.
pub struct ChargeSource {
    pub origin: ChargeOrigin,
    pub add_on: Option<AddOnId>,
    pub stored: StoredCharge,
}

fn without_product(path: &str) -> Problem {
    Problem::not_expressible(path, "a charge without a product: read it with API v1")
}

/// The subscription's charges, plan and amendment charges first (in the order they were added),
/// then its add-ons'.
pub fn charge_sources(details: &SubscriptionDetails) -> Result<Vec<ChargeSource>, Problem> {
    let sub = &details.subscription;
    let mut components: Vec<_> = details.price_components.iter().collect();
    components.sort_by_key(|component| component.id.as_uuid());
    let mut add_ons: Vec<_> = details.add_ons.iter().collect();
    add_ons.sort_by_key(|add_on| add_on.id.as_uuid());
    let mut sources = Vec::with_capacity(components.len() + add_ons.len());
    for component in components {
        let path = format!("/charges/{}", sources.len());
        let product_id = component.product_id.ok_or_else(|| without_product(&path))?;
        sources.push(ChargeSource {
            origin: if component.price_component_id.is_some() && !component.added_by_amendment {
                ChargeOrigin::Plan
            } else {
                ChargeOrigin::Amendment
            },
            add_on: None,
            stored: fee_charge(
                &component.name,
                product_id,
                &component.fee,
                component.period,
                sub.period,
                &sub.currency,
            ),
        });
    }
    for add_on in add_ons {
        let path = format!("/charges/{}", sources.len());
        let product_id = add_on.product_id.ok_or_else(|| without_product(&path))?;
        sources.push(ChargeSource {
            origin: ChargeOrigin::AddOn,
            add_on: Some(add_on.add_on_id),
            stored: fee_charge(
                &add_on.name,
                product_id,
                &add_on.fee,
                add_on.period,
                sub.period,
                &sub.currency,
            ),
        });
    }
    Ok(sources)
}

/// The charges of `sources`, their references named by code.
pub async fn render_charges(
    state: &AppState,
    tenant_id: TenantId,
    sources: &[ChargeSource],
    add_on_codes: &HashMap<AddOnId, String>,
) -> Result<Vec<SubscriptionCharge>, Problem> {
    let names = CatalogNames::load(state, tenant_id, sources.iter().map(|s| &s.stored)).await?;
    sources
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let charge = render(&source.stored, &[], &names, &format!("/charges/{index}"))?;
            Ok(SubscriptionCharge {
                origin: source.origin,
                add_on: source.add_on.map(|id| add_on_ref(add_on_codes, id)),
                charge,
            })
        })
        .collect()
}

/// The add-on of `id`, by the code `codes` holds of it.
pub fn add_on_ref(codes: &HashMap<AddOnId, String>, id: AddOnId) -> AddOnRef {
    AddOnRef {
        id,
        code: codes.get(&id).cloned().unwrap_or_else(|| id.to_string()),
    }
}

pub async fn add_on_codes(
    state: &AppState,
    tenant_id: TenantId,
    ids: Vec<AddOnId>,
) -> Result<HashMap<AddOnId, String>, Problem> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(state
        .store
        .list_add_ons_by_ids(tenant_id, ids)
        .await?
        .into_iter()
        .map(|add_on| (add_on.id, add_on.code))
        .collect())
}

// ── Quantities ──────────────────────────────────────────────────────

/// The managed quantities the subscription's seat charges count.
pub fn held_quantities(details: &SubscriptionDetails) -> Vec<ManagedQuantityId> {
    let mut ids: Vec<ManagedQuantityId> = details
        .price_components
        .iter()
        .map(|component| &component.fee)
        .chain(details.add_ons.iter().map(|add_on| &add_on.fee))
        .filter_map(|fee| match fee {
            SubscriptionFee::Slot {
                quantity_id: Some(id),
                ..
            } => Some(*id),
            _ => None,
        })
        .collect();
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();
    ids
}

pub async fn quantity_codes(
    state: &AppState,
    tenant_id: TenantId,
) -> Result<HashMap<ManagedQuantityId, String>, Problem> {
    Ok(state
        .store
        .list_managed_quantities(tenant_id)
        .await?
        .into_iter()
        .map(|quantity| (quantity.id, quantity.code))
        .collect())
}

/// The count of `quantity` the subscription holds now, and the counts to come: a decrease waiting
/// for the period's end, an increase waiting for its payment or the next period.
pub async fn quantity_state(
    state: &AppState,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    quantity: ManagedQuantityId,
) -> Result<QuantityState, Problem> {
    let held = state
        .store
        .get_active_slots_value(tenant_id, subscription_id, quantity, None)
        .await?;
    let now = common_clock::now().naive_utc();
    let mut waiting: Vec<SlotTransaction> = state
        .store
        .list_slot_transactions(tenant_id, subscription_id, Some(quantity), None)
        .await?
        .into_iter()
        .map(SlotTransaction::from)
        .filter(|tx| tx.status == SlotTransactionStatusEnum::Pending || tx.effective_at > now)
        .collect();
    waiting.sort_by_key(|tx| tx.effective_at);
    let mut count = i64::from(held);
    let pending = waiting
        .into_iter()
        .map(|tx| {
            count += i64::from(tx.delta);
            PendingQuantity {
                value: Decimal::from(count.max(0)),
                effective_at: tx.effective_at.into(),
            }
        })
        .collect();
    Ok(QuantityState {
        held: Decimal::from(held),
        pending,
    })
}

// ── The subscription ────────────────────────────────────────────────

/// What the subscription's own row reads as; `plan` names it.
pub fn base(
    sub: &DomainSubscription,
    external_id: Option<String>,
    updated_at: DateTime<Utc>,
    plan: PlanRef,
    resource: Option<ResourceRef>,
) -> Subscription {
    Subscription {
        id: sub.id,
        external_id,
        customer: CustomerRef {
            id: sub.customer_id,
            alias: sub.customer_alias.clone(),
            name: sub.customer_name.clone(),
        },
        plan,
        status: sub.status.clone().into(),
        currency: sub.currency.clone(),
        period: sub.period.into(),
        activation: sub.activation_condition.clone().into(),
        billing_day_anchor: sub.billing_day_anchor,
        dates: Dates {
            start: sub.start_date,
            end: sub.end_date,
            billing_start: sub.billing_start_date,
            activated_at: sub.activated_at.map(Into::into),
            ended_at: sub.terminated_at.map(Into::into),
        },
        current_period: CurrentPeriod {
            start: sub.current_period_start,
            end: sub.current_period_end,
        },
        trial: sub
            .trial_duration
            .filter(|days| *days > 0)
            .map(|days| TrialState {
                days,
                free: None,
                ends_on: None,
            }),
        quantities: None,
        levels: None,
        charges: None,
        add_ons: None,
        coupons: None,
        schedule: None,
        cancellation: None,
        minimum: None,
        mrr: Money {
            amount: i64::try_from(sub.mrr_cents).unwrap_or(i64::MAX),
            currency: sub.currency.clone(),
        },
        tax_inclusive: sub.tax_inclusive,
        resource,
        entitlement_overrides: None,
        warnings: Vec::new(),
        metrics_not_attributed: Vec::new(),
        invoicing: Invoicing {
            net_terms: sub.net_terms,
            purchase_order: sub.purchase_order.clone(),
            memo: sub.invoice_memo.clone(),
            auto_finalize: sub.auto_advance_invoices,
            charge_automatically: sub.charge_automatically,
        },
        custom_properties: sub.custom_properties.clone(),
        created_at: sub.created_at.into(),
        updated_at: updated_at.into(),
    }
}

/// The subscription's resource as read; by its id alone when it was deleted since (a
/// subscription keeps naming it).
fn resource_of(
    sub: &DomainSubscription,
    resources: &HashMap<ResourceId, ResourceRef>,
) -> Option<ResourceRef> {
    let id = sub.resource_id?;
    Some(
        resources
            .get(&id)
            .cloned()
            .unwrap_or_else(|| ResourceRef::of_id(id)),
    )
}

/// The subscription's own open rows, canonical: by feature code, level, resource type and
/// condition.
pub async fn entitlement_overrides(
    state: &AppState,
    tenant_id: TenantId,
    id: SubscriptionId,
) -> Result<Vec<Entitlement>, Problem> {
    let rows = state
        .store
        .list_entitlements_by_entity(EntitlementEntityId::Subscription(id), tenant_id)
        .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let codes = Codes::of_rows(state, tenant_id, &rows).await?;
    let mut read: Vec<Entitlement> = rows.into_iter().map(|row| codes.entitlement(row)).collect();
    read.sort_by_cached_key(|row| {
        (
            row.feature.clone(),
            serde_json::to_string(&row.level).unwrap_or_default(),
            row.resource_type.clone(),
            serde_json::to_string(&row.condition).unwrap_or_default(),
        )
    });
    Ok(read)
}

pub async fn plan_code(
    state: &AppState,
    tenant_id: TenantId,
    plan_id: PlanId,
) -> Result<String, Problem> {
    Ok(state
        .store
        .plan_codes(tenant_id, &[plan_id])
        .await?
        .remove(&plan_id)
        .unwrap_or_else(|| plan_id.to_string()))
}

pub fn version_number(version: u32) -> i32 {
    i32::try_from(version).unwrap_or(i32::MAX)
}

/// A page of subscriptions, as lists read them.
pub async fn summaries(
    state: &AppState,
    tenant_id: TenantId,
    listed: Vec<ListedSubscription>,
) -> Result<Vec<Subscription>, Problem> {
    let mut plan_ids: Vec<PlanId> = listed.iter().map(|l| l.subscription.plan_id).collect();
    plan_ids.sort_by_key(|id| id.as_uuid());
    plan_ids.dedup();
    let codes = state.store.plan_codes(tenant_id, &plan_ids).await?;
    let resources = resource_refs(
        state,
        tenant_id,
        listed.iter().filter_map(|l| l.subscription.resource_id),
    )
    .await?;
    let mut options: HashMap<PlanVersionId, HashMap<PlanOptionId, String>> = HashMap::new();
    for item in &listed {
        let version = item.subscription.plan_version_id;
        if item.subscription.plan_option_id.is_some() && !options.contains_key(&version) {
            let aliases = state
                .store
                .list_plan_options(tenant_id, version)
                .await?
                .into_iter()
                .map(|option| (option.id, option.alias))
                .collect();
            options.insert(version, aliases);
        }
    }
    Ok(listed
        .into_iter()
        .map(|item| {
            let sub = &item.subscription;
            let plan = PlanRef {
                id: sub.plan_id,
                code: codes
                    .get(&sub.plan_id)
                    .cloned()
                    .unwrap_or_else(|| sub.plan_id.to_string()),
                version: Some(version_number(sub.version)),
                option: sub.plan_option_id.and_then(|option| {
                    options
                        .get(&sub.plan_version_id)
                        .and_then(|aliases| aliases.get(&option).cloned())
                }),
            };
            let resource = resource_of(sub, &resources);
            base(
                sub,
                item.external_id.clone(),
                item.updated_at,
                plan,
                resource,
            )
        })
        .collect())
}

/// The subscription as lists read it: its own fields, nothing read beside.
pub async fn summary(
    state: &AppState,
    tenant_id: TenantId,
    id: SubscriptionId,
) -> Result<Subscription, Problem> {
    let listed = state.store.get_listed_subscription(tenant_id, id).await?;
    summaries(state, tenant_id, vec![listed])
        .await?
        .pop()
        .ok_or_else(Problem::internal)
}

/// The subscription, read whole.
pub async fn subscription(
    state: &AppState,
    tenant_id: TenantId,
    id: SubscriptionId,
) -> Result<Subscription, Problem> {
    let listed = state.store.get_listed_subscription(tenant_id, id).await?;
    let details = state.store.get_subscription_details(tenant_id, id).await?;
    detailed(state, tenant_id, listed, &details).await
}

pub async fn detailed(
    state: &AppState,
    tenant_id: TenantId,
    listed: ListedSubscription,
    details: &SubscriptionDetails,
) -> Result<Subscription, Problem> {
    let level_products = level_products(state, tenant_id, details).await?;
    let warnings = state
        .store
        .get_subscription_warnings(tenant_id, details.subscription.id)
        .await?;
    let mut out = read_whole(state, tenant_id, listed, details, &level_products).await?;
    out.warnings = warnings.codes;
    out.metrics_not_attributed = warnings
        .metrics_not_attributed
        .into_iter()
        .map(|m| m.catalog_code)
        .collect();
    Ok(out)
}

/// The products the subscription's usage-levels charges bill; none when its levels cannot be
/// read (a read never fails on them).
async fn level_products(
    state: &AppState,
    tenant_id: TenantId,
    details: &SubscriptionDetails,
) -> Result<HashSet<ProductId>, Problem> {
    let levels = match state
        .services
        .list_capacity_levels(tenant_id, details.subscription.id)
        .await
    {
        Ok(levels) => levels,
        Err(error) => {
            log::warn!(
                "The levels of subscription {} were not read: {error:?}",
                details.subscription.id
            );
            Vec::new()
        }
    };
    Ok(levels
        .iter()
        .filter_map(|levels| {
            details
                .price_components
                .iter()
                .find(|component| component.id == levels.subscription_component_id)
                .and_then(|component| component.product_id)
        })
        .collect())
}

/// The level of each usage-levels charge among `sources` (those billing `level_products`), by
/// the product code its charge reads.
fn levels_of(
    sources: &[ChargeSource],
    charges: &[SubscriptionCharge],
    level_products: &HashSet<ProductId>,
) -> BTreeMap<String, String> {
    sources
        .iter()
        .zip(charges)
        .filter(|(source, _)| {
            source.origin != ChargeOrigin::AddOn
                && level_products.contains(&source.stored.product_id)
        })
        .filter_map(|(source, charge)| match source.stored.prices.first() {
            Some(StoredPrice {
                pricing: Pricing::Capacity { included, .. },
                ..
            }) => Some((charge.charge.product.clone(), included.to_string())),
            _ => None,
        })
        .collect()
}

/// `details` read whole, its usage-levels charges those billing `level_products`.
async fn read_whole(
    state: &AppState,
    tenant_id: TenantId,
    listed: ListedSubscription,
    details: &SubscriptionDetails,
    level_products: &HashSet<ProductId>,
) -> Result<Subscription, Problem> {
    let sub = &details.subscription;
    let plan = PlanRef {
        id: sub.plan_id,
        code: plan_code(state, tenant_id, sub.plan_id).await?,
        version: Some(version_number(sub.version)),
        option: details.plan_option.as_ref().map(|o| o.alias.clone()),
    };
    let resources = resource_refs(state, tenant_id, sub.resource_id).await?;
    let resource = resource_of(sub, &resources);
    let mut out = base(sub, listed.external_id, listed.updated_at, plan, resource);
    out.entitlement_overrides = Some(entitlement_overrides(state, tenant_id, sub.id).await?);
    if let Some(trial) = &details.trial_config {
        out.trial = Some(TrialState {
            days: trial.duration_days,
            free: Some(trial.is_free),
            ends_on: trial.ends_on,
        });
    }

    let mut add_on_ids: Vec<AddOnId> = details.add_ons.iter().map(|a| a.add_on_id).collect();
    add_on_ids.sort_by_key(|id| id.as_uuid());
    add_on_ids.dedup();
    let add_on_codes = add_on_codes(state, tenant_id, add_on_ids).await?;
    let sources = charge_sources(details)?;
    let charges = render_charges(state, tenant_id, &sources, &add_on_codes).await?;
    out.levels = Some(levels_of(&sources, &charges, level_products));
    out.charges = Some(charges);
    let mut add_ons: Vec<_> = details.add_ons.iter().collect();
    add_ons.sort_by_key(|add_on| add_on.id.as_uuid());
    out.add_ons = Some(
        add_ons
            .into_iter()
            .map(|add_on| SubscribedAddOn {
                add_on: add_on_ref(&add_on_codes, add_on.add_on_id),
                name: add_on.name.clone(),
                quantity: add_on.quantity,
            })
            .collect(),
    );

    let held = held_quantities(details);
    let mut quantities = BTreeMap::new();
    if !held.is_empty() {
        let codes = quantity_codes(state, tenant_id).await?;
        for quantity in held {
            let code = codes
                .get(&quantity)
                .cloned()
                .unwrap_or_else(|| quantity.to_string());
            quantities.insert(
                code,
                quantity_state(state, tenant_id, sub.id, quantity).await?,
            );
        }
    }
    out.quantities = Some(quantities);

    out.coupons = Some(
        details
            .applied_coupons
            .iter()
            .map(|applied| AppliedCoupon {
                coupon: CouponRef {
                    id: applied.coupon.id,
                    code: applied.coupon.code.clone(),
                },
                active: applied.applied_coupon.is_active,
                applied_count: applied.applied_coupon.applied_count,
            })
            .collect(),
    );
    out.schedule = Some(schedule(state, tenant_id, &details.pending_events).await?);
    out.cancellation = cancellation(&details.pending_events);
    out.minimum = minimum(state, tenant_id, details).await?;
    Ok(out)
}

/// What a change sets beyond the charges and add-ons it leaves.
#[derive(Default)]
pub struct AfterChange {
    /// The plan moved to.
    pub plan: Option<PlanRef>,
    pub status: Option<SubscriptionStatus>,
    /// In minor units.
    pub mrr: Option<i64>,
    /// The counts held once it applies, by managed quantity code.
    pub quantities: BTreeMap<String, Decimal>,
    /// The products it sells in levels beyond those the subscription does (a new plan's).
    pub level_products: HashSet<ProductId>,
    pub cancellation: Option<Cancellation>,
}

/// The subscription as it would read once a change is in force: `after`, the details the change
/// leaves (its charges, add-ons and minimum, from the store's preview), read as a subscription is
/// read, with what `change` sets.
pub async fn after_change(
    state: &AppState,
    tenant_id: TenantId,
    current: &SubscriptionDetails,
    after: &SubscriptionDetails,
    change: AfterChange,
) -> Result<Subscription, Problem> {
    let listed = state
        .store
        .get_listed_subscription(tenant_id, current.subscription.id)
        .await?;
    let mut level_products = level_products(state, tenant_id, current).await?;
    level_products.extend(change.level_products);
    let mut out = read_whole(state, tenant_id, listed, after, &level_products).await?;
    if let Some(plan) = change.plan {
        out.plan = plan;
    }
    if let Some(status) = change.status {
        out.status = status;
    }
    if let Some(mrr) = change.mrr {
        out.mrr.amount = mrr;
    }
    let quantities = out.quantities.get_or_insert_with(BTreeMap::new);
    for (code, held) in change.quantities {
        quantities
            .entry(code)
            .and_modify(|state| state.held = held)
            .or_insert_with(|| QuantityState {
                held,
                pending: Vec::new(),
            });
    }
    if change.cancellation.is_some() {
        out.cancellation = change.cancellation;
    }
    Ok(out)
}

/// The changes waiting for their date, soonest first.
pub async fn schedule(
    state: &AppState,
    tenant_id: TenantId,
    events: &[PendingScheduledEvent],
) -> Result<Vec<ScheduledChange>, Problem> {
    let mut events: Vec<&PendingScheduledEvent> = events.iter().collect();
    events.sort_by_key(|event| event.scheduled_date);
    let mut changes = Vec::with_capacity(events.len());
    for event in events {
        let (kind, plan, amendment) = match event.event_type {
            ScheduledEventTypeEnum::ApplyPlanChange => {
                let plan = match event.new_plan_version_id {
                    Some(version_id) => {
                        let target = state
                            .store
                            .get_plan_by_version_id(version_id, tenant_id)
                            .await?;
                        Some(PlanRef {
                            id: target.plan.id,
                            code: target.plan.code,
                            version: Some(target.version.map(|v| v.version).unwrap_or_default()),
                            option: None,
                        })
                    }
                    None => None,
                };
                (ScheduledChangeType::PlanChange, plan, None)
            }
            ScheduledEventTypeEnum::CancelSubscription => {
                (ScheduledChangeType::Cancellation, None, None)
            }
            ScheduledEventTypeEnum::ApplyAmendment => (
                ScheduledChangeType::Amendment,
                None,
                event
                    .amendment_summary
                    .as_ref()
                    .map(|summary| AmendmentSummary {
                        added_charges: summary.added_component_names.clone(),
                        removed_charges: summary.removed_component_names.clone(),
                        added_add_ons: summary.added_add_on_names.clone(),
                        removed_add_ons: summary.removed_add_on_names.clone(),
                    }),
            ),
        };
        changes.push(ScheduledChange {
            id: event.id,
            kind,
            effective_date: event.scheduled_date,
            plan,
            reason: event.cancel_reason.clone(),
            initiated_by_customer: event.customer_initiated,
            amendment,
        });
    }
    Ok(changes)
}

/// The cancellation waiting for its date.
pub fn cancellation(events: &[PendingScheduledEvent]) -> Option<Cancellation> {
    events
        .iter()
        .filter(|event| event.event_type == ScheduledEventTypeEnum::CancelSubscription)
        .min_by_key(|event| event.scheduled_date)
        .map(|event| Cancellation {
            effective_date: event.scheduled_date,
            reason: event.cancel_reason.clone(),
            initiated_by_customer: event.customer_initiated,
        })
}

async fn minimum(
    state: &AppState,
    tenant_id: TenantId,
    details: &SubscriptionDetails,
) -> Result<Option<Minimum>, Problem> {
    let Some(minimum) = &details.minimum else {
        return Ok(None);
    };
    let products = match &minimum.scope {
        MinimumScope::AllComponents => None,
        MinimumScope::Products(ids) => {
            let codes: HashMap<ProductId, String> = if ids.is_empty() {
                HashMap::new()
            } else {
                state
                    .store
                    .find_products_by_ids(ids, tenant_id)
                    .await?
                    .into_iter()
                    .map(|product| (product.id, product.code))
                    .collect()
            };
            Some(
                ids.iter()
                    .map(|id| codes.get(id).cloned().unwrap_or_else(|| id.to_string()))
                    .collect(),
            )
        }
    };
    Ok(Some(Minimum {
        amount: minimum.amount_subunit,
        currency: Some(details.subscription.currency.clone()),
        products,
    }))
}

// ── Documents ───────────────────────────────────────────────────────

/// A document a change issues, its lines rendered as an invoice's (`documents::lines`).
pub fn document(
    kind: DocumentType,
    content: &ComputedInvoiceContent,
    currency: &str,
    products: &ProductCodes,
) -> Document {
    Document {
        kind,
        currency: currency.to_owned(),
        subtotal: Some(content.subtotal),
        discount: Some(content.discount),
        tax: Some(content.tax_amount),
        total: content.total,
        amount_due: Some(content.amount_due),
        lines: documents::lines(&content.invoice_lines, products),
        taxes: documents::taxes(&content.tax_breakdown),
        coupons: documents::coupons(&content.applied_coupons),
    }
}

/// The documents a change issues, their lines' products named by code.
pub async fn change_documents(
    state: &AppState,
    tenant_id: TenantId,
    contents: &[(DocumentType, &ComputedInvoiceContent)],
    currency: &str,
) -> Result<Vec<Document>, Problem> {
    let products = ProductCodes::load(
        state,
        tenant_id,
        contents
            .iter()
            .flat_map(|(_, content)| content.invoice_lines.iter()),
    )
    .await?;
    Ok(contents
        .iter()
        .map(|(kind, content)| document(*kind, content, currency, &products))
        .collect())
}

/// What an amendment (or a level move) issues now, and the next renewal under it.
pub async fn amendment_documents(
    state: &AppState,
    tenant_id: TenantId,
    preview: &AmendmentPreviewExtended,
    currency: &str,
) -> Result<Vec<Document>, Problem> {
    let mut contents = Vec::new();
    if let Some(invoice) = &preview.adjustment_invoice {
        contents.push((DocumentType::Invoice, invoice));
    }
    if let Some(note) = &preview.credit_note {
        contents.push((DocumentType::CreditNote, note));
    }
    if let Some(next) = &preview.next_invoice {
        contents.push((DocumentType::NextInvoice, next));
    }
    change_documents(state, tenant_id, &contents, currency).await
}

/// What a plan change issues when it applies (its adjustment, or the new plan's opening invoice
/// when it ends a free trial, and its credit), and the renewal under it.
pub async fn plan_change_documents(
    state: &AppState,
    tenant_id: TenantId,
    outcome: &PlanChangeOutcome,
    currency: &str,
) -> Result<Vec<Document>, Problem> {
    let mut contents = Vec::new();
    if let Some(invoice) = &outcome.invoice {
        contents.push((DocumentType::Invoice, invoice));
    }
    if let Some(note) = &outcome.credit_note {
        contents.push((DocumentType::CreditNote, note));
    }
    if let Some(next) = &outcome.next_invoice {
        contents.push((DocumentType::NextInvoice, next));
    }
    change_documents(state, tenant_id, &contents, currency).await
}

/// What a cancellation issues: the final invoice of the period it ends and the credit for the
/// time billed in advance after it.
pub async fn cancellation_documents(
    state: &AppState,
    tenant_id: TenantId,
    preview: &CancellationPreview,
    currency: &str,
) -> Result<Vec<Document>, Problem> {
    let mut contents = Vec::new();
    if let Some(invoice) = &preview.final_invoice {
        contents.push((DocumentType::Invoice, invoice));
    }
    if let Some(note) = &preview.credit_note {
        contents.push((DocumentType::CreditNote, note));
    }
    change_documents(state, tenant_id, &contents, currency).await
}
