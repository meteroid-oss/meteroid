//! Features and their rows (ADR 0011 §3.1, §3.2). Rows are addressed by lineage, and every edit
//! revokes the open row and inserts a successor.
use crate::domain::entitlement_checks::ResolvedCustomer;
use crate::domain::entitlement_fold::{
    Condition, Contribution, Effect, EvalLevel, GrantValue, RowLevel, Tier, fold,
};
use crate::domain::entitlements::{
    EntitlementSpec, FeatureKind, FeatureRef, ResolvedEntitlement, ResolvedEntitlementValue,
    ResolvedOrigin, ValueType, from_json, to_json,
};
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::{FeatureStatusEnum, SubscriptionStatusEnum};
use crate::domain::prices::FeeStructure;
use crate::domain::resources::Attributes;
use crate::domain::{
    Entitlement, EntitlementNew, EntitlementUpdate, Feature, FeatureListOrder, FeatureNew,
    FeatureUpdate, PaginatedVec, PaginationRequest,
};
use crate::errors::StoreError;
use crate::repositories::credit::inputs::InputsOwner;
use crate::repositories::resources::ResourceRefusal;
use crate::services::entitlements::resolve::{add_on_quantities, held_units, seat_quantities};
use crate::services::entitlements::{readout_of, winning};
use crate::store::PgConn;
use crate::{Store, StoreResult};
use common_domain::ids::{
    AddOnId, AliasOr, BaseId, CustomerId, EntitlementEntityId, EntitlementId, FeatureId,
    ManagedQuantityId, PlanId, PlanVersionId, ProductId, QuoteId, ResourceId, ResourceTypeId,
    SubscriptionId, TenantId,
};
use diesel_models::add_ons::AddOnRow;
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::customer_credit::CreditRecomputeMarkRow;
use diesel_models::entitlement_app_counts::EntitlementAppCountRow;
use diesel_models::entitlements::{
    EntitlementRow, EntitlementRowNew, FeatureRow, FeatureRowNew, FeatureRowPatch,
};
use diesel_models::enums::{
    ActorTypeEnum, EntitlementEffectEnum, EntitlementEntityTypeEnum, EntitlementLevelEnum,
    EntitlementModeEnum, FeatureStatusEnum as DbFeatureStatusEnum,
};
use diesel_models::errors::{DatabaseError, DatabaseErrorContainer};
use diesel_models::managed_quantities::ManagedQuantityRow;
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::plans::PlanRow;
use diesel_models::products::ProductRow;
use diesel_models::query::entity_display::{
    EntitlementSourceIds, EntitlementSourceNames, entitlement_source_names,
};
use diesel_models::query::managed_quantities::{
    feature_limit_quantities, set_feature_limit_quantity,
};
use diesel_models::query::price_components::list_product_ids_for_plan_version;
use diesel_models::quote_add_ons::QuoteAddOnRow;
use diesel_models::quotes::QuoteComponentRow;
use diesel_models::quotes::QuoteRow;
use diesel_models::resources::{ResourceRow, ResourceTypeRow};
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use itertools::Itertools;
use scoped_futures::ScopedFutureExt;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Dispatcher target for `resolve_entitlements_for_entity`; `Product` scopes by
/// `feature.product_id` only.
#[derive(Clone, Debug)]
pub enum ResolveTarget {
    Product(ProductId),
    AddOn(AddOnId),
    PlanVersion(PlanVersionId),
    Subscription(SubscriptionId),
    Quote(QuoteId),
    /// In-flight selection during quote or subscription creation.
    Selection {
        plan_version_id: PlanVersionId,
        add_on_ids: Vec<AddOnId>,
        extra_product_ids: Vec<ProductId>,
    },
}

#[async_trait::async_trait]
pub trait EntitlementsInterface {
    async fn create_feature(&self, actor: Actor, feature: FeatureNew) -> StoreResult<Feature>;

    async fn get_feature(&self, id: FeatureId, tenant_id: TenantId) -> StoreResult<Feature>;

    async fn get_feature_by_id_or_code(
        &self,
        id_or_code: AliasOr<FeatureId>,
        tenant_id: TenantId,
    ) -> StoreResult<Feature>;

    /// `level`: `Some(None)` lists the features applying per customer.
    async fn list_features(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        statuses: Option<Vec<FeatureStatusEnum>>,
        product_id: Option<ProductId>,
        search: Option<String>,
        level: Option<Option<ResourceTypeId>>,
        order: FeatureListOrder,
    ) -> StoreResult<PaginatedVec<Feature>>;

