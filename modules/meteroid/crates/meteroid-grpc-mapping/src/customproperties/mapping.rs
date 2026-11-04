pub mod definitions {
    use crate::customproperties::error::CustomPropertiesApiError;
    use common_domain::ids::{CustomPropertyDefinitionId, TenantId};
    use meteroid_grpc::meteroid::api::customproperties::v1 as server;
    use meteroid_store::domain::custom_property_definitions as domain;
    use meteroid_store::domain::enums as store_enums;

    type Result<T> = std::result::Result<T, CustomPropertiesApiError>;

    fn invalid(msg: impl Into<String>) -> CustomPropertiesApiError {
        CustomPropertiesApiError::InvalidArgument(msg.into())
    }

    fn missing(field: &str) -> CustomPropertiesApiError {
        CustomPropertiesApiError::MissingArgument(field.to_string())
    }

    pub fn entity_type_from_proto(v: i32) -> Result<store_enums::CustomPropertyEntityType> {
        use server::CustomPropertyEntityType as P;
        use store_enums::CustomPropertyEntityType as D;
        match P::try_from(v).unwrap_or(P::Unspecified) {
            P::Customer => Ok(D::Customer),
            P::Subscription => Ok(D::Subscription),
            P::Invoice => Ok(D::Invoice),
            P::CreditNote => Ok(D::CreditNote),
            P::Quote => Ok(D::Quote),
            P::Unspecified => Err(invalid("entity_type is required")),
        }
    }

    pub fn entity_type_opt_from_proto(
        v: Option<i32>,
    ) -> Result<Option<store_enums::CustomPropertyEntityType>> {
        v.map(entity_type_from_proto).transpose()
    }

    fn entity_type_to_proto(t: store_enums::CustomPropertyEntityType) -> i32 {
        use server::CustomPropertyEntityType as P;
        use store_enums::CustomPropertyEntityType as D;
        let p = match t {
            D::Customer => P::Customer,
            D::Subscription => P::Subscription,
            D::Invoice => P::Invoice,
            D::CreditNote => P::CreditNote,
            D::Quote => P::Quote,
        };
        p as i32
    }

    fn property_type_from_proto(v: i32) -> Result<store_enums::CustomPropertyType> {
        use server::CustomPropertyType as P;
        use store_enums::CustomPropertyType as D;
        match P::try_from(v).unwrap_or(P::Unspecified) {
            P::Text => Ok(D::Text),
            P::Number => Ok(D::Number),
            P::Boolean => Ok(D::Boolean),
            P::Date => Ok(D::Date),
            P::Datetime => Ok(D::Datetime),
            P::SingleSelect => Ok(D::SingleSelect),
            P::MultiSelect => Ok(D::MultiSelect),
            P::Json => Ok(D::Json),
            P::Url => Ok(D::Url),
            P::Email => Ok(D::Email),
            P::Unspecified => Err(invalid("property_type is required")),
        }
    }

    fn property_type_to_proto(t: store_enums::CustomPropertyType) -> i32 {
        use server::CustomPropertyType as P;
        use store_enums::CustomPropertyType as D;
        let p = match t {
            D::Text => P::Text,
            D::Number => P::Number,
            D::Boolean => P::Boolean,
            D::Date => P::Date,
            D::Datetime => P::Datetime,
            D::SingleSelect => P::SingleSelect,
            D::MultiSelect => P::MultiSelect,
            D::Json => P::Json,
            D::Url => P::Url,
            D::Email => P::Email,
        };
        p as i32
    }

    fn config_from_proto(c: server::PropertyConfig) -> domain::PropertyConfig {
        let options = if c.options.is_empty() {
            None
        } else {
            Some(
                c.options
                    .into_iter()
                    .map(|o| domain::SelectOption {
                        value: o.value,
                        label: o.label,
                    })
                    .collect(),
            )
        };
        domain::PropertyConfig {
            options,
            min: c.min,
            max: c.max,
            max_length: c.max_length,
        }
    }

    fn config_to_proto(c: domain::PropertyConfig) -> server::PropertyConfig {
        server::PropertyConfig {
            options: c
                .options
                .unwrap_or_default()
                .into_iter()
                .map(|o| server::SelectOption {
                    value: o.value,
                    label: o.label,
                })
                .collect(),
            min: c.min,
            max: c.max,
            max_length: c.max_length,
        }
    }

    fn default_value_from_proto(s: Option<String>) -> Result<Option<serde_json::Value>> {
        s.map(|raw| serde_json::from_str(&raw).map_err(|e| invalid(format!("default_value: {e}"))))
            .transpose()
    }

    fn default_value_to_proto(v: Option<serde_json::Value>) -> Option<String> {
        v.map(|value| value.to_string())
    }

    pub fn domain_to_proto(
        d: domain::CustomPropertyDefinition,
    ) -> server::CustomPropertyDefinition {
        server::CustomPropertyDefinition {
            id: d.id.as_proto(),
            entity_type: entity_type_to_proto(d.entity_type),
            key: d.key,
            name: d.name,
            description: d.description,
            property_type: property_type_to_proto(d.property_type),
            config: Some(config_to_proto(d.config)),
            required: d.required,
            default_value: default_value_to_proto(d.default_value),
            display_order: d.display_order,
            archived: d.archived_at.is_some(),
        }
    }

    pub fn proto_to_new(
        data: server::CustomPropertyDefinitionData,
        tenant_id: TenantId,
    ) -> Result<domain::CustomPropertyDefinitionNew> {
        Ok(domain::CustomPropertyDefinitionNew {
            tenant_id,
            entity_type: entity_type_from_proto(data.entity_type)?,
            key: data.key,
            name: data.name,
            description: data.description,
            property_type: property_type_from_proto(data.property_type)?,
            config: data.config.map(config_from_proto).unwrap_or_default(),
            required: data.required,
            default_value: default_value_from_proto(data.default_value)?,
            display_order: data.display_order,
        })
    }

    pub fn proto_to_patch(
        id: CustomPropertyDefinitionId,
        tenant_id: TenantId,
        data: server::CustomPropertyDefinitionUpdate,
    ) -> Result<domain::CustomPropertyDefinitionPatch> {
        Ok(domain::CustomPropertyDefinitionPatch {
            id,
            tenant_id,
            name: data.name,
            // gRPC can set a description but not explicitly clear it to null.
            description: data.description.map(Some),
            config: data.config.map(config_from_proto),
            required: data.required,
            default_value: default_value_from_proto(data.default_value)?.map(Some),
            display_order: data.display_order,
            archived_at: None,
        })
    }

    pub fn parse_id(id: &str) -> Result<CustomPropertyDefinitionId> {
        CustomPropertyDefinitionId::from_proto(id).map_err(|_| missing("id"))
    }
}
