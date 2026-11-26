//! Resource types and stored resources (ADR 0011 §3.4): one level below the customer.
use crate::StoreResult;
use crate::domain::resources::{
    ResolvedResource, Resource, ResourceInput, ResourceType, resolve_input,
};
use crate::domain::{PaginatedVec, PaginationRequest};
use crate::errors::StoreError;
use crate::services::entitlements::limits::rules::{MAX_GROUP_VALUE_BYTES, validate_group_value};
use crate::store::{PgConn, Store};
use common_domain::ids::{
    AliasOr, BaseId, CustomerId, EntitlementId, ResourceId, ResourceTypeId, TenantId,
};
use diesel::{ExpressionMethods, QueryDsl, QueryableByName, sql_types};
use diesel_async::RunQueryDsl;
use diesel_models::customer_credit::CreditRecomputeMarkRow;
use diesel_models::customers::CustomerRow;
use diesel_models::entitlements::EntitlementRow;
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::errors::DatabaseError;
use diesel_models::resources::{
    ResourceRow, ResourceRowNew, ResourceTypeKeyRow, ResourceTypeRow, ResourceTypeRowNew,
    ResourceTypeRowPatch,
};
use diesel_models::tenants::TenantRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use std::collections::{BTreeMap, HashMap};

/// A refusal the APIs answer with its own error code, attached to the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceRefusal {
    KeyClash,
    HierarchyUnsupported,
    LimitReached,
    TypeArchived,
}

impl ResourceRefusal {
    pub const fn code(self) -> &'static str {
        match self {
            Self::KeyClash => "RESOURCE_KEY_CLASH",
            Self::HierarchyUnsupported => "RESOURCE_HIERARCHY_UNSUPPORTED",
            Self::LimitReached => "RESOURCE_LIMIT_REACHED",
            Self::TypeArchived => "RESOURCE_TYPE_ARCHIVED",
        }
    }

    /// `KeyClash` is a conflict, the others are invalid arguments.
    pub fn report(self, message: impl Into<String>) -> Report<StoreError> {
        let message = message.into();
        let error = match self {
            Self::KeyClash => StoreError::Conflict(message),
            _ => StoreError::InvalidArgument(message),
        };
        Report::new(error).attach_opaque(self)
    }

    pub fn of(error: &Report<StoreError>) -> Option<Self> {
        error
            .frames()
            .find_map(|frame| frame.downcast_ref::<Self>())
            .copied()
    }
}

/// Attached to a refusal of the resource a subscription, a quote or a checkout session names (not
/// the customer's, archived, sold over the period, claiming usage already invoiced), so an API
/// points at the resource it was given rather than at what else the write names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceAtFault;

impl ResourceAtFault {
    pub fn of(error: &Report<StoreError>) -> bool {
        error
            .frames()
            .any(|frame| frame.downcast_ref::<Self>().is_some())
    }
}

/// Marks `error` as the named resource's fault.
pub(crate) fn resource_at_fault(error: Report<StoreError>) -> Report<StoreError> {
    error.attach_opaque(ResourceAtFault)
}

#[derive(Debug, Clone)]
pub struct ResourceTypeNew {
    pub tenant_id: TenantId,
    pub code: String,
    pub name: String,
    pub plural_name: String,
    /// Always refused in v1.
    pub parent_type_id: Option<ResourceTypeId>,
    pub keys: Vec<String>,
    pub attributes: BTreeMap<String, Vec<String>>,
}

/// `None` leaves a field unchanged. The code is immutable: conditions and paths name it.
#[derive(Debug, Clone, Default)]
pub struct ResourceTypePatch {
    pub name: Option<String>,
    pub plural_name: Option<String>,
    /// Always refused in v1.
    pub parent_type_id: Option<ResourceTypeId>,
    pub keys: Option<Vec<String>>,
    pub attributes: Option<BTreeMap<String, Vec<String>>>,
    pub archived: Option<bool>,
}

