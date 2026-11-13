use crate::domain::entitlements::{
    ConfigValue, ConfigValueType, EntitlementSpec, EntitlementValue, FeatureRef, FeatureType,
    OverageBehavior, RawResolvedEntitlement, ResetPeriod, ResolvedEntitlement,
    ResolvedEntitlementValue, ResolvedOrigin, merge_thresholds,
};
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::{EntitlementModeEnum, FeatureStatusEnum};
use crate::domain::usage_limits::{MAX_GRACE_PCT, validate_thresholds};
use crate::domain::{
    BillableMetric, Entitlement, EntitlementNew, EntitlementUpdate, Feature, FeatureNew,
    FeatureUpdate, PaginatedVec, PaginationRequest,
};
use crate::errors::StoreError;
use crate::repositories::credit::inputs::InputsOwner;
use crate::services::entitlements::limits::rules::{ungroupable_reason, validate_group_property};
use crate::store::PgConn;
use crate::{Store, StoreResult};
use common_domain::ids::{
    AddOnId, AliasOr, BaseId, CustomerId, EntitlementEntityId, EntitlementId, FeatureId, PlanId,
    PlanVersionId, ProductId, QuoteId, SubscriptionId, TenantId,
};
use diesel_models::add_ons::AddOnRow;
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::entitlements::{
    EntitlementRow, EntitlementRowNew, EntitlementRowPatch, FeatureRow, FeatureRowNew,
    FeatureRowPatch, FeatureWithProductRow,
};
use diesel_models::enums::{EntitlementEntityTypeEnum, FeatureStatusEnum as DbFeatureStatusEnum};
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::plans::PlanRow;
use diesel_models::products::ProductRow;
use diesel_models::query::price_components::list_product_ids_for_plan_version;
use diesel_models::quote_add_ons::QuoteAddOnRow;
use diesel_models::quotes::QuoteComponentRow;
use diesel_models::quotes::QuoteRow;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use itertools::Itertools;
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;
use std::collections::HashSet;

/// Dispatcher target for `resolve_for_entity`.
/// `Product` is a dispatcher-level concept only — features are product-scoped via
/// `feature.product_id`, not through an `EntitlementEntityId::Product` variant.
#[derive(Clone, Debug)]
pub enum ResolveTarget {
    Product(ProductId),
    AddOn(AddOnId),
    PlanVersion(PlanVersionId),
    Subscription(SubscriptionId),
    Quote(QuoteId),
    /// In-flight selection during quote/subscription creation. No real entity exists yet;
    /// the resolution chain contains Plan, the selected AddOns, and the PlanVersion
    /// (PlanVersion overrides linked AddOns — see [`EntitlementEntityId::priority`]).
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

    async fn list_features(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        statuses: Option<Vec<FeatureStatusEnum>>,
        product_id: Option<ProductId>,
        search: Option<String>,
    ) -> StoreResult<PaginatedVec<Feature>>;

    /// The non-archived features among `ids`, with their default entitlement.
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

    /// Transition the feature status. Only Active entitlements resolve. Archiving releases the code
    /// and hides the feature's entitlements without deleting them, and restoring to Active brings
    /// them back; it fails with `FailedPrecondition` while another feature holds the code.
    async fn set_feature_status(
        &self,
        actor: Actor,
        id: FeatureId,
        tenant_id: TenantId,
        status: FeatureStatusEnum,
    ) -> StoreResult<()>;

    /// The archived feature that used `code`, to unarchive it by code. Fails with
    /// `FailedPrecondition` while a non-archived feature holds the code or when several archived
    /// features used it.
    async fn find_archived_feature_by_code(
        &self,
        code: &str,
        tenant_id: TenantId,
    ) -> StoreResult<FeatureId>;

    /// For an archived feature, the non-archived feature now using its code, which blocks
    /// unarchiving.
    async fn feature_code_holder(
        &self,
        id: FeatureId,
        tenant_id: TenantId,
    ) -> StoreResult<Option<FeatureRef>>;

    async fn create_entitlement(&self, entitlement: EntitlementNew) -> StoreResult<Entitlement>;

    async fn get_entitlement(
        &self,
        id: EntitlementId,
        tenant_id: TenantId,
    ) -> StoreResult<Entitlement>;

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

    async fn update_entitlement(
        &self,
        id: EntitlementId,
        tenant_id: TenantId,
        update: EntitlementUpdate,
    ) -> StoreResult<Entitlement>;

    async fn delete_entitlement(&self, id: EntitlementId, tenant_id: TenantId) -> StoreResult<()>;

    /// Resolve all entitlements in effect for a customer across their active subscriptions.
    /// Returns one `ResolvedEntitlement` per feature, composed across every layer that can
    /// grant it. Priority ladder (low → high):
    /// `Feature → Plan → AddOn → PlanVersion → Subscription → Customer`. PlanVersion overrides
    /// its linked AddOns because the plan version is the authoritative composition; `Customer`
    /// is the top-priority **force** — an account-wide grant that overrides whatever a
    /// subscription composes or sets. See [`EntitlementEntityId::priority`].
    ///
    /// Each active subscription is resolved independently and the results are OR-combined, so a
    /// value set on one subscription never masks another's (a feature is enabled for the
    /// customer if it resolves enabled on any subscription). A customer-level force resolves the
    /// same in every subscription, so it surfaces uniformly. A customer with no active
    /// subscription still receives the `Feature` (tenant-global) and `Customer` layers — the
    /// baseline for every customer regardless of what they have subscribed to.
    async fn resolve_entitlements_for_customer(
        &self,
        conn: &mut PgConn,
        customer_id: CustomerId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ResolvedEntitlement>>;

    /// Single-feature variant of [`get_effective_entitlements`].
    /// Returns `None` when the feature is outside the customer's product scope or yields no grants.
    async fn resolve_entitlements_for_feature(
        &self,
        conn: &mut PgConn,
        customer_id: CustomerId,
        tenant_id: TenantId,
        feature_id: FeatureId,
    ) -> StoreResult<Option<ResolvedEntitlement>>;

    /// Resolve entitlements for a single target — product, add-on, plan version, subscription,
    /// quote, or in-flight `Selection` — used for per-entity previews in the UI.
    /// Returns one `ResolvedEntitlement` per feature in the target's product scope.
    async fn resolve_entitlements_for_entity(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        target: ResolveTarget,
    ) -> StoreResult<Vec<ResolvedEntitlement>>;

    /// Create multiple entitlements for a single entity in one operation, skipping any that
    /// already exist (same feature_id + entity). Returns all successfully inserted rows.
    async fn batch_create_entitlements(
        &self,
        tenant_id: TenantId,
        target: EntitlementEntityId,
        specs: Vec<EntitlementSpec>,
    ) -> StoreResult<Vec<Entitlement>>;
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

fn ensure_editable(feature: &Feature) -> StoreResult<()> {
    let reason = match feature.status {
        FeatureStatusEnum::Active | FeatureStatusEnum::Disabled => return Ok(()),
        FeatureStatusEnum::Archived => "feature is archived; unarchive it first",
    };
    Err(Report::new(StoreError::InvalidArgument(reason.into())))
}

/// Reject combinations where the entitlement value's variant does not match the feature's
/// declared type. Without this check the resolver silently drops mismatched rows at read
/// time (see `resolve()` `_ => log::error!(...)` branch).
pub fn validate_value_matches_feature_type(
    value: &EntitlementValue,
    feature_type: &FeatureType,
) -> Result<(), Report<StoreError>> {
    match (feature_type, value) {
        (FeatureType::Boolean, EntitlementValue::Boolean { .. }) => Ok(()),
        (
            FeatureType::Metered { .. },
            EntitlementValue::Metered {
                limit,
                reset_period,
                overage_behavior,
                thresholds,
                group_by,
                ..
            },
        ) => validate_metered_value(
            limit.as_ref(),
            reset_period,
            overage_behavior,
            thresholds,
            group_by.as_deref(),
        ),
        (FeatureType::Config { value_type }, EntitlementValue::Config { value }) => {
            validate_config_value(value_type, value)
        }
        _ => Err(Report::new(StoreError::InvalidArgument(
            "entitlement value variant does not match feature type".into(),
        ))),
    }
}

/// A grouped limit needs a metric whose usage can be told per group; otherwise the engine
/// would silently leave the entitlement unmonitored.
async fn validate_grouping(
    conn: &mut PgConn,
    tenant_id: TenantId,
    feature_type: &FeatureType,
    value: &EntitlementValue,
) -> Result<(), Report<StoreError>> {
    let (
        FeatureType::Metered { metric_id },
        EntitlementValue::Metered {
            group_by: Some(property),
            ..
        },
    ) = (feature_type, value)
    else {
        return Ok(());
    };
    let row = BillableMetricRow::get_by_ids(conn, &[*metric_id], &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .next()
        .ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(format!(
                "metric {metric_id} not found"
            )))
        })?;
    let metric: BillableMetric = row.try_into()?;
    match ungroupable_reason(&metric, Some(property)) {
        Some(reason) => Err(Report::new(StoreError::InvalidArgument(format!(
            "usage cannot be limited per {property}: {reason}"
        )))),
        None => Ok(()),
    }
}

/// A zero interval would divide by zero in `calendar_period`/`fixed_window`, and a negative
/// limit has no meaning for a usage cap.
fn validate_metered_value(
    limit: Option<&Decimal>,
    reset_period: &ResetPeriod,
    overage_behavior: &OverageBehavior,
    thresholds: &[u32],
    group_by: Option<&str>,
) -> Result<(), Report<StoreError>> {
    if let OverageBehavior::Block {
        grace_period_pct: Some(grace),
    } = overage_behavior
        && !(1..=MAX_GRACE_PCT).contains(grace)
    {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "grace percentage must be between 1 and {MAX_GRACE_PCT}"
        ))));
    }
    validate_thresholds(thresholds)
        .map_err(|e| Report::new(StoreError::InvalidArgument(e.to_string())))?;
    if let Some(property) = group_by {
        validate_group_property(property)
            .map_err(|e| Report::new(StoreError::InvalidArgument(e.to_string())))?;
    }
    if limit.is_some_and(|l| l.is_sign_negative()) {
        return Err(Report::new(StoreError::InvalidArgument(
            "metered entitlement limit must not be negative".into(),
        )));
    }
    let default_overage = matches!(
        overage_behavior,
        OverageBehavior::Block {
            grace_period_pct: None
        }
    );
    if limit.is_none() && (!default_overage || !thresholds.is_empty()) {
        return Err(Report::new(StoreError::InvalidArgument(
            "overage behavior, grace and alert thresholds apply to a limit: set a limit or remove them"
                .into(),
        )));
    }
    if !thresholds.is_empty() && matches!(reset_period, ResetPeriod::SlidingWindow { .. }) {
        return Err(Report::new(StoreError::InvalidArgument(
            "a sliding window sends no alerts, so it takes no alert thresholds".into(),
        )));
    }
    match reset_period {
        ResetPeriod::Calendar { interval: 0, .. }
        | ResetPeriod::FixedWindow { interval: 0, .. }
        | ResetPeriod::SlidingWindow { interval: 0, .. } => Err(Report::new(
            StoreError::InvalidArgument("reset period interval must be at least 1".into()),
        )),
        _ => Ok(()),
    }
}

