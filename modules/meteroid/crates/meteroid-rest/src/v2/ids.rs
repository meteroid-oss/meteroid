//! `{id_or_code}`: every catalog path and reference takes an id (`plan_…`, `prd_…`) or a code
//! (ADR-0013 §3). A string starting like the resource's ids is read as one when it parses; a
//! code never starts so (`validate_code`).
//!
//! An id is checked to be the tenant's, as a code is found in the tenant only: another tenant's
//! id reads as not found.
//!
//! A subscription path takes `{id_or_external_id}` the same way, resolving the external id with
//! `CatalogCodesInterface::find_subscription_id_by_external_id`; a customer is named by its id, or
//! by its alias (its external id) behind an `ext:` prefix.
use crate::AppState;
use crate::v2::problem::Problem;
use common_domain::ids::{
    AddOnId, AliasOr, BaseId, BillableMetricId, CustomerId, FeatureId, ManagedQuantityId, PlanId,
    ProductFamilyId, ProductId, RateCardId, ResourceTypeId, SubscriptionId, TenantId,
};
use meteroid_store::Store;
use meteroid_store::domain::BillableMetric;
use meteroid_store::domain::catalog_codes::validate_code;
use meteroid_store::domain::entitlements::Feature;
use meteroid_store::domain::managed_quantities::ManagedQuantity;
use meteroid_store::domain::resources::ResourceType;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::entitlements::EntitlementsInterface;
use meteroid_store::repositories::products::ProductInterface;
use meteroid_store::repositories::{
    CatalogCodesInterface, CustomersInterface, ManagedQuantitiesInterface, ProductFamilyInterface,
    RateCardsInterface, ResourcesInterface,
};
use std::str::FromStr;

/// Refuses a code that is not one, at `/code`, before anything is written.
pub fn check_code(code: Option<&str>, id_prefix: &str) -> Result<(), Problem> {
    match code {
        None => Ok(()),
        Some(code) => validate_code(code, id_prefix).map_err(|error| {
            Problem::field(
                "/code",
                "invalid",
                crate::v2::problem::store_message(&error),
            )
        }),
    }
}

fn parse<ID: BaseId<IdType = ID> + FromStr>(raw: &str) -> AliasOr<ID> {
    let raw = raw.trim();
    // Responses carry prefixed ids (`plan_…`); a UUID is read too. A code is never either
    // (`validate_code`).
    if let Ok(id) = ID::parse_uuid(raw) {
        return AliasOr::Id(id);
    }
    AliasOr::from_str(raw).unwrap_or_else(|_| AliasOr::Alias(raw.to_owned()))
}

/// The plan, checked to be the tenant's.
pub async fn plan_id(state: &AppState, tenant_id: TenantId, raw: &str) -> Result<PlanId, Problem> {
    match parse::<PlanId>(raw) {
        AliasOr::Id(id) => {
            if state
                .store
                .plan_codes(tenant_id, &[id])
                .await?
                .contains_key(&id)
            {
                Ok(id)
            } else {
                Err(Problem::not_found(format!("plan {}", raw.trim())))
            }
        }
        AliasOr::Alias(code) => Ok(state.store.find_plan_id_by_code(tenant_id, &code).await?),
    }
}