/// Creates the `(customer, type, key value)` resource, or replaces its name, attributes and archive.
#[derive(Debug, Clone)]
pub struct ResourceUpsert {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    /// The type's id or code.
    pub resource_type: AliasOr<ResourceTypeId>,
    pub key_value: String,
    pub name: Option<String>,
    /// Without the type code.
    pub attributes: BTreeMap<String, String>,
    pub archived: bool,
}

#[async_trait::async_trait]
pub trait ResourcesInterface {
    /// By code; archived types still resolve in checks, so they are listed when asked.
    async fn list_resource_types(
        &self,
        tenant_id: TenantId,
        include_archived: bool,
    ) -> StoreResult<Vec<ResourceType>>;

    async fn get_resource_type(
        &self,
        tenant_id: TenantId,
        resource_type: AliasOr<ResourceTypeId>,
    ) -> StoreResult<ResourceType>;

    async fn create_resource_type(&self, new: ResourceTypeNew) -> StoreResult<ResourceType>;

    async fn update_resource_type(
        &self,
        tenant_id: TenantId,
        resource_type: AliasOr<ResourceTypeId>,
        patch: ResourceTypePatch,
    ) -> StoreResult<ResourceType>;

    async fn upsert_resource(&self, upsert: ResourceUpsert) -> StoreResult<Resource>;

    async fn list_resources(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        resource_type: Option<AliasOr<ResourceTypeId>>,
        include_archived: bool,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<Resource>>;

    /// A page of the customer's resources after `after`, by id.
    async fn list_customer_resources_after(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        resource_type_id: Option<ResourceTypeId>,
        include_archived: bool,
        after: Option<ResourceId>,
        limit: usize,
    ) -> StoreResult<Vec<Resource>>;

    async fn get_resource(&self, tenant_id: TenantId, id: ResourceId) -> StoreResult<Resource>;

    /// The tenant's resources among `ids`; those not found are left out.
    async fn get_resources_by_ids(
        &self,
        tenant_id: TenantId,
        ids: &[ResourceId],
    ) -> StoreResult<Vec<Resource>>;

    /// The customer's stored resource of a type (id or code) by key value, archived ones included.
    async fn find_resource(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        resource_type: AliasOr<ResourceTypeId>,
        key_value: &str,
    ) -> StoreResult<Resource>;

    async fn set_resource_archived(
        &self,
        tenant_id: TenantId,
        id: ResourceId,
        archived: bool,
    ) -> StoreResult<Resource>;

    /// Refused while a subscription (not aborted), quote or checkout session is for it, or once an
    /// override, current or past, names it: archive it instead.
    async fn delete_resource(&self, tenant_id: TenantId, id: ResourceId) -> StoreResult<()>;

    /// [`load_for_check`] on its own connection, e.g. to validate a client token's claims.
    async fn resolve_resource_input(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        input: &ResourceInput,
    ) -> StoreResult<Option<ResolvedResource>>;
}

/// Resolves a check's resource against the tenant's types (archived included) and the customer's
/// stored resource for the key value it names. `None` when it names the customer only.
pub async fn load_for_check(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    input: &ResourceInput,
) -> StoreResult<Option<ResolvedResource>> {
    if input.resources.is_empty() {
        return Ok(resolve_input(&[], &[], input)?);
    }
    let types = load_types(conn, tenant_id).await?;
    let named = input
        .resources
        .iter()
        .filter_map(|(key, value)| {
            types
                .iter()
                .find(|t| t.keys.contains(key))
                .map(|t| (t.id, value.clone()))
        })
        .collect::<Vec<_>>();
    let stored = ResourceRow::load_by_keys(conn, tenant_id, customer_id, &named)
        .await?
        .into_iter()
        .map(Resource::try_from)
        .collect::<StoreResult<Vec<_>>>()?;
    Ok(resolve_input(&types, &stored, input)?)
}

/// The tenant's types with their keys, archived ones included.
pub async fn load_types(conn: &mut PgConn, tenant_id: TenantId) -> StoreResult<Vec<ResourceType>> {
    let rows = ResourceTypeRow::list(conn, tenant_id, true).await?;
    let mut keys = HashMap::<ResourceTypeId, Vec<String>>::new();
    for key in ResourceTypeKeyRow::list_by_tenant(conn, tenant_id).await? {
        keys.entry(key.resource_type_id).or_default().push(key.key);
    }
    rows.into_iter()
        .map(|row| {
            let type_keys = keys.remove(&row.id).unwrap_or_default();
            ResourceType::try_from((row, type_keys))
        })
        .collect()
}

async fn find_type(
    conn: &mut PgConn,
    tenant_id: TenantId,
    resource_type: &AliasOr<ResourceTypeId>,
) -> StoreResult<ResourceType> {
    load_types(conn, tenant_id)
        .await?
        .into_iter()
        .find(|t| match resource_type {
            AliasOr::Id(id) => t.id == *id,
            AliasOr::Alias(code) => t.code == *code,
        })
        .ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(format!(
                "resource type {resource_type}"
            )))
        })
}

