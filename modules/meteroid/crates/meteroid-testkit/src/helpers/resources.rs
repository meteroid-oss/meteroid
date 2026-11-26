//! Subscriptions for a resource in tests: the `api_key` type (alias `key`) the usage filters of
//! earlier scenarios became, and a customer's resource per key.
use common_domain::actor::Actor;
use common_domain::ids::{AliasOr, CustomerId, ResourceId, TenantId};
use meteroid_store::Store;
use meteroid_store::domain::resources::ResourceType;
use meteroid_store::domain::{BillableMetricUpdate, MetricAttribution};
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::resources::{
    ResourceTypeNew, ResourceUpsert, ResourcesInterface,
};

pub const API_KEY: &str = "api_key";
pub const API_KEY_ALIAS: &str = "key";

pub fn attribute(attributed_to: Option<MetricAttribution>) -> BillableMetricUpdate {
    BillableMetricUpdate {
        name: None,
        description: None,
        unit_conversion_factor: None,
        unit_conversion_rounding: None,
        segmentation_matrix: None,
        filters: None,
        unit_label: None,
        usage_breakdown_key: None,
        attributed_to: Some(attributed_to),
    }
}

/// The tenant's type of `code` keyed `keys`, made on first use with every active metric
/// attributed to it, as the leftover usage filters were converted.
pub async fn keyed_type(
    store: &Store,
    tenant_id: TenantId,
    code: &str,
    keys: &[&str],
) -> ResourceType {
    if let Ok(found) = store
        .get_resource_type(tenant_id, AliasOr::Alias(code.to_owned()))
        .await
    {
        return found;
    }
    let created = store
        .create_resource_type(ResourceTypeNew {
            tenant_id,
            code: code.to_owned(),
            name: code.to_owned(),
            plural_name: code.to_owned(),
            parent_type_id: None,
            keys: keys.iter().map(|k| (*k).to_owned()).collect(),
            attributes: Default::default(),
        })
        .await
        .expect("a resource type");
    for metric in store
        .list_active_billable_metrics(tenant_id)
        .await
        .expect("metrics")
    {
        store
            .update_billable_metric(
                Actor::System,
                metric.id,
                tenant_id,
                attribute(Some(MetricAttribution {
                    type_id: created.id,
                    property: None,
                })),
            )
            .await
            .expect("a metric attributed to the type");
    }
    created
}

/// The customer's stored resource of `resource_type` keyed `key_value`.
pub async fn resource(
    store: &Store,
    resource_type: &ResourceType,
    customer_id: CustomerId,
    key_value: &str,
) -> ResourceId {
    store
        .upsert_resource(ResourceUpsert {
            tenant_id: resource_type.tenant_id,
            customer_id,
            resource_type: AliasOr::Id(resource_type.id),
            key_value: key_value.to_owned(),
            name: None,
            attributes: Default::default(),
            archived: false,
        })
        .await
        .expect("a stored resource")
        .id
}

/// The customer's `api_key` resource for `key` (alias `key`).
pub async fn api_key_resource(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    key: &str,
) -> ResourceId {
    let resource_type = keyed_type(store, tenant_id, API_KEY, &[API_KEY, API_KEY_ALIAS]).await;
    resource(store, &resource_type, customer_id, key).await
}