    /// The non-archived features among `ids`, with their default rows.
    async fn list_features_by_ids(
        &self,
        ids: &[FeatureId],
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Feature>>;

    async fn update_feature(
        &self,
        id: FeatureId,
        tenant_id: TenantId,
        update: FeatureUpdate,
    ) -> StoreResult<Feature>;

    /// `create_feature` under the id the caller chose, with the managed quantity its limit counts
    /// per unit of (R-133) and its default rows (`defaults`, each of this feature), in one
    /// transaction. `feature.default` is one more default row.
    async fn create_feature_with_defaults(
        &self,
        actor: &Actor,
        id: FeatureId,
        feature: FeatureNew,
        per_unit_of: Option<ManagedQuantityId>,
        defaults: Vec<EntitlementSpec>,
    ) -> StoreResult<Feature>;

    /// `update_feature` when `update` is set, the managed quantity its limit counts per unit of
    /// when `per_unit_of` is (`Some(None)`: per subscription), and the replacement of the
    /// feature's default rows (`replace_entitlements` on the feature) when `defaults` is, in one
    /// transaction: a refusal of any writes none. The defaults are checked against the feature as
    /// updated.
    async fn update_feature_with_defaults(
        &self,
        actor: &Actor,
        id: FeatureId,
        tenant_id: TenantId,
        update: Option<FeatureUpdate>,
        per_unit_of: Option<Option<ManagedQuantityId>>,
        defaults: Option<Vec<EntitlementSpec>>,
    ) -> StoreResult<Feature>;

    /// Only Active features resolve. Archiving releases the code and hides the rows; restoring
    /// fails with `FailedPrecondition` while another feature holds the code.
    async fn set_feature_status(
        &self,
        actor: Actor,
        id: FeatureId,
        tenant_id: TenantId,
        status: FeatureStatusEnum,
    ) -> StoreResult<()>;

    /// The archived feature that used `code`; `FailedPrecondition` while a non-archived feature
    /// holds it or several archived ones used it.
    async fn find_archived_feature_by_code(
        &self,
        code: &str,
        tenant_id: TenantId,
    ) -> StoreResult<FeatureId>;

    /// For an archived feature, the non-archived feature now using its code.
    async fn feature_code_holder(
        &self,
        id: FeatureId,
        tenant_id: TenantId,
    ) -> StoreResult<Option<FeatureRef>>;

    async fn create_entitlement(
        &self,
        actor: &Actor,
        entitlement: EntitlementNew,
    ) -> StoreResult<Entitlement>;

    /// Creates rows on one owner, all or none.
    async fn create_entitlements(
        &self,
        actor: &Actor,
        tenant_id: TenantId,
        entity: EntitlementEntityId,
        resource_id: Option<ResourceId>,
        specs: Vec<EntitlementSpec>,
    ) -> StoreResult<Vec<Entitlement>>;

    /// The open row of the lineage.
    async fn get_entitlement(
        &self,
        id: EntitlementId,
        tenant_id: TenantId,
    ) -> StoreResult<Entitlement>;

    /// Every version of the lineage, oldest first.
    async fn get_entitlement_history(
        &self,
        id: EntitlementId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Entitlement>>;

    async fn list_entitlements_by_feature(
        &self,
        feature_id: FeatureId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Entitlement>>;

    async fn list_entitlements_by_entity(
        &self,
        entity: EntitlementEntityId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Entitlement>>;

    /// Revokes the open row and inserts its successor, in one transaction.
    async fn update_entitlement(
        &self,
        actor: &Actor,
        id: EntitlementId,
        tenant_id: TenantId,
        update: EntitlementUpdate,
    ) -> StoreResult<Entitlement>;

    /// Revokes the open row; its history stays.
    async fn delete_entitlement(
        &self,
        actor: &Actor,
        id: EntitlementId,
        tenant_id: TenantId,
    ) -> StoreResult<()>;

    /// Creates rows on one owner, skipping those clashing with an open row.
    async fn batch_create_entitlements(
        &self,
        actor: &Actor,
        tenant_id: TenantId,
        target: EntitlementEntityId,
        specs: Vec<EntitlementSpec>,
    ) -> StoreResult<Vec<Entitlement>>;

    /// The legacy customer-wide readout: each feature's own level without a resource, as
    /// `resolve_customer_entitlements` decides it. Limits counted by the app are left out.
    async fn resolve_entitlements_for_customer(
        &self,
        conn: &mut PgConn,
        customer_id: CustomerId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ResolvedEntitlement>>;

    /// A preview of one target's rows at each feature's own level, without conditions or
    /// resource overrides.
    async fn resolve_entitlements_for_entity(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        target: ResolveTarget,
    ) -> StoreResult<Vec<ResolvedEntitlement>>;

    /// The tenant's features of `filter` after the feature `after` in the filter's order, at
    /// most `limit`, each with its default rows: a keyset page. An `after` that is not a feature
    /// of the tenant is refused.
    async fn list_features_after(
        &self,
        tenant_id: TenantId,
        filter: FeaturePageFilter,
        after: Option<FeatureId>,
        limit: usize,
    ) -> StoreResult<Vec<Feature>>;

    /// Makes the owner's open rows of live features exactly `specs`, in one transaction under the
    /// owner's lock: a row a spec repeats is kept, a changed one gets a successor in its lineage,
    /// one left out is revoked, and a new spec is inserted. A spec is matched to a row by
    /// feature, level and condition. Rows of archived features stay hidden as they are; naming
    /// one is refused. The owner is an add-on, a plan, a draft plan version, a feature (its
    /// defaults) or a subscription not ended (its overrides); a draft's replaced rows are
    /// deleted, a published version is refused (write it through a draft).
    async fn replace_entitlements(
        &self,
        actor: &Actor,
        tenant_id: TenantId,
        entity: EntitlementEntityId,
        specs: Vec<EntitlementSpec>,
    ) -> StoreResult<Vec<Entitlement>>;

    /// Replaces the default rows of the product's live features with `specs`, in one
    /// transaction: each spec names a live feature of the product, and a feature left out loses
    /// its defaults.
    async fn set_product_feature_defaults(
        &self,
        actor: &Actor,
        tenant_id: TenantId,
        product_id: ProductId,
        specs: Vec<EntitlementSpec>,
    ) -> StoreResult<()>;
}

/// What a keyset page of features is narrowed to, and its order.
#[derive(Debug, Clone, Default)]
pub struct FeaturePageFilter {
    pub statuses: Option<Vec<FeatureStatusEnum>>,
    pub product_id: Option<ProductId>,
    /// `Some(None)` keeps the features applying per customer.
    pub level: Option<Option<ResourceTypeId>>,
    /// `None` pages by id.
    pub order: Option<FeatureListOrder>,
}

fn invalid(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.into()))
}

/// A feature may only be attached to a product of its own tenant.
async fn ensure_product_in_tenant(
    conn: &mut PgConn,
    tenant_id: TenantId,
    product_id: Option<ProductId>,
) -> StoreResult<()> {
    let Some(product_id) = product_id else {
        return Ok(());
    };
    ProductRow::find_by_id_and_tenant_id(conn, product_id, tenant_id)
        .await
        .map_err(|e| match e.error.current_context() {
            diesel_models::errors::DatabaseError::NotFound => Report::new(
                StoreError::InvalidArgument(format!("product {product_id} not found")),
            ),
            _ => e.into(),
        })?;
    Ok(())
}

/// A select loses its order only while its rows are all ones an unordered value takes (§3.2).
async fn ensure_rows_take_no_order(conn: &mut PgConn, feature: &Feature) -> StoreResult<()> {
    let own = feature.own_level();
    let needing_order = EntitlementRow::list_by_feature(conn, feature.id, feature.tenant_id)
        .await?
        .into_iter()
        .filter(|row| {
            let level = RowLevel::from_columns(row.level, row.level_type_id);
            row.effect != EntitlementEffectEnum::Set || level.eval(feature.level_type_id) != own
        })
        .count();
    if needing_order > 0 {
        return Err(invalid(format!(
            "{needing_order} row(s) need an order (an effect other than Set to, or another level): revoke them first"
        )));
    }
    Ok(())
}

/// The lineage's open row, locked. A lineage another request edited while this one waited for
/// the lock is a conflict, not a miss.
async fn lock_open(
    conn: &mut PgConn,
    tenant_id: TenantId,
    id: EntitlementId,
) -> StoreResult<EntitlementRow> {
    match EntitlementRow::lock_open(conn, tenant_id, id).await {
        Err(e)
            if matches!(
                e.error.current_context(),
                diesel_models::errors::DatabaseError::NotFound
            ) && EntitlementRow::has_open(conn, tenant_id, id).await? =>
        {
            Err(Report::new(StoreError::Conflict(
                "the row was edited meanwhile: read it again".into(),
            )))
        }
        locked => Ok(locked?),
    }
}

fn ensure_editable(feature: &Feature) -> StoreResult<()> {
    match feature.status {
        FeatureStatusEnum::Archived => Err(invalid("feature is archived; unarchive it first")),
        FeatureStatusEnum::Active | FeatureStatusEnum::Disabled => Ok(()),
    }
}

/// An archived type keeps resolving but takes no new rows or features (§3.4).
async fn active_type(
    conn: &mut PgConn,
    tenant_id: TenantId,
    type_id: ResourceTypeId,
) -> StoreResult<ResourceTypeRow> {
    let row = ResourceTypeRow::find_by_id(conn, tenant_id, type_id).await?;
    match row.archived_at {
        Some(_) => {
            Err(ResourceRefusal::TypeArchived
                .report(format!("resource type {} is archived", row.code)))
        }
        None => Ok(row),
    }
}

/// A limit counted from events applies per type only on a metric attributed to that type.
async fn ensure_metric_attributed(
    conn: &mut PgConn,
    tenant_id: TenantId,
    kind: &FeatureKind,
    type_row: &ResourceTypeRow,
) -> StoreResult<()> {
    let Some(metric_id) = kind.metric_id() else {
        return Ok(());
    };
    let attributed = BillableMetricRow::get_by_ids(conn, &[metric_id], &tenant_id)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| invalid(format!("metric {metric_id} not found")))?
        .attributed_to_type_id;
    if attributed == Some(type_row.id) {
        Ok(())
    } else {
        Err(invalid(format!(
            "a limit counted from events applies per {} only on a metric attributed to it",
            type_row.code
        )))
    }
}

/// The kind, policy and level of a feature as written.
async fn validate_feature(
    conn: &mut PgConn,
    tenant_id: TenantId,
    kind: &FeatureKind,
    policy: Option<&crate::domain::entitlement_policy::FeaturePolicy>,
    level_type_id: Option<ResourceTypeId>,
) -> StoreResult<()> {
    if let FeatureKind::Value(ValueType::Select { options, .. }) = kind
        && (options.is_empty() || options.iter().unique().count() != options.len())
    {
        return Err(invalid("a select needs distinct options"));
    }
    match (kind, policy) {
        (FeatureKind::Limit(_), Some(policy)) => policy.validate(kind.counted_by_app())?,
        (_, Some(_)) => return Err(invalid("only a limit has a policy")),
        (_, None) => {}
    }
    if let Some(type_id) = level_type_id {
        let type_row = active_type(conn, tenant_id, type_id).await?;
        ensure_metric_attributed(conn, tenant_id, kind, &type_row).await?;
    }
    Ok(())
}

/// What the actor writes into `created_by` and `created_by_kind`.
fn author(actor: Option<&Actor>) -> (Option<uuid::Uuid>, Option<ActorTypeEnum>) {
    actor.map_or((None, None), |a| {
        (a.as_uuid(), Some(ActorTypeEnum::from(a.actor_type())))
    })
}

/// Whether an add-on's rows Add by default: it is bought several times, or it charges per seat
/// and its rows count the seats held (R-138).
async fn is_addon_additive(
    conn: &mut PgConn,
    add_on_id: AddOnId,
    tenant_id: TenantId,
) -> StoreResult<bool> {
    let add_on = AddOnRow::list_by_ids(conn, &[add_on_id], &tenant_id)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| invalid(format!("add-on {add_on_id} not found")))?;
    let per_seat = matches!(
        serde_json::from_value::<FeeStructure>(add_on.fee_structure),
        Ok(FeeStructure::Slot { .. })
    );
    Ok(per_seat || add_on.max_instances_per_subscription.is_none_or(|n| n > 1))
}

/// The plan of a plan or plan version owner.
async fn owner_plan(
    conn: &mut PgConn,
    tenant_id: TenantId,
    entity: &EntitlementEntityId,
) -> StoreResult<Option<PlanRow>> {
    let plan_id = match entity {
        EntitlementEntityId::Plan(id) => *id,
        EntitlementEntityId::PlanVersion(id) => {
            match PlanVersionRow::get_plan_ids_by_version_ids(conn, &[*id])
                .await?
                .get(id)
            {
                Some(plan_id) => *plan_id,
                None => return Ok(None),
            }
        }
        _ => return Ok(None),
    };
    Ok(PlanRow::list_by_ids(conn, &[plan_id], tenant_id)
        .await?
        .into_iter()
        .next())
}

/// An owner known to exist in the tenant. Customers and subscriptions are checked by the lock
/// `InputsOwner::of_entitlement` takes before the insert; a feature by `new_row`.
#[derive(Clone, Copy)]
struct CheckedOwner(EntitlementEntityId);

impl CheckedOwner {
    /// An existing owner whose rows are written: a published plan version's are fixed, written
    /// through a draft (the plan's own rows apply to every version).
    async fn check(
        conn: &mut PgConn,
        tenant_id: TenantId,
        entity: EntitlementEntityId,
    ) -> StoreResult<Self> {
        Self::verify(conn, tenant_id, entity, false).await
    }

    /// An owner the caller's transaction creates with its rows: a version published at once.
    async fn being_created(
        conn: &mut PgConn,
        tenant_id: TenantId,
        entity: EntitlementEntityId,
    ) -> StoreResult<Self> {
        Self::verify(conn, tenant_id, entity, true).await
    }