fn invalid(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.into()))
}

fn is_slug(s: &str) -> bool {
    s.len() <= 64
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn refuse_parent(parent_type_id: Option<ResourceTypeId>) -> StoreResult<()> {
    match parent_type_id {
        Some(_) => Err(ResourceRefusal::HierarchyUnsupported
            .report("resource types sit directly below the customer: no parent type in v1")),
        None => Ok(()),
    }
}

fn validate_names(name: &str, plural_name: &str) -> StoreResult<()> {
    if name.trim().is_empty() || plural_name.trim().is_empty() {
        return Err(invalid("a resource type needs a name and a plural name"));
    }
    Ok(())
}

/// A key the type `holds` already passes as it is: usage filters converted to types keep their
/// event property, which need not be a slug.
fn validate_keys(keys: &[String], holds: &[String]) -> StoreResult<()> {
    if keys.is_empty() {
        return Err(invalid(
            "a resource type needs at least one identifying key",
        ));
    }
    if let Some(key) = keys.iter().find(|k| !is_slug(k) && !holds.contains(k)) {
        return Err(invalid(format!(
            "key `{key}` must be lowercase letters, digits and underscores"
        )));
    }
    if let Some(key) = keys
        .iter()
        .enumerate()
        .find_map(|(i, k)| keys[..i].contains(k).then_some(k))
    {
        return Err(invalid(format!("key `{key}` is listed twice")));
    }
    Ok(())
}

fn validate_attributes(attributes: &BTreeMap<String, Vec<String>>) -> StoreResult<()> {
    for (attribute, values) in attributes {
        if !is_slug(attribute) {
            return Err(invalid(format!(
                "attribute `{attribute}` must be lowercase letters, digits and underscores"
            )));
        }
        let distinct = values
            .iter()
            .enumerate()
            .all(|(i, v)| !values[..i].contains(v));
        if values.is_empty() || values.iter().any(|v| v.is_empty()) || !distinct {
            return Err(invalid(format!(
                "attribute `{attribute}` needs a list of distinct, non-empty values"
            )));
        }
    }
    Ok(())
}

/// Keys are unique per tenant across every type, archived ones included.
async fn check_key_clash(
    conn: &mut PgConn,
    tenant_id: TenantId,
    own: Option<ResourceTypeId>,
    keys: &[String],
) -> StoreResult<()> {
    let types = load_types(conn, tenant_id).await?;
    let clashes = types
        .iter()
        .filter(|t| Some(t.id) != own)
        .flat_map(|t| {
            t.keys
                .iter()
                .filter(|k| keys.contains(k))
                .map(move |k| format!("`{k}` (type `{}`)", t.code))
        })
        .collect::<Vec<_>>();
    if clashes.is_empty() {
        return Ok(());
    }
    Err(ResourceRefusal::KeyClash.report(format!(
        "keys are unique across the tenant's resource types, archived ones included: {} already used",
        clashes.join(", ")
    )))
}

async fn replace_keys(
    conn: &mut PgConn,
    tenant_id: TenantId,
    resource_type_id: ResourceTypeId,
    keys: &[String],
) -> StoreResult<()> {
    ResourceTypeKeyRow::replace_for_type(conn, tenant_id, resource_type_id, keys)
        .await
        .map_err(|err| {
            if matches!(err.error.current_context(), DatabaseError::UniqueViolation) {
                ResourceRefusal::KeyClash
                    .report("a key is already used by another resource type of the tenant")
            } else {
                err.into()
            }
        })?;
    Ok(())
}

#[derive(QueryableByName)]
struct ValueUsage {
    #[diesel(sql_type = sql_types::Text)]
    attribute: String,
    #[diesel(sql_type = sql_types::Text)]
    value: String,
    #[diesel(sql_type = sql_types::Uuid)]
    owner: uuid::Uuid,
    /// The resource's key value; `NULL` for an entitlement row.
    #[diesel(sql_type = sql_types::Nullable<sql_types::Text>)]
    key_value: Option<String>,
}

const VALUE_USAGES_SQL: &str = "
SELECT kv.key AS attribute, kv.value AS value, r.customer_id AS owner, r.key_value AS key_value
FROM resource r, jsonb_each_text(r.attributes) kv
WHERE r.tenant_id = $1 AND r.resource_type_id = $2 AND kv.key || '=' || kv.value = ANY ($3)
UNION ALL
SELECT substr(c.key, length($4) + 1), v.value, e.lineage_id, NULL
FROM entitlement e, jsonb_each(e.condition) c, jsonb_array_elements_text(c.value) v
WHERE e.tenant_id = $1 AND e.revoked_at IS NULL AND jsonb_typeof(e.condition) = 'object'
  AND left(c.key, length($4)) = $4 AND substr(c.key, length($4) + 1) || '=' || v.value = ANY ($3)
LIMIT 50";

/// Refuses removing attribute values a stored resource or an open entitlement condition uses.
async fn check_removed_values(
    conn: &mut PgConn,
    current: &ResourceType,
    next: &BTreeMap<String, Vec<String>>,
) -> StoreResult<()> {
    let removed = current
        .attributes
        .iter()
        .flat_map(|(a, values)| values.iter().map(move |v| (a, v)))
        .filter(|(a, v)| !next.get(*a).is_some_and(|values| values.contains(v)))
        .map(|(a, v)| format!("{a}={v}"))
        .collect::<Vec<_>>();
    if removed.is_empty() {
        return Ok(());
    }
    let usages: Vec<ValueUsage> = diesel::sql_query(VALUE_USAGES_SQL)
        .bind::<sql_types::Uuid, _>(current.tenant_id.as_uuid())
        .bind::<sql_types::Uuid, _>(current.id.as_uuid())
        .bind::<sql_types::Array<sql_types::Text>, _>(removed)
        .bind::<sql_types::Text, _>(format!("{}.", current.code))
        .load(conn)
        .await
        .map_err(|err| Report::new(StoreError::from(err)))?;
    if usages.is_empty() {
        return Ok(());
    }
    let listed = usages
        .iter()
        .map(|u| match &u.key_value {
            Some(key_value) => format!(
                "`{}={}` by resource `{key_value}` of customer {}",
                u.attribute,
                u.value,
                CustomerId::from(u.owner)
            ),
            None => format!(
                "`{}={}` by entitlement {}",
                u.attribute,
                u.value,
                EntitlementId::from(u.owner)
            ),
        })
        .collect::<Vec<_>>();
    Err(Report::new(StoreError::FailedPrecondition(format!(
        "these attribute values are still used: {}",
        listed.join("; ")
    ))))
}

/// What still names the resource: subscriptions other than aborted ones (an ended one keeps
/// its scope on its drafts), quotes and checkout sessions, each of which would become customer-wide.
async fn referenced_by(
    conn: &mut PgConn,
    tenant_id: TenantId,
    id: ResourceId,
) -> StoreResult<Vec<&'static str>> {
    use diesel::dsl::{exists, select};
    use diesel_models::schema::checkout_session::dsl as cs;
    use diesel_models::schema::quote::dsl as q;
    use diesel_models::schema::subscription::dsl as s;

    let (subscription, quote, checkout) = select((
        exists(
            s::subscription
                .filter(s::tenant_id.eq(tenant_id.as_uuid()))
                .filter(s::resource_id.eq(id.as_uuid()))
                .filter(s::status.ne(SubscriptionStatusEnum::Aborted)),
        ),
        exists(
            q::quote
                .filter(q::tenant_id.eq(tenant_id.as_uuid()))
                .filter(q::resource_id.eq(id.as_uuid())),
        ),
        exists(
            cs::checkout_session
                .filter(cs::tenant_id.eq(tenant_id.as_uuid()))
                .filter(cs::resource_id.eq(id.as_uuid())),
        ),
    ))
    .get_result::<(bool, bool, bool)>(conn)
    .await
    .map_err(|err| Report::new(StoreError::from(err)))?;
    Ok([
        (subscription, "subscriptions"),
        (quote, "quotes"),
        (checkout, "checkout sessions"),
    ]
    .into_iter()
    .filter_map(|(found, what)| found.then_some(what))
    .collect())
}

