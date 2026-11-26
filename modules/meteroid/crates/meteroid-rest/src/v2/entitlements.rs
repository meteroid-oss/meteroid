//! Entitlements in API v2 (ADR-0013 §23): the rows a feature's defaults, a plan version, an
//! add-on, a product (its features' defaults) and a subscription hold, each list written whole
//! with its owner as `EntitlementInput`s and read back canonical: by feature code, level,
//! resource type, resource and condition, then validity. A row reads at its resolved level.
use crate::AppState;
use crate::v2::bounds;
use crate::v2::problem::Problem;
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, FeatureId, ProductId, TenantId};
use meteroid_store::domain::entitlement_fold::{Condition, Tier};
use meteroid_store::domain::entitlements::{
    Entitlement as DomainEntitlement, EntitlementSpec, Feature,
};
use meteroid_store::domain::enums::FeatureStatusEnum;
use meteroid_store::domain::resources::ResourceType;
use meteroid_store::repositories::ResourcesInterface;
use meteroid_store::repositories::entitlements::{EntitlementsInterface, FeaturePageFilter};
use model::FeatureKey;
use std::collections::{BTreeMap, HashMap, HashSet};

pub mod model;
pub(crate) mod rows;

pub use model::{
    BillingCycleReset, CalendarReset, Codes, Entitlement, EntitlementInput, FixedWindowReset,
    NeverReset, Reset, SlidingWindowReset, Unit,
};
pub use rows::routes;

type SortKey<'a> = (
    &'a str,
    u8,
    Option<&'a str>,
    Option<(&'a str, &'a str)>,
    Option<&'a BTreeMap<String, Vec<String>>>,
);

fn sort_key(row: &Entitlement) -> SortKey<'_> {
    (
        row.feature.as_str(),
        row.level as u8,
        row.resource_type.as_deref(),
        row.resource
            .as_ref()
            .map(|r| (r.resource_type.as_str(), r.key.as_str())),
        row.condition.as_ref(),
    )
}

/// `rows` as v2 reads them, canonical; rows of features `codes` does not name (archived) are
/// left out, as the store hides them.
pub fn canonical(codes: &Codes, rows: Vec<DomainEntitlement>) -> Vec<Entitlement> {
    let mut keyed: Vec<(Option<DateTime<Utc>>, Entitlement)> = rows
        .into_iter()
        .filter(|row| codes.features.contains_key(&row.feature_id))
        .map(|row| (row.valid_from, codes.entitlement(row)))
        .collect();
    keyed.sort_by(|(from_a, a), (from_b, b)| {
        sort_key(a)
            .cmp(&sort_key(b))
            .then_with(|| from_a.cmp(from_b))
    });
    keyed.into_iter().map(|(_, row)| row).collect()
}

