//! Resolution (ADR 0011 §3.2, §3.3, §3.7): every level on the check's path, each decided by one
//! subscription and folded alone. One batched query set loads the customer's rows.
use crate::domain::BillableMetric;
use crate::domain::UsagePeriod;
use crate::domain::deciding_subscription::{Candidate, Deciding, deciding};
use crate::domain::entitlement_checks::{
    DecidingCandidate, FeatureScope, LevelUsage, LevelValue, LevelWindow, ResolvedCustomer,
    ResolvedFeature, ResolvedLevel, Uncountable,
};
use crate::domain::entitlement_etag::next_change;
use crate::domain::entitlement_fold::{EvalLevel, FoldRow, Folded, fold};
use crate::domain::entitlement_levels::{levels_on_path, resource_required};
use crate::domain::entitlements::{Entitlement, Feature, FeatureKind, LimitCountedBy};
use crate::domain::enums::FeatureStatusEnum;
use crate::domain::managed_quantities::quantity_of_unit;
use crate::domain::prices::FeeStructure;
use crate::domain::resources::{
    Attributes, CUSTOMER_PATH, ResolvedResource, ResourceInput, ResourceType, canonical_path,
};
use crate::domain::usage_attribution::resource_scope;
use crate::errors::StoreError;
use crate::repositories::resources::load_for_check;
use crate::services::entitlements::limits::load_metrics;
use crate::services::entitlements::limits::rules::{BillingCyclePeriod, compute_usage_period};
use crate::services::subscriptions::effective_plan::effective_plan_versions;
use crate::store::PgConn;
use crate::{Store, StoreResult};
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{
    AddOnId, AliasOr, BillableMetricId, CustomerId, EntitlementEntityId, FeatureId,
    ManagedQuantityId, PlanId, PlanVersionId, ProductId, ResourceId, ResourceTypeId,
    SubscriptionId, TenantId,
};
use diesel_models::customers::CustomerRow;
use diesel_models::entitlement_app_counts::EntitlementAppCountRow;
use diesel_models::entitlements::{EntitlementRow, FeatureRow};
use diesel_models::managed_quantities::ManagedQuantityRow;
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::query::entitlements::{
    add_on_fee_structures, included_products_of_subscriptions,
};
use diesel_models::query::managed_quantities::feature_limit_quantities;
use diesel_models::query::portal_credit::list_active_slots;
use diesel_models::query::subscription_facts::read_day;
use diesel_models::resources::{ResourceRow, ResourceTypeKeyRow, ResourceTypeRow};
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use itertools::Itertools;
use opentelemetry::metrics::Counter;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// A customer subscription with its rows on the plan, plan version, add-on and subscription tiers.
pub(crate) struct SubscriptionRows {
    pub row: SubscriptionRow,
    /// The stored resource it is for, with its type and path.
    pub resource: Option<(ResourceTypeId, ResourceId, String)>,
    /// Its components' and add-ons' products, and those its plan includes without a price.
    pub products: HashSet<ProductId>,
    /// What multiplies each add-on's rows: its instances, or a seat add-on's seats held.
    pub quantities: HashMap<AddOnId, u32>,
    /// The units held of each managed quantity a limit counts per unit of (R-133).
    pub units: HashMap<ManagedQuantityId, u32>,
    pub rows: Vec<Entitlement>,
}

impl SubscriptionRows {
    /// The units of `per_unit` it holds, which multiply its grant of a limit counted per unit;
    /// none for a limit counted per subscription.
    fn units_for(&self, per_unit: Option<ManagedQuantityId>) -> Option<u32> {
        per_unit.map(|quantity| self.units.get(&quantity).copied().unwrap_or(0))
    }
}

/// Everything resolution reads, loaded in a constant number of queries.
pub(crate) struct CustomerRows {
    pub features: Vec<Feature>,
    pub defaults: Vec<Entitlement>,
    /// Customer rows, resource overrides included.
    pub customer: Vec<Entitlement>,
    pub subscriptions: Vec<SubscriptionRows>,
    /// The managed quantity each limit counting per unit counts (R-133).
    pub per_unit: HashMap<FeatureId, ManagedQuantityId>,
    /// What a window with no activation to anchor on falls back to.
    pub customer_created_at: DateTime<Utc>,
}