/// Aborted subscriptions never bill, so they let go of the resource instead of keeping it.
async fn release_aborted(
    conn: &mut PgConn,
    tenant_id: TenantId,
    id: ResourceId,
) -> StoreResult<()> {
    use diesel_models::schema::subscription::dsl as s;

    diesel::update(
        s::subscription
            .filter(s::tenant_id.eq(tenant_id.as_uuid()))
            .filter(s::resource_id.eq(id.as_uuid()))
            .filter(s::status.eq(SubscriptionStatusEnum::Aborted)),
    )
    .set(s::resource_id.eq(None::<uuid::Uuid>))
    .execute(conn)
    .await
    .map(|_| ())
    .map_err(|err| Report::new(StoreError::from(err)))
}

fn attributes_json<T: serde::Serialize>(attributes: &T) -> StoreResult<serde_json::Value> {
    serde_json::to_value(attributes)
        .map_err(|e| Report::new(StoreError::SerdeError("resource attributes".into(), e)))
}

#[async_trait::async_trait]
impl ResourcesInterface for Store {
    async fn list_resource_types(
        &self,
        tenant_id: TenantId,
        include_archived: bool,
    ) -> StoreResult<Vec<ResourceType>> {
        let mut conn = self.get_conn().await?;
        let mut types = load_types(&mut conn, tenant_id).await?;
        types.retain(|t| include_archived || t.archived_at.is_none());
        Ok(types)
    }