    async fn verify(
        conn: &mut PgConn,
        tenant_id: TenantId,
        entity: EntitlementEntityId,
        created: bool,
    ) -> StoreResult<Self> {
        let missing = || invalid(format!("{} not found", owner_label(&entity)));
        let not_found_as_invalid = |e: DatabaseErrorContainer| match e.error.current_context() {
            DatabaseError::NotFound => missing(),
            _ => e.into(),
        };
        match entity {
            EntitlementEntityId::Plan(id) => {
                if PlanRow::list_by_ids(conn, &[id], tenant_id)
                    .await?
                    .is_empty()
                {
                    return Err(missing());
                }
            }
            EntitlementEntityId::PlanVersion(id) => {
                let version = PlanVersionRow::list_by_ids_and_tenant_id(conn, &[id], tenant_id)
                    .await?
                    .into_iter()
                    .next()
                    .ok_or_else(missing)?;
                if !created && !version.is_draft_version {
                    return Err(Report::new(StoreError::FailedPrecondition(
                        "a published version's entitlements are fixed: write them through a draft"
                            .into(),
                    )));
                }
            }
            EntitlementEntityId::AddOn(id) => {
                AddOnRow::get_by_id(conn, tenant_id, id)
                    .await
                    .map_err(not_found_as_invalid)?;
            }
            EntitlementEntityId::Quote(id) => {
                QuoteRow::find_by_id(conn, tenant_id, id)
                    .await
                    .map_err(not_found_as_invalid)?;
            }
            EntitlementEntityId::Feature(_)
            | EntitlementEntityId::Customer(_)
            | EntitlementEntityId::Subscription(_) => {}
        }
        Ok(Self(entity))
    }
}

fn owner_label(entity: &EntitlementEntityId) -> String {
    match entity {
        EntitlementEntityId::Feature(id) => format!("feature {id}"),
        EntitlementEntityId::Customer(id) => format!("customer {id}"),
        EntitlementEntityId::Plan(id) => format!("plan {id}"),
        EntitlementEntityId::PlanVersion(id) => format!("plan version {id}"),
        EntitlementEntityId::AddOn(id) => format!("add-on {id}"),
        EntitlementEntityId::Subscription(id) => format!("subscription {id}"),
        EntitlementEntityId::Quote(id) => format!("quote {id}"),
    }
}

/// A condition names attributes of the level's resource, declared with these values (§3.2).
fn validate_condition(condition: &Condition, level: Option<&ResourceTypeRow>) -> StoreResult<()> {
    let Some(type_row) = level else {
        return Err(invalid(
            "a condition matches attributes of the level's resource; the customer level has none",
        ));
    };
    let declared: BTreeMap<String, Vec<String>> =
        from_json(type_row.attributes.clone(), "resource type attributes")?;
    let prefix = format!("{}.", type_row.code);
    if condition.0.is_empty() {
        return Err(invalid("a condition needs at least one attribute"));
    }
    condition.0.iter().try_for_each(|(qualified, values)| {
        let allowed = qualified
            .strip_prefix(&prefix)
            .and_then(|attribute| declared.get(attribute))
            .ok_or_else(|| {
                invalid(format!(
                    "`{qualified}` is not an attribute of {}",
                    type_row.code
                ))
            })?;
        match values.iter().find(|v| !allowed.contains(v)) {
            _ if values.is_empty() => Err(invalid(format!("`{qualified}` needs a value"))),
            Some(v) => Err(invalid(format!("`{v}` is not a value of `{qualified}`"))),
            None => Ok(()),
        }
    })
}

/// Validates a row against its feature (§3.1–§3.3, §3.5) and builds it; `lineage` keeps the
/// lineage of an edit.
#[allow(clippy::too_many_arguments)]
async fn new_row(
    conn: &mut PgConn,
    tenant_id: TenantId,
    feature: &Feature,
    owner: CheckedOwner,
    resource_id: Option<ResourceId>,
    spec: EntitlementSpec,
    actor: Option<&Actor>,
    lineage: Option<EntitlementId>,
) -> StoreResult<EntitlementRowNew> {
    let entity = owner.0;
    if matches!(entity, EntitlementEntityId::Feature(id) if id != feature.id) {
        return Err(invalid("a feature's default must be for that feature"));
    }
    ensure_editable(feature)?;
    feature.kind.check_value(&spec.value).map_err(invalid)?;
    if resource_id.is_some() && !matches!(entity, EntitlementEntityId::Customer(_)) {
        return Err(invalid("only a customer row can be on one resource"));
    }
    let tier = Tier::of(&entity, resource_id.is_some());
    let order = feature.value_order();
    let effect = match spec.effect {
        Some(effect) => effect,
        None => {
            let additive = match entity {
                EntitlementEntityId::AddOn(id) => is_addon_additive(conn, id, tenant_id).await?,
                _ => false,
            };
            tier.default_effect(&order, additive)
        }
    };
    if !tier.allowed_effects(&order).contains(&effect) {
        let (effect, tier): (&str, &str) = (effect.into(), tier.into());
        return Err(invalid(format!(
            "{effect} is not allowed on the {tier} tier"
        )));
    }
    if let Some(policy) = &spec.policy_override {
        if effect != Effect::Set || !matches!(feature.kind, FeatureKind::Limit(_)) {
            return Err(invalid(
                "only a Set to row of a limit carries a policy override",
            ));
        }
        policy.validate(feature.kind.counted_by_app())?;
    }
    if let (Some(from), Some(until)) = (spec.valid_from, spec.valid_until)
        && from >= until
    {
        return Err(invalid("valid_from must be before valid_until"));
    }

    let level = spec.level.normalised(feature.level_type_id);
    let eval = level.eval(feature.level_type_id);
    if order.is_unordered() && eval != feature.own_level() {
        return Err(invalid(
            "an unordered value only has rows on its feature's level",
        ));
    }
    let type_row = match eval.type_id() {
        Some(type_id) => Some(active_type(conn, tenant_id, type_id).await?),
        None => None,
    };
    if let Some(resource_id) = resource_id {
        let resource = ResourceRow::find_by_id(conn, tenant_id, resource_id)
            .await?
            .ok_or_else(|| invalid(format!("resource {resource_id} not found")))?;
        if EntitlementEntityId::Customer(resource.customer_id) != entity {
            return Err(invalid("the resource belongs to another customer"));
        }
        if eval.type_id() != Some(resource.resource_type_id) {
            return Err(invalid(
                "a resource override applies at its resource's level",
            ));
        }
    }
    if let Some(condition) = &spec.condition {
        validate_condition(condition, type_row.as_ref())?;
    }
    if let Some(type_row) = &type_row {
        ensure_metric_attributed(conn, tenant_id, &feature.kind, type_row).await?;
    }
    if eval == EvalLevel::Customer
        && feature.level_type_id.is_some()
        && owner_plan(conn, tenant_id, &entity)
            .await?
            .is_some_and(|plan| plan.sold_for_resource_type_id.is_some())
    {
        return Err(invalid(
            "a plan sold for a resource type can't carry rows across all its resources",
        ));
    }

    let id = EntitlementId::new();
    let (level, level_type_id) = level.columns();
    let (created_by, created_by_kind) = author(actor);
    let now = common_clock::now();
    Ok(EntitlementRowNew {
        id,
        tenant_id,
        feature_id: feature.id,
        entity_id: entity.as_uuid(),
        entity_type: EntitlementEntityTypeEnum::from(&entity),
        mode: if effect == Effect::Add {
            EntitlementModeEnum::Stack
        } else {
            EntitlementModeEnum::Override
        },
        value: to_json(&spec.value),
        lineage_id: lineage.unwrap_or(id),
        lineage_created_at: now,
        created_at: now,
        effect: effect.into(),
        level,
        level_type_id,
        resource_id,
        condition_hash: Condition::hash_of(spec.condition.as_ref()),
        condition: spec.condition.as_ref().map(to_json),
        valid_from: spec.valid_from,
        valid_until: spec.valid_until,
        reason: spec.reason,
        created_by,
        created_by_kind,
        policy_override: spec.policy_override.as_ref().map(to_json),
    })
}

/// A clash with an open row of the same feature, owner, level, resource and condition whose
/// validity overlaps (§3.2).
fn duplicate() -> Report<StoreError> {
    Report::new(StoreError::DuplicateValue {
        entity: "entitlement",
        key: Some("feature, owner, level, resource, condition and validity".into()),
    })
}

/// Refuses rows another open row, or an earlier one of them, overlaps.
async fn ensure_no_overlap(conn: &mut PgConn, rows: &[EntitlementRowNew]) -> StoreResult<()> {
    let mut rows = rows.to_vec();
    rows.sort_by_cached_key(EntitlementRowNew::lock_key);
    for (i, row) in rows.iter().enumerate() {
        if row.overlaps(conn, &rows[..i]).await? {
            return Err(duplicate());
        }
    }
    Ok(())
}

async fn feature_of(conn: &mut PgConn, id: FeatureId, tenant_id: TenantId) -> StoreResult<Feature> {
    FeatureRow::find_by_id(conn, id, tenant_id)
        .await?
        .try_into()
}

async fn with_defaults(
    conn: &mut PgConn,
    tenant_id: TenantId,
    mut features: Vec<Feature>,
) -> StoreResult<Vec<Feature>> {
    let ids: Vec<FeatureId> = features.iter().map(|f| f.id).collect();
    let mut defaults: HashMap<FeatureId, Vec<Entitlement>> = HashMap::new();
    for row in EntitlementRow::list_feature_level_entitlements(conn, tenant_id, Some(&ids)).await? {
        let entitlement = Entitlement::try_from(row)?;
        defaults
            .entry(entitlement.feature_id)
            .or_default()
            .push(entitlement);
    }
    for feature in &mut features {
        feature.defaults = defaults.remove(&feature.id).unwrap_or_default();
    }
    Ok(features)
}

async fn one_with_defaults(
    conn: &mut PgConn,
    tenant_id: TenantId,
    feature: Feature,
) -> StoreResult<Feature> {
    Ok(with_defaults(conn, tenant_id, vec![feature])
        .await?
        .pop()
        .expect("one feature in, one out"))
}

/// Validates a batch of rows for one owner; archived features' specs are dropped with
/// `skip_archived`, refused otherwise.
async fn build_entitlement_rows(
    conn: &mut PgConn,
    specs: Vec<EntitlementSpec>,
    owner: CheckedOwner,
    tenant_id: TenantId,
    actor: Option<&Actor>,
    skip_archived: bool,
) -> StoreResult<Vec<EntitlementRowNew>> {
    let feature_ids: Vec<FeatureId> = specs.iter().map(|s| s.feature_id).unique().collect();
    let archived: HashSet<FeatureId> = FeatureRow::find_archived_ids(conn, tenant_id, &feature_ids)
        .await?
        .into_iter()
        .collect();
    let features: HashMap<FeatureId, Feature> =
        FeatureRow::find_by_ids(conn, tenant_id, &feature_ids)
            .await?
            .into_iter()
            .map(|row| Feature::try_from(row).map(|f| (f.id, f)))
            .collect::<StoreResult<_>>()?;

    let mut rows = Vec::with_capacity(specs.len());
    for spec in specs {
        if archived.contains(&spec.feature_id) {
            if skip_archived {
                continue;
            }
            return Err(invalid(format!("feature {} is archived", spec.feature_id)));
        }
        let feature = features
            .get(&spec.feature_id)
            .ok_or_else(|| invalid(format!("feature {} not found", spec.feature_id)))?;
        rows.push(new_row(conn, tenant_id, feature, owner, None, spec, actor, None).await?);
    }
    Ok(rows)
}

/// Inserts rows on an owner being created, in the caller's transaction. Archived features'
/// specs are skipped.
pub(crate) async fn insert_entitlement_specs(
    conn: &mut PgConn,
    specs: Vec<EntitlementSpec>,
    entity: EntitlementEntityId,
    tenant_id: TenantId,
) -> StoreResult<Vec<Entitlement>> {
    if specs.is_empty() {
        return Ok(vec![]);
    }
    let owner = CheckedOwner::being_created(conn, tenant_id, entity).await?;
    let rows = build_entitlement_rows(conn, specs, owner, tenant_id, None, true).await?;
    if rows.is_empty() {
        return Ok(vec![]);
    }
    ensure_no_overlap(conn, &rows).await?;
    EntitlementRowNew::insert_batch(&rows, conn)
        .await?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
}

#[async_trait::async_trait]
impl EntitlementsInterface for Store {
    async fn create_feature(&self, actor: Actor, feature: FeatureNew) -> StoreResult<Feature> {
        let tenant_id = feature.tenant_id;
        let default = feature.default.clone();
        {
            let mut conn = self.get_conn().await?;
            ensure_product_in_tenant(&mut conn, tenant_id, feature.product_id).await?;
            validate_feature(
                &mut conn,
                tenant_id,
                &feature.kind,
                feature.policy.as_ref(),
                feature.level_type_id,
            )
            .await?;
        }
        let row: FeatureRowNew = feature.into();
        self.transaction_bumping_tenants(|conn| {
            let actor = &actor;
            async move {
                let inserted = row.insert(conn).await?;
                let mut f = feature_of(conn, inserted.id, tenant_id).await?;

                let bumped = default.is_some().then_some(tenant_id);
                if let Some(value) = default {
                    let owner =
                        CheckedOwner::check(conn, tenant_id, EntitlementEntityId::Feature(f.id))
                            .await?;
                    let spec = EntitlementSpec::new(f.id, value);
                    let default =
                        new_row(conn, tenant_id, &f, owner, None, spec, Some(actor), None).await?;
                    f.defaults = vec![default.insert(conn).await?.try_into()?];
                }

                let activity = Activity::new(
                    ActivityType::FeatureCreated,
                    EntityType::Feature,
                    f.id.as_uuid(),
                )
                .with_metadata(serde_json::json!({ "code": f.code, "name": f.name }));
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;

                Ok((f, bumped.into_iter().collect()))
            }
            .scope_boxed()
        })
        .await
    }