/// The managed quantity each seat add-on among `add_on_ids` counts (R-138). Its seats held
/// multiply its rows, not its instances: a seat add-on is held once.
pub(crate) async fn seat_quantities(
    conn: &mut PgConn,
    tenant_id: TenantId,
    add_on_ids: &[AddOnId],
) -> StoreResult<HashMap<AddOnId, ManagedQuantityId>> {
    let seats: Vec<(AddOnId, Option<ManagedQuantityId>, String)> =
        add_on_fee_structures(conn, tenant_id, add_on_ids)
            .await?
            .into_iter()
            .filter_map(
                |(id, structure)| match serde_json::from_value::<FeeStructure>(structure) {
                    Ok(FeeStructure::Slot {
                        quantity_id,
                        unit_name,
                        ..
                    }) => Some((id, quantity_id, unit_name)),
                    _ => None,
                },
            )
            .collect();
    // A structure stored before quantities were named counts the one its unit names.
    let quantities = if seats.iter().any(|(_, quantity, _)| quantity.is_none()) {
        ManagedQuantityRow::list(conn, tenant_id).await?
    } else {
        vec![]
    };
    Ok(seats
        .into_iter()
        .filter_map(|(add_on, quantity, unit)| {
            let quantity =
                quantity.or_else(|| quantity_of_unit(&quantities, &unit).map(|q| q.id))?;
            Some((add_on, quantity))
        })
        .collect())
}

/// The units each subscription holds of each quantity, at `now`, from the seat ledger; a pair
/// without a transaction holds none.
pub(crate) async fn held_units(
    conn: &mut PgConn,
    tenant_id: TenantId,
    pairs: &[(SubscriptionId, ManagedQuantityId)],
    now: DateTime<Utc>,
) -> StoreResult<HashMap<(SubscriptionId, ManagedQuantityId), u32>> {
    Ok(list_active_slots(conn, tenant_id, pairs, now.naive_utc())
        .await?
        .into_iter()
        .map(|row| {
            (
                (row.subscription_id, row.quantity_id),
                u32::try_from(row.active_slots).unwrap_or(0),
            )
        })
        .collect())
}

/// Each add-on's multiplier on one subscription: the sum of its instances in force, or for a
/// seat add-on the seats it holds (R-137).
pub(crate) fn add_on_quantities<'a>(
    subscription_id: SubscriptionId,
    add_ons: impl IntoIterator<Item = &'a SubscriptionAddOnRow>,
    seats: &HashMap<AddOnId, ManagedQuantityId>,
    held: &HashMap<(SubscriptionId, ManagedQuantityId), u32>,
) -> HashMap<AddOnId, u32> {
    let mut quantities: HashMap<AddOnId, u32> = HashMap::new();
    for a in add_ons {
        match seats.get(&a.add_on_id) {
            Some(quantity) => {
                let seats_held = held
                    .get(&(subscription_id, *quantity))
                    .copied()
                    .unwrap_or(0);
                quantities.insert(a.add_on_id, seats_held);
            }
            None => *quantities.entry(a.add_on_id).or_default() += a.quantity.max(0) as u32,
        }
    }
    quantities
}