    async fn get_resource_type(
        &self,
        tenant_id: TenantId,
        resource_type: AliasOr<ResourceTypeId>,
    ) -> StoreResult<ResourceType> {
        let mut conn = self.get_conn().await?;
        find_type(&mut conn, tenant_id, &resource_type).await
    }

    async fn create_resource_type(&self, new: ResourceTypeNew) -> StoreResult<ResourceType> {
        refuse_parent(new.parent_type_id)?;
        if !is_slug(&new.code) {
            return Err(invalid(format!(
                "code `{}` must be lowercase letters, digits and underscores",
                new.code
            )));
        }
        validate_names(&new.name, &new.plural_name)?;
        validate_keys(&new.keys, &[])?;
        validate_attributes(&new.attributes)?;
        let row = ResourceTypeRowNew {
            id: ResourceTypeId::new(),
            tenant_id: new.tenant_id,
            code: new.code,
            name: new.name,
            plural_name: new.plural_name,
            attributes: attributes_json(&new.attributes)?,
        };
        let keys = new.keys;

        self.transaction(|conn| {
            async move {
                check_key_clash(conn, row.tenant_id, None, &keys).await?;
                let code = row.code.clone();
                let inserted = row.insert(conn).await.map_err(|err| {
                    if matches!(err.error.current_context(), DatabaseError::UniqueViolation) {
                        Report::new(StoreError::DuplicateValue {
                            entity: "resource type",
                            key: Some(code),
                        })
                    } else {
                        err.into()
                    }
                })?;
                replace_keys(conn, inserted.tenant_id, inserted.id, &keys).await?;
                ResourceType::try_from((inserted, keys))
            }
            .scope_boxed()
        })
        .await
    }