/// The codes owner lists are read with: their live features' and the tenant's resource types'.
/// Owner lists name no stored resource (only a customer's overrides do).
async fn owner_codes(
    state: &AppState,
    tenant_id: TenantId,
    rows: &[&DomainEntitlement],
) -> Result<Codes, Problem> {
    let mut codes = Codes::types(state, tenant_id).await?;
    let ids: Vec<FeatureId> = rows
        .iter()
        .map(|row| row.feature_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if !ids.is_empty() {
        codes.features = state
            .store
            .list_features_by_ids(&ids, tenant_id)
            .await?
            .into_iter()
            .filter(|feature| feature.status != FeatureStatusEnum::Archived)
            .map(|feature| (feature.id, FeatureKey::from(&feature)))
            .collect();
    }
    Ok(codes)
}

/// One owner's rows, canonical.
pub async fn render(
    state: &AppState,
    tenant_id: TenantId,
    rows: Vec<DomainEntitlement>,
) -> Result<Vec<Entitlement>, Problem> {
    let codes = owner_codes(state, tenant_id, &rows.iter().collect::<Vec<_>>()).await?;
    Ok(canonical(&codes, rows))
}

/// Several owners' rows (a page of add-ons), their features read once.
pub async fn render_each(
    state: &AppState,
    tenant_id: TenantId,
    owners: Vec<Vec<DomainEntitlement>>,
) -> Result<Vec<Vec<Entitlement>>, Problem> {
    let all: Vec<&DomainEntitlement> = owners.iter().flatten().collect();
    let codes = owner_codes(state, tenant_id, &all).await?;
    Ok(owners
        .into_iter()
        .map(|rows| canonical(&codes, rows))
        .collect())
}

/// Whether two rows' validities meet; an undated start is the earliest, an undated end never
/// comes.
fn validities_meet(a: &EntitlementSpec, b: &EntitlementSpec) -> bool {
    let starts_before_end =
        |from: Option<DateTime<Utc>>, until: Option<DateTime<Utc>>| match (from, until) {
            (_, None) | (None, _) => true,
            (Some(from), Some(until)) => from < until,
        };
    starts_before_end(a.valid_from, b.valid_until) && starts_before_end(b.valid_from, a.valid_until)
}

fn same_key(a: &EntitlementSpec, b: &EntitlementSpec) -> bool {
    let hash = |condition: &Option<Condition>| Condition::hash_of(condition.as_ref());
    a.feature_id == b.feature_id && a.level == b.level && hash(&a.condition) == hash(&b.condition)
}

/// One row asked, checked against `feature` and the owner's `tier` at `at` (the row's pointer);
/// `earlier` are the rows before it in its list, of which none may share its key over the same
/// dates.
fn checked(
    input: &EntitlementInput,
    feature: &Feature,
    types: &[ResourceType],
    tier: Tier,
    at: &str,
    earlier: &[&EntitlementSpec],
) -> Result<EntitlementSpec, Problem> {
    // The store gives an add-on its default effect: it knows how the add-on is bought.
    let spec = model::spec_of(input, feature, types, tier, true, at)?;
    if let Some(first) = earlier
        .iter()
        .position(|other| same_key(other, &spec) && validities_meet(other, &spec))
    {
        return Err(Problem::field(
            format!("{at}/feature"),
            "duplicate",
            format!("Row {first} has the same feature, level and condition over the same dates"),
        ));
    }
    Ok(spec)
}

/// The rows asked, each resolved and checked against its feature and the owner's `tier` at
/// `{path}/{index}`, before anything is written: an unknown or archived feature, a value, an
/// effect, a level or a policy the row cannot take, two rows of one key whose validities meet.
/// The store checks conditions against their resource types.
pub async fn resolve(
    state: &AppState,
    tenant_id: TenantId,
    inputs: &[EntitlementInput],
    tier: Tier,
    path: &str,
) -> Result<Vec<(Feature, EntitlementSpec)>, Problem> {
    bounds::count(inputs.len(), bounds::MAX_ENTITLEMENTS, path, "entitlements")?;
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let types = state.store.list_resource_types(tenant_id, true).await?;
    let mut features: HashMap<String, Feature> = HashMap::new();
    let mut resolved: Vec<(Feature, EntitlementSpec)> = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        let at = format!("{path}/{index}");
        let raw = input.feature.trim().to_owned();
        let feature = match features.get(&raw) {
            Some(feature) => feature.clone(),
            None => {
                let feature =
                    model::granted_feature(state, tenant_id, &raw, &format!("{at}/feature"))
                        .await?;
                features.insert(raw, feature.clone());
                feature
            }
        };
        let earlier: Vec<&EntitlementSpec> = resolved.iter().map(|(_, spec)| spec).collect();
        let spec = checked(input, &feature, &types, tier, &at, &earlier)?;
        resolved.push((feature, spec));
    }
    Ok(resolved)
}