impl CustomerRows {
    pub(crate) async fn load(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        features: Option<&[FeatureId]>,
        now: DateTime<Utc>,
    ) -> StoreResult<Self> {
        let today = now.date_naive();
        let customer_created_at =
            CustomerRow::find_by_id_or_alias(conn, tenant_id, AliasOr::Id(customer_id))
                .await?
                .created_at
                .and_utc();
        let subs = SubscriptionRow::find_active_by_customer(conn, customer_id, tenant_id).await?;
        let sub_ids: Vec<SubscriptionId> = subs.iter().map(|s| s.id).collect();
        // The plan tier reads the plan in effect: a trial's trialing plan (ADR 0011 §3.9).
        let version_of = effective_plan_versions(conn, tenant_id, &subs).await?;
        let version_ids: Vec<PlanVersionId> = version_of.values().copied().unique().collect();
        let (plan_of, add_ons, components, included) = if subs.is_empty() {
            Default::default()
        } else {
            (
                PlanVersionRow::get_plan_ids_by_version_ids(conn, &version_ids).await?,
                SubscriptionAddOnRow::list_all_by_subscriptions(conn, tenant_id, &sub_ids).await?,
                SubscriptionComponentRow::list_all_by_subscriptions(conn, tenant_id, &sub_ids)
                    .await?,
                included_products_of_subscriptions(conn, tenant_id, &sub_ids).await?,
            )
        };
        let mut included = included.into_iter().into_group_map();
        // Add-ons and components in force on each subscription's read day.
        let read_days: HashMap<SubscriptionId, NaiveDate> = subs
            .iter()
            .map(|s| (s.id, read_day(today, s.current_period_end)))
            .collect();
        let in_force =
            |subscription_id: &SubscriptionId, from: NaiveDate, to: Option<NaiveDate>| {
                let day = read_days.get(subscription_id).copied().unwrap_or(today);
                from <= day && to.is_none_or(|to| to > day)
            };
        let add_ons: Vec<SubscriptionAddOnRow> = add_ons
            .into_iter()
            .filter(|a| in_force(&a.subscription_id, a.effective_from, a.effective_to))
            .collect();
        let mut components = components
            .into_iter()
            .filter(|c| in_force(&c.subscription_id, c.effective_from, c.effective_to))
            .into_group_map_by(|c| c.subscription_id);

        let resource_ids: Vec<ResourceId> =
            subs.iter().filter_map(|s| s.resource_id).unique().collect();
        let mut resources: HashMap<ResourceId, (ResourceTypeId, String)> = HashMap::new();
        if !resource_ids.is_empty() {
            let codes: HashMap<ResourceTypeId, String> =
                ResourceTypeRow::list(conn, tenant_id, true)
                    .await?
                    .into_iter()
                    .map(|t| (t.id, t.code))
                    .collect();
            for r in ResourceRow::find_by_ids(conn, tenant_id, &resource_ids).await? {
                let code = codes.get(&r.resource_type_id).map_or("", String::as_str);
                resources.insert(
                    r.id,
                    (r.resource_type_id, canonical_path(code, &r.key_value)),
                );
            }
        }

        let mut owners = vec![EntitlementEntityId::Customer(customer_id)];
        owners.extend(
            sub_ids
                .iter()
                .copied()
                .map(EntitlementEntityId::Subscription),
        );
        owners.extend(
            version_ids
                .iter()
                .copied()
                .map(EntitlementEntityId::PlanVersion),
        );
        owners.extend(
            plan_of
                .values()
                .unique()
                .copied()
                .map(EntitlementEntityId::Plan),
        );
        owners.extend(
            add_ons
                .iter()
                .map(|a| a.add_on_id)
                .unique()
                .map(EntitlementEntityId::AddOn),
        );
        let rows = EntitlementRow::list_open_for_owners(conn, tenant_id, &owners, features).await?;
        let defaults = EntitlementRow::list_feature_level_entitlements(
            conn,
            tenant_id,
            features.map(<[FeatureId]>::to_vec).as_ref(),
        )
        .await?;

        let feature_ids: Vec<FeatureId> = rows
            .iter()
            .chain(&defaults)
            .map(|r| r.feature_id)
            .unique()
            .collect();
        let features: Vec<Feature> = if feature_ids.is_empty() {
            vec![]
        } else {
            FeatureRow::find_by_ids(conn, tenant_id, &feature_ids)
                .await?
                .into_iter()
                .map(Feature::try_from)
                .filter_ok(|f| f.status == FeatureStatusEnum::Active)
                .collect::<StoreResult<_>>()?
        };
        let to_domain = |rows: Vec<EntitlementRow>| -> StoreResult<Vec<Entitlement>> {
            rows.into_iter().map(Entitlement::try_from).collect()
        };
        let (customer, owned): (Vec<Entitlement>, Vec<Entitlement>) = to_domain(rows)?
            .into_iter()
            .partition(|e| matches!(e.entity, EntitlementEntityId::Customer(_)));

        let limits: Vec<FeatureId> = features
            .iter()
            .filter(|f| matches!(f.kind, FeatureKind::Limit(_)))
            .map(|f| f.id)
            .collect();
        let per_unit = feature_limit_quantities(conn, tenant_id, &limits).await?;
        let add_on_ids: Vec<AddOnId> = add_ons.iter().map(|a| a.add_on_id).unique().collect();
        let seats = seat_quantities(conn, tenant_id, &add_on_ids).await?;
        let mut pairs: Vec<(SubscriptionId, ManagedQuantityId)> = Vec::new();
        for id in &sub_ids {
            pairs.extend(per_unit.values().map(|quantity| (*id, *quantity)));
        }
        pairs.extend(
            add_ons
                .iter()
                .filter_map(|a| Some((a.subscription_id, *seats.get(&a.add_on_id)?))),
        );
        let pairs: Vec<_> = pairs.into_iter().unique().collect();
        let held = held_units(conn, tenant_id, &pairs, now).await?;

        let subscriptions = subs
            .into_iter()
            .map(|row| {
                let version_id = version_of[&row.id];
                let plan_id: Option<PlanId> = plan_of.get(&version_id).copied();
                let mine: Vec<&SubscriptionAddOnRow> = add_ons
                    .iter()
                    .filter(|a| a.subscription_id == row.id)
                    .collect();
                let quantities = add_on_quantities(row.id, mine.iter().copied(), &seats, &held);
                let units = held
                    .iter()
                    .filter(|((subscription_id, _), _)| *subscription_id == row.id)
                    .map(|((_, quantity), units)| (*quantity, *units))
                    .collect();
                let products = components
                    .remove(&row.id)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|c| c.product_id)
                    .chain(mine.iter().filter_map(|a| a.product_id))
                    .chain(included.remove(&row.id).unwrap_or_default())
                    .collect();
                let rows = owned
                    .iter()
                    .filter(|e| match e.entity {
                        EntitlementEntityId::Subscription(id) => id == row.id,
                        EntitlementEntityId::PlanVersion(id) => id == version_id,
                        EntitlementEntityId::Plan(id) => Some(id) == plan_id,
                        EntitlementEntityId::AddOn(id) => quantities.contains_key(&id),
                        _ => false,
                    })
                    .cloned()
                    .collect();
                SubscriptionRows {
                    resource: row
                        .resource_id
                        .and_then(|id| resources.get(&id).map(|(t, path)| (*t, id, path.clone()))),
                    row,
                    products,
                    quantities,
                    units,
                    rows,
                }
            })
            .collect();