    async fn update_resource_type(
        &self,
        tenant_id: TenantId,
        resource_type: AliasOr<ResourceTypeId>,
        patch: ResourceTypePatch,
    ) -> StoreResult<ResourceType> {
        refuse_parent(patch.parent_type_id)?;
        if let Some(attributes) = &patch.attributes {
            validate_attributes(attributes)?;
        }

        self.transaction(|conn| {
            async move {
                let mut current = find_type(conn, tenant_id, &resource_type).await?;
                if patch.keys.is_some() {
                    // Serializes with the sales of its resources; read again once they committed.
                    ResourceTypeRow::lock_keys_exclusive(conn, current.id).await?;
                    current = find_type(conn, tenant_id, &AliasOr::Id(current.id)).await?;
                }
                if let Some(keys) = &patch.keys {
                    validate_keys(keys, &current.keys)?;
                }
                validate_names(
                    patch.name.as_deref().unwrap_or(&current.name),
                    patch.plural_name.as_deref().unwrap_or(&current.plural_name),
                )?;
                if let Some(attributes) = &patch.attributes {
                    check_removed_values(conn, &current, attributes).await?;
                }
                if let Some(keys) = &patch.keys
                    && *keys != current.keys
                {
                    crate::repositories::subscriptions::usage_attribution::refuse_key_change(
                        conn, tenant_id, current.id,
                    )
                    .await?;
                }
                if let Some(keys) = &patch.keys {
                    check_key_clash(conn, tenant_id, Some(current.id), keys).await?;
                    replace_keys(conn, tenant_id, current.id, keys).await?;
                }
                let row = ResourceTypeRow::update(
                    conn,
                    tenant_id,
                    current.id,
                    &ResourceTypeRowPatch {
                        name: patch.name,
                        plural_name: patch.plural_name,
                        attributes: patch.attributes.as_ref().map(attributes_json).transpose()?,
                        archived_at: patch.archived.map(|archived| {
                            archived.then(|| current.archived_at.unwrap_or_else(common_clock::now))
                        }),
                        updated_at: common_clock::now(),
                    },
                )
                .await?;
                ResourceType::try_from((row, patch.keys.unwrap_or(current.keys)))
            }
            .scope_boxed()
        })
        .await
    }