/// The product, checked to be the tenant's.
pub async fn product_id(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<ProductId, Problem> {
    product_id_in(&state.store, tenant_id, raw).await
}

/// [`product_id`] for a module's handler, which holds the store but not core's state.
pub async fn product_id_in(
    store: &Store,
    tenant_id: TenantId,
    raw: &str,
) -> Result<ProductId, Problem> {
    match parse::<ProductId>(raw) {
        AliasOr::Id(id) => Ok(store.find_product_by_id(id, tenant_id).await?.id),
        AliasOr::Alias(code) => Ok(store.find_product_id_by_code(tenant_id, &code).await?),
    }
}

/// The add-on, checked to be the tenant's.
pub async fn add_on_id(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<AddOnId, Problem> {
    match parse::<AddOnId>(raw) {
        AliasOr::Id(id) => Ok(state.store.get_add_on_by_id(tenant_id, id).await?.id),
        AliasOr::Alias(code) => Ok(state.store.find_add_on_id_by_code(tenant_id, &code).await?),
    }
}

/// The product family, checked to be the tenant's.
pub async fn product_family_id(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<ProductFamilyId, Problem> {
    match parse::<ProductFamilyId>(raw) {
        AliasOr::Id(id) => Ok(state
            .store
            .find_product_family_by_id(id, tenant_id)
            .await?
            .id),
        AliasOr::Alias(code) => Ok(state
            .store
            .find_product_family_id_by_code(tenant_id, &code)
            .await?),
    }
}

/// The rate card, checked to be the tenant's.
pub async fn rate_card_id(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<RateCardId, Problem> {
    match parse::<RateCardId>(raw) {
        AliasOr::Id(id) => Ok(state.store.get_rate_card(tenant_id, id).await?.0.id),
        AliasOr::Alias(code) => Ok(state
            .store
            .find_rate_card_id_by_code(tenant_id, &code)
            .await?),
    }
}

/// The metric of an id or a code: its own code, not the event code its usage events carry
/// (several metrics may read one).
pub async fn metric(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<BillableMetric, Problem> {
    match parse::<BillableMetricId>(raw) {
        AliasOr::Id(id) => Ok(state
            .store
            .find_billable_metric_by_id(id, tenant_id)
            .await?),
        AliasOr::Alias(code) => Ok(state
            .store
            .find_billable_metric_by_catalog_code(tenant_id, &code)
            .await?),
    }
}

/// The managed quantity of an id or a code.
pub async fn quantity(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<ManagedQuantity, Problem> {
    let quantities = state.store.list_managed_quantities(tenant_id).await?;
    let found = match parse::<ManagedQuantityId>(raw) {
        AliasOr::Id(id) => quantities.into_iter().find(|q| q.id == id),
        AliasOr::Alias(code) => quantities.into_iter().find(|q| q.code == code),
    };
    found.ok_or_else(|| Problem::not_found(format!("managed quantity {}", raw.trim())))
}

/// An id or a code as a path or a body names a resource of `ID`.
pub fn id_or_code<ID: BaseId<IdType = ID> + FromStr>(raw: &str) -> AliasOr<ID> {
    parse(raw)
}

/// The feature of an id or a code; a code names the live feature.
pub async fn feature(state: &AppState, tenant_id: TenantId, raw: &str) -> Result<Feature, Problem> {
    feature_in(&state.store, tenant_id, raw).await
}

/// [`feature`] for a module's handler.
pub async fn feature_in(store: &Store, tenant_id: TenantId, raw: &str) -> Result<Feature, Problem> {
    Ok(store
        .get_feature_by_id_or_code(parse::<FeatureId>(raw), tenant_id)
        .await?)
}

/// The resource type of an id or a code, archived ones included.
pub async fn resource_type(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<ResourceType, Problem> {
    Ok(state
        .store
        .get_resource_type(tenant_id, parse::<ResourceTypeId>(raw))
        .await?)
}

/// The prefix naming a customer by its alias, the integrator's own id for it.
pub const EXTERNAL_PREFIX: &str = "ext:";

/// The subscription of an id or an external id. An id is not checked to be the tenant's here:
/// every read and change of it is scoped by tenant.
pub async fn subscription_id(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<SubscriptionId, Problem> {
    subscription_id_in(&state.store, tenant_id, raw).await
}

/// [`subscription_id`] for a module's handler.
pub async fn subscription_id_in(
    store: &Store,
    tenant_id: TenantId,
    raw: &str,
) -> Result<SubscriptionId, Problem> {
    match parse::<SubscriptionId>(raw) {
        AliasOr::Id(id) => Ok(id),
        AliasOr::Alias(external_id) => Ok(store
            .find_subscription_id_by_external_id(tenant_id, &external_id)
            .await?),
    }
}

/// Whether `raw` would read as a subscription id: an external id never does.
pub fn reads_as_subscription_id(raw: &str) -> bool {
    reads_as_id::<SubscriptionId>(raw)
}

/// The id `raw` reads as, prefixed or a UUID; none when it reads as none.
pub fn parse_id<ID: BaseId<IdType = ID> + FromStr>(raw: &str) -> Option<ID> {
    match parse::<ID>(raw) {
        AliasOr::Id(id) => Some(id),
        AliasOr::Alias(_) => None,
    }
}

/// Whether `raw` would read as an id of `ID` in a path: a code never may.
pub fn reads_as_id<ID: BaseId<IdType = ID> + FromStr>(raw: &str) -> bool {
    matches!(parse::<ID>(raw), AliasOr::Id(_))
}

/// The customer of an id, or of an alias sent as `ext:<alias>`, checked to be the tenant's.
pub async fn customer(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<meteroid_store::domain::Customer, Problem> {
    customer_in(&state.store, tenant_id, raw).await
}

/// [`customer`] for a module's handler.
pub async fn customer_in(
    store: &Store,
    tenant_id: TenantId,
    raw: &str,
) -> Result<meteroid_store::domain::Customer, Problem> {
    let raw = raw.trim();
    let id = match raw.strip_prefix(EXTERNAL_PREFIX) {
        Some(alias) => {
            store
                .find_customer_id_by_alias(alias.trim().to_owned(), tenant_id)
                .await?
                .id
        }
        None => match parse::<CustomerId>(raw) {
            AliasOr::Id(id) => id,
            AliasOr::Alias(_) => {
                return Err(Problem::not_found(format!(
                    "customer {raw}: name a customer by its id, or by its alias as \
                     `{EXTERNAL_PREFIX}<alias>`"
                )));
            }
        },
    };
    Ok(store.find_customer_by_id(id, tenant_id).await?)
}

/// The id of a resource that has no code nor external id (an invoice, a credit note): its
/// prefixed form or its UUID. Not checked to be the tenant's here: every read of it is scoped by
/// tenant.
pub fn plain_id<ID: FromStr>(raw: &str, what: &str) -> Result<ID, Problem> {
    let raw = raw.trim();
    ID::from_str(raw).map_err(|_| Problem::not_found(format!("{what} {raw}")))
}