        Ok(Self {
            features,
            defaults: to_domain(defaults)?,
            customer,
            subscriptions,
            per_unit,
            customer_created_at,
        })
    }
}

/// The rows of one feature at one level, as the fold reads them.
struct LevelRows<'a> {
    feature: &'a Feature,
    level: EvalLevel,
    attributes: &'a Attributes,
    /// The quantity the feature's limit counts per unit of (R-133).
    per_unit: Option<ManagedQuantityId>,
}

impl LevelRows<'_> {
    fn at<'e>(
        &self,
        rows: impl IntoIterator<Item = &'e Entitlement>,
    ) -> impl Iterator<Item = &'e Entitlement> {
        let (feature, level) = (self.feature, self.level);
        // A row whose value doesn't fit the kind (a legacy mismatch) grants nothing.
        rows.into_iter().filter(move |e| {
            e.feature_id == feature.id
                && e.level.eval(feature.level_type_id) == level
                && feature.kind.check_value(&e.value).is_ok()
        })
    }

    /// `subscription`: the one whose grant the rows make, which multiplies them.
    fn fold_rows<'e>(
        &self,
        rows: impl IntoIterator<Item = &'e Entitlement>,
        subscription: Option<&SubscriptionRows>,
    ) -> Vec<FoldRow> {
        let units = subscription.and_then(|s| s.units_for(self.per_unit));
        self.at(rows)
            .map(|e| {
                let quantity = match (e.entity, subscription) {
                    (EntitlementEntityId::AddOn(id), Some(s)) => {
                        s.quantities.get(&id).copied().unwrap_or(0)
                    }
                    _ => 1,
                };
                e.fold_row(quantity, units, self.attributes)
            })
            .collect()
    }
}

fn product_of(feature: &Feature) -> Option<ProductId> {
    feature.product.as_ref().map(|p| p.id)
}

/// Customer rows apply everywhere, a resource override only on its resource.
fn reaches(e: &Entitlement, resource: Option<&ResolvedResource>) -> bool {
    e.resource_id
        .is_none_or(|id| resource.and_then(|r| r.reference.stored_id()) == Some(id))
}

fn level_path(level: EvalLevel, resource: Option<&ResolvedResource>) -> String {
    match (level, resource) {
        (EvalLevel::Type(t), Some(r)) if r.type_id == t => r.path.clone(),
        _ => CUSTOMER_PATH.to_owned(),
    }
}

