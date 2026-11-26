use common_domain::ids::{AliasOr, ResourceTypeId};
use meteroid_app::conversions::ProtoConv;
use meteroid_grpc::meteroid::api::resources::v1 as server;
use meteroid_store::domain::resources as domain;
use std::collections::{BTreeMap, HashMap};

pub fn type_to_proto(t: domain::ResourceType) -> server::ResourceType {
    server::ResourceType {
        id: t.id.as_proto(),
        code: t.code,
        name: t.name,
        plural_name: t.plural_name,
        parent_type_id: None,
        keys: t.keys,
        attributes: attributes_to_proto(t.attributes),
        archived: t.archived_at.is_some(),
        created_at: t.created_at.as_proto(),
    }
}

pub fn resource_to_proto(r: domain::Resource) -> server::Resource {
    server::Resource {
        id: r.id.as_proto(),
        customer_id: r.customer_id.as_proto(),
        resource_type_id: r.resource_type_id.as_proto(),
        key_value: r.key_value,
        name: r.name,
        attributes: r.attributes.into_iter().collect(),
        archived: r.archived_at.is_some(),
    }
}

pub fn attributes_to_proto(
    attributes: BTreeMap<String, Vec<String>>,
) -> HashMap<String, server::AttributeValues> {
    attributes
        .into_iter()
        .map(|(a, values)| (a, server::AttributeValues { values }))
        .collect()
}

pub fn attributes_from_proto(
    attributes: HashMap<String, server::AttributeValues>,
) -> BTreeMap<String, Vec<String>> {
    attributes.into_iter().map(|(a, v)| (a, v.values)).collect()
}

/// An id when it parses as one, else a code.
pub fn type_ref(id_or_code: &str) -> AliasOr<ResourceTypeId> {
    id_or_code
        .parse()
        .unwrap_or_else(|_| AliasOr::Alias(id_or_code.to_owned()))
}