/// A feature's default rows, checked against `feature` (as it will be once written: a feature
/// being created, or as its patch leaves it) at `{path}/{index}`; each names the feature itself.
pub async fn own_defaults(
    state: &AppState,
    tenant_id: TenantId,
    feature: &Feature,
    inputs: &[EntitlementInput],
    path: &str,
) -> Result<Vec<EntitlementSpec>, Problem> {
    bounds::count(inputs.len(), bounds::MAX_ENTITLEMENTS, path, "entitlements")?;
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let types = state.store.list_resource_types(tenant_id, true).await?;
    let mut specs: Vec<EntitlementSpec> = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        let at = format!("{path}/{index}");
        let raw = input.feature.trim();
        let names_it = raw == feature.code
            || raw == feature.id.to_string()
            || raw == feature.id.as_uuid().to_string();
        if !names_it {
            return Err(Problem::field(
                format!("{at}/feature"),
                "invalid",
                format!("A default of '{}' names that feature", feature.code),
            ));
        }
        let spec = checked(
            input,
            feature,
            &types,
            Tier::Default,
            &at,
            &specs.iter().collect::<Vec<_>>(),
        )?;
        specs.push(spec);
    }
    Ok(specs)
}

/// [`resolve`]'s specs.
pub async fn specs(
    state: &AppState,
    tenant_id: TenantId,
    inputs: &[EntitlementInput],
    tier: Tier,
    path: &str,
) -> Result<Vec<EntitlementSpec>, Problem> {
    Ok(resolve(state, tenant_id, inputs, tier, path)
        .await?
        .into_iter()
        .map(|(_, spec)| spec)
        .collect())
}

/// The defaults `inputs` sets on the product's features, each checked to be one of its own:
/// written by `update_product_with_feature_defaults` with the product's fields.
pub async fn product_specs(
    state: &AppState,
    tenant_id: TenantId,
    product_id: ProductId,
    inputs: &[EntitlementInput],
    path: &str,
) -> Result<Vec<EntitlementSpec>, Problem> {
    let resolved = resolve(state, tenant_id, inputs, Tier::Default, path).await?;
    for (index, (feature, _)) in resolved.iter().enumerate() {
        if feature.product.as_ref().map(|product| product.id) != Some(product_id) {
            return Err(Problem::field(
                format!("{path}/{index}/feature"),
                "invalid",
                format!(
                    "The feature '{}' is not the product's: a product holds the defaults of its \
                     own features",
                    feature.code
                ),
            ));
        }
    }
    Ok(resolved.into_iter().map(|(_, spec)| spec).collect())
}

/// What each of `products` holds: the defaults of its live features, canonical, all read once.
pub async fn of_products(
    state: &AppState,
    tenant_id: TenantId,
    products: &[ProductId],
) -> Result<HashMap<ProductId, Vec<Entitlement>>, Problem> {
    if products.is_empty() {
        return Ok(HashMap::new());
    }
    let filter = |product_id| FeaturePageFilter {
        statuses: Some(vec![FeatureStatusEnum::Active, FeatureStatusEnum::Disabled]),
        product_id,
        ..FeaturePageFilter::default()
    };
    let features = match products {
        [one] => {
            state
                .store
                .list_features_after(tenant_id, filter(Some(*one)), None, usize::MAX)
                .await?
        }
        _ => {
            state
                .store
                .list_features_after(tenant_id, filter(None), None, usize::MAX)
                .await?
        }
    };
    let mut codes = Codes::types(state, tenant_id).await?;
    let mut rows: HashMap<ProductId, Vec<DomainEntitlement>> = HashMap::new();
    for feature in features {
        let Some(product) = feature.product.as_ref().map(|p| p.id) else {
            continue;
        };
        if !products.contains(&product) {
            continue;
        }
        codes
            .features
            .insert(feature.id, FeatureKey::from(&feature));
        rows.entry(product).or_default().extend(feature.defaults);
    }
    Ok(rows
        .into_iter()
        .map(|(product, rows)| (product, canonical(&codes, rows)))
        .collect())
}