pub(super) fn resolve_feature(
    feature: &Feature,
    data: &CustomerRows,
    resource: Option<&ResolvedResource>,
    now: DateTime<Utc>,
) -> Option<ResolvedFeature> {
    let product = product_of(feature);
    let default_reaches = |s: &SubscriptionRows| product.is_none_or(|p| s.products.contains(&p));
    let customer: Vec<&Entitlement> = data
        .customer
        .iter()
        .filter(|e| reaches(e, resource))
        .collect();
    let no_attributes = Attributes::new();
    let has_rows = |level: EvalLevel| {
        let rows = LevelRows {
            feature,
            level,
            attributes: &no_attributes,
            per_unit: None,
        };
        let defaults_reach = product.is_none() || data.subscriptions.iter().any(default_reaches);
        (defaults_reach && rows.at(&data.defaults).next().is_some())
            || rows.at(customer.iter().copied()).next().is_some()
            || data
                .subscriptions
                .iter()
                .any(|s| rows.at(&s.rows).next().is_some())
    };

    let own = feature.own_level();
    let path_type = resource.map(|r| r.type_id);
    // Without a resource, overrides on the own level still make it one a resource must answer.
    let override_only = resource.is_none()
        && LevelRows {
            feature,
            level: own,
            attributes: &no_attributes,
            per_unit: None,
        }
        .at(&data.customer)
        .next()
        .is_some();
    let own_has_rows = has_rows(own) || override_only;
    let mut levels: Vec<(EvalLevel, bool)> = levels_on_path(feature.level_type_id, path_type)
        .into_iter()
        .filter(|l| *l == own || has_rows(*l))
        .map(|l| (l, l == own))
        .collect();
    if !own_has_rows {
        levels.retain(|(l, _)| *l != own);
        match levels.first_mut() {
            Some((_, answering)) => *answering = true,
            None => return None,
        }
    }

    let levels: Vec<ResolvedLevel> = levels
        .into_iter()
        .map(|(level, answering)| {
            if level == own && resource_required(own, path_type, own_has_rows) {
                return unresolved(
                    feature,
                    level,
                    answering,
                    String::new(),
                    LevelValue::ResourceRequired,
                );
            }
            resolve_level(feature, data, &customer, resource, level, answering, now)
        })
        .collect();
    let per_resource = levels.iter().any(|l| {
        matches!(
            l.value,
            LevelValue::ResourceRequired | LevelValue::NotSubscribed
        )
    });
    Some(ResolvedFeature {
        feature: feature.clone(),
        levels,
        scope: if per_resource {
            FeatureScope::Resource
        } else {
            FeatureScope::Customer
        },
    })
}

fn unresolved(
    feature: &Feature,
    level: EvalLevel,
    own: bool,
    path: String,
    value: LevelValue,
) -> ResolvedLevel {
    ResolvedLevel {
        level,
        own,
        path,
        value,
        policy: feature.policy_or_default(),
        policy_from: None,
        deciding_subscription: None,
        resource_omitted: false,
        window: None,
        next_change: None,
        explain: vec![],
        candidates: vec![],
    }
}

