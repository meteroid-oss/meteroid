use crate::customproperties::model as rest;
use common_domain::ids::{CustomPropertyDefinitionId, TenantId};
use meteroid_store::domain;
use meteroid_store::domain::custom_property_definitions as store;
use meteroid_store::domain::enums as store_enums;

pub fn entity_type_to_domain(
    t: rest::CustomPropertyEntityType,
) -> store_enums::CustomPropertyEntityType {
    match t {
        rest::CustomPropertyEntityType::Customer => store_enums::CustomPropertyEntityType::Customer,
        rest::CustomPropertyEntityType::Subscription => {
            store_enums::CustomPropertyEntityType::Subscription
        }
        rest::CustomPropertyEntityType::Invoice => store_enums::CustomPropertyEntityType::Invoice,
        rest::CustomPropertyEntityType::CreditNote => {
            store_enums::CustomPropertyEntityType::CreditNote
        }
        rest::CustomPropertyEntityType::Quote => store_enums::CustomPropertyEntityType::Quote,
    }
}

pub fn entity_type_from_domain(
    t: store_enums::CustomPropertyEntityType,
) -> rest::CustomPropertyEntityType {
    match t {
        store_enums::CustomPropertyEntityType::Customer => rest::CustomPropertyEntityType::Customer,
        store_enums::CustomPropertyEntityType::Subscription => {
            rest::CustomPropertyEntityType::Subscription
        }
        store_enums::CustomPropertyEntityType::Invoice => rest::CustomPropertyEntityType::Invoice,
        store_enums::CustomPropertyEntityType::CreditNote => {
            rest::CustomPropertyEntityType::CreditNote
        }
        store_enums::CustomPropertyEntityType::Quote => rest::CustomPropertyEntityType::Quote,
    }
}

pub fn property_type_to_domain(t: rest::CustomPropertyType) -> store_enums::CustomPropertyType {
    use rest::CustomPropertyType as R;
    use store_enums::CustomPropertyType as D;
    match t {
        R::Text => D::Text,
        R::Number => D::Number,
        R::Boolean => D::Boolean,
        R::Date => D::Date,
        R::Datetime => D::Datetime,
        R::SingleSelect => D::SingleSelect,
        R::MultiSelect => D::MultiSelect,
        R::Json => D::Json,
        R::Url => D::Url,
        R::Email => D::Email,
    }
}

pub fn property_type_from_domain(t: store_enums::CustomPropertyType) -> rest::CustomPropertyType {
    use rest::CustomPropertyType as R;
    use store_enums::CustomPropertyType as D;
    match t {
        D::Text => R::Text,
        D::Number => R::Number,
        D::Boolean => R::Boolean,
        D::Date => R::Date,
        D::Datetime => R::Datetime,
        D::SingleSelect => R::SingleSelect,
        D::MultiSelect => R::MultiSelect,
        D::Json => R::Json,
        D::Url => R::Url,
        D::Email => R::Email,
    }
}

fn config_to_domain(c: rest::PropertyConfig) -> store::PropertyConfig {
    store::PropertyConfig {
        options: c.options.map(|opts| {
            opts.into_iter()
                .map(|o| store::SelectOption {
                    value: o.value,
                    label: o.label,
                })
                .collect()
        }),
        min: c.min,
        max: c.max,
        max_length: c.max_length,
    }
}

fn config_from_domain(c: store::PropertyConfig) -> rest::PropertyConfig {
    rest::PropertyConfig {
        options: c.options.map(|opts| {
            opts.into_iter()
                .map(|o| rest::SelectOption {
                    value: o.value,
                    label: o.label,
                })
                .collect()
        }),
        min: c.min,
        max: c.max,
        max_length: c.max_length,
    }
}

pub fn domain_to_rest(d: domain::CustomPropertyDefinition) -> rest::CustomPropertyDefinition {
    rest::CustomPropertyDefinition {
        id: d.id,
        entity_type: entity_type_from_domain(d.entity_type),
        key: d.key,
        name: d.name,
        description: d.description,
        property_type: property_type_from_domain(d.property_type),
        config: config_from_domain(d.config),
        required: d.required,
        default_value: d.default_value,
        display_order: d.display_order,
        archived: d.archived_at.is_some(),
    }
}

pub fn create_req_to_domain(
    tenant_id: TenantId,
    req: rest::CustomPropertyDefinitionCreateRequest,
) -> domain::CustomPropertyDefinitionNew {
    domain::CustomPropertyDefinitionNew {
        tenant_id,
        entity_type: entity_type_to_domain(req.entity_type),
        key: req.key,
        name: req.name,
        description: req.description,
        property_type: property_type_to_domain(req.property_type),
        config: config_to_domain(req.config),
        required: req.required,
        default_value: req.default_value,
        display_order: req.display_order,
    }
}

pub fn update_req_to_domain(
    id: CustomPropertyDefinitionId,
    tenant_id: TenantId,
    req: rest::CustomPropertyDefinitionUpdateRequest,
) -> domain::CustomPropertyDefinitionPatch {
    domain::CustomPropertyDefinitionPatch {
        id,
        tenant_id,
        name: req.name,
        description: req.description,
        config: req.config.map(config_to_domain),
        required: req.required,
        default_value: req.default_value,
        display_order: req.display_order,
        archived_at: None,
    }
}
