use crate::StoreResult;
use crate::domain::custom_property_definitions::{
    CustomPropertyDefinition, CustomPropertyDefinitionNew, CustomPropertyMode,
    validate_and_normalize,
};
use crate::domain::enums::CustomPropertyEntityType;
use crate::errors::StoreError;
use crate::store::{PgConn, Store, StoreInternal};
use cached::{Cached, TimedSizedCache};
use common_domain::ids::{BaseId, CustomPropertyDefinitionId, TenantId};
use diesel_models::custom_property_definitions::{
    CustomPropertyDefinitionRow, CustomPropertyDefinitionRowNew,
};
use error_stack::Report;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

#[async_trait::async_trait]
pub trait CustomPropertyDefinitionsInterface {
    async fn list_custom_property_definitions(
        &self,
        tenant_id: TenantId,
        entity_type: Option<CustomPropertyEntityType>,
        include_archived: bool,
    ) -> StoreResult<Vec<CustomPropertyDefinition>>;

    async fn get_custom_property_definition(
        &self,
        tenant_id: TenantId,
        id: CustomPropertyDefinitionId,
    ) -> StoreResult<CustomPropertyDefinition>;
}

#[async_trait::async_trait]
impl CustomPropertyDefinitionsInterface for Store {
    async fn list_custom_property_definitions(
        &self,
        tenant_id: TenantId,
        entity_type: Option<CustomPropertyEntityType>,
        include_archived: bool,
    ) -> StoreResult<Vec<CustomPropertyDefinition>> {
        let mut conn = self.get_conn().await?;

        CustomPropertyDefinitionRow::list(
            &mut conn,
            tenant_id,
            entity_type.map(Into::into),
            include_archived,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn get_custom_property_definition(
        &self,
        tenant_id: TenantId,
        id: CustomPropertyDefinitionId,
    ) -> StoreResult<CustomPropertyDefinition> {
        let mut conn = self.get_conn().await?;

        CustomPropertyDefinitionRow::get_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?
            .try_into()
    }
}

impl Store {
    /// Defines a custom property, as a sandbox preset seeds one; the definitions users write go
    /// through the custom-properties module.
    pub async fn insert_custom_property_definition(
        &self,
        new: CustomPropertyDefinitionNew,
    ) -> StoreResult<CustomPropertyDefinition> {
        let valid_key = !new.key.is_empty()
            && new
                .key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid_key {
            return Err(Report::new(StoreError::InvalidArgument(
                "custom property key may only contain letters, digits and underscores".to_owned(),
            )));
        }
        let (tenant_id, entity_type) = (new.tenant_id, new.entity_type);
        let row = CustomPropertyDefinitionRowNew {
            id: CustomPropertyDefinitionId::new(),
            tenant_id,
            entity_type: entity_type.into(),
            key: new.key,
            name: new.name,
            description: new.description,
            property_type: new.property_type.into(),
            config: (&new.config).try_into()?,
            required: new.required,
            default_value: new.default_value,
            display_order: new.display_order,
        };
        let mut conn = self.get_conn().await?;
        let created: CustomPropertyDefinition = row
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .try_into()?;
        self.custom_property_definitions_changed(tenant_id, entity_type);
        Ok(created)
    }

    /// Call after writing a definition, so this store's validation stops using the cached set.
    pub fn custom_property_definitions_changed(
        &self,
        tenant_id: TenantId,
        entity_type: CustomPropertyEntityType,
    ) {
        self.internal
            .custom_property_definitions
            .invalidate(tenant_id, entity_type);
    }
}

impl StoreInternal {
    /// Validates and normalizes a custom-property bag for an entity write, using the tenant's
    /// (cached) active definitions for `entity_type`. Returns the full map to persist on the row.
    ///
    /// Meant to run inside the entity's write transaction (right before insert/update). `existing`
    /// is the entity's current bag and only matters in [`CustomPropertyMode::Merge`].
    pub async fn validate_and_normalize_custom_properties(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        entity_type: CustomPropertyEntityType,
        provided: &serde_json::Value,
        existing: &serde_json::Value,
        mode: CustomPropertyMode,
    ) -> StoreResult<serde_json::Value> {
        // A null `provided` means the caller isn't managing custom properties in this write
        // (e.g. an internal flow, or a request that omitted the field) — keep existing untouched.
        // This is what keeps required-property enforcement from breaking system-driven creates.
        if provided.is_null() {
            return Ok(existing.clone());
        }
        let definitions = self
            .custom_property_definitions
            .load_active(conn, tenant_id, entity_type)
            .await?;
        let normalized = validate_and_normalize(definitions.as_slice(), provided, existing, mode)?;
        Ok(normalized)
    }
}

type DefinitionsKey = (TenantId, CustomPropertyEntityType);

// Per-store cache of active definitions on the write hot path, so stores on different databases
// never share entries. Short TTL: other replicas only see a definition change once it expires.
pub(crate) struct CustomPropertyDefinitionsCache(
    Mutex<TimedSizedCache<DefinitionsKey, Arc<Vec<CustomPropertyDefinition>>>>,
);

impl Default for CustomPropertyDefinitionsCache {
    fn default() -> Self {
        Self(Mutex::new(TimedSizedCache::with_size_and_lifespan(
            200,
            Duration::from_secs(60),
        )))
    }
}

impl CustomPropertyDefinitionsCache {
    fn entries(
        &self,
    ) -> MutexGuard<'_, TimedSizedCache<DefinitionsKey, Arc<Vec<CustomPropertyDefinition>>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) async fn load_active(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        entity_type: CustomPropertyEntityType,
    ) -> StoreResult<Arc<Vec<CustomPropertyDefinition>>> {
        let key = (tenant_id, entity_type);
        let cached = self.entries().cache_get(&key).cloned();
        if let Some(hit) = cached {
            return Ok(hit);
        }

        let rows = CustomPropertyDefinitionRow::list_active_for_entity(
            conn,
            tenant_id,
            entity_type.into(),
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;
        let definitions = Arc::new(
            rows.into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<CustomPropertyDefinition>, Report<StoreError>>>()?,
        );

        self.entries().cache_set(key, definitions.clone());
        Ok(definitions)
    }

    fn invalidate(&self, tenant_id: TenantId, entity_type: CustomPropertyEntityType) {
        self.entries().cache_remove(&(tenant_id, entity_type));
    }
}