fn resolve_level(
    feature: &Feature,
    data: &CustomerRows,
    customer: &[&Entitlement],
    resource: Option<&ResolvedResource>,
    level: EvalLevel,
    own: bool,
    now: DateTime<Utc>,
) -> ResolvedLevel {
    let empty = Attributes::new();
    let attributes = match (level, resource) {
        (EvalLevel::Type(t), Some(r)) if r.type_id == t => &r.attributes,
        _ => &empty,
    };
    let rows = LevelRows {
        feature,
        level,
        attributes,
        per_unit: data.per_unit.get(&feature.id).copied(),
    };
    let order = feature.value_order();
    let policy = feature.policy_or_default();
    let base = rows.fold_rows(customer.iter().copied(), None);
    let product = product_of(feature);

    let mut considered: Vec<FoldRow> = base.clone();
    let folds: Vec<(&SubscriptionRows, Folded)> = data
        .subscriptions
        .iter()
        .filter_map(|s| {
            let own_rows = rows.fold_rows(&s.rows, Some(s));
            let by_product =
                level == feature.own_level() && product.is_some_and(|p| s.products.contains(&p));
            if own_rows.is_empty() && !by_product {
                return None;
            }
            let mut all = rows.fold_rows(&data.defaults, Some(s));
            all.extend(own_rows);
            all.extend(base.iter().cloned());
            considered.extend(all.iter().cloned());
            fold(&order, own, &policy, &all, now).map(|f| (s, f))
        })
        .collect();
    let candidates: Vec<Candidate> = folds
        .iter()
        .map(|(s, f)| Candidate {
            subscription_id: s.row.id,
            resource: s.resource.as_ref().map(|(t, id, _)| (*t, *id)),
            start_date: s.row.start_date,
            value: f.value.clone(),
        })
        .collect();
    let walk = folds
        .iter()
        .map(|(s, f)| DecidingCandidate {
            subscription_id: s.row.id,
            resource_path: s.resource.as_ref().map(|(_, _, path)| path.clone()),
            value: f.value.clone(),
        })
        .collect();

    let (folded, subscription, resource_omitted) = match deciding(&order, &candidates, resource) {
        Deciding::Subscription {
            id,
            resource_omitted,
        } => {
            let (s, f) = folds
                .into_iter()
                .find(|(s, _)| s.row.id == id)
                .expect("the deciding subscription is a candidate");
            (Some(f), Some(s), resource_omitted)
        }
        Deciding::NoSubscription => {
            // Defaults reach as `has_rows` saw them: no product, or a subscription holding it.
            let defaults_reach =
                product.is_none_or(|p| data.subscriptions.iter().any(|s| s.products.contains(&p)));
            let mut all = if defaults_reach {
                rows.fold_rows(&data.defaults, None)
            } else {
                vec![]
            };
            all.extend(base.iter().cloned());
            considered.extend(all.iter().cloned());
            (fold(&order, own, &policy, &all, now), None, false)
        }
        Deciding::ResourceRequired => {
            let path = level_path(level, resource);
            return unresolved(feature, level, own, path, LevelValue::ResourceRequired);
        }
        Deciding::NotSubscribed => {
            let path = level_path(level, resource);
            return unresolved(feature, level, own, path, LevelValue::NotSubscribed);
        }
    };

    let anchor = folded.as_ref().and_then(|f| {
        f.explain
            .iter()
            .filter(|c| c.running.is_some())
            .filter_map(|c| considered.iter().find(|r| r.lineage_id == c.lineage_id))
            .map(|r| r.lineage_anchor)
            .min()
    });
    let (value, policy, policy_from, explain) = match folded {
        Some(f) => (
            f.value.map_or(LevelValue::Empty, LevelValue::Value),
            f.policy,
            f.policy_from,
            f.explain,
        ),
        None => (LevelValue::Empty, policy, None, vec![]),
    };
    let window = matches!(feature.kind, FeatureKind::Limit(LimitCountedBy::Events(_)))
        .then(|| window_of(&policy, subscription.map(|s| &s.row), anchor, data, now));
    ResolvedLevel {
        level,
        own,
        path: level_path(level, resource),
        value,
        policy,
        policy_from,
        deciding_subscription: subscription.map(|s| s.row.id),
        resource_omitted,
        window,
        next_change: next_change(&considered, now),
        explain,
        candidates: walk,
    }
}

/// The deciding subscription's cycle, or the calendar month without one (§3.7). A fixed or
/// never-ending window without an activation starts at a business date, which an import keeps:
/// the deciding subscription's start, else the deciding rows' lineage anchor: the earlier of the
/// lineage's first `valid_from` and its creation, which no edit moves.
fn window_of(
    policy: &crate::domain::entitlement_policy::FeaturePolicy,
    subscription: Option<&SubscriptionRow>,
    row_anchor: Option<DateTime<Utc>>,
    data: &CustomerRows,
    now: DateTime<Utc>,
) -> LevelWindow {
    let reset_period = policy.reset_period(subscription.is_some());
    let cycle = subscription.map(|s| BillingCyclePeriod {
        period_start: s.current_period_start,
        period_end: s.current_period_end,
    });
    let activation = match subscription {
        Some(s) => s.activated_at.or_else(|| s.start_date.and_hms_opt(0, 0, 0)),
        None => data
            .subscriptions
            .iter()
            .filter_map(|s| s.row.activated_at)
            .min()
            .or_else(|| Some(row_anchor.unwrap_or(data.customer_created_at).naive_utc())),
    };
    let bounds = compute_usage_period(&reset_period, cycle, activation, now.naive_utc());
    LevelWindow {
        reset_period,
        start: bounds.start.and_utc(),
        end: bounds.end.map(|end| end.and_utc()),
    }
}

impl Store {
    /// Every feature with rows reaching the customer (or only `features`), at each level on the
    /// path to `resource`. Features without any reaching row are left out: not entitled.
    pub async fn resolve_customer_entitlements(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        resource: Option<&ResolvedResource>,
        features: Option<&[FeatureId]>,
        now: DateTime<Utc>,
    ) -> StoreResult<ResolvedCustomer> {
        resolve_customer(conn, tenant_id, customer_id, resource, features, now).await
    }

    /// The usage a level's limit is measured against: the exact app count of its path, or the
    /// metric's events reaching its resource over its window (§3.7, Counting).
    pub(crate) async fn level_usage(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        feature: &Feature,
        level: &ResolvedLevel,
        resource: Option<&(ResolvedResource, ResourceType)>,
        now: DateTime<Utc>,
    ) -> StoreResult<Option<Decimal>> {
        self.level_usage_with(tenant_id, customer_id, feature, level, resource, now, None)
            .await
    }