    async fn get_feature(&self, id: FeatureId, tenant_id: TenantId) -> StoreResult<Feature> {
        let mut conn = self.get_conn().await?;
        let feature = feature_of(&mut conn, id, tenant_id).await?;
        one_with_defaults(&mut conn, tenant_id, feature).await
    }

    async fn get_feature_by_id_or_code(
        &self,
        id_or_code: AliasOr<FeatureId>,
        tenant_id: TenantId,
    ) -> StoreResult<Feature> {
        let mut conn = self.get_conn().await?;
        let feature = FeatureRow::find_by_id_or_code(&mut conn, id_or_code, tenant_id)
            .await?
            .try_into()?;
        one_with_defaults(&mut conn, tenant_id, feature).await
    }

    async fn list_features(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        statuses: Option<Vec<FeatureStatusEnum>>,
        product_id: Option<ProductId>,
        search: Option<String>,
        level: Option<Option<ResourceTypeId>>,
        order: FeatureListOrder,
    ) -> StoreResult<PaginatedVec<Feature>> {
        let mut conn = self.get_conn().await?;
        let statuses = statuses.map(|v| {
            v.into_iter()
                .map(Into::<DbFeatureStatusEnum>::into)
                .collect()
        });
        let rows = FeatureRow::list(
            &mut conn,
            tenant_id,
            pagination.into(),
            statuses,
            product_id,
            search,
            level,
            order.into(),
        )
        .await?;
        let features = rows
            .items
            .into_iter()
            .map(Feature::try_from)
            .collect::<StoreResult<Vec<_>>>()?;
        Ok(PaginatedVec {
            items: with_defaults(&mut conn, tenant_id, features).await?,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        })
    }

    async fn list_features_by_ids(
        &self,
        ids: &[FeatureId],
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Feature>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let mut conn = self.get_conn().await?;
        let features = FeatureRow::find_by_ids(&mut conn, tenant_id, ids)
            .await?
            .into_iter()
            .map(Feature::try_from)
            .collect::<StoreResult<Vec<_>>>()?;
        with_defaults(&mut conn, tenant_id, features).await
    }

    async fn update_feature(
        &self,
        id: FeatureId,
        tenant_id: TenantId,
        update: FeatureUpdate,
    ) -> StoreResult<Feature> {
        let feature = self
            .transaction_bumping_tenant(tenant_id, |conn| {
                async move { update_feature_tx(conn, id, tenant_id, update).await }.scope_boxed()
            })
            .await?;
        let mut conn = self.get_conn().await?;
        one_with_defaults(&mut conn, tenant_id, feature).await
    }

    async fn create_feature_with_defaults(
        &self,
        actor: &Actor,
        id: FeatureId,
        feature: FeatureNew,
        per_unit_of: Option<ManagedQuantityId>,
        mut defaults: Vec<EntitlementSpec>,
    ) -> StoreResult<Feature> {
        let tenant_id = feature.tenant_id;
        if let Some(value) = feature.default.clone() {
            defaults.push(EntitlementSpec::new(id, value));
        }
        self.transaction_bumping_tenants(|conn| {
            async move {
                ensure_product_in_tenant(conn, tenant_id, feature.product_id).await?;
                validate_feature(
                    conn,
                    tenant_id,
                    &feature.kind,
                    feature.policy.as_ref(),
                    feature.level_type_id,
                )
                .await?;
                let mut row: FeatureRowNew = feature.into();
                row.id = id;
                row.insert(conn).await?;
                let created = feature_of(conn, id, tenant_id).await?;
                // A new feature has no rows of other owners yet: its own only move resolutions.
                let bumped = per_unit_of.is_some() || !defaults.is_empty();
                if per_unit_of.is_some() {
                    set_limit_quantity_tx(conn, tenant_id, &created, per_unit_of).await?;
                }
                if !defaults.is_empty() {
                    replace_entitlements_tx(
                        conn,
                        Some(actor),
                        tenant_id,
                        EntitlementEntityId::Feature(id),
                        defaults,
                    )
                    .await?;
                }
                let activity = Activity::new(
                    ActivityType::FeatureCreated,
                    EntityType::Feature,
                    id.as_uuid(),
                )
                .with_metadata(serde_json::json!({ "code": created.code, "name": created.name }));
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;
                let created = one_with_defaults(conn, tenant_id, created).await?;
                Ok((created, bumped.then_some(tenant_id).into_iter().collect()))
            }
            .scope_boxed()
        })
        .await
    }

    async fn update_feature_with_defaults(
        &self,
        actor: &Actor,
        id: FeatureId,
        tenant_id: TenantId,
        update: Option<FeatureUpdate>,
        per_unit_of: Option<Option<ManagedQuantityId>>,
        defaults: Option<Vec<EntitlementSpec>>,
    ) -> StoreResult<Feature> {
        self.transaction_bumping_tenant(tenant_id, |conn| {
            async move {
                let feature = match update {
                    Some(update) => update_feature_tx(conn, id, tenant_id, update).await?,
                    None => feature_of(conn, id, tenant_id).await?,
                };
                if let Some(quantity) = per_unit_of {
                    set_limit_quantity_tx(conn, tenant_id, &feature, quantity).await?;
                }
                if let Some(specs) = defaults {
                    replace_entitlements_tx(
                        conn,
                        Some(actor),
                        tenant_id,
                        EntitlementEntityId::Feature(id),
                        specs,
                    )
                    .await?;
                }
                one_with_defaults(conn, tenant_id, feature).await
            }
            .scope_boxed()
        })
        .await
    }