/// Ensure a config entitlement value matches the feature's declared value type. A `Select`
/// feature additionally requires a text value drawn from its allowed options.
fn validate_config_value(
    value_type: &ConfigValueType,
    value: &ConfigValue,
) -> Result<(), Report<StoreError>> {
    let matches = match (value_type, value) {
        (ConfigValueType::Number, ConfigValue::Number(_)) => true,
        (ConfigValueType::Boolean, ConfigValue::Boolean(_)) => true,
        (ConfigValueType::Text, ConfigValue::Text(_)) => true,
        (ConfigValueType::Map | ConfigValueType::Json, ConfigValue::Json(_)) => true,
        (ConfigValueType::Select { options }, ConfigValue::Text(s)) => {
            if !options.iter().any(|o| o == s) {
                return Err(Report::new(StoreError::InvalidArgument(
                    "config value is not one of the feature's allowed options".into(),
                )));
            }
            true
        }
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(Report::new(StoreError::InvalidArgument(
            "config value does not match the feature's value type".into(),
        )))
    }
}

#[async_trait::async_trait]
impl EntitlementsInterface for Store {
    async fn create_feature(&self, actor: Actor, feature: FeatureNew) -> StoreResult<Feature> {
        let mut conn = self.get_conn().await?;
        let entitlement_value = feature.entitlement.clone();
        let tenant_id = feature.tenant_id;
        ensure_product_in_tenant(&mut conn, tenant_id, feature.product_id).await?;
        if let Some(value) = &entitlement_value {
            validate_value_matches_feature_type(value, &feature.feature_type)?;
            validate_grouping(&mut conn, tenant_id, &feature.feature_type, value).await?;
        }
        let row: FeatureRowNew = feature.into();
        drop(conn);
        self.transaction_bumping_tenants(|conn| {
            let actor = &actor;
            async move {
                let inserted = row
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let joined = FeatureRow::find_by_id(conn, inserted.id, inserted.tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let mut f: Feature = joined.try_into()?;

                // A feature reaches customers only through an entitlement.
                let bumped = entitlement_value.is_some().then_some(tenant_id);
                if let Some(value) = entitlement_value {
                    let entity = EntitlementEntityId::Feature(f.id);
                    let json_value = value_to_json(&value)?;
                    let entitlement: Entitlement = EntitlementRowNew {
                        id: EntitlementId::new(),
                        tenant_id,
                        feature_id: f.id,
                        entity_id: entity.as_uuid(),
                        entity_type: EntitlementEntityTypeEnum::from(&entity),
                        mode: EntitlementModeEnum::Override.into(),
                        value: json_value,
                    }
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                    .and_then(TryInto::try_into)?;
                    f.entitlement = Some(entitlement);
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

        let row = FeatureRow::find_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut feature: Feature = row.try_into()?;

        // Like `list_features`, keeps an archived feature's default so both reads agree.
        let entitlement_rows =
            EntitlementRow::list_feature_level_entitlements(&mut conn, tenant_id, Some(&vec![id]))
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        feature.entitlement = entitlement_rows
            .into_iter()
            .next()
            .map(TryInto::try_into)
            .transpose()?;

        Ok(feature)
    }

    async fn get_feature_by_id_or_code(
        &self,
        id_or_code: AliasOr<FeatureId>,
        tenant_id: TenantId,
    ) -> StoreResult<Feature> {
        let mut conn = self.get_conn().await?;

        let row = FeatureRow::find_by_id_or_code(&mut conn, id_or_code, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let feature_id = row.feature.id;
        let mut feature: Feature = row.try_into()?;

        let entitlement_rows = EntitlementRow::list_feature_level_entitlements(
            &mut conn,
            tenant_id,
            Some(&vec![feature_id]),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        feature.entitlement = entitlement_rows
            .into_iter()
            .next()
            .map(TryInto::try_into)
            .transpose()?;

        Ok(feature)
    }

    async fn list_features(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        statuses: Option<Vec<FeatureStatusEnum>>,
        product_id: Option<ProductId>,
        search: Option<String>,
    ) -> StoreResult<PaginatedVec<Feature>> {
        let mut conn = self.get_conn().await?;

        let db_statuses = statuses.map(|v| {
            v.into_iter()
                .map(Into::<DbFeatureStatusEnum>::into)
                .collect::<Vec<_>>()
        });
        let rows = FeatureRow::list(
            &mut conn,
            tenant_id,
            pagination.into(),
            db_statuses,
            product_id,
            search,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let feature_ids: Vec<FeatureId> = rows.items.iter().map(|r| r.feature.id).collect();

        let entitlement_rows = EntitlementRow::list_feature_level_entitlements(
            &mut conn,
            tenant_id,
            Some(&feature_ids),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let mut entitlement_map = build_feature_entitlement_map(entitlement_rows)?;

        let items = rows
            .items
            .into_iter()
            .map(|row| {
                let mut f: Feature = row.try_into()?;
                f.entitlement = entitlement_map.remove(&f.id);
                Ok::<Feature, Report<StoreError>>(f)
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(PaginatedVec {
            items,
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

        let rows = FeatureRow::find_by_ids(&mut conn, tenant_id, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let feature_ids: Vec<FeatureId> = rows.iter().map(|r| r.feature.id).collect();
        let entitlement_rows = EntitlementRow::list_feature_level_entitlements(
            &mut conn,
            tenant_id,
            Some(&feature_ids),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let mut entitlement_map = build_feature_entitlement_map(entitlement_rows)?;

        rows.into_iter()
            .map(|row| {
                let mut f: Feature = row.try_into()?;
                f.entitlement = entitlement_map.remove(&f.id);
                Ok(f)
            })
            .collect()
    }

    async fn update_feature(
        &self,
        id: FeatureId,
        tenant_id: TenantId,
        update: FeatureUpdate,
    ) -> StoreResult<Feature> {
        let mut conn = self.get_conn().await?;

        let current: Feature = FeatureRow::find_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(TryInto::try_into)?;
        ensure_editable(&current)?;
        ensure_product_in_tenant(&mut conn, tenant_id, update.product_id.flatten()).await?;

        // When replacing a select feature's options, reject removing any option that an
        // existing entitlement still references.
        let config_options_patch = if let Some(new_options) = &update.config_options {
            let FeatureType::Config {
                value_type:
                    ConfigValueType::Select {
                        options: current_options,
                    },
            } = current.feature_type
            else {
                return Err(Report::new(StoreError::InvalidArgument(
                    "config options can only be set on select config features".into(),
                )));
            };
            if new_options.is_empty() {
                return Err(Report::new(StoreError::InvalidArgument(
                    "a select config feature requires at least one option".into(),
                )));
            }
            let removed: Vec<String> = current_options
                .into_iter()
                .filter(|o| !new_options.contains(o))
                .collect();
            if !removed.is_empty() {
                let rows = EntitlementRow::list_by_feature(&mut conn, id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let mut in_use: Vec<String> = Vec::new();
                for row in rows {
                    if let Ok(EntitlementValue::Config {
                        value: ConfigValue::Text(t),
                    }) = serde_json::from_value::<EntitlementValue>(row.value)
                        && removed.contains(&t)
                        && !in_use.contains(&t)
                    {
                        in_use.push(t);
                    }
                }
                if !in_use.is_empty() {
                    return Err(Report::new(StoreError::InvalidArgument(format!(
                        "Cannot remove option(s) still used by entitlements: {}",
                        in_use.join(", ")
                    ))));
                }
            }
            Some(Some(new_options.iter().cloned().map(Some).collect()))
        } else {
            None
        };

        let patch = FeatureRowPatch {
            name: update.name,
            description: update.description,
            product_id: update.product_id,
            config_options: config_options_patch,
            status: None,
            updated_at: Some(common_domain::clock::now()),
        };

        drop(conn);

        let row = self
            .transaction_bumping_tenant(tenant_id, |conn| {
                async move {
                    FeatureRow::update(conn, id, tenant_id, patch)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)
                }
                .scope_boxed()
            })
            .await?;

        let mut feature: Feature = row.try_into()?;

        let mut conn = self.get_conn().await?;
        let entitlement_rows =
            EntitlementRow::list_by_entity(&mut conn, tenant_id, EntitlementEntityId::Feature(id))
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        feature.entitlement = entitlement_rows
            .into_iter()
            .next()
            .map(TryInto::try_into)
            .transpose()?;

        Ok(feature)
    }

    async fn set_feature_status(
        &self,
        actor: Actor,
        id: FeatureId,
        tenant_id: TenantId,
        status: FeatureStatusEnum,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        let current: Feature = FeatureRow::find_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(TryInto::try_into)?;
        if current.status == status {
            return Ok(());
        }
        let activity_type = match (&current.status, &status) {
            (FeatureStatusEnum::Archived, FeatureStatusEnum::Disabled) => {
                return Err(Report::new(StoreError::InvalidArgument(
                    "feature is archived; unarchive it first".into(),
                )));
            }
            (_, FeatureStatusEnum::Archived) => ActivityType::FeatureArchived,
            (FeatureStatusEnum::Archived, _) => ActivityType::FeatureUnarchived,
            (_, FeatureStatusEnum::Active) => ActivityType::FeatureEnabled,
            (_, FeatureStatusEnum::Disabled) => ActivityType::FeatureDisabled,
        };
        if current.status == FeatureStatusEnum::Archived
            && let Some(holder) =
                FeatureRow::find_unarchived_by_code(&mut conn, tenant_id, &current.code, id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
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
                FeatureRow::set_status(conn, id, tenant_id, status.into())
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

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
        let rows = FeatureRow::list_by_code(&mut conn, tenant_id, code)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
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
        let feature: Feature = FeatureRow::find_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .try_into()?;
        if feature.status != FeatureStatusEnum::Archived {
            return Ok(None);
        }
        let Some(holder) =
            FeatureRow::find_unarchived_by_code(&mut conn, tenant_id, &feature.code, id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        else {
            return Ok(None);
        };
        let holder: Feature = FeatureRow::find_by_id(&mut conn, holder.id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .try_into()?;
        Ok(Some(FeatureRef {
            id: holder.id,
            name: holder.name,
            code: holder.code,
            product: holder.product,
        }))
    }

    async fn create_entitlement(&self, entitlement: EntitlementNew) -> StoreResult<Entitlement> {
        let mut conn = self.get_conn().await?;

        let feature: Feature =
            FeatureRow::find_by_id(&mut conn, entitlement.feature_id, entitlement.tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)
                .and_then(TryInto::try_into)?;
        ensure_editable(&feature)?;
        validate_value_matches_feature_type(&entitlement.value, &feature.feature_type)?;
        validate_grouping(
            &mut conn,
            entitlement.tenant_id,
            &feature.feature_type,
            &entitlement.value,
        )
        .await?;

        let mode =
            resolve_mode_for_entity(&mut conn, &entitlement.entity, entitlement.tenant_id).await?;
        let json_value = value_to_json(&entitlement.value)?;
        let row = EntitlementRowNew {
            id: EntitlementId::new(),
            tenant_id: entitlement.tenant_id,
            feature_id: entitlement.feature_id,
            entity_id: entitlement.entity.as_uuid(),
            entity_type: EntitlementEntityTypeEnum::from(&entitlement.entity),
            mode: mode.into(),
            value: json_value,
        };

        drop(conn);

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
                    let inserted = row.insert(conn).await?;
                    Ok((inserted, owner.bump(conn).await?))
                }
                .scope_boxed()
            })
            .await?;
        row.try_into()
    }

    async fn get_entitlement(
        &self,
        id: EntitlementId,
        tenant_id: TenantId,
    ) -> StoreResult<Entitlement> {
        let mut conn = self.get_conn().await?;

        EntitlementRow::find_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn list_entitlements_by_feature(
        &self,
        feature_id: FeatureId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Entitlement>> {
        let mut conn = self.get_conn().await?;

        let rows = EntitlementRow::list_by_feature(&mut conn, feature_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        rows.into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn list_entitlements_by_entity(
        &self,
        entity: EntitlementEntityId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Entitlement>> {
        let mut conn = self.get_conn().await?;

        let rows = EntitlementRow::list_by_entity(&mut conn, tenant_id, entity)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        rows.into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn update_entitlement(
        &self,
        id: EntitlementId,
        tenant_id: TenantId,
        update: EntitlementUpdate,
    ) -> StoreResult<Entitlement> {
        let mut conn = self.get_conn().await?;

        let patch_value = if let Some(value) = &update.value {
            let existing: Entitlement = EntitlementRow::find_by_id(&mut conn, id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)
                .and_then(TryInto::try_into)?;
            let feature: Feature =
                FeatureRow::find_by_id(&mut conn, existing.feature_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                    .and_then(|r| r.try_into())?;
            ensure_editable(&feature)?;
            validate_value_matches_feature_type(value, &feature.feature_type)?;
            validate_grouping(&mut conn, tenant_id, &feature.feature_type, value).await?;
            Some(value_to_json(value)?)
        } else {
            None
        };

        let patch = EntitlementRowPatch {
            mode: None,
            value: patch_value,
            updated_at: Some(common_domain::clock::now()),
        };

        drop(conn);

        let row = self
            .transaction_bumping_tenants(|conn| {
                async move {
                    let owner = entitlement_owner(conn, id, tenant_id).await?;
                    let updated = EntitlementRow::update(conn, id, tenant_id, patch).await?;
                    Ok((updated, owner.bump(conn).await?))
                }
                .scope_boxed()
            })
            .await?;
        row.try_into()
    }

    async fn delete_entitlement(&self, id: EntitlementId, tenant_id: TenantId) -> StoreResult<()> {
        self.transaction_bumping_tenants(|conn| {
            async move {
                let owner = entitlement_owner(conn, id, tenant_id).await?;
                EntitlementRow::delete(conn, id, tenant_id).await?;
                Ok(((), owner.bump(conn).await?))
            }
            .scope_boxed()
        })
        .await
    }

    async fn resolve_entitlements_for_customer(
        &self,
        conn: &mut PgConn,
        customer_id: CustomerId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ResolvedEntitlement>> {
        self.transaction_with(conn, |conn| {
            async move { resolve_entitlements_tx(conn, customer_id, tenant_id, None).await }
                .scope_boxed()
        })
        .await
    }

    async fn resolve_entitlements_for_entity(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        target: ResolveTarget,
    ) -> StoreResult<Vec<ResolvedEntitlement>> {
        let scope = build_chain_and_products(conn, tenant_id, target).await?;
        resolve_for_chain(
            conn,
            tenant_id,
            &scope.chain,
            &scope.product_ids,
            scope.include_globals,
            None,
        )
        .await
    }

    async fn resolve_entitlements_for_feature(
        &self,
        conn: &mut PgConn,
        customer_id: CustomerId,
        tenant_id: TenantId,
        feature_id: FeatureId,
    ) -> StoreResult<Option<ResolvedEntitlement>> {
        self.transaction_with(conn, |conn| {
            async move {
                let mut all =
                    resolve_entitlements_tx(conn, customer_id, tenant_id, Some(feature_id)).await?;
                Ok(all.pop())
            }
            .scope_boxed()
        })
        .await
    }

    async fn batch_create_entitlements(
        &self,
        tenant_id: TenantId,
        target: EntitlementEntityId,
        specs: Vec<EntitlementSpec>,
    ) -> StoreResult<Vec<Entitlement>> {
        if specs.is_empty() {
            return Ok(vec![]);
        }
        let mut conn = self.get_conn().await?;
        let rows = build_entitlement_rows(&mut conn, specs, &target, tenant_id, false).await?;
        let entity_type = EntitlementEntityTypeEnum::from(&target);
        let entity_id = target.as_uuid();
        drop(conn);
        let inserted = self
            .transaction_bumping_tenants(|conn| {
                async move {
                    let owner =
                        InputsOwner::of_entitlement(conn, tenant_id, &entity_type, entity_id)
                            .await?;
                    let inserted =
                        EntitlementRowNew::insert_batch_skip_conflicts(&rows, conn).await?;
                    Ok((inserted, owner.bump(conn).await?))
                }
                .scope_boxed()
            })
            .await?;
        inserted.into_iter().map(TryInto::try_into).collect()
    }
}

pub(crate) use crate::services::entitlements::limits::rules::{
    BillingCyclePeriod, compute_usage_period,
};

async fn entitlement_owner(
    conn: &mut PgConn,
    id: EntitlementId,
    tenant_id: TenantId,
) -> StoreResult<InputsOwner> {
    let existing = EntitlementRow::find_by_id(conn, id, tenant_id).await?;
    InputsOwner::of_entitlement(conn, tenant_id, &existing.entity_type, existing.entity_id).await
}

fn build_feature_entitlement_map(
    rows: Vec<EntitlementRow>,
) -> StoreResult<std::collections::HashMap<FeatureId, Entitlement>> {
    rows.into_iter()
        .map(|row| {
            let fid = FeatureId::from(row.entity_id);
            TryInto::<Entitlement>::try_into(row).map(|e| (fid, e))
        })
        .collect()
}

/// Enrich each [`RawResolvedEntitlement`] with a display name for its origin entity, producing
/// [`ResolvedEntitlement`]. Batch-loads names per variant in a single round-trip each. The
/// non-optional `origin.name` on the returned type is the type-level signal that enrichment
/// has happened; empty string indicates the linked entity was deleted or has no name.
async fn with_origin_names(
    conn: &mut PgConn,
    tenant_id: TenantId,
    items: Vec<RawResolvedEntitlement>,
) -> StoreResult<Vec<ResolvedEntitlement>> {
    use std::collections::{HashMap, HashSet};

    // 1) Collect distinct ids per variant.
    let mut plan_ids: HashSet<PlanId> = HashSet::new();
    let mut pv_ids: HashSet<PlanVersionId> = HashSet::new();
    let mut addon_ids: HashSet<AddOnId> = HashSet::new();
    let mut feature_ids: HashSet<FeatureId> = HashSet::new();
    for it in items.iter() {
        match it.origin_entity {
            EntitlementEntityId::Plan(id) => {
                plan_ids.insert(id);
            }
            EntitlementEntityId::PlanVersion(id) => {
                pv_ids.insert(id);
            }
            EntitlementEntityId::AddOn(id) => {
                addon_ids.insert(id);
            }
            EntitlementEntityId::Feature(id) => {
                feature_ids.insert(id);
            }
            EntitlementEntityId::Customer(_)
            | EntitlementEntityId::Subscription(_)
            | EntitlementEntityId::Quote(_) => {}
        }
    }

    // 2) Batch-load plan names (needed for Plan and PlanVersion origins).
    //    For PlanVersion we also need the version number and parent plan_id from the PV row.
    let pv_info: HashMap<PlanVersionId, (PlanId, i32)> = if pv_ids.is_empty() {
        HashMap::new()
    } else {
        let ids: Vec<PlanVersionId> = pv_ids.into_iter().collect();
        PlanVersionRow::list_by_ids_and_tenant_id(conn, &ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|pv| (pv.id, (pv.plan_id, pv.version)))
            .collect()
    };
    // Add any parent plan_ids from PV rows that aren't already in plan_ids.
    for (plan_id, _) in pv_info.values() {
        plan_ids.insert(*plan_id);
    }

    let plan_names: HashMap<PlanId, String> = if plan_ids.is_empty() {
        HashMap::new()
    } else {
        let ids: Vec<PlanId> = plan_ids.into_iter().collect();
        PlanRow::list_by_ids(conn, &ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|p| (p.id, p.name))
            .collect()
    };

    let addon_names: HashMap<AddOnId, String> = if addon_ids.is_empty() {
        HashMap::new()
    } else {
        let ids: Vec<AddOnId> = addon_ids.into_iter().collect();
        AddOnRow::list_by_ids(conn, &ids, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|a| (a.id, a.name))
            .collect()
    };

    let feature_names: HashMap<FeatureId, String> = if feature_ids.is_empty() {
        HashMap::new()
    } else {
        let ids: Vec<FeatureId> = feature_ids.into_iter().collect();
        FeatureRow::find_by_ids(conn, tenant_id, &ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|r| (r.feature.id, r.feature.name))
            .collect()
    };

    let lookup = |entity: &EntitlementEntityId| -> Option<String> {
        match entity {
            EntitlementEntityId::Plan(id) => plan_names.get(id).cloned(),
            EntitlementEntityId::PlanVersion(id) => pv_info
                .get(id)
                .and_then(|(plan_id, ver)| plan_names.get(plan_id).map(|n| format!("{n} v{ver}"))),
            EntitlementEntityId::AddOn(id) => addon_names.get(id).cloned(),
            EntitlementEntityId::Feature(id) => feature_names.get(id).cloned(),
            EntitlementEntityId::Customer(_)
            | EntitlementEntityId::Subscription(_)
            | EntitlementEntityId::Quote(_) => None,
        }
    };

    Ok(items
        .into_iter()
        .map(|raw| ResolvedEntitlement {
            feature: raw.feature,
            value: raw.value,
            origin: ResolvedOrigin {
                name: lookup(&raw.origin_entity),
                entity: raw.origin_entity,
            },
        })
        .collect())
}

fn value_to_json(value: &EntitlementValue) -> StoreResult<serde_json::Value> {
    serde_json::to_value(value).map_err(|e| {
        Report::new(StoreError::InvalidArgument(format!(
            "failed to serialize entitlement value: {e}"
        )))
    })
}

/// Pick the composition mode automatically based on the owning entity.
/// AddOn with `max_instances_per_subscription > 1` (or NULL = unbounded) → `Stack`, so each
/// instance of the add-on stacks additively (e.g. each credits pack adds 1000). Everything
/// else → `Override` (the entity's value replaces lower-priority levels in the chain).
pub(crate) async fn resolve_mode_for_entity(
    conn: &mut PgConn,
    entity: &EntitlementEntityId,
    tenant_id: TenantId,
) -> StoreResult<EntitlementModeEnum> {
    match entity {
        EntitlementEntityId::AddOn(add_on_id) => {
            is_addon_multi_instance(conn, *add_on_id, tenant_id)
                .await
                .map(|multi| {
                    if multi {
                        EntitlementModeEnum::Stack
                    } else {
                        EntitlementModeEnum::Override
                    }
                })
        }
        _ => Ok(EntitlementModeEnum::Override),
    }
}

async fn is_addon_multi_instance(
    conn: &mut PgConn,
    add_on_id: AddOnId,
    tenant_id: TenantId,
) -> StoreResult<bool> {
    let rows = AddOnRow::list_by_ids(conn, &[add_on_id], &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let Some(ao) = rows.into_iter().next() else {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "add-on {add_on_id} not found"
        ))));
    };
    Ok(match ao.max_instances_per_subscription {
        None => true,
        Some(n) => n > 1,
    })
}

/// Build the `EntitlementRowNew` batch for one entity, validating each spec's value variant
/// against its feature's type. Shared by `insert_entitlement_specs` and
/// `batch_create_entitlements` — they differ only in which insert path runs afterwards
/// (`insert_batch` vs `insert_batch_skip_conflicts`).
/// Archived features take no new entitlements: with `skip_archived` their specs are dropped, so
/// creating an entity never fails on a feature's status, and otherwise they are rejected.
async fn build_entitlement_rows(
    conn: &mut PgConn,
    specs: Vec<EntitlementSpec>,
    entity: &EntitlementEntityId,
    tenant_id: TenantId,
    skip_archived: bool,
) -> StoreResult<Vec<EntitlementRowNew>> {
    let feature_ids: Vec<FeatureId> = specs.iter().map(|s| s.feature_id).collect();
    let archived: HashSet<FeatureId> = FeatureRow::find_archived_ids(conn, tenant_id, &feature_ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .collect();
    let features: Vec<FeatureWithProductRow> =
        FeatureRow::find_by_ids(conn, tenant_id, &feature_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    let feature_type_map: std::collections::HashMap<FeatureId, Feature> = features
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<Vec<Feature>, _>>()?
        .into_iter()
        .map(|f| (f.id, f))
        .collect();

    let mode = resolve_mode_for_entity(conn, entity, tenant_id).await?;
    let entity_id = entity.as_uuid();
    let entity_type = EntitlementEntityTypeEnum::from(entity);

    let mut rows = Vec::with_capacity(specs.len());
    for spec in specs {
        if archived.contains(&spec.feature_id) {
            if skip_archived {
                continue;
            }
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "feature {} is archived",
                spec.feature_id
            ))));
        }
        let feature = feature_type_map.get(&spec.feature_id).ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(format!(
                "feature {} not found",
                spec.feature_id
            )))
        })?;
        validate_value_matches_feature_type(&spec.value, &feature.feature_type)?;
        validate_grouping(conn, tenant_id, &feature.feature_type, &spec.value).await?;
        let json_value = value_to_json(&spec.value)?;
        rows.push(EntitlementRowNew {
            id: EntitlementId::new(),
            tenant_id,
            feature_id: spec.feature_id,
            entity_id,
            entity_type,
            mode: mode.clone().into(),
            value: json_value,
        });
    }
    Ok(rows)
}

/// Insert a batch of entitlement specs for a single entity inside an existing transaction.
/// The caller provides the entity (the thing being created), tenant/actor context, and the specs.
/// Specs for archived features are skipped, see `build_entitlement_rows`.
pub(crate) async fn insert_entitlement_specs(
    conn: &mut PgConn,
    specs: Vec<EntitlementSpec>,
    entity: EntitlementEntityId,
    tenant_id: TenantId,
) -> StoreResult<Vec<Entitlement>> {
    if specs.is_empty() {
        return Ok(vec![]);
    }

    let rows = build_entitlement_rows(conn, specs, &entity, tenant_id, true).await?;
    if rows.is_empty() {
        return Ok(vec![]);
    }
    EntitlementRowNew::insert_batch(&rows, conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .and_then(|rows| rows.into_iter().map(TryInto::try_into).collect())
}

fn build_chain_for_product(_: ProductId) -> Vec<EntitlementEntityId> {
    Vec::new()
}

fn build_chain_for_addon(addon_id: AddOnId) -> Vec<EntitlementEntityId> {
    vec![EntitlementEntityId::AddOn(addon_id)]
}

/// Pure chain assembly for a plan version target: `[Plan, PlanVersion, AddOn₁..n]`.
/// Shared by PlanVersion / Subscription / Quote / Selection targets. Vec order is
/// insertion order, not priority order — `resolve()` re-sorts by priority, which
/// puts AddOns below PlanVersion (PlanVersion overrides its linked AddOns).
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
    let mut chain = plan_version_chain(sub.plan_id, sub.subscription.plan_version_id, &addon_ids);
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
            Ok(ResolutionScope {
                chain: build_chain_for_subscription(conn, tenant_id, sid).await?,
                product_ids: itertools::chain!(comp_prod, addon_prod).unique().collect(),
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

/// True when `chain` contains both a `Subscription(_)` and a `Quote(_)` — a configuration
/// the resolver assumes never occurs (a chain is built per customer subscription *or* per
/// quote, never both). Used by a `debug_assert!` in [`resolve_for_chain`].
fn chain_mixes_subscription_and_quote(chain: &[EntitlementEntityId]) -> bool {
    let mut has_sub = false;
    let mut has_quote = false;
    for e in chain {
        match e {
            EntitlementEntityId::Subscription(_) => has_sub = true,
            EntitlementEntityId::Quote(_) => has_quote = true,
            _ => {}
        }
        if has_sub && has_quote {
            return true;
        }
    }
    false
}

/// Shared resolution pipeline used by both `resolve_for_entity` and `resolve_entitlements_tx`.
///
/// ## Resolution algorithm
/// 1. Load scoped features (not deleted) for `product_ids`, including
///    globally-scoped features (`product_id IS NULL`). Product names are joined at DB level.
/// 2. Load entity-bound entitlements for every id in `chain`, optionally narrowed to
///    `filter_feature_id` at the DB level.
/// 3. Load feature-level (tenant-default) entitlements, restricted to scoped feature ids
///    to prevent cross-product leakage.
/// 4. Drop rows outside the product scope or `filter_feature_id`.
/// 5. [`resolve`]: sort by `entity.priority()` asc — `Feature(0) < Plan(1) < AddOn(2) <
///    PlanVersion(3) < Subscription(4) = Quote(4) < Customer(5)` — then fold. Higher priority replaces
///    accumulated; same-priority merges permissively (or additively for multi-instance
///    add-ons). Features with status != Active are skipped.
/// 6. [`with_origin_names`]: batch-load display names for each distinct origin entity variant.
async fn resolve_for_chain(
    conn: &mut PgConn,
    tenant_id: TenantId,
    chain: &[EntitlementEntityId],
    product_ids: &[ProductId],
    include_globals: bool,
    filter_feature_id: Option<FeatureId>,
) -> StoreResult<Vec<ResolvedEntitlement>> {
    debug_assert!(
        !chain_mixes_subscription_and_quote(chain),
        "resolution chain must not mix Subscription and Quote (they share priority 4)"
    );

    let scoped_feature_rows =
        FeatureRow::find_active_for_products(conn, tenant_id, product_ids, include_globals)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    let scoped_features: Vec<Feature> = scoped_feature_rows
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<Vec<_>, _>>()?;
    let scoped_feature_ids: Vec<FeatureId> = scoped_features.iter().map(|f| f.id).collect();

    let entity_rows = if chain.is_empty() {
        Vec::new()
    } else {
        EntitlementRow::list_by_entity_ids(conn, tenant_id, chain, filter_feature_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
    };
    let feature_level_rows =
        EntitlementRow::list_feature_level_entitlements(conn, tenant_id, Some(&scoped_feature_ids))
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

    let all_entitlements: Vec<Entitlement> = entity_rows
        .into_iter()
        .chain(feature_level_rows)
        .map(TryInto::try_into)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|e: &Entitlement| scoped_feature_ids.contains(&e.feature_id))
        .filter(|e| filter_feature_id.is_none_or(|fid| e.feature_id == fid))
        .collect();

    let features_in_play: Vec<Feature> = match filter_feature_id {
        Some(fid) => scoped_features
            .into_iter()
            .filter(|f| f.id == fid)
            .collect(),
        None => scoped_features,
    };

    let multi_instance_addons = load_multi_instance_addons(conn, tenant_id, chain).await?;

    with_origin_names(
        conn,
        tenant_id,
        resolve(all_entitlements, features_in_play, &multi_instance_addons),
    )
    .await
}

/// Identify add-ons in the chain that are multi-instance (`max_instances_per_subscription` > 1
/// or NULL). Entitlements owned by these add-ons compose additively when colliding at the same
/// priority; other entities compose permissively. See [`resolve`].
async fn load_multi_instance_addons(
    conn: &mut PgConn,
    tenant_id: TenantId,
    chain: &[EntitlementEntityId],
) -> StoreResult<std::collections::HashSet<AddOnId>> {
    let add_on_ids: Vec<AddOnId> = chain
        .iter()
        .filter_map(|e| match e {
            EntitlementEntityId::AddOn(id) => Some(*id),
            _ => None,
        })
        .collect();
    if add_on_ids.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    let rows = AddOnRow::list_by_ids(conn, &add_on_ids, &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(rows
        .into_iter()
        .filter(|a| a.max_instances_per_subscription.is_none_or(|n| n > 1))
        .map(|a| a.id)
        .collect())
}

/// Customer-wide entitlement resolution.
///
/// Each active subscription is resolved **independently** (its own chain — including the customer
/// and feature layers — via [`ResolveTarget::Subscription`]) and the results are OR-combined
/// across subscriptions: a feature is enabled for the customer if it resolves enabled on *any*
/// subscription; metered limits take the most permissive. Folding every subscription into one
/// flat chain would instead let a value set on one subscription mask another's — e.g. one site
/// disabling a feature would hide another site's grant — so the per-subscription pass is what
/// keeps `enabled on site A / disabled on site B` correct. Per-subscription resolution stays
/// authoritative for site-scoped checks; this answers "does the customer have it anywhere".
///
/// Crucially the OR is across subscriptions **only**. Each subscription pass already folds in the
/// `Feature` (baseline) and `Customer` (top-priority force) layers, so a customer-level force wins
/// within every scope while a plan or subscription can still override the feature default. The
/// separate account-level pass (`Feature` + `Customer` only) is a **fallback**: it fills features
/// no subscription's product scope covers (e.g. a customer force on an unsubscribed product) and
/// is the sole result for a customer with no active subscription. It is not OR'd as a peer — doing
/// so would let the low-priority `Feature` default resurrect a feature a subscription disabled.
async fn resolve_entitlements_tx(
    conn: &mut PgConn,
    customer_id: CustomerId,
    tenant_id: TenantId,
    filter_feature_id: Option<FeatureId>,
) -> StoreResult<Vec<ResolvedEntitlement>> {
    use std::collections::HashSet;

    let sub_rows = SubscriptionRow::find_active_by_customer(conn, customer_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    // Account-level baseline: tenant-global feature defaults (priority 0) plus this customer's
    // own overrides (priority 1). The product scope includes the products of any feature the
    // customer overrides so product-scoped customer grants resolve too (globals are always in
    // scope via `include_globals`).
    let account_chain = [EntitlementEntityId::Customer(customer_id)];
    let account_products = customer_override_product_ids(conn, tenant_id, customer_id).await?;
    let account = resolve_for_chain(
        conn,
        tenant_id,
        &account_chain,
        &account_products,
        true,
        filter_feature_id,
    )
    .await?;

    if sub_rows.is_empty() {
        return Ok(account);
    }

    let mut sub_scopes: Vec<Vec<ResolvedEntitlement>> = Vec::with_capacity(sub_rows.len());
    for sub in &sub_rows {
        let scope =
            build_chain_and_products(conn, tenant_id, ResolveTarget::Subscription(sub.id)).await?;
        let resolved = resolve_for_chain(
            conn,
            tenant_id,
            &scope.chain,
            &scope.product_ids,
            scope.include_globals,
            filter_feature_id,
        )
        .await?;
        sub_scopes.push(resolved);
    }

    let mut combined = aggregate_scopes_permissive(sub_scopes);

    // Fallback: surface account-level entitlements for features no subscription scope covered.
    let covered: HashSet<FeatureId> = combined.iter().map(|e| e.feature.id).collect();
    combined.extend(
        account
            .into_iter()
            .filter(|e| !covered.contains(&e.feature.id)),
    );

    Ok(combined)
}

/// Product ids of every feature this customer has a customer-level entitlement for, so the
/// account-level resolution scope can be widened to include product-scoped customer overrides.
/// Global features (`product_id IS NULL`) are always in scope and need no widening.
async fn customer_override_product_ids(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> StoreResult<Vec<ProductId>> {
    let rows =
        EntitlementRow::list_by_entity(conn, tenant_id, EntitlementEntityId::Customer(customer_id))
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let feature_ids: Vec<FeatureId> = rows.iter().map(|r| r.feature_id).collect();
    let features = FeatureRow::find_by_ids(conn, tenant_id, &feature_ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(features
        .into_iter()
        .filter_map(|f| f.feature.product_id)
        .unique()
        .collect())
}

/// OR-combine per-scope resolutions into the customer-wide view. Groups by feature; when a
/// feature resolves in more than one scope, keeps the most permissive value — boolean OR, and
/// for metered the larger limit (`None` = unlimited) carrying its own reset/overage. The origin
/// follows the contributing scope: an already-enabled boolean keeps its origin, and a later
/// scope only takes it over when it flips the value on.
fn aggregate_scopes_permissive(scopes: Vec<Vec<ResolvedEntitlement>>) -> Vec<ResolvedEntitlement> {
    use std::collections::HashMap;
    use std::collections::hash_map::Entry;

    let mut by_feature: HashMap<FeatureId, ResolvedEntitlement> = HashMap::new();
    for scope in scopes {
        for ent in scope {
            match by_feature.entry(ent.feature.id) {
                Entry::Vacant(v) => {
                    v.insert(ent);
                }
                Entry::Occupied(mut o) => {
                    let merged = merge_resolved_permissive(o.get().clone(), ent);
                    o.insert(merged);
                }
            }
        }
    }
    by_feature.into_values().collect()
}

/// Permissive OR of two already-resolved values for the same feature. Mirrors
/// [`merge_permissive`] but over [`ResolvedEntitlementValue`], and tracks which scope's origin
/// to keep. Used only by [`aggregate_scopes_permissive`].
fn merge_resolved_permissive(
    a: ResolvedEntitlement,
    b: ResolvedEntitlement,
) -> ResolvedEntitlement {
    match (a.value, b.value) {
        (
            ResolvedEntitlementValue::Boolean { enabled: ea },
            ResolvedEntitlementValue::Boolean { enabled: eb },
        ) => {
            let origin = if eb && !ea { b.origin } else { a.origin };
            ResolvedEntitlement {
                feature: a.feature,
                value: ResolvedEntitlementValue::Boolean { enabled: ea || eb },
                origin,
            }
        }
        (
            ResolvedEntitlementValue::Metered {
                metric_id,
                limit: la,
                reset_period: rpa,
                overage_behavior: oba,
                thresholds: ta,
                enabled: ea,
                group_by: gba,
            },
            ResolvedEntitlementValue::Metered {
                limit: lb,
                reset_period: rpb,
                overage_behavior: obb,
                thresholds: tb,
                enabled: eb,
                group_by: gbb,
                ..
            },
        ) => {
            // Larger limit wins (None = unlimited) and carries its own reset/overage/origin.
            let b_wins = match (la, lb) {
                (None, _) => false,
                (Some(_), None) => true,
                (Some(x), Some(y)) => y > x,
            };
            let (limit, reset_period, overage_behavior, group_by, origin) = if b_wins {
                (lb, rpb, obb, gbb, b.origin)
            } else {
                (la, rpa, oba, gba, a.origin)
            };
            ResolvedEntitlement {
                feature: a.feature,
                value: ResolvedEntitlementValue::Metered {
                    metric_id,
                    limit,
                    reset_period,
                    overage_behavior,
                    thresholds: merge_thresholds(&ta, &tb),
                    enabled: ea || eb,
                    group_by,
                },
                origin,
            }
        }
        // Same feature ⇒ same variant, so a mismatch is unreachable. Keep `a` defensively.
        (value, _) => ResolvedEntitlement {
            feature: a.feature,
            value,
            origin: a.origin,
        },
    }
}

/// Per-feature fold over a pre-filtered, pre-scoped set of entitlements.
///
/// Priority ladder (low → high):
/// `Feature(0) < Plan(1) < AddOn(2) < PlanVersion(3) < Subscription(4) = Quote(4) < Customer(5)`.
/// A customer-level force wins for a customer; PlanVersion overrides its linked AddOns
/// because the plan version is the authoritative composition for the plan.
///
/// Algorithm:
/// - Skip switched-off features (global off-switch) and deleted ones.
/// - Sort entitlements by `entity.priority()` asc, then `created_at`, then `id` (stable).
/// - Walk in order. Per entitlement:
///   - **Same priority as accumulator**: merge with the accumulated value.
///     - If entity is a multi-instance add-on (member of `multi_instance_addons`) → additive
///       sum via [`merge_additive`] (algorithm §3).
///     - Else → permissive merge via [`merge_permissive`] — max for metered limits, OR for
///       boolean (algorithm §2).
///   - **Higher priority**: replace the accumulator (algorithm §1).
/// - `origin` records the entity whose value actually contributed to the resolved value
///   (algorithm §4). Override / first-seen: the new entity wins. Same-priority permissive
///   merge: the side that contributed the visible field (the metered limit) wins; boolean
///   ties keep the existing origin as a stable convention. Same-priority additive merge:
///   both sides contribute, so the earlier-created entity (the existing accumulator's
///   origin, since entries within a priority bucket are sorted by `created_at` ascending)
///   is kept as the stable choice.
///
/// The stored `Entitlement.mode` column is **ignored** — composition is derived from
/// `multi_instance_addons` at resolve time, so changes to an add-on's `max_instances_per_subscription`
/// take effect without rewriting historic entitlement rows.
fn resolve(
    entitlements: Vec<Entitlement>,
    features: Vec<Feature>,
    multi_instance_addons: &std::collections::HashSet<AddOnId>,
) -> Vec<RawResolvedEntitlement> {
    use std::collections::HashMap;

    let feature_map: HashMap<FeatureId, Feature> = features
        .into_iter()
        .filter(|f| f.status == FeatureStatusEnum::Active)
        .map(|f| (f.id, f))
        .collect();

    let by_feature = entitlements.into_iter().into_group_map_by(|e| e.feature_id);

    let mut resolved = Vec::new();

    for (feature_id, mut ents) in by_feature {
        let Some(feature) = feature_map.get(&feature_id) else {
            continue;
        };

        ents.sort_by(|a, b| {
            a.entity
                .priority()
                .cmp(&b.entity.priority())
                .then(a.created_at.cmp(&b.created_at))
                .then(a.id.cmp(&b.id))
        });

        let mut accumulated: Option<EntitlementValue> = None;
        let mut accumulated_priority: Option<u8> = None;
        let mut winning_origin: Option<EntitlementEntityId> = None;

        for ent in ents {
            let pri = ent.entity.priority();
            let same_priority = accumulated_priority == Some(pri);
            let is_additive = matches!(
                ent.entity,
                EntitlementEntityId::AddOn(aid) if multi_instance_addons.contains(&aid)
            );
            // Decide origin before mutating accumulated. New entity wins unless the
            // existing accumulator's value still dominates the visible field
            // (permissive merge) or the merge is additive (keep the earlier-created
            // origin as a stable choice — see fn-level doc).
            let new_origin = match (&accumulated, same_priority, is_additive) {
                (Some(_), true, true) => winning_origin.unwrap_or(ent.entity),
                (Some(prev), true, false) => {
                    if prev_dominates_under_permissive(prev, &ent.value) {
                        winning_origin.unwrap_or(ent.entity)
                    } else {
                        ent.entity
                    }
                }
                _ => ent.entity,
            };
            accumulated = Some(match (accumulated, same_priority) {
                (Some(prev), true) if is_additive => merge_additive(prev, ent.value),
                (Some(prev), true) => merge_permissive(prev, ent.value),
                _ => ent.value,
            });
            winning_origin = Some(new_origin);
            accumulated_priority = Some(pri);
        }

        let Some(value) = accumulated else { continue };

        let resolved_value = match (feature.feature_type.clone(), value) {
            (FeatureType::Boolean, EntitlementValue::Boolean { enabled }) => {
                ResolvedEntitlementValue::Boolean { enabled }
            }
            (
                FeatureType::Metered { metric_id },
                EntitlementValue::Metered {
                    limit,
                    reset_period,
                    overage_behavior,
                    thresholds,
                    enabled,
                    group_by,
                },
            ) => ResolvedEntitlementValue::Metered {
                metric_id,
                limit,
                reset_period,
                overage_behavior,
                thresholds,
                enabled,
                group_by,
            },
            (FeatureType::Config { .. }, EntitlementValue::Config { value }) => {
                ResolvedEntitlementValue::Config { value }
            }
            _ => {
                log::error!(
                    "[BUG] entitlement value type mismatch for feature {} — skipping",
                    feature_id
                );
                continue;
            }
        };

        resolved.push(RawResolvedEntitlement {
            feature: FeatureRef {
                id: feature.id,
                name: feature.name.clone(),
                code: feature.code.clone(),
                product: feature.product.clone(),
            },
            value: resolved_value,
            origin_entity: winning_origin.expect("origin set whenever accumulated is Some"),
        });
    }

    resolved
}

/// Returns true when `prev` already dominates `new` on the visible field under permissive
/// merge — i.e. merging will not change the field a user reads off the resolved entitlement.
/// Used by `resolve` to keep the accumulator's origin when its value still wins, so the
/// reported `origin` matches the entity that actually contributed the resolved value
/// rather than the latest-iterated one.
///
/// - Metered: compare `limit` (None = unlimited > any finite).
/// - Boolean: OR is symmetric; treat the new entity as the contributor when it flips the
///   value, otherwise keep the existing origin.
fn prev_dominates_under_permissive(prev: &EntitlementValue, new: &EntitlementValue) -> bool {
    match (prev, new) {
        (
            EntitlementValue::Metered { limit: la, .. },
            EntitlementValue::Metered { limit: lb, .. },
        ) => match (la, lb) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(x), Some(y)) => x >= y,
        },
        (EntitlementValue::Boolean { enabled: a }, EntitlementValue::Boolean { enabled: b }) => {
            // Only `(false, true)` lets `new` flip the OR; in every other case `prev`
            // is at least as enabled and keeps the origin.
            !matches!((a, b), (false, true))
        }
        // Config override is last-writer-wins (see `merge_permissive`), so the new grant is
        // the contributor — do not keep `prev`'s origin.
        (EntitlementValue::Config { .. }, EntitlementValue::Config { .. }) => false,
        // Type mismatch should never reach here (validated at insert; logged as `[BUG]`
        // by the caller). Keep `prev`'s origin to avoid spurious origin churn.
        _ => true,
    }
}

/// Combine two same-priority `EntitlementValue`s, taking the more permissive of each
/// *visible* field (algorithm rule §2):
/// - `limit`: `max` (None = unlimited absorbs any finite).
/// - `enabled`: OR — kill-switch must be set on *every* grant to disable the feature.
/// - `thresholds`: union — alert at every threshold any grant asks for.
///
/// `reset_period`, `overage_behavior` and `group_by` are not part of the "more permissive" comparison;
/// they take `b`'s value — i.e. the later-iterated grant under the `(priority, created_at, id)`
/// sort. Callers who need a guaranteed winner across same-priority grants should set the
/// behavioural fields consistently on every grant for a given feature.
fn merge_permissive(a: EntitlementValue, b: EntitlementValue) -> EntitlementValue {
    match (a, b) {
        (EntitlementValue::Boolean { enabled: ea }, EntitlementValue::Boolean { enabled: eb }) => {
            EntitlementValue::Boolean { enabled: ea || eb }
        }
        (
            EntitlementValue::Metered {
                limit: la,
                reset_period: _,
                overage_behavior: _,
                thresholds: ta,
                enabled: ea,
                group_by: _,
            },
            EntitlementValue::Metered {
                limit: lb,
                reset_period: rpb,
                overage_behavior: obb,
                thresholds: tb,
                enabled: eb,
                group_by: gb,
            },
        ) => EntitlementValue::Metered {
            limit: match (la, lb) {
                (None, _) | (_, None) => None,
                (Some(x), Some(y)) => Some(x.max(y)),
            },
            reset_period: rpb,
            overage_behavior: obb,
            thresholds: merge_thresholds(&ta, &tb),
            enabled: ea || eb,
            group_by: gb,
        },
        // Config has no "more permissive" ordering; same-priority collisions resolve
        // last-writer-wins (`b`, later under the (priority, created_at, id) sort).
        (EntitlementValue::Config { .. }, EntitlementValue::Config { value }) => {
            EntitlementValue::Config { value }
        }
        (a, b) => {
            log::error!(
                "[BUG] merge_permissive called with mismatched variants — keeping `a`. \
                 a={a:?} b={b:?}"
            );
            a
        }
    }
}

/// Combine two same-priority `EntitlementValue`s additively (algorithm §3). Triggered when
/// both entries come from multi-instance add-ons (e.g. two "extra 1000 credits" packs):
/// - `limit`: sum (`None` stays unlimited).
/// - `enabled`: OR — kill-switch must be set on every grant to disable. Matches
///   [`merge_permissive`]: a single enabled grant keeps the entitlement on.
/// - `thresholds`: take `b`'s when it has any, otherwise fall back to `a`'s.
/// - `reset_period`, `overage_behavior`, `group_by`: take `b`'s value — set them consistently across the
///   multi-instance grants if you depend on a particular outcome.
fn merge_additive(a: EntitlementValue, b: EntitlementValue) -> EntitlementValue {
    match (a, b) {
        (EntitlementValue::Boolean { enabled: ea }, EntitlementValue::Boolean { enabled: eb }) => {
            EntitlementValue::Boolean { enabled: ea || eb }
        }
        (
            EntitlementValue::Metered {
                limit: la,
                reset_period: _rpa,
                overage_behavior: _,
                thresholds: ta,
                enabled: ea,
                group_by: _,
            },
            EntitlementValue::Metered {
                limit: lb,
                reset_period: rpb,
                overage_behavior: obb,
                thresholds: tb,
                enabled: eb,
                group_by: gb,
            },
        ) => EntitlementValue::Metered {
            limit: match (la, lb) {
                (None, _) | (_, None) => None,
                (Some(x), Some(y)) => Some(x + y),
            },
            reset_period: rpb,
            overage_behavior: obb,
            thresholds: if tb.is_empty() { ta } else { tb },
            enabled: ea || eb,
            group_by: gb,
        },
        // Config is not additive; keep last-writer-wins to match `merge_permissive`.
        (EntitlementValue::Config { .. }, EntitlementValue::Config { value }) => {
            EntitlementValue::Config { value }
        }
        (a, b) => {
            log::error!(
                "[BUG] merge_additive called with mismatched variants — keeping `a`. \
                 a={a:?} b={b:?}"
            );
            a
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entitlements::{
        FeatureType, OverageBehavior, PeriodUnit, ResetPeriod, ResolvedEntitlementValue,
    };

    use common_domain::ids::{
        AddOnId, BaseId, BillableMetricId, PlanId, PlanVersionId, QuoteId, SubscriptionId,
    };

    fn metered(
        limit: Option<i64>,
        reset_period: ResetPeriod,
        overage_behavior: OverageBehavior,
        thresholds: Vec<u32>,
    ) -> Result<(), Report<StoreError>> {
        validate_value_matches_feature_type(
            &EntitlementValue::Metered {
                limit: limit.map(Decimal::from),
                reset_period,
                overage_behavior,
                thresholds,
                enabled: true,
                group_by: None,
            },
            &FeatureType::metered(BillableMetricId::new()),
        )
    }

    #[test]
    fn limit_settings_without_a_limit_are_rejected() {
        let block = OverageBehavior::Block {
            grace_period_pct: None,
        };
        let graced = OverageBehavior::Block {
            grace_period_pct: Some(10),
        };
        assert!(metered(None, ResetPeriod::BillingCycle, block.clone(), vec![]).is_ok());
        assert!(
            metered(
                None,
                ResetPeriod::BillingCycle,
                OverageBehavior::Allow,
                vec![]
            )
            .is_err()
        );
        assert!(metered(None, ResetPeriod::BillingCycle, graced.clone(), vec![]).is_err());
        assert!(metered(None, ResetPeriod::BillingCycle, block.clone(), vec![80]).is_err());
        assert!(metered(Some(100), ResetPeriod::BillingCycle, graced, vec![80]).is_ok());
        assert!(
            metered(
                Some(100),
                ResetPeriod::Never,
                OverageBehavior::Allow,
                vec![50, 80, 100]
            )
            .is_ok()
        );
    }

    #[test]
    fn thresholds_are_distinct_ascending_percentages() {
        let block = || OverageBehavior::Block {
            grace_period_pct: None,
        };
        for bad in [
            vec![0],
            vec![101],
            vec![80, 80],
            vec![90, 80],
            (1..=11).collect(),
        ] {
            assert!(
                metered(Some(100), ResetPeriod::BillingCycle, block(), bad.clone()).is_err(),
                "{bad:?}"
            );
        }
        assert!(
            metered(
                Some(100),
                ResetPeriod::BillingCycle,
                block(),
                (1..=10).collect()
            )
            .is_ok()
        );
    }

    #[test]
    fn a_stored_value_with_inert_terms_is_copied_without_them() {
        let feature = FeatureType::metered(BillableMetricId::new());
        let unlimited = EntitlementValue::Metered {
            limit: None,
            reset_period: ResetPeriod::BillingCycle,
            overage_behavior: OverageBehavior::Allow,
            thresholds: vec![80],
            enabled: true,
            group_by: None,
        };
        assert!(validate_value_matches_feature_type(&unlimited, &feature).is_err());
        let copied = unlimited.without_inert_limit_terms();
        assert!(validate_value_matches_feature_type(&copied, &feature).is_ok());

        let sliding = EntitlementValue::Metered {
            limit: Some(Decimal::from(200)),
            reset_period: ResetPeriod::SlidingWindow {
                unit: PeriodUnit::Hour,
                interval: 5,
            },
            overage_behavior: OverageBehavior::Allow,
            thresholds: vec![80],
            enabled: true,
            group_by: None,
        };
        let copied = sliding.without_inert_limit_terms();
        assert!(matches!(
            copied,
            EntitlementValue::Metered {
                overage_behavior: OverageBehavior::Allow,
                ref thresholds,
                ..
            } if thresholds.is_empty()
        ));
    }

    #[test]
    fn a_sliding_window_takes_no_thresholds() {
        let sliding = ResetPeriod::SlidingWindow {
            unit: PeriodUnit::Hour,
            interval: 5,
        };
        let block = OverageBehavior::Block {
            grace_period_pct: None,
        };
        assert!(metered(Some(200), sliding.clone(), block.clone(), vec![80]).is_err());
        assert!(metered(Some(200), sliding, block, vec![]).is_ok());
    }

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
    fn chain_mixes_sub_and_quote_detects_collision() {
        // Chains assembled by the builders are disjoint (a chain belongs to one customer
        // subscription OR one quote). The predicate backs the debug_assert in
        // `resolve_for_chain`; both Sub-only and Quote-only must be accepted, and the
        // mixed combination rejected.
        let plan = EntitlementEntityId::Plan(PlanId::new());
        let pv = EntitlementEntityId::PlanVersion(PlanVersionId::new());
        let sub = EntitlementEntityId::Subscription(SubscriptionId::new());
        let quote = EntitlementEntityId::Quote(QuoteId::new());
        assert!(!chain_mixes_subscription_and_quote(&[]));
        assert!(!chain_mixes_subscription_and_quote(&[plan, pv, sub]));
        assert!(!chain_mixes_subscription_and_quote(&[plan, pv, quote]));
        assert!(chain_mixes_subscription_and_quote(&[plan, pv, sub, quote]));
        assert!(chain_mixes_subscription_and_quote(&[quote, sub]));
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

    fn make_feature(id: FeatureId, feature_type: FeatureType) -> Feature {
        Feature {
            id,
            tenant_id: TenantId::new(),
            product: None,
            name: id.to_string(),
            code: id.to_string(),
            description: None,
            feature_type,
            status: FeatureStatusEnum::Active,
            created_at: common_domain::clock::now(),
            updated_at: common_domain::clock::now(),
            entitlement: None,
        }
    }

    fn make_entitlement(
        feature_id: FeatureId,
        entity: EntitlementEntityId,
        mode: EntitlementModeEnum,
        value: EntitlementValue,
    ) -> Entitlement {
        Entitlement {
            id: EntitlementId::new(),
            tenant_id: TenantId::new(),
            feature_id,
            entity,
            mode,
            value,
            created_at: common_domain::clock::now(),
            updated_at: common_domain::clock::now(),
        }
    }

    #[test]
    fn grant_boolean_merges_permissively() {
        let fid = FeatureId::new();
        let features = vec![make_feature(fid, FeatureType::Boolean)];
        let entitlements = vec![
            make_entitlement(
                fid,
                EntitlementEntityId::Plan(PlanId::new()),
                EntitlementModeEnum::Stack,
                EntitlementValue::Boolean { enabled: false },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::Subscription(SubscriptionId::new()),
                EntitlementModeEnum::Stack,
                EntitlementValue::Boolean { enabled: true },
            ),
        ];

        let result = resolve(entitlements, features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        assert!(matches!(
            result[0].value,
            ResolvedEntitlementValue::Boolean { enabled: true, .. }
        ));
    }

    #[test]
    fn customer_overrides_feature_default() {
        // A customer-level force replaces the tenant-wide Feature(0) default for that customer.
        let fid = FeatureId::new();
        let features = vec![make_feature(fid, FeatureType::Boolean)];
        let entitlements = vec![
            make_entitlement(
                fid,
                EntitlementEntityId::Feature(fid),
                EntitlementModeEnum::Override,
                EntitlementValue::Boolean { enabled: false },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::Customer(CustomerId::new()),
                EntitlementModeEnum::Override,
                EntitlementValue::Boolean { enabled: true },
            ),
        ];
        let result = resolve(entitlements, features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        assert!(matches!(
            result[0].value,
            ResolvedEntitlementValue::Boolean { enabled: true, .. }
        ));
        assert!(matches!(
            result[0].origin_entity,
            EntitlementEntityId::Customer(_)
        ));
    }

    #[test]
    fn customer_force_overrides_plan() {
        // A customer-level force (top priority) wins over a plan grant.
        let fid = FeatureId::new();
        let features = vec![make_feature(fid, FeatureType::Boolean)];
        let entitlements = vec![
            make_entitlement(
                fid,
                EntitlementEntityId::Plan(PlanId::new()),
                EntitlementModeEnum::Override,
                EntitlementValue::Boolean { enabled: false },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::Customer(CustomerId::new()),
                EntitlementModeEnum::Override,
                EntitlementValue::Boolean { enabled: true },
            ),
        ];
        let result = resolve(entitlements, features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        assert!(matches!(
            result[0].value,
            ResolvedEntitlementValue::Boolean { enabled: true, .. }
        ));
        assert!(matches!(
            result[0].origin_entity,
            EntitlementEntityId::Customer(_)
        ));
    }

    #[test]
    fn customer_force_overrides_subscription() {
        // A customer-level force wins even over an explicit subscription entry — "enabled for the
        // customer, and it sticks across every subscription" (the account-wide force).
        let fid = FeatureId::new();
        let features = vec![make_feature(fid, FeatureType::Boolean)];
        let entitlements = vec![
            make_entitlement(
                fid,
                EntitlementEntityId::Subscription(SubscriptionId::new()),
                EntitlementModeEnum::Override,
                EntitlementValue::Boolean { enabled: false },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::Customer(CustomerId::new()),
                EntitlementModeEnum::Override,
                EntitlementValue::Boolean { enabled: true },
            ),
        ];
        let result = resolve(entitlements, features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        assert!(matches!(
            result[0].value,
            ResolvedEntitlementValue::Boolean { enabled: true, .. }
        ));
        assert!(matches!(
            result[0].origin_entity,
            EntitlementEntityId::Customer(_)
        ));
    }

    #[test]
    fn aggregate_ors_enabled_across_scopes() {
        // Customer-wide view: enabled on one subscription, disabled on another → enabled overall,
        // origin pointing at the scope that has it on. This is the fix for one subscription's
        // value masking another's.
        let fid = FeatureId::new();
        let on_sub = SubscriptionId::new();
        let make = |enabled, origin| ResolvedEntitlement {
            feature: FeatureRef {
                id: fid,
                name: fid.to_string(),
                code: fid.to_string(),
                product: None,
            },
            value: ResolvedEntitlementValue::Boolean { enabled },
            origin: ResolvedOrigin {
                entity: origin,
                name: None,
            },
        };
        let scope_a = vec![make(
            false,
            EntitlementEntityId::Subscription(SubscriptionId::new()),
        )];
        let scope_b = vec![make(true, EntitlementEntityId::Subscription(on_sub))];
        let agg = aggregate_scopes_permissive(vec![scope_a, scope_b]);
        assert_eq!(agg.len(), 1);
        assert!(matches!(
            agg[0].value,
            ResolvedEntitlementValue::Boolean { enabled: true }
        ));
        assert!(
            matches!(agg[0].origin.entity, EntitlementEntityId::Subscription(id) if id == on_sub)
        );
    }

    #[test]
    fn disabled_feature_suppressed() {
        let fid = FeatureId::new();
        let mut feature = make_feature(fid, FeatureType::Boolean);
        feature.status = FeatureStatusEnum::Disabled;
        let entitlements = vec![make_entitlement(
            fid,
            EntitlementEntityId::Plan(PlanId::new()),
            EntitlementModeEnum::Stack,
            EntitlementValue::Boolean { enabled: true },
        )];

        let result = resolve(
            entitlements,
            vec![feature],
            &std::collections::HashSet::new(),
        );
        assert!(result.is_empty());
    }

    #[test]
    fn override_replaces_lower_priority() {
        let fid = FeatureId::new();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        let entitlements = vec![
            make_entitlement(
                fid,
                EntitlementEntityId::Plan(PlanId::new()),
                EntitlementModeEnum::Stack,
                EntitlementValue::Metered {
                    group_by: None,
                    limit: Some(100.into()),
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    thresholds: vec![],
                    enabled: true,
                },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::Subscription(SubscriptionId::new()),
                EntitlementModeEnum::Override,
                EntitlementValue::Metered {
                    group_by: None,
                    limit: Some(500.into()),
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    thresholds: vec![],
                    enabled: true,
                },
            ),
        ];

        let result = resolve(entitlements, features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        assert!(
            matches!(result[0].value, ResolvedEntitlementValue::Metered { limit: Some(l), .. } if l == 500.into())
        );
    }

    #[test]
    fn metered_grants_sum_additively() {
        let fid = FeatureId::new();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        // Two AddOn grants of 1000 each → 2000 total (multi-instance add-ons stack).
        let aid1 = AddOnId::new();
        let aid2 = AddOnId::new();
        let multi: std::collections::HashSet<AddOnId> = [aid1, aid2].into_iter().collect();
        let entitlements = vec![
            make_entitlement(
                fid,
                EntitlementEntityId::AddOn(aid1),
                EntitlementModeEnum::Stack,
                EntitlementValue::Metered {
                    group_by: None,
                    limit: Some(1000.into()),
                    reset_period: ResetPeriod::BillingCycle,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    thresholds: vec![90],
                    enabled: true,
                },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::AddOn(aid2),
                EntitlementModeEnum::Stack,
                EntitlementValue::Metered {
                    group_by: None,
                    limit: Some(1000.into()),
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    thresholds: vec![],
                    enabled: true,
                },
            ),
        ];

        let result = resolve(entitlements, features, &multi);
        assert_eq!(result.len(), 1);
        let ResolvedEntitlementValue::Metered { limit, .. } = &result[0].value else {
            panic!("expected metered value");
        };
        assert_eq!(*limit, Some(2000.into()));
    }

    #[test]
    fn merge_additive_metered_enabled_ors_kill_switch() {
        // Multi-instance add-ons stack additively. The kill-switch on metered grants must
        // OR — a single enabled grant keeps the entitlement on, matching the boolean and
        // permissive-merge rules. Order-independence is the contract being pinned.
        let fid = FeatureId::new();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        let aid_on = AddOnId::new();
        let aid_off = AddOnId::new();
        let multi: std::collections::HashSet<AddOnId> = [aid_on, aid_off].into_iter().collect();

        let metered = |enabled: bool| EntitlementValue::Metered {
            group_by: None,
            limit: Some(100.into()),
            reset_period: ResetPeriod::Never,
            overage_behavior: OverageBehavior::Block {
                grace_period_pct: None,
            },
            thresholds: vec![],
            enabled,
        };

        // (on, off) and (off, on) must both resolve to enabled=true.
        for (first, second) in [(aid_on, aid_off), (aid_off, aid_on)] {
            let mut a = make_entitlement(
                fid,
                EntitlementEntityId::AddOn(first),
                EntitlementModeEnum::Stack,
                metered(first == aid_on),
            );
            a.created_at = common_domain::clock::now() - chrono::Duration::seconds(60);
            let mut b = make_entitlement(
                fid,
                EntitlementEntityId::AddOn(second),
                EntitlementModeEnum::Stack,
                metered(second == aid_on),
            );
            b.created_at = common_domain::clock::now();
            let result = resolve(vec![a, b], features.clone(), &multi);
            assert_eq!(result.len(), 1);
            let ResolvedEntitlementValue::Metered { enabled, .. } = &result[0].value else {
                panic!("expected metered");
            };
            assert!(
                *enabled,
                "additive merge must OR the kill-switch (order: {first:?} then {second:?})"
            );
        }
    }

    #[test]
    fn metered_unlimited_wins_over_capped() {
        let fid = FeatureId::new();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        let entitlements = vec![
            make_entitlement(
                fid,
                EntitlementEntityId::Plan(PlanId::new()),
                EntitlementModeEnum::Stack,
                EntitlementValue::Metered {
                    group_by: None,
                    limit: Some(100.into()),
                    reset_period: ResetPeriod::Calendar {
                        unit: PeriodUnit::Month,
                        interval: 1,
                    },
                    overage_behavior: OverageBehavior::Allow,
                    thresholds: vec![80],
                    enabled: true,
                },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::AddOn(AddOnId::new()),
                EntitlementModeEnum::Stack,
                EntitlementValue::Metered {
                    group_by: None,
                    limit: None,
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    thresholds: vec![],
                    enabled: true,
                },
            ),
        ];

        let result = resolve(entitlements, features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        let ResolvedEntitlementValue::Metered { limit, .. } = &result[0].value else {
            panic!("expected metered value");
        };
        assert_eq!(*limit, None);
    }

    #[test]
    fn override_replaces_accumulated_value() {
        let fid = FeatureId::new();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        let entitlements = vec![
            make_entitlement(
                fid,
                EntitlementEntityId::Plan(PlanId::new()),
                EntitlementModeEnum::Stack,
                EntitlementValue::Metered {
                    group_by: None,
                    limit: Some(100.into()),
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    thresholds: vec![70],
                    enabled: true,
                },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::PlanVersion(PlanVersionId::new()),
                EntitlementModeEnum::Override,
                EntitlementValue::Metered {
                    group_by: None,
                    limit: Some(200.into()),
                    reset_period: ResetPeriod::BillingCycle,
                    overage_behavior: OverageBehavior::Allow,
                    thresholds: vec![90],
                    enabled: true,
                },
            ),
        ];

        let result = resolve(entitlements, features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        let ResolvedEntitlementValue::Metered {
            limit,
            reset_period,
            overage_behavior,
            thresholds,
            ..
        } = &result[0].value
        else {
            panic!("expected metered value");
        };
        assert_eq!(*limit, Some(200.into()));
        assert!(matches!(reset_period, ResetPeriod::BillingCycle));
        assert!(matches!(overage_behavior, OverageBehavior::Allow));
        assert_eq!(*thresholds, vec![90]);
    }

    #[test]
    fn unknown_feature_excluded() {
        let fid = FeatureId::new();
        let features = vec![];
        let entitlements = vec![make_entitlement(
            fid,
            EntitlementEntityId::Plan(PlanId::new()),
            EntitlementModeEnum::Stack,
            EntitlementValue::Boolean { enabled: true },
        )];

        let result = resolve(entitlements, features, &std::collections::HashSet::new());
        assert!(result.is_empty());
    }

    #[test]
    fn disabled_resolves_to_higher_priority() {
        // Metered: Plan (low priority) disables (enabled: false); Subscription (high priority)
        // re-enables (enabled: true) → the higher-priority enabled:true wins.
        let fid = FeatureId::new();
        let mid = BillableMetricId::new();
        let features = vec![make_feature(fid, FeatureType::metered(mid))];
        let entitlements = vec![
            make_entitlement(
                fid,
                EntitlementEntityId::Plan(PlanId::new()),
                EntitlementModeEnum::Stack,
                EntitlementValue::Metered {
                    group_by: None,
                    limit: Some(100.into()),
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    thresholds: vec![],
                    enabled: false,
                },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::Subscription(SubscriptionId::new()),
                EntitlementModeEnum::Stack,
                EntitlementValue::Metered {
                    group_by: None,
                    limit: Some(100.into()),
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    thresholds: vec![],
                    enabled: true,
                },
            ),
        ];
        let result = resolve(entitlements, features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        assert!(matches!(
            result[0].value,
            ResolvedEntitlementValue::Metered { enabled: true, .. }
        ));
    }

    #[test]
    fn disabled_entitlement_still_resolved() {
        // A metered entitlement with enabled:false is NOT filtered out —
        // it surfaces so the UI can show + override it.
        let fid = FeatureId::new();
        let mid = BillableMetricId::new();
        let features = vec![make_feature(fid, FeatureType::metered(mid))];
        let entitlements = vec![make_entitlement(
            fid,
            EntitlementEntityId::Plan(PlanId::new()),
            EntitlementModeEnum::Stack,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(100.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: false,
            },
        )];
        let result = resolve(entitlements, features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        assert!(matches!(
            result[0].value,
            ResolvedEntitlementValue::Metered { enabled: false, .. }
        ));
    }

    #[test]
    fn origin_reports_highest_priority_contributing_entity() {
        let fid = FeatureId::new();
        let plan_id = PlanId::new();
        let sub_id = SubscriptionId::new();
        let features = vec![make_feature(fid, FeatureType::Boolean)];
        let entitlements = vec![
            make_entitlement(
                fid,
                EntitlementEntityId::Plan(plan_id),
                EntitlementModeEnum::Stack,
                EntitlementValue::Boolean { enabled: true },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::Subscription(sub_id),
                EntitlementModeEnum::Override,
                EntitlementValue::Boolean { enabled: false },
            ),
        ];
        let result = resolve(entitlements, features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        assert!(matches!(
            result[0].origin_entity,
            EntitlementEntityId::Subscription(_)
        ));
    }

    #[test]
    fn same_priority_metered_takes_max() {
        // Two Plans at priority 1, conflicting metered limits.
        // Smaller value created later → current last-wins returns 100.
        // Permissive rule (algorithm §2) requires max → 200.
        let fid = FeatureId::new();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        let mut a = make_entitlement(
            fid,
            EntitlementEntityId::Plan(PlanId::new()),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(200.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        a.created_at = common_domain::clock::now() - chrono::Duration::seconds(60);
        let mut b = make_entitlement(
            fid,
            EntitlementEntityId::Plan(PlanId::new()),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(100.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        b.created_at = common_domain::clock::now();

        let result = resolve(vec![a, b], features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        let ResolvedEntitlementValue::Metered { limit, .. } = &result[0].value else {
            panic!("expected metered");
        };
        assert_eq!(*limit, Some(200.into()), "permissive merge should pick max");
    }

    #[test]
    fn same_priority_boolean_takes_or() {
        // Two AddOns at same priority, one disables and one enables boolean.
        // Permissive rule: enabled wins regardless of order/created_at.
        let fid = FeatureId::new();
        let features = vec![make_feature(fid, FeatureType::Boolean)];
        let mut a = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(AddOnId::new()),
            EntitlementModeEnum::Override,
            EntitlementValue::Boolean { enabled: true },
        );
        a.created_at = common_domain::clock::now() - chrono::Duration::seconds(60);
        let mut b = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(AddOnId::new()),
            EntitlementModeEnum::Override,
            EntitlementValue::Boolean { enabled: false },
        );
        b.created_at = common_domain::clock::now();

        let result = resolve(vec![a, b], features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        assert!(matches!(
            result[0].value,
            ResolvedEntitlementValue::Boolean { enabled: true }
        ));
    }

    #[test]
    fn winning_origin_after_permissive_merge_tracks_max_contributor() {
        // Two Plans at the same priority, conflicting metered limits. The earlier-created
        // entitlement carries the larger limit; the permissive merge picks max. The
        // reported origin must point to the entity that actually contributed the
        // winning limit, not the latest-iterated one.
        let fid = FeatureId::new();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        let winning_plan_id = PlanId::new();
        let losing_plan_id = PlanId::new();
        let mut winner = make_entitlement(
            fid,
            EntitlementEntityId::Plan(winning_plan_id),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(500.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        winner.created_at = common_domain::clock::now() - chrono::Duration::seconds(60);
        let mut loser = make_entitlement(
            fid,
            EntitlementEntityId::Plan(losing_plan_id),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(100.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        loser.created_at = common_domain::clock::now();
        let result = resolve(
            vec![winner, loser],
            features,
            &std::collections::HashSet::new(),
        );
        assert_eq!(result.len(), 1);
        let ResolvedEntitlementValue::Metered { limit, .. } = &result[0].value else {
            panic!("expected metered");
        };
        assert_eq!(*limit, Some(500.into()));
        assert!(
            matches!(result[0].origin_entity, EntitlementEntityId::Plan(p) if p == winning_plan_id),
            "origin should point to the entity that contributed the winning limit"
        );
    }

    #[test]
    fn winning_origin_after_additive_merge_stays_on_earliest_contributor() {
        // Two multi-instance add-ons at the same priority — additive merge sums limits.
        // Both contribute, so origin is anchored to the earlier-created entity for
        // determinism.
        let fid = FeatureId::new();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        let first_addon = AddOnId::new();
        let second_addon = AddOnId::new();
        let mut a = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(first_addon),
            EntitlementModeEnum::Stack,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(100.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        a.created_at = common_domain::clock::now() - chrono::Duration::seconds(60);
        let mut b = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(second_addon),
            EntitlementModeEnum::Stack,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(50.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        b.created_at = common_domain::clock::now();
        let mut multi = std::collections::HashSet::new();
        multi.insert(first_addon);
        multi.insert(second_addon);
        let result = resolve(vec![a, b], features, &multi);
        assert_eq!(result.len(), 1);
        let ResolvedEntitlementValue::Metered { limit, .. } = &result[0].value else {
            panic!("expected metered");
        };
        assert_eq!(*limit, Some(150.into()), "additive merge sums limits");
        assert!(
            matches!(result[0].origin_entity, EntitlementEntityId::AddOn(id) if id == first_addon),
            "additive origin should pin to the earlier-created entity"
        );
    }

    #[test]
    fn plan_version_overrides_add_on() {
        // Algorithm §1: Plan Version > Add-on. PlanVersion's value must win.
        let fid = FeatureId::new();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        let add_on = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(AddOnId::new()),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(100.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        let plan_version = make_entitlement(
            fid,
            EntitlementEntityId::PlanVersion(PlanVersionId::new()),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(200.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        let result = resolve(
            vec![add_on, plan_version],
            features,
            &std::collections::HashSet::new(),
        );
        assert_eq!(result.len(), 1);
        let ResolvedEntitlementValue::Metered { limit, .. } = &result[0].value else {
            panic!("expected metered");
        };
        assert_eq!(
            *limit,
            Some(200.into()),
            "PlanVersion should override AddOn"
        );
        assert!(matches!(
            result[0].origin_entity,
            EntitlementEntityId::PlanVersion(_)
        ));
    }

    #[test]
    fn multi_instance_add_ons_sum_via_derive_mode() {
        // Two different multi-instance add-ons; stored mode is Override but resolve must derive
        // additive from the multi_instance set. Algorithm §3.
        let fid = FeatureId::new();
        let aid1 = AddOnId::new();
        let aid2 = AddOnId::new();
        let multi: std::collections::HashSet<AddOnId> = [aid1, aid2].into_iter().collect();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        let a = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(aid1),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(1000.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        let b = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(aid2),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(1000.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        let result = resolve(vec![a, b], features, &multi);
        assert_eq!(result.len(), 1);
        let ResolvedEntitlementValue::Metered { limit, .. } = &result[0].value else {
            panic!("expected metered");
        };
        assert_eq!(*limit, Some(2000.into()));
    }

    #[test]
    fn single_instance_add_ons_take_max_via_derive_mode() {
        // Two different single-instance add-ons; stored mode is Stack (stale) but resolve must
        // derive permissive from the multi_instance set (empty). Take max, not sum.
        let fid = FeatureId::new();
        let multi: std::collections::HashSet<AddOnId> = std::collections::HashSet::new();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        let a = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(AddOnId::new()),
            EntitlementModeEnum::Stack,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(1000.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        let b = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(AddOnId::new()),
            EntitlementModeEnum::Stack,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(500.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        let result = resolve(vec![a, b], features, &multi);
        assert_eq!(result.len(), 1);
        let ResolvedEntitlementValue::Metered { limit, .. } = &result[0].value else {
            panic!("expected metered");
        };
        assert_eq!(*limit, Some(1000.into()), "max, not sum");
    }

    #[test]
    fn feature_level_default_flows_through_when_no_entity_grant() {
        // Only a Feature-level (tenant default) entitlement, no chain entity-bound row.
        // Should be returned as-is — Feature is the lowest priority but the only contributor wins.
        let fid = FeatureId::new();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        let only = make_entitlement(
            fid,
            EntitlementEntityId::Feature(fid),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(50.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        let result = resolve(vec![only], features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        let ResolvedEntitlementValue::Metered { limit, .. } = &result[0].value else {
            panic!("expected metered");
        };
        assert_eq!(*limit, Some(50.into()));
        assert!(matches!(
            result[0].origin_entity,
            EntitlementEntityId::Feature(_)
        ));
    }

    #[test]
    fn full_priority_chain_resolves_highest_wins() {
        // One entitlement at every level. Subscription (4) must win.
        let fid = FeatureId::new();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        let mk = |entity, limit: u64| {
            make_entitlement(
                fid,
                entity,
                EntitlementModeEnum::Override,
                EntitlementValue::Metered {
                    group_by: None,
                    limit: Some(limit.into()),
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    thresholds: vec![],
                    enabled: true,
                },
            )
        };
        let entitlements = vec![
            mk(EntitlementEntityId::Feature(fid), 10),
            mk(EntitlementEntityId::Plan(PlanId::new()), 20),
            mk(EntitlementEntityId::AddOn(AddOnId::new()), 30),
            mk(EntitlementEntityId::PlanVersion(PlanVersionId::new()), 40),
            mk(EntitlementEntityId::Subscription(SubscriptionId::new()), 50),
        ];
        let result = resolve(entitlements, features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        let ResolvedEntitlementValue::Metered { limit, .. } = &result[0].value else {
            panic!("expected metered");
        };
        assert_eq!(
            *limit,
            Some(50.into()),
            "Subscription (top priority) must win"
        );
        assert!(matches!(
            result[0].origin_entity,
            EntitlementEntityId::Subscription(_)
        ));
    }

    #[test]
    fn cross_product_collision_takes_max_per_algorithm_rule_2() {
        // Same globally-scoped feature, two different AddOns (different products) granting
        // conflicting Override values at the same priority. Permissive rule applies.
        let fid = FeatureId::new();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        let a = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(AddOnId::new()),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(300.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        let b = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(AddOnId::new()),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(700.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        let result = resolve(vec![a, b], features, &std::collections::HashSet::new());
        assert_eq!(result.len(), 1);
        let ResolvedEntitlementValue::Metered { limit, .. } = &result[0].value else {
            panic!("expected metered");
        };
        assert_eq!(*limit, Some(700.into()));
    }

    #[test]
    fn higher_priority_finite_replaces_lower_priority_unlimited() {
        // Algorithm §1: higher priority replaces entirely (no permissive merge across priorities).
        // PlanVersion finite 100 must override Feature-level unlimited None.
        let fid = FeatureId::new();
        let features = vec![make_feature(
            fid,
            FeatureType::metered(BillableMetricId::new()),
        )];
        let global = make_entitlement(
            fid,
            EntitlementEntityId::Feature(fid),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                group_by: None,
                limit: None,
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        let plan_version = make_entitlement(
            fid,
            EntitlementEntityId::PlanVersion(PlanVersionId::new()),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                group_by: None,
                limit: Some(100.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                thresholds: vec![],
                enabled: true,
            },
        );
        let result = resolve(
            vec![global, plan_version],
            features,
            &std::collections::HashSet::new(),
        );
        assert_eq!(result.len(), 1);
        let ResolvedEntitlementValue::Metered { limit, .. } = &result[0].value else {
            panic!("expected metered");
        };
        assert_eq!(*limit, Some(100.into()));
    }
}