    /// [`Self::level_usage`] with the metrics already loaded, when the caller reads many levels.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn level_usage_with(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        feature: &Feature,
        level: &ResolvedLevel,
        resource: Option<&(ResolvedResource, ResourceType)>,
        now: DateTime<Utc>,
        metrics: Option<&HashMap<BillableMetricId, BillableMetric>>,
    ) -> StoreResult<Option<Decimal>> {
        match self
            .level_counting(
                tenant_id,
                customer_id,
                feature,
                level,
                resource,
                now,
                metrics,
            )
            .await?
        {
            Counting::Known(used) => Ok(used),
            Counting::Uncountable(_) => Ok(None),
            Counting::Metered { metric, period } => self
                .usage_client
                .fetch_total_usage(&tenant_id, &customer_id, &metric, period)
                .await
                .map(Some),
        }
    }

    /// A level's usage for a check, a snapshot or a readout: metering answers within
    /// [`CHECK_USAGE_DEADLINE`] or the usage is unavailable; the store's own failures stay errors.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn level_usage_read(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        feature: &Feature,
        level: &ResolvedLevel,
        resource: Option<&(ResolvedResource, ResourceType)>,
        now: DateTime<Utc>,
        metrics: Option<&HashMap<BillableMetricId, BillableMetric>>,
        reading: &UsageReading,
    ) -> StoreResult<LevelUsage> {
        let (metric, period) = match self
            .level_counting(
                tenant_id,
                customer_id,
                feature,
                level,
                resource,
                now,
                metrics,
            )
            .await?
        {
            Counting::Known(used) => return Ok(LevelUsage::Read(used)),
            Counting::Uncountable(cause) => return Ok(LevelUsage::Uncountable(cause)),
            Counting::Metered { metric, period } => (metric, period),
        };
        if reading.metering_down.load(Ordering::Relaxed) {
            count_unavailable(reading.reader, "skipped");
            return Ok(LevelUsage::Unavailable);
        }
        match self
            .usage_client
            .fetch_total_usage_within(
                &tenant_id,
                &customer_id,
                &metric,
                period,
                CHECK_USAGE_DEADLINE,
            )
            .await
        {
            Ok(used) => Ok(LevelUsage::Read(Some(used))),
            Err(error) if metering_unavailable(&error) => {
                reading.metering_down.store(true, Ordering::Relaxed);
                let cause = match error.current_context() {
                    StoreError::UsageUnavailable => "deadline",
                    _ => "error",
                };
                count_unavailable(reading.reader, cause);
                tracing::warn!(
                    %tenant_id,
                    %customer_id,
                    feature_id = %feature.id,
                    reader = reading.reader,
                    cause,
                    "entitlement usage unavailable: {error:?}"
                );
                Ok(LevelUsage::Unavailable)
            }
            Err(error) => Err(error),
        }
    }

    /// What a level's usage takes: an exact app count, nothing, or a metering read of the
    /// metric's events reaching its resource over its window; or why it cannot be counted.
    #[allow(clippy::too_many_arguments)]
    async fn level_counting(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        feature: &Feature,
        level: &ResolvedLevel,
        resource: Option<&(ResolvedResource, ResourceType)>,
        now: DateTime<Utc>,
        metrics: Option<&HashMap<BillableMetricId, BillableMetric>>,
    ) -> StoreResult<Counting> {
        if !matches!(level.value, LevelValue::Value(_)) {
            return Ok(Counting::Known(None));
        }
        let metric_id = match feature.kind {
            FeatureKind::Limit(LimitCountedBy::App) => {
                let mut conn = self.get_conn().await?;
                let count = match level.level {
                    EvalLevel::Customer => {
                        EntitlementAppCountRow::sum_for_customer(
                            &mut conn,
                            tenant_id,
                            customer_id,
                            feature.id,
                        )
                        .await?
                    }
                    EvalLevel::Type(_) => EntitlementAppCountRow::find(
                        &mut conn,
                        tenant_id,
                        customer_id,
                        feature.id,
                        &level.path,
                    )
                    .await?
                    .map_or(Decimal::ZERO, |row| row.count),
                };
                return Ok(Counting::Known(Some(count.max(Decimal::ZERO))));
            }
            FeatureKind::Limit(LimitCountedBy::Events(metric_id)) => metric_id,
            _ => return Ok(Counting::Known(None)),
        };
        let Some(window) = &level.window else {
            return Ok(Counting::Known(None));
        };
        let end = window.end.map_or(now, |end| end.min(now)).naive_utc();
        let start = window.start.naive_utc();
        if start >= end {
            return Ok(Counting::Known(Some(Decimal::ZERO)));
        }
        let loaded = match metrics {
            Some(metrics) => metrics.get(&metric_id).cloned(),
            None => {
                let mut conn = self.get_conn().await?;
                load_metrics(&mut conn, &[metric_id], tenant_id)
                    .await?
                    .remove(&metric_id)
            }
        };
        let Some(mut metric) = loaded else {
            tracing::warn!(
                %tenant_id,
                feature_id = %feature.id,
                %metric_id,
                "entitlement limit counts a metric that cannot be loaded"
            );
            return Ok(Counting::Uncountable(Uncountable::MetricNotFound));
        };
        if let EvalLevel::Type(type_id) = level.level {
            match resource {
                Some((r, t)) if r.type_id == type_id => {
                    metric.usage_scope = resource_scope(&metric, t, Some(r.key_value.as_str()));
                }
                _ => return Ok(Counting::Uncountable(Uncountable::ResourceRequired)),
            }
        }
        Ok(Counting::Metered {
            metric: Box::new(metric),
            period: UsagePeriod { start, end },
        })
    }
}