    async fn set_feature_status(
        &self,
        actor: Actor,
        id: FeatureId,
        tenant_id: TenantId,
        status: FeatureStatusEnum,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        let current = feature_of(&mut conn, id, tenant_id).await?;
        if current.status == status {
            return Ok(());
        }
        let activity_type = match (&current.status, &status) {
            (FeatureStatusEnum::Archived, FeatureStatusEnum::Disabled) => {
                return Err(invalid("feature is archived; unarchive it first"));
            }
            (_, FeatureStatusEnum::Archived) => ActivityType::FeatureArchived,
            (FeatureStatusEnum::Archived, _) => ActivityType::FeatureUnarchived,
            (_, FeatureStatusEnum::Active) => ActivityType::FeatureEnabled,
            (_, FeatureStatusEnum::Disabled) => ActivityType::FeatureDisabled,
        };
        if current.status == FeatureStatusEnum::Archived
            && let Some(holder) =
                FeatureRow::find_unarchived_by_code(&mut conn, tenant_id, &current.code, id).await?
        {
            return Err(Report::new(StoreError::FailedPrecondition(format!(
                "code {} is used by feature {}",
                current.code, holder.name
            ))));
        }
        drop(conn);
        self.transaction_bumping_tenant(tenant_id, |conn| {
            let actor = &actor;
            async move {
                FeatureRow::set_status(conn, id, tenant_id, status.into()).await?;
                let activity = Activity::new(activity_type, EntityType::Feature, id.as_uuid());
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn find_archived_feature_by_code(
        &self,
        code: &str,
        tenant_id: TenantId,
    ) -> StoreResult<FeatureId> {
        let mut conn = self.get_conn().await?;
        let rows = FeatureRow::list_by_code(&mut conn, tenant_id, code).await?;
        if rows
            .iter()
            .any(|r| r.status != DbFeatureStatusEnum::Archived)
        {
            return Err(Report::new(StoreError::FailedPrecondition(
                "a non-archived feature already holds this code".into(),
            )));
        }
        match rows.as_slice() {
            [] => Err(Report::new(StoreError::ValueNotFound(format!(
                "feature {code}"
            )))),
            [row] => Ok(row.id),
            _ => Err(Report::new(StoreError::FailedPrecondition(
                "several archived features used this code; use the ID".into(),
            ))),
        }
    }

    async fn feature_code_holder(
        &self,
        id: FeatureId,
        tenant_id: TenantId,
    ) -> StoreResult<Option<FeatureRef>> {
        let mut conn = self.get_conn().await?;
        let feature = feature_of(&mut conn, id, tenant_id).await?;
        if feature.status != FeatureStatusEnum::Archived {
            return Ok(None);
        }
        let Some(holder) =
            FeatureRow::find_unarchived_by_code(&mut conn, tenant_id, &feature.code, id).await?
        else {
            return Ok(None);
        };
        Ok(Some(
            feature_of(&mut conn, holder.id, tenant_id).await?.to_ref(),
        ))
    }

    async fn create_entitlement(
        &self,
        actor: &Actor,
        entitlement: EntitlementNew,
    ) -> StoreResult<Entitlement> {
        let EntitlementNew {
            tenant_id,
            entity,
            resource_id,
            spec,
        } = entitlement;
        let row = {
            let mut conn = self.get_conn().await?;
            let owner = CheckedOwner::check(&mut conn, tenant_id, entity).await?;
            let feature = feature_of(&mut conn, spec.feature_id, tenant_id).await?;
            new_row(
                &mut conn,
                tenant_id,
                &feature,
                owner,
                resource_id,
                spec,
                Some(actor),
                None,
            )
            .await?
        };
        let row = self
            .transaction_bumping_tenants(|conn| {
                async move {
                    let owner = InputsOwner::of_entitlement(
                        conn,
                        row.tenant_id,
                        &row.entity_type,
                        row.entity_id,
                    )
                    .await?;
                    ensure_no_overlap(conn, std::slice::from_ref(&row)).await?;
                    let inserted = row.insert(conn).await?;
                    Ok((inserted, owner.bump(conn).await?))
                }
                .scope_boxed()
            })
            .await?;
        row.try_into()
    }

    async fn create_entitlements(
        &self,
        actor: &Actor,
        tenant_id: TenantId,
        entity: EntitlementEntityId,
        resource_id: Option<ResourceId>,
        specs: Vec<EntitlementSpec>,
    ) -> StoreResult<Vec<Entitlement>> {
        let rows = {
            let mut conn = self.get_conn().await?;
            let owner = CheckedOwner::check(&mut conn, tenant_id, entity).await?;
            let mut rows = Vec::with_capacity(specs.len());
            for spec in specs {
                let feature = feature_of(&mut conn, spec.feature_id, tenant_id).await?;
                rows.push(
                    new_row(
                        &mut conn,
                        tenant_id,
                        &feature,
                        owner,
                        resource_id,
                        spec,
                        Some(actor),
                        None,
                    )
                    .await?,
                );
            }
            rows
        };
        let (entity_type, entity_id) = (EntitlementEntityTypeEnum::from(&entity), entity.as_uuid());
        let inserted = self
            .transaction_bumping_tenants(|conn| {
                async move {
                    let owner =
                        InputsOwner::of_entitlement(conn, tenant_id, &entity_type, entity_id)
                            .await?;
                    ensure_no_overlap(conn, &rows).await?;
                    let inserted = EntitlementRowNew::insert_batch(&rows, conn).await?;
                    Ok((inserted, owner.bump(conn).await?))
                }
                .scope_boxed()
            })
            .await?;
        inserted.into_iter().map(TryInto::try_into).collect()
    }

    async fn get_entitlement(
        &self,
        id: EntitlementId,
        tenant_id: TenantId,
    ) -> StoreResult<Entitlement> {
        self.get_entitlement_history(id, tenant_id)
            .await?
            .into_iter()
            .rfind(|e| e.revoked_at.is_none())
            .ok_or_else(|| Report::new(StoreError::ValueNotFound(format!("entitlement {id}"))))
    }

    async fn get_entitlement_history(
        &self,
        id: EntitlementId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Entitlement>> {
        let mut conn = self.get_conn().await?;
        EntitlementRow::list_history(&mut conn, tenant_id, id)
            .await?
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }

    async fn list_entitlements_by_feature(
        &self,
        feature_id: FeatureId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Entitlement>> {
        let mut conn = self.get_conn().await?;
        EntitlementRow::list_by_feature(&mut conn, feature_id, tenant_id)
            .await?
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }

    async fn list_entitlements_by_entity(
        &self,
        entity: EntitlementEntityId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Entitlement>> {
        let mut conn = self.get_conn().await?;
        EntitlementRow::list_by_entity(&mut conn, tenant_id, entity)
            .await?
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }

    async fn update_entitlement(
        &self,
        actor: &Actor,
        id: EntitlementId,
        tenant_id: TenantId,
        update: EntitlementUpdate,
    ) -> StoreResult<Entitlement> {
        let row = self
            .transaction_bumping_tenants(|conn| {
                async move {
                    let open: Entitlement = lock_open(conn, tenant_id, id).await?.try_into()?;
                    let feature = feature_of(conn, open.feature_id, tenant_id).await?;
                    let spec = EntitlementSpec {
                        feature_id: open.feature_id,
                        value: update.value.unwrap_or(open.value),
                        effect: Some(update.effect.unwrap_or(open.effect)),
                        level: open.level,
                        condition: update.condition.unwrap_or(open.condition),
                        valid_from: update.valid_from.unwrap_or(open.valid_from),
                        valid_until: update.valid_until.unwrap_or(open.valid_until),
                        reason: update.reason.unwrap_or(open.reason),
                        policy_override: update.policy_override.unwrap_or(open.policy_override),
                    };
                    let owner = CheckedOwner::check(conn, tenant_id, open.entity).await?;
                    let successor = new_row(
                        conn,
                        tenant_id,
                        &feature,
                        owner,
                        open.resource_id,
                        spec,
                        Some(actor),
                        Some(id),
                    )
                    .await?;
                    let owner = InputsOwner::of_entitlement(
                        conn,
                        tenant_id,
                        &successor.entity_type,
                        successor.entity_id,
                    )
                    .await?;
                    ensure_no_overlap(conn, std::slice::from_ref(&successor)).await?;
                    let inserted = successor.insert_successor(conn, actor.as_uuid()).await?;
                    Ok((inserted, owner.bump(conn).await?))
                }
                .scope_boxed()
            })
            .await?;
        row.try_into()
    }

    async fn delete_entitlement(
        &self,
        actor: &Actor,
        id: EntitlementId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.transaction_bumping_tenants(|conn| {
            async move {
                let open = lock_open(conn, tenant_id, id).await?;
                CheckedOwner::check(conn, tenant_id, EntitlementEntityId::from(&open)).await?;
                let owner =
                    InputsOwner::of_entitlement(conn, tenant_id, &open.entity_type, open.entity_id)
                        .await?;
                EntitlementRow::revoke_open(conn, tenant_id, id, actor.as_uuid()).await?;
                Ok(((), owner.bump(conn).await?))
            }
            .scope_boxed()
        })
        .await
    }

    async fn batch_create_entitlements(
        &self,
        actor: &Actor,
        tenant_id: TenantId,
        target: EntitlementEntityId,
        specs: Vec<EntitlementSpec>,
    ) -> StoreResult<Vec<Entitlement>> {
        if specs.is_empty() {
            return Ok(vec![]);
        }
        let rows = {
            let mut conn = self.get_conn().await?;
            let owner = CheckedOwner::check(&mut conn, tenant_id, target).await?;
            build_entitlement_rows(&mut conn, specs, owner, tenant_id, Some(actor), false).await?
        };
        let entity_type = EntitlementEntityTypeEnum::from(&target);
        let entity_id = target.as_uuid();
        let inserted = self
            .transaction_bumping_tenants(|conn| {
                async move {
                    let owner =
                        InputsOwner::of_entitlement(conn, tenant_id, &entity_type, entity_id)
                            .await?;
                    let inserted =
                        EntitlementRowNew::insert_batch_skip_conflicts(rows, conn).await?;
                    Ok((inserted, owner.bump(conn).await?))
                }
                .scope_boxed()
            })
            .await?;
        inserted.into_iter().map(TryInto::try_into).collect()
    }

    async fn resolve_entitlements_for_customer(
        &self,
        conn: &mut PgConn,
        customer_id: CustomerId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ResolvedEntitlement>> {
        let ResolvedCustomer { features } = self
            .resolve_customer_entitlements(
                conn,
                tenant_id,
                customer_id,
                None,
                None,
                common_clock::now(),
            )
            .await?;
        with_origin_names(
            conn,
            tenant_id,
            features.iter().filter_map(readout_of).collect(),
        )
        .await
    }

    async fn resolve_entitlements_for_entity(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        target: ResolveTarget,
    ) -> StoreResult<Vec<ResolvedEntitlement>> {
        let now = common_clock::now();
        let (subscription, add_ons) = match &target {
            ResolveTarget::Subscription(id) => {
                // On the read day its chain's rows are read on.
                let period_end = SubscriptionRow::get_subscription_by_id(conn, &tenant_id, *id)
                    .await?
                    .subscription
                    .current_period_end;
                let add_ons = SubscriptionAddOnRow::list_by_subscription_id_active(
                    conn,
                    &tenant_id,
                    id,
                    Some(diesel_models::query::subscription_facts::read_day(
                        now.date_naive(),
                        period_end,
                    )),
                )
                .await?;
                (Some(*id), add_ons)
            }
            _ => (None, vec![]),
        };
        let scope = build_chain_and_products(conn, tenant_id, target).await?;
        let features: Vec<Feature> = FeatureRow::find_active_for_products(
            conn,
            tenant_id,
            &scope.product_ids,
            scope.include_globals,
        )
        .await?
        .into_iter()
        .map(Feature::try_from)
        .filter_ok(|f| f.status == FeatureStatusEnum::Active)
        .collect::<StoreResult<_>>()?;
        let feature_ids: Vec<FeatureId> = features.iter().map(|f| f.id).collect();
        let rows: Vec<Entitlement> =
            EntitlementRow::list_open_for_owners(conn, tenant_id, &scope.chain, Some(&feature_ids))
                .await?
                .into_iter()
                .chain(
                    EntitlementRow::list_feature_level_entitlements(
                        conn,
                        tenant_id,
                        Some(&feature_ids),
                    )
                    .await?,
                )
                .map(Entitlement::try_from)
                .collect::<StoreResult<_>>()?;

        // What multiplies the subscription's rows: add-on instances, seats held (R-133, R-138).
        let (quantities, per_unit, held) = match subscription {
            Some(subscription_id) => {
                let limits: Vec<FeatureId> = features
                    .iter()
                    .filter(|f| matches!(f.kind, FeatureKind::Limit(_)))
                    .map(|f| f.id)
                    .collect();
                let per_unit = feature_limit_quantities(conn, tenant_id, &limits).await?;
                let add_on_ids: Vec<AddOnId> =
                    add_ons.iter().map(|a| a.add_on_id).unique().collect();
                let seats = seat_quantities(conn, tenant_id, &add_on_ids).await?;
                let pairs: Vec<(SubscriptionId, ManagedQuantityId)> = per_unit
                    .values()
                    .chain(seats.values())
                    .unique()
                    .map(|quantity| (subscription_id, *quantity))
                    .collect();
                let held = held_units(conn, tenant_id, &pairs, now).await?;
                let quantities = add_on_quantities(subscription_id, &add_ons, &seats, &held);
                (quantities, per_unit, held)
            }
            None => Default::default(),
        };
        let no_attributes = Attributes::new();
        let items = features
            .iter()
            .filter_map(|feature| {
                let own = feature.own_level();
                let units = subscription
                    .zip(per_unit.get(&feature.id))
                    .map(|(s, quantity)| held.get(&(s, *quantity)).copied().unwrap_or(0));
                let fold_rows: Vec<_> = rows
                    .iter()
                    .filter(|e| {
                        e.feature_id == feature.id
                            && e.resource_id.is_none()
                            && e.level.eval(feature.level_type_id) == own
                    })
                    .filter_map(|e| {
                        let quantity = match (e.entity, subscription) {
                            (EntitlementEntityId::AddOn(id), Some(_)) => *quantities.get(&id)?,
                            _ => 1,
                        };
                        Some(e.fold_row(quantity, units, &no_attributes))
                    })
                    .collect();
                let policy = feature.policy_or_default();
                let folded = fold(&feature.value_order(), true, &policy, &fold_rows, now)?;
                let value = ResolvedEntitlementValue::of(
                    &feature.kind,
                    folded.value.as_ref()?,
                    &folded.policy,
                    true,
                )?;
                let origin = winning(&folded.explain)
                    .map_or(EntitlementEntityId::Feature(feature.id), |c| c.source);
                Some((feature.to_ref(), value, origin))
            })
            .collect();
        with_origin_names(conn, tenant_id, items).await
    }

    async fn list_features_after(
        &self,
        tenant_id: TenantId,
        filter: FeaturePageFilter,
        after: Option<FeatureId>,
        limit: usize,
    ) -> StoreResult<Vec<Feature>> {
        let mut conn = self.get_conn().await?;
        if let Some(after) = after {
            FeatureRow::find_by_id(&mut conn, after, tenant_id)
                .await
                .map_err(|e| match e.error.current_context() {
                    DatabaseError::NotFound => invalid(format!("no feature {after} to page after")),
                    _ => e.into(),
                })?;
        }
        let statuses = filter.statuses.map(|statuses| {
            statuses
                .into_iter()
                .map(Into::<DbFeatureStatusEnum>::into)
                .collect::<Vec<_>>()
        });
        let features = FeatureRow::list_after(
            &mut conn,
            tenant_id,
            statuses,
            filter.product_id,
            filter.level,
            filter.order.map(Into::into),
            after,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await?
        .into_iter()
        .map(Feature::try_from)
        .collect::<StoreResult<Vec<_>>>()?;
        with_defaults(&mut conn, tenant_id, features).await
    }

    async fn replace_entitlements(
        &self,
        actor: &Actor,
        tenant_id: TenantId,
        entity: EntitlementEntityId,
        specs: Vec<EntitlementSpec>,
    ) -> StoreResult<Vec<Entitlement>> {
        let (entity_type, entity_id) = (EntitlementEntityTypeEnum::from(&entity), entity.as_uuid());
        self.transaction_bumping_tenants(|conn| {
            async move {
                // A subscription's rows are written under its customer's lock, which is bumped.
                let owner =
                    InputsOwner::of_entitlement(conn, tenant_id, &entity_type, entity_id).await?;
                let rows =
                    replace_entitlements_tx(conn, Some(actor), tenant_id, entity, specs).await?;
                Ok((rows, owner.bump(conn).await?))
            }
            .scope_boxed()
        })
        .await
    }

    async fn set_product_feature_defaults(
        &self,
        actor: &Actor,
        tenant_id: TenantId,
        product_id: ProductId,
        specs: Vec<EntitlementSpec>,
    ) -> StoreResult<()> {
        self.transaction_bumping_tenant(tenant_id, |conn| {
            async move {
                set_product_feature_defaults_tx(conn, Some(actor), tenant_id, product_id, specs)
                    .await
            }
            .scope_boxed()
        })
        .await
    }
}

/// Counts the limit `feature` grants per unit of `quantity` (R-133), or per subscription again
/// when none, as `set_feature_limit_quantity` does, on the caller's transaction.
async fn set_limit_quantity_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    feature: &Feature,
    quantity: Option<ManagedQuantityId>,
) -> StoreResult<()> {
    if !matches!(feature.kind, FeatureKind::Limit(_)) {
        return Err(invalid("only a limit counts per unit of a quantity"));
    }
    ensure_editable(feature)?;
    if let Some(quantity) = quantity {
        let live = ManagedQuantityRow::find(conn, tenant_id, quantity)
            .await?
            .is_some_and(|row| row.archived_at.is_none());
        if !live {
            return Err(invalid(format!("no live managed quantity {quantity}")));
        }
    }
    set_feature_limit_quantity(conn, feature.id, quantity).await?;
    CreditRecomputeMarkRow::mark_for_entitlement_owner(
        conn,
        tenant_id,
        &EntitlementEntityTypeEnum::Feature,
        feature.id.as_uuid(),
    )
    .await?;
    Ok(())
}

/// A feature's edit on the caller's transaction: its checks, then its row.
async fn update_feature_tx(
    conn: &mut PgConn,
    id: FeatureId,
    tenant_id: TenantId,
    update: FeatureUpdate,
) -> StoreResult<Feature> {
    let current = feature_of(conn, id, tenant_id).await?;
    ensure_editable(&current)?;
    ensure_product_in_tenant(conn, tenant_id, update.product_id.flatten()).await?;

    let mut kind = current.kind.clone();
    match (&mut kind, update.number_higher_is_better) {
        (FeatureKind::Value(ValueType::Number { higher_is_better }), Some(new)) => {
            *higher_is_better = new
        }
        (_, Some(_)) => return Err(invalid("only a number has a direction")),
        (_, None) => {}
    }
    if update.select_options.is_some() || update.select_ordered.is_some() {
        let FeatureKind::Value(ValueType::Select { options, ordered }) = &mut kind else {
            return Err(invalid("only a select has options"));
        };
        if let Some(new_options) = &update.select_options {
            let removed: Vec<&String> = options
                .iter()
                .filter(|o| !new_options.contains(o))
                .collect();
            let in_use: Vec<String> = EntitlementRow::list_by_feature(conn, id, tenant_id)
                .await?
                .into_iter()
                .filter_map(
                    |row| match from_json::<GrantValue>(row.value, "entitlement value") {
                        Ok(GrantValue::Text(t)) if removed.contains(&&t) => Some(t),
                        _ => None,
                    },
                )
                .unique()
                .collect();
            if !in_use.is_empty() {
                return Err(invalid(format!(
                    "Cannot remove option(s) still used by entitlements: {}",
                    in_use.join(", ")
                )));
            }
            *options = new_options.clone();
        }
        if let Some(new_ordered) = update.select_ordered {
            if *ordered && !new_ordered {
                ensure_rows_take_no_order(conn, &current).await?;
            }
            *ordered = new_ordered;
        }
    }
    let new_level = update
        .level_type_id
        .flatten()
        .filter(|_| update.level_type_id != Some(current.level_type_id));
    let policy = update.policy.clone().or(current.policy.clone());
    validate_feature(conn, tenant_id, &kind, policy.as_ref(), new_level).await?;
    // Rows and counts were written for the old level, which a new one would reinterpret.
    if update
        .level_type_id
        .is_some_and(|l| l != current.level_type_id)
    {
        if !EntitlementRow::list_by_feature(conn, id, tenant_id)
            .await?
            .is_empty()
        {
            return Err(invalid(
                "the level of a feature can't change while it has rows: revoke them first",
            ));
        }
        if kind.counted_by_app()
            && EntitlementAppCountRow::exists_for_feature(conn, tenant_id, id).await?
        {
            return Err(invalid(
                "the level of a limit counted by the app can't change once it has counts",
            ));
        }
    }

    let select = match &kind {
        FeatureKind::Value(ValueType::Select { options, ordered }) => Some((options, *ordered)),
        _ => None,
    };
    let options_changed = update.select_options.is_some();
    let patch = FeatureRowPatch {
        name: update.name,
        description: update.description,
        product_id: update.product_id,
        config_options: select
            .filter(|_| options_changed)
            .map(|(options, _)| Some(options.iter().cloned().map(Some).collect())),
        status: None,
        updated_at: Some(common_clock::now()),
        unit: update.unit,
        number_higher_is_better: update.number_higher_is_better,
        select_options: select
            .filter(|_| options_changed)
            .map(|(options, _)| Some(serde_json::json!(options))),
        select_ordered: update.select_ordered,
        level_resource_type_id: update.level_type_id,
        policy: update.policy.as_ref().map(|p| Some(to_json(p))),
    };
    FeatureRow::update(conn, id, tenant_id, patch)
        .await?
        .try_into()
}

/// Whether the owner's replaced rows are deleted rather than revoked: those of a draft plan
/// version, never published. Refuses an owner whose rows are not replaced whole.
async fn replaced_by_deletion(
    conn: &mut PgConn,
    tenant_id: TenantId,
    entity: &EntitlementEntityId,
) -> StoreResult<bool> {
    match entity {
        EntitlementEntityId::AddOn(_) | EntitlementEntityId::Plan(_) => Ok(false),
        EntitlementEntityId::PlanVersion(id) => {
            Ok(
                PlanVersionRow::list_by_ids_and_tenant_id(conn, &[*id], tenant_id)
                    .await?
                    .first()
                    .is_some_and(|version| version.is_draft_version),
            )
        }
        EntitlementEntityId::Feature(id) => {
            feature_of(conn, *id, tenant_id).await?;
            Ok(false)
        }
        EntitlementEntityId::Subscription(id) => {
            let status: SubscriptionStatusEnum = SubscriptionRow::get_status(conn, &tenant_id, *id)
                .await?
                .into();
            if status.is_terminal() {
                return Err(Report::new(StoreError::Conflict(
                    "The subscription has ended: its entitlements no longer change".into(),
                )));
            }
            Ok(false)
        }
        EntitlementEntityId::Customer(_) | EntitlementEntityId::Quote(_) => Err(invalid(
            "only the rows of an add-on, a plan, a draft version, a feature or a subscription are \
             replaced whole",
        )),
    }
}

/// What a spec is matched to an open row by: its feature, level and condition.
fn replace_key(
    feature_id: FeatureId,
    level: EntitlementLevelEnum,
    level_type_id: Option<ResourceTypeId>,
    condition_hash: &str,
) -> String {
    format!("{feature_id}:{level:?}:{level_type_id:?}:{condition_hash}")
}

/// Whether the open row already is what the spec, built as `row`, asks for.
fn repeats(open: &Entitlement, row: &EntitlementRowNew, spec: &EntitlementSpec) -> bool {
    open.value == spec.value
        && EntitlementEffectEnum::from(open.effect) == row.effect
        && open.valid_from == spec.valid_from
        && open.valid_until == spec.valid_until
        && open.reason == spec.reason
        && open.policy_override == spec.policy_override
}

/// `replace_entitlements` on the caller's transaction, so a write of the owner and of its rows
/// commit together. The caller holds the owner's lock and bumps its inputs
/// (`InputsOwner::of_entitlement`: the customer of a subscription, else the tenant).
pub(crate) async fn replace_entitlements_tx(
    conn: &mut PgConn,
    actor: Option<&Actor>,
    tenant_id: TenantId,
    entity: EntitlementEntityId,
    specs: Vec<EntitlementSpec>,
) -> StoreResult<Vec<Entitlement>> {
    let deleting = replaced_by_deletion(conn, tenant_id, &entity).await?;
    let owner = CheckedOwner::check(conn, tenant_id, entity).await?;
    let built = build_entitlement_rows(conn, specs.clone(), owner, tenant_id, actor, false).await?;

    let mut open: HashMap<String, Vec<Entitlement>> = HashMap::new();
    for row in EntitlementRow::list_by_entity(conn, tenant_id, entity).await? {
        let key = replace_key(
            row.feature_id,
            row.level,
            row.level_type_id,
            &row.condition_hash,
        );
        open.entry(key).or_default().push(row.try_into()?);
    }
    for rows in open.values_mut() {
        rows.sort_by_key(|e| (e.valid_from.unwrap_or(e.lineage_created_at), e.created_at));
    }
    let mut wanted: Vec<(EntitlementRowNew, EntitlementSpec)> =
        built.into_iter().zip(specs).collect();
    wanted.sort_by_key(|(row, _)| row.valid_from);

    // Paired in validity order within a key; what is left on either side goes or comes.
    let mut inserts: Vec<EntitlementRowNew> = Vec::new();
    let mut replaced: Vec<EntitlementId> = Vec::new();
    for (mut row, spec) in wanted {
        let key = replace_key(
            row.feature_id,
            row.level,
            row.level_type_id,
            &row.condition_hash,
        );
        let paired = open
            .get_mut(&key)
            .filter(|rows| !rows.is_empty())
            .map(|rows| rows.remove(0));
        match paired {
            Some(current) if repeats(&current, &row, &spec) => {}
            Some(current) => {
                row.lineage_id = current.id;
                row.lineage_created_at = current.lineage_created_at;
                replaced.push(current.id);
                inserts.push(row);
            }
            None => inserts.push(row),
        }
    }
    let gone: Vec<EntitlementId> = open
        .into_values()
        .flatten()
        .map(|e| e.id)
        .chain(replaced)
        .collect();

    if deleting {
        EntitlementRow::delete_lineages(conn, tenant_id, entity, &gone).await?;
    } else {
        let revoked_by = actor.and_then(Actor::as_uuid);
        for lineage in &gone {
            EntitlementRow::revoke_open(conn, tenant_id, *lineage, revoked_by).await?;
        }
    }
    ensure_no_overlap(conn, &inserts).await?;
    EntitlementRowNew::insert_batch(&inserts, conn).await?;

    EntitlementRow::list_by_entity(conn, tenant_id, entity)
        .await?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
}

/// `set_product_feature_defaults` on the caller's transaction. The caller bumps the tenant's
/// inputs.
pub(crate) async fn set_product_feature_defaults_tx(
    conn: &mut PgConn,
    actor: Option<&Actor>,
    tenant_id: TenantId,
    product_id: ProductId,
    specs: Vec<EntitlementSpec>,
) -> StoreResult<()> {
    ProductRow::find_by_id_and_tenant_id(conn, product_id, tenant_id).await?;
    let features = FeatureRow::list_live_by_product(conn, tenant_id, product_id).await?;
    if let Some(stranger) = specs
        .iter()
        .find(|spec| !features.iter().any(|f| f.id == spec.feature_id))
    {
        return Err(invalid(format!(
            "feature {} is not a live feature of product {product_id}",
            stranger.feature_id
        )));
    }
    let mut by_feature = specs.into_iter().into_group_map_by(|spec| spec.feature_id);
    for feature in &features {
        replace_entitlements_tx(
            conn,
            actor,
            tenant_id,
            EntitlementEntityId::Feature(feature.id),
            by_feature.remove(&feature.id).unwrap_or_default(),
        )
        .await?;
    }
    Ok(())
}

/// The readout with a display name for each origin.
pub(crate) async fn with_origin_names(
    conn: &mut PgConn,
    tenant_id: TenantId,
    items: Vec<(FeatureRef, ResolvedEntitlementValue, EntitlementEntityId)>,
) -> StoreResult<Vec<ResolvedEntitlement>> {
    let origins = items
        .iter()
        .map(|(_, _, origin)| *origin)
        .filter(|origin| !matches!(origin, EntitlementEntityId::Subscription(_)));
    let names = SourceNames::load(conn, tenant_id, origins, []).await?;
    Ok(items
        .into_iter()
        .map(|(feature, value, entity)| ResolvedEntitlement {
            feature,
            value,
            origin: ResolvedOrigin {
                name: names.entity(&entity),
                entity,
            },
        })
        .collect())
}

/// Display names of entitlement owners, read in one query.
pub(crate) struct SourceNames(EntitlementSourceNames);

impl SourceNames {
    pub(crate) async fn load(
        conn: &mut PgConn,
        tenant_id: TenantId,
        entities: impl IntoIterator<Item = EntitlementEntityId>,
        resources: impl IntoIterator<Item = ResourceId>,
    ) -> StoreResult<Self> {
        let mut ids = EntitlementSourceIds::default();
        for entity in entities.into_iter().unique() {
            match entity {
                EntitlementEntityId::Feature(id) => ids.features.push(*id),
                EntitlementEntityId::Plan(id) => ids.plans.push(*id),
                EntitlementEntityId::PlanVersion(id) => ids.plan_versions.push(*id),
                EntitlementEntityId::AddOn(id) => ids.add_ons.push(*id),
                EntitlementEntityId::Subscription(id) => ids.subscriptions.push(*id),
                EntitlementEntityId::Quote(_) | EntitlementEntityId::Customer(_) => {}
            }
        }
        ids.resources = resources.into_iter().unique().map(|id| *id).collect();
        Ok(Self(entitlement_source_names(conn, tenant_id, &ids).await?))
    }

    /// The names `contributions` need: the owners of rows above the defaults and below overrides.
    pub(crate) async fn for_contributions<'a>(
        conn: &mut PgConn,
        tenant_id: TenantId,
        contributions: impl IntoIterator<Item = &'a Contribution>,
        resources: impl IntoIterator<Item = ResourceId>,
    ) -> StoreResult<Self> {
        let owners = contributions
            .into_iter()
            .filter(|c| {
                !matches!(
                    c.tier,
                    Tier::Default | Tier::Customer | Tier::ResourceOverride
                )
            })
            .map(|c| c.source)
            .collect::<Vec<_>>();
        Self::load(conn, tenant_id, owners, resources).await
    }

    /// An origin of the legacy readout: its own name; none for a customer, subscription or quote.
    pub(crate) fn entity(&self, entity: &EntitlementEntityId) -> Option<String> {
        let names = &self.0;
        match entity {
            EntitlementEntityId::Feature(id) => names.features.get(&**id),
            EntitlementEntityId::Plan(id) => names.plans.get(&**id),
            EntitlementEntityId::PlanVersion(id) => names.plan_versions.get(&**id),
            EntitlementEntityId::AddOn(id) => names.add_ons.get(&**id),
            EntitlementEntityId::Customer(_)
            | EntitlementEntityId::Subscription(_)
            | EntitlementEntityId::Quote(_) => None,
        }
        .cloned()
    }

    /// A resource override is named after `resource`, the one its level was read for.
    pub(crate) fn contribution(
        &self,
        c: &Contribution,
        resource: Option<ResourceId>,
    ) -> Option<String> {
        match (c.tier, c.source) {
            (Tier::Default, _) => Some("Default".to_owned()),
            (Tier::Customer, _) => Some("Customer override".to_owned()),
            (Tier::ResourceOverride, _) => {
                resource.and_then(|id| self.0.resources.get(&*id).cloned())
            }
            (_, EntitlementEntityId::Subscription(id)) => self.0.subscriptions.get(&*id).cloned(),
            (_, entity) => self.entity(&entity),
        }
    }

    pub(crate) fn name<'a>(
        &self,
        contributions: impl IntoIterator<Item = &'a mut Contribution>,
        resource: Option<ResourceId>,
    ) {
        for c in contributions {
            c.source_name = self.contribution(c, resource);
        }
    }
}

fn build_chain_for_product(_: ProductId) -> Vec<EntitlementEntityId> {
    Vec::new()
}

fn build_chain_for_addon(addon_id: AddOnId) -> Vec<EntitlementEntityId> {
    vec![EntitlementEntityId::AddOn(addon_id)]
}

/// The owners of a plan version target, `[Plan, PlanVersion, AddOn₁..n]`; the fold orders them by tier.
fn plan_version_chain(
    plan_id: PlanId,
    plan_version_id: PlanVersionId,
    add_on_ids: &[AddOnId],
) -> Vec<EntitlementEntityId> {
    let mut chain = Vec::with_capacity(2 + add_on_ids.len());
    chain.push(EntitlementEntityId::Plan(plan_id));
    chain.push(EntitlementEntityId::PlanVersion(plan_version_id));
    chain.extend(add_on_ids.iter().copied().map(EntitlementEntityId::AddOn));
    chain
}

async fn build_chain_for_subscription(
    conn: &mut PgConn,
    tenant_id: TenantId,
    sub_id: SubscriptionId,
) -> StoreResult<Vec<EntitlementEntityId>> {
    let sub = SubscriptionRow::get_subscription_by_id(conn, &tenant_id, sub_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let addon_ids = SubscriptionAddOnRow::list_active_add_on_ids(conn, &[sub_id], &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    // The plan in effect, as checks read it: a trial's trialing plan (§3.9).
    let version = crate::services::effective_plan_versions(
        conn,
        tenant_id,
        std::slice::from_ref(&sub.subscription),
    )
    .await?
    .get(&sub_id)
    .copied()
    .unwrap_or(sub.subscription.plan_version_id);
    let plan_id = if version == sub.subscription.plan_version_id {
        sub.plan_id
    } else {
        PlanVersionRow::get_plan_ids_by_version_ids(conn, &[version])
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .get(&version)
            .copied()
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(format!("plan version {version}")))
            })?
    };
    let mut chain = plan_version_chain(plan_id, version, &addon_ids);
    chain.push(EntitlementEntityId::Customer(sub.customer_id));
    chain.push(EntitlementEntityId::Subscription(sub_id));
    Ok(chain)
}

async fn build_chain_for_quote(
    conn: &mut PgConn,
    tenant_id: TenantId,
    quote_id: QuoteId,
) -> StoreResult<Vec<EntitlementEntityId>> {
    let quote = QuoteRow::find_by_id(conn, tenant_id, quote_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let pv_id = quote.plan_version_id;
    let plan_id_map = PlanVersionRow::get_plan_ids_by_version_ids(conn, &[pv_id])
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let Some(plan_id) = plan_id_map.get(&pv_id).copied() else {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "plan version {pv_id} not found"
        ))));
    };
    let addon_ids = QuoteAddOnRow::list_add_on_ids(conn, &[quote_id], &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let mut chain = plan_version_chain(plan_id, pv_id, &addon_ids);
    chain.push(EntitlementEntityId::Customer(quote.customer_id));
    chain.push(EntitlementEntityId::Quote(quote_id));
    Ok(chain)
}

/// Resolution scope for a single [`ResolveTarget`].
struct ResolutionScope {
    /// Ordered list of `EntitlementEntityId`s that can own entitlements at this target.
    chain: Vec<EntitlementEntityId>,
    /// Product ids whose features are in scope (features link to products via `feature.product_id`).
    product_ids: Vec<ProductId>,
    /// Whether tenant-global features (`feature.product_id IS NULL`) also apply.
    include_globals: bool,
}

/// Build a [`ResolutionScope`] for a `ResolveTarget`.
///
/// Inheritance rules (matches the product spec):
/// - **Product** — only the product's own features. No globals. No chain.
/// - **AddOn** — only the add-on's product features. No globals. Chain `[AddOn]` so the
///   add-on can override the inherited product entitlement values.
/// - **PlanVersion** — globals + features owned by the plan version's price-component
///   products (NOT add-on products). Chain `[Plan, PlanVersion]` only — add-ons compose
///   at the Subscription / Quote layer, not the PlanVersion layer.
/// - **Subscription / Quote** — globals + features owned by price-component products and
///   add-on products. Chain `[Plan, PlanVersion, AddOn…, Subscription|Quote]`.
/// - **Selection** — same scope/chain as the Subscription/Quote it previews.
async fn build_chain_and_products(
    conn: &mut PgConn,
    tenant_id: TenantId,
    target: ResolveTarget,
) -> StoreResult<ResolutionScope> {
    use itertools::Itertools;
    match target {
        ResolveTarget::Product(pid) => Ok(ResolutionScope {
            chain: build_chain_for_product(pid),
            product_ids: vec![pid],
            include_globals: false,
        }),
        ResolveTarget::AddOn(aid) => {
            let rows = AddOnRow::list_by_ids(conn, &[aid], &tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            let Some(addon) = rows.into_iter().next() else {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "add-on {aid} not found"
                ))));
            };
            Ok(ResolutionScope {
                chain: build_chain_for_addon(aid),
                product_ids: vec![addon.product_id],
                include_globals: false,
            })
        }
        ResolveTarget::PlanVersion(pvid) => {
            let pv_prods = list_product_ids_for_plan_version(conn, &tenant_id, pvid)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            let plan_id_map = PlanVersionRow::get_plan_ids_by_version_ids(conn, &[pvid])
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            let Some(plan_id) = plan_id_map.get(&pvid).copied() else {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "plan version {pvid} not found"
                ))));
            };
            Ok(ResolutionScope {
                chain: vec![
                    EntitlementEntityId::Plan(plan_id),
                    EntitlementEntityId::PlanVersion(pvid),
                ],
                product_ids: pv_prods,
                include_globals: true,
            })
        }
        ResolveTarget::Subscription(sid) => {
            let comp_prod = SubscriptionComponentRow::list_product_ids(conn, &[sid], &tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            let addon_prod =
                SubscriptionAddOnRow::list_active_product_ids(conn, &[sid], &tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
            let included =
                SubscriptionComponentRow::list_included_product_ids(conn, &[sid], &tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
            Ok(ResolutionScope {
                chain: build_chain_for_subscription(conn, tenant_id, sid).await?,
                product_ids: itertools::chain!(comp_prod, addon_prod, included)
                    .unique()
                    .collect(),
                include_globals: true,
            })
        }
        ResolveTarget::Quote(qid) => {
            let comp_prod = QuoteComponentRow::list_product_ids(conn, &[qid], &tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            let addon_prod = QuoteAddOnRow::list_product_ids(conn, &[qid], &tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            Ok(ResolutionScope {
                chain: build_chain_for_quote(conn, tenant_id, qid).await?,
                product_ids: itertools::chain!(comp_prod, addon_prod).unique().collect(),
                include_globals: true,
            })
        }
        ResolveTarget::Selection {
            plan_version_id,
            add_on_ids,
            extra_product_ids,
        } => {
            let pv_prods = list_product_ids_for_plan_version(conn, &tenant_id, plan_version_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            let addon_prods: Vec<ProductId> = if add_on_ids.is_empty() {
                Vec::new()
            } else {
                AddOnRow::list_by_ids(conn, &add_on_ids, &tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .into_iter()
                    .map(|a| a.product_id)
                    .collect()
            };
            let plan_id_map = PlanVersionRow::get_plan_ids_by_version_ids(conn, &[plan_version_id])
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            let Some(plan_id) = plan_id_map.get(&plan_version_id).copied() else {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "plan version {plan_version_id} not found"
                ))));
            };
            Ok(ResolutionScope {
                chain: plan_version_chain(plan_id, plan_version_id, &add_on_ids),
                product_ids: itertools::chain!(pv_prods, addon_prods, extra_product_ids)
                    .unique()
                    .collect(),
                include_globals: true,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_for_product_is_empty() {
        let pid = ProductId::new();
        assert!(build_chain_for_product(pid).is_empty());
    }

    #[test]
    fn chain_for_addon_is_just_addon() {
        let aid = AddOnId::new();
        assert_eq!(
            build_chain_for_addon(aid),
            vec![EntitlementEntityId::AddOn(aid)]
        );
    }

    #[test]
    fn plan_version_chain_includes_linked_add_ons() {
        // Algorithm §PlanVersion step 2: add-on entitlements linked to the plan version are
        // part of the chain. Order: Plan → PlanVersion → AddOn₁..n.
        let plan_id = PlanId::new();
        let pv_id = PlanVersionId::new();
        let aid1 = AddOnId::new();
        let aid2 = AddOnId::new();
        assert_eq!(
            plan_version_chain(plan_id, pv_id, &[aid1, aid2]),
            vec![
                EntitlementEntityId::Plan(plan_id),
                EntitlementEntityId::PlanVersion(pv_id),
                EntitlementEntityId::AddOn(aid1),
                EntitlementEntityId::AddOn(aid2),
            ]
        );
    }

    #[test]
    fn plan_version_chain_without_add_ons() {
        let plan_id = PlanId::new();
        let pv_id = PlanVersionId::new();
        assert_eq!(
            plan_version_chain(plan_id, pv_id, &[]),
            vec![
                EntitlementEntityId::Plan(plan_id),
                EntitlementEntityId::PlanVersion(pv_id),
            ]
        );
    }
}