    async fn upsert_resource(&self, upsert: ResourceUpsert) -> StoreResult<Resource> {
        if upsert.key_value.is_empty() {
            return Err(invalid("a resource needs a key value"));
        }
        // Limits count a resource's usage by its key: one they can't group by would go unmonitored.
        if validate_group_value(&upsert.key_value).is_err() {
            return Err(invalid(format!(
                "a resource key value is at most {MAX_GROUP_VALUE_BYTES} bytes of text, without control \
                 characters or surrounding spaces"
            )));
        }
        self.transaction(|conn| {
            async move {
                let ResourceUpsert {
                    tenant_id,
                    customer_id,
                    resource_type,
                    key_value,
                    name,
                    attributes,
                    archived,
                } = upsert;
                CustomerRow::find_by_id(conn, &customer_id, &tenant_id).await?;
                let resource_type = find_type(conn, tenant_id, &resource_type).await?;
                resource_type.validate_attributes(&attributes)?;

                let count =
                    ResourceRow::count_locked(conn, tenant_id, customer_id, resource_type.id)
                        .await?;
                let existing = ResourceRow::load_by_keys(
                    conn,
                    tenant_id,
                    customer_id,
                    &[(resource_type.id, key_value.clone())],
                )
                .await?
                .into_iter()
                .next();
                if existing.is_none() {
                    if resource_type.archived_at.is_some() {
                        return Err(ResourceRefusal::TypeArchived.report(format!(
                            "resource type `{}` is archived: it takes no new resources",
                            resource_type.code
                        )));
                    }
                    let max = TenantRow::find_by_id(conn, tenant_id)
                        .await?
                        .max_resources_per_customer_type;
                    if count >= i64::from(max) {
                        return Err(ResourceRefusal::LimitReached.report(format!(
                            "a customer holds at most {max} stored `{}` resources",
                            resource_type.code
                        )));
                    }
                }

                let archived_at = archived.then(|| {
                    existing
                        .as_ref()
                        .and_then(|r| r.archived_at)
                        .unwrap_or_else(common_clock::now)
                });
                let attributes = attributes_json(&attributes)?;
                // Conditions read attributes: a new or changed set moves the monitors' tiers.
                if existing.as_ref().map(|r| &r.attributes) != Some(&attributes) {
                    CreditRecomputeMarkRow::mark_one(conn, tenant_id, customer_id).await?;
                }
                ResourceRowNew {
                    id: ResourceId::new(),
                    tenant_id,
                    customer_id,
                    resource_type_id: resource_type.id,
                    key_value,
                    name,
                    attributes,
                    archived_at,
                }
                .upsert(conn)
                .await?
                .try_into()
            }
            .scope_boxed()
        })
        .await
    }