/// How long a check or a snapshot waits for metering before answering without the usage.
pub const CHECK_USAGE_DEADLINE: std::time::Duration = std::time::Duration::from_millis(1500);

enum Counting {
    Known(Option<Decimal>),
    Uncountable(Uncountable),
    Metered {
        metric: Box<BillableMetric>,
        period: UsagePeriod,
    },
}

/// The reads of one check, snapshot or readout: once metering fails, the others answer
/// unavailable at once rather than each waiting for its deadline.
pub(crate) struct UsageReading {
    reader: &'static str,
    metering_down: AtomicBool,
}

impl UsageReading {
    pub(crate) fn check() -> Self {
        Self::of("check")
    }

    pub(crate) fn snapshot() -> Self {
        Self::of("snapshot")
    }

    pub(crate) fn readout() -> Self {
        Self::of("readout")
    }

    fn of(reader: &'static str) -> Self {
        Self {
            reader,
            metering_down: AtomicBool::new(false),
        }
    }
}

/// Metering down, late or not ready; a malformed request or a store failure is not.
fn metering_unavailable(error: &Report<StoreError>) -> bool {
    matches!(
        error.current_context(),
        StoreError::MeteringServiceError
            | StoreError::MeteringNotReady
            | StoreError::UsageUnavailable
    )
}

static USAGE_UNAVAILABLE: LazyLock<Counter<u64>> = LazyLock::new(|| {
    common_logging::GLOBAL_METER
        .u64_counter("entitlement_usage_unavailable")
        .with_description(
            "Entitlement levels answered without their usage, by reader (check, snapshot or \
             readout) and cause (deadline, error, or skipped after another read failed)",
        )
        .build()
});

fn count_unavailable(reader: &'static str, cause: &'static str) {
    USAGE_UNAVAILABLE.add(
        1,
        &[
            common_logging::KeyValue::new("reader", reader),
            common_logging::KeyValue::new("cause", cause),
        ],
    );
}

/// `Store::resolve_customer_entitlements` on a connection alone, for the hooks that get no store.
pub async fn resolve_customer(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    resource: Option<&ResolvedResource>,
    features: Option<&[FeatureId]>,
    now: DateTime<Utc>,
) -> StoreResult<ResolvedCustomer> {
    let data = CustomerRows::load(conn, tenant_id, customer_id, features, now).await?;
    Ok(ResolvedCustomer {
        features: data
            .features
            .iter()
            .filter_map(|f| resolve_feature(f, &data, resource, now))
            .collect(),
    })
}

/// The resource a check or snapshot names (`load_for_check`), with its type for counting.
pub async fn resolve_resource_input(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    input: &ResourceInput,
) -> StoreResult<Option<(ResolvedResource, ResourceType)>> {
    let Some(resolved) = load_for_check(conn, tenant_id, customer_id, input).await? else {
        return Ok(None);
    };
    let keys = ResourceTypeKeyRow::list_by_tenant(conn, tenant_id)
        .await?
        .into_iter()
        .filter(|k| k.resource_type_id == resolved.type_id)
        .map(|k| k.key)
        .collect();
    let row = ResourceTypeRow::find_by_id(conn, tenant_id, resolved.type_id).await?;
    Ok(Some((resolved, ResourceType::try_from((row, keys))?)))
}
