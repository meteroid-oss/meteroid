use crate::config::SchemaRegistryConfig;
use crate::workers::kafka::CUSTOMER_OUTBOX_TOPIC;
use schema_registry_converter::async_impl::schema_registry::{post_schema, SrSettings};
use schema_registry_converter::error::SRCError;
use schema_registry_converter::schema_registry_common::SubjectNameStrategy::{
    RecordNameStrategy, TopicNameStrategy,
};
use schema_registry_converter::schema_registry_common::{
    RegisteredSchema, SchemaType, SubjectNameStrategy, SuppliedReference, SuppliedSchema,
};

pub async fn register_schemas(sr_config: &SchemaRegistryConfig) -> Result<(), anyhow::Error> {
    if let Some(sr_url) = &sr_config.url {
        let sr_settings = SrSettings::new(sr_url.clone());

        let address = register_schema(
            RecordNameStrategy("address".into()),
            include_str!("../../../avro/address.avsc"),
            &sr_settings,
            vec![],
        )
        .await?;

        let address_ref = SuppliedReference {
            name: "com.meteroid.avro.Address".into(),
            subject: address.subject.clone(),
            schema: address.schema.schema.clone(),
            references: vec![],
        };

        let shipping_address = register_schema(
            RecordNameStrategy("shipping_address".into()),
            include_str!("../../../avro/shipping_address.avsc"),
            &sr_settings,
            vec![address_ref.clone()],
        )
        .await?;

        let shipping_address_ref = SuppliedReference {
            name: "com.meteroid.avro.ShippingAddress".into(),
            subject: shipping_address.subject.clone(),
            schema: shipping_address.schema.schema.clone(),
            references: vec![address_ref.clone()],
        };

        register_schema(
            TopicNameStrategy(CUSTOMER_OUTBOX_TOPIC.into(), false),
            include_str!("../../../avro/customer.avsc"),
            &sr_settings,
            vec![address_ref.clone(), shipping_address_ref.clone()],
        )
        .await?;
    }

    Ok(())
}

async fn register_schema(
    subject_name_strategy: SubjectNameStrategy,
    schema: &str,
    sr_settings: &SrSettings,
    references: Vec<SuppliedReference>,
) -> Result<RichRegisteredSchema, SRCError> {
    let supplied_schema = SuppliedSchema {
        name: None,
        schema_type: SchemaType::Avro,
        schema: schema.into(),
        references,
    };

    let subject = subject_name_strategy.get_subject()?;

    post_schema(sr_settings, subject.clone(), supplied_schema)
        .await
        .map(|schema| RichRegisteredSchema { subject, schema })
}

struct RichRegisteredSchema {
    subject: String,
    schema: RegisteredSchema,
}