    async fn list_resources(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        resource_type: Option<AliasOr<ResourceTypeId>>,
        include_archived: bool,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<Resource>> {
        let mut conn = self.get_conn().await?;
        let resource_type_id = match &resource_type {
            Some(r) => Some(find_type(&mut conn, tenant_id, r).await?.id),
            None => None,
        };
        let page = ResourceRow::list(
            &mut conn,
            tenant_id,
            customer_id,
            resource_type_id,
            include_archived,
            pagination.into(),
        )
        .await?;
        Ok(PaginatedVec {
            items: page
                .items
                .into_iter()
                .map(Resource::try_from)
                .collect::<StoreResult<_>>()?,
            total_pages: page.total_pages,
            total_results: page.total_results,
        })
    }

    async fn list_customer_resources_after(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        resource_type_id: Option<ResourceTypeId>,
        include_archived: bool,
        after: Option<ResourceId>,
        limit: usize,
    ) -> StoreResult<Vec<Resource>> {
        let mut conn = self.get_conn().await?;
        ResourceRow::list_after(
            &mut conn,
            tenant_id,
            customer_id,
            resource_type_id,
            include_archived,
            after,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await?
        .into_iter()
        .map(Resource::try_from)
        .collect()
    }

    async fn get_resources_by_ids(
        &self,
        tenant_id: TenantId,
        ids: &[ResourceId],
    ) -> StoreResult<Vec<Resource>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let mut conn = self.get_conn().await?;
        ResourceRow::find_by_ids(&mut conn, tenant_id, ids)
            .await?
            .into_iter()
            .map(Resource::try_from)
            .collect()
    }

    async fn get_resource(&self, tenant_id: TenantId, id: ResourceId) -> StoreResult<Resource> {
        let mut conn = self.get_conn().await?;
        ResourceRow::find_by_id(&mut conn, tenant_id, id)
            .await?
            .ok_or_else(|| Report::new(StoreError::ValueNotFound(format!("resource {id}"))))?
            .try_into()
    }

    async fn find_resource(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        resource_type: AliasOr<ResourceTypeId>,
        key_value: &str,
    ) -> StoreResult<Resource> {
        let mut conn = self.get_conn().await?;
        let resource_type = find_type(&mut conn, tenant_id, &resource_type).await?;
        ResourceRow::load_by_keys(
            &mut conn,
            tenant_id,
            customer_id,
            &[(resource_type.id, key_value.to_owned())],
        )
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(format!(
                "resource `{key_value}` of type `{}`",
                resource_type.code
            )))
        })?
        .try_into()
    }

    async fn set_resource_archived(
        &self,
        tenant_id: TenantId,
        id: ResourceId,
        archived: bool,
    ) -> StoreResult<Resource> {
        let current = self.get_resource(tenant_id, id).await?;
        let archived_at = archived.then(|| current.archived_at.unwrap_or_else(common_clock::now));
        let mut conn = self.get_conn().await?;
        ResourceRow::set_archived(&mut conn, tenant_id, id, archived_at)
            .await?
            .try_into()
    }

    async fn delete_resource(&self, tenant_id: TenantId, id: ResourceId) -> StoreResult<()> {
        self.transaction(|conn| {
            async move {
                let found = ResourceRow::find_by_id(conn, tenant_id, id)
                    .await?
                    .ok_or_else(|| {
                        Report::new(StoreError::ValueNotFound(format!("resource {id}")))
                    })?;
                // Locked before the check: an override created meanwhile is seen, or waits.
                let lock = CustomerRow::lock(conn, tenant_id, found.customer_id).await?;
                let resource =
                    ResourceRow::lock_by_id(conn, &lock, id)
                        .await?
                        .ok_or_else(|| {
                            Report::new(StoreError::ValueNotFound(format!("resource {id}")))
                        })?;
                if EntitlementRow::exists_for_resource(conn, tenant_id, id).await? {
                    return Err(Report::new(StoreError::FailedPrecondition(
                        "this resource has overrides, current or past: archive it instead".into(),
                    )));
                }
                let referenced = referenced_by(conn, tenant_id, id).await?;
                if !referenced.is_empty() {
                    return Err(Report::new(StoreError::FailedPrecondition(format!(
                        "{} are for this resource: archive it instead",
                        referenced.join(", ")
                    ))));
                }
                release_aborted(conn, tenant_id, id).await?;
                ResourceRow::delete(conn, tenant_id, id).await?;
                CreditRecomputeMarkRow::mark_one(conn, tenant_id, resource.customer_id).await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn resolve_resource_input(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        input: &ResourceInput,
    ) -> StoreResult<Option<ResolvedResource>> {
        let mut conn = self.get_conn().await?;
        load_for_check(&mut conn, tenant_id, customer_id, input).await
    }
}

impl TryFrom<(ResourceTypeRow, Vec<String>)> for ResourceType {
    type Error = Report<StoreError>;

    fn try_from((row, keys): (ResourceTypeRow, Vec<String>)) -> Result<Self, Self::Error> {
        Ok(ResourceType {
            id: row.id,
            tenant_id: row.tenant_id,
            code: row.code,
            name: row.name,
            plural_name: row.plural_name,
            keys,
            attributes: serde_json::from_value(row.attributes).map_err(|e| {
                Report::new(StoreError::SerdeError("resource type attributes".into(), e))
            })?,
            archived_at: row.archived_at,
            created_at: row.created_at,
        })
    }
}

impl TryFrom<ResourceRow> for Resource {
    type Error = Report<StoreError>;

    fn try_from(row: ResourceRow) -> Result<Self, Self::Error> {
        Ok(Resource {
            id: row.id,
            customer_id: row.customer_id,
            resource_type_id: row.resource_type_id,
            key_value: row.key_value,
            name: row.name,
            attributes: serde_json::from_value(row.attributes).map_err(|e| {
                Report::new(StoreError::SerdeError("resource attributes".into(), e))
            })?,
            archived_at: row.archived_at,
        })
    }
}
