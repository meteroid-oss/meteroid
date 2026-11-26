//! The sites a type level's monitors partition (ADR 0011 §3.7): each stored resource of the type
//! the customer's subscriptions or rows single out, resolved alone on one load of its rows.
use super::resolve::{CustomerRows, resolve_feature};
use crate::StoreResult;
use crate::domain::entitlement_checks::ResolvedCustomer;
use crate::domain::resources::{
    ResolvedResource, Resource, ResourceInput, ResourceRef, ResourceType, canonical_path,
    resolve_input,
};
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, CustomerId, FeatureId, ResourceId, TenantId};
use diesel_models::extend::pagination::PaginationRequest;
use diesel_models::resources::ResourceRow;
use itertools::Itertools;
use std::collections::{BTreeMap, HashSet};

pub struct TypeSite {
    pub resource: ResolvedResource,
    pub resolved: ResolvedCustomer,
}

pub struct TypeSites {
    pub sites: Vec<TypeSite>,
    /// Any other resource of the type: neither a subscription, an override nor a condition
    /// singles it out, so the type's rows decide it alike.
    pub others: ResolvedCustomer,
}

/// An ad-hoc resource of the type without key or attributes, which only its type's rows reach.
pub fn any_resource(resource_type: &ResourceType) -> ResolvedResource {
    ResolvedResource {
        type_id: resource_type.id,
        type_code: resource_type.code.clone(),
        key_value: String::new(),
        path: canonical_path(&resource_type.code, ""),
        reference: ResourceRef::AdHoc {
            type_id: resource_type.id,
            key_value: String::new(),
        },
        attributes: Default::default(),
        attributes_ignored: vec![],
    }
}

/// `features` on each resource of the type the customer's subscriptions (oldest first) or
/// resource overrides name, and on every stored one with attributes once a row has a condition.
pub async fn resolve_type_sites(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    resource_type: &ResourceType,
    features: &[FeatureId],
    now: DateTime<Utc>,
) -> StoreResult<TypeSites> {
    let data = CustomerRows::load(conn, tenant_id, customer_id, Some(features), now).await?;
    let subscribed: Vec<ResourceId> = data
        .subscriptions
        .iter()
        .sorted_by_key(|s| (s.row.start_date, s.row.id.as_uuid()))
        .filter_map(|s| s.resource.as_ref())
        .filter(|(type_id, _, _)| *type_id == resource_type.id)
        .map(|(_, id, _)| *id)
        .unique()
        .collect();
    let overridden: HashSet<ResourceId> =
        data.customer.iter().filter_map(|e| e.resource_id).collect();
    let conditioned = data
        .defaults
        .iter()
        .chain(&data.customer)
        .chain(data.subscriptions.iter().flat_map(|s| &s.rows))
        .any(|e| e.condition.is_some());
    let stored = ResourceRow::list(
        conn,
        tenant_id,
        Some(customer_id),
        Some(resource_type.id),
        true,
        PaginationRequest {
            per_page: Some(100_000),
            page: 0,
        },
    )
    .await?
    .items
    .into_iter()
    .map(Resource::try_from)
    .collect::<StoreResult<Vec<_>>>()?;
    let picked =
        |r: &&Resource| overridden.contains(&r.id) || (conditioned && !r.attributes.is_empty());
    let ordered: Vec<&Resource> = subscribed
        .iter()
        .filter_map(|id| stored.iter().find(|r| r.id == *id))
        .chain(
            stored
                .iter()
                .filter(|r| !subscribed.contains(&r.id))
                .filter(picked),
        )
        .collect();
    let types = std::slice::from_ref(resource_type);
    let key = resource_type.keys.first().cloned().unwrap_or_default();
    let resolve = |resource: Option<&ResolvedResource>| ResolvedCustomer {
        features: data
            .features
            .iter()
            .filter_map(|f| resolve_feature(f, &data, resource, now))
            .collect(),
    };
    let mut sites = Vec::with_capacity(ordered.len());
    for r in ordered {
        let input = ResourceInput {
            resources: BTreeMap::from([(key.clone(), r.key_value.clone())]),
            attributes: BTreeMap::new(),
        };
        let Some(resource) = resolve_input(types, std::slice::from_ref(r), &input)? else {
            continue;
        };
        sites.push(TypeSite {
            resolved: resolve(Some(&resource)),
            resource,
        });
    }
    Ok(TypeSites {
        others: resolve(Some(&any_resource(resource_type))),
        sites,
    })
}
