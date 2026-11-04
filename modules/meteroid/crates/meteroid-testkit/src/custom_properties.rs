//! Custom property definitions are written by the custom-properties module; core tests seed them
//! here, and drop the store's cached definitions as the module does after each write.

use common_domain::ids::{BaseId, CustomPropertyDefinitionId, TenantId};
use diesel::{ExpressionMethods, SelectableHelper};
use diesel_async::RunQueryDsl;
use diesel_models::custom_property_definitions::CustomPropertyDefinitionRow;
use diesel_models::enums::{CustomPropertyEntityTypeEnum, CustomPropertyTypeEnum};
use diesel_models::schema::custom_property_definition as cpd;
use meteroid_store::Store;
use meteroid_store::domain::custom_property_definitions::{
    CustomPropertyDefinition, CustomPropertyDefinitionNew,
};

pub async fn create_custom_property_definition(
    store: &Store,
    new: CustomPropertyDefinitionNew,
) -> CustomPropertyDefinition {
    let config: serde_json::Value = (&new.config).try_into().expect("serialize config");
    let entity_type: CustomPropertyEntityTypeEnum = new.entity_type.into();
    let property_type: CustomPropertyTypeEnum = new.property_type.into();

    let mut conn = store.get_conn().await.expect("db connection");
    let row = diesel::insert_into(cpd::table)
        .values((
            cpd::id.eq(CustomPropertyDefinitionId::new()),
            cpd::tenant_id.eq(new.tenant_id),
            cpd::entity_type.eq(entity_type),
            cpd::key.eq(new.key),
            cpd::name.eq(new.name),
            cpd::description.eq(new.description),
            cpd::property_type.eq(property_type),
            cpd::config.eq(config),
            cpd::required.eq(new.required),
            cpd::default_value.eq(new.default_value),
            cpd::display_order.eq(new.display_order),
        ))
        .returning(CustomPropertyDefinitionRow::as_returning())
        .get_result::<CustomPropertyDefinitionRow>(&mut conn)
        .await
        .expect("insert custom property definition");

    store.custom_property_definitions_changed(new.tenant_id, new.entity_type);
    CustomPropertyDefinition::try_from(row).expect("custom property definition")
}

pub async fn archive_custom_property_definition(
    store: &Store,
    tenant_id: TenantId,
    id: CustomPropertyDefinitionId,
) -> CustomPropertyDefinition {
    let now = chrono::Utc::now().naive_utc();

    let mut conn = store.get_conn().await.expect("db connection");
    let row = diesel::update(cpd::table)
        .filter(cpd::id.eq(id))
        .filter(cpd::tenant_id.eq(tenant_id))
        .set((
            cpd::archived_at.eq(Some(now)),
            cpd::updated_at.eq(Some(now)),
        ))
        .returning(CustomPropertyDefinitionRow::as_returning())
        .get_result::<CustomPropertyDefinitionRow>(&mut conn)
        .await
        .expect("archive custom property definition");

    let definition = CustomPropertyDefinition::try_from(row).expect("custom property definition");
    store.custom_property_definitions_changed(tenant_id, definition.entity_type);
    definition
}
