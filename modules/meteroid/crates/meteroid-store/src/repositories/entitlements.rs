use crate::domain::entitlements::{
    EntitlementSpec, EntitlementValue, FeatureRef, FeatureType, PeriodUnit, RawResolvedEntitlement,
    ResetPeriod, ResolvedEntitlement, ResolvedEntitlementValue, ResolvedOrigin,
};
use crate::domain::enums::{EntitlementModeEnum, FeatureStatusEnum};
use crate::domain::{
    Entitlement, EntitlementNew, EntitlementUpdate, Feature, FeatureNew, FeatureUpdate,
    PaginatedVec, PaginationRequest,
};
use crate::errors::StoreError;
use crate::store::PgConn;
use crate::{Store, StoreResult};
use chrono::{DateTime, Datelike, Days, Duration, Months, NaiveDate, NaiveDateTime, NaiveTime};
use common_domain::ids::{
    AddOnId, BaseId, CustomerId, EntitlementEntityId, EntitlementId, FeatureId, PlanId,
    PlanVersionId, ProductId, QuoteId, SubscriptionId, TenantId,
};
use diesel_models::add_ons::AddOnRow;
use diesel_models::entitlements::{
    EntitlementRow, EntitlementRowNew, EntitlementRowPatch, FeatureRow, FeatureRowNew,
    FeatureRowPatch, FeatureWithProductRow,
};
use diesel_models::enums::{EntitlementEntityTypeEnum, FeatureStatusEnum as DbFeatureStatusEnum};
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::plans::PlanRow;
use diesel_models::query::price_components::list_product_ids_for_plan_version;
use diesel_models::quote_add_ons::QuoteAddOnRow;
use diesel_models::quotes::QuoteComponentRow;
use diesel_models::quotes::QuoteRow;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use itertools::Itertools;
use scoped_futures::ScopedFutureExt;

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
    async fn create_feature(&self, feature: FeatureNew) -> StoreResult<Feature>;

    async fn get_feature(&self, id: FeatureId, tenant_id: TenantId) -> StoreResult<Feature>;

    async fn list_features(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        statuses: Option<Vec<FeatureStatusEnum>>,
        product_id: Option<ProductId>,
        search: Option<String>,
    ) -> StoreResult<PaginatedVec<Feature>>;

    async fn update_feature(
        &self,
        id: FeatureId,
        tenant_id: TenantId,
        update: FeatureUpdate,
    ) -> StoreResult<Feature>;

    /// Transition the feature status. Operators use this to disable the feature globally
    /// (Disabled), archive it (Archived — entitlements become dormant but are preserved),
    /// or re-activate it (Active).
    async fn set_feature_status(
        &self,
        id: FeatureId,
        tenant_id: TenantId,
        status: FeatureStatusEnum,
    ) -> StoreResult<()>;

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
    /// Returns one `ResolvedEntitlement` per feature in the customer's product scope,
    /// composed across every layer that can grant it. Priority ladder (low → high):
    /// `Feature → Plan → AddOn → PlanVersion → Subscription`. Subscription-level grants
    /// always win; PlanVersion overrides its linked AddOns because the plan version is
    /// the authoritative composition. See [`EntitlementEntityId::priority`].
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

/// Reject combinations where the entitlement value's variant does not match the feature's
/// declared type. Without this check the resolver silently drops mismatched rows at read
/// time (see `resolve()` `_ => log::error!(...)` branch).
fn validate_value_matches_feature_type(
    value: &EntitlementValue,
    feature_type: &FeatureType,
) -> Result<(), Report<StoreError>> {
    match (feature_type, value) {
        (FeatureType::Boolean, EntitlementValue::Boolean { .. })
        | (FeatureType::Metered { .. }, EntitlementValue::Metered { .. }) => Ok(()),
        _ => Err(Report::new(StoreError::InvalidArgument(
            "entitlement value variant does not match feature type".into(),
        ))),
    }
}

#[async_trait::async_trait]
impl EntitlementsInterface for Store {
    async fn create_feature(&self, feature: FeatureNew) -> StoreResult<Feature> {
        let mut conn = self.get_conn().await?;
        let entitlement_value = feature.entitlement.clone();
        let tenant_id = feature.tenant_id;
        if let Some(value) = &entitlement_value {
            validate_value_matches_feature_type(value, &feature.feature_type)?;
        }
        let row: FeatureRowNew = feature.into();
        if let Some(value) = entitlement_value {
            self.transaction_with(&mut conn, |conn| {
                async move {
                    let inserted = row
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    let joined = FeatureRow::find_by_id(conn, inserted.id, inserted.tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    let mut f: Feature = joined.try_into()?;

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
                    Ok(f)
                }
                .scope_boxed()
            })
            .await
        } else {
            let inserted = row
                .insert(&mut conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            FeatureRow::find_by_id(&mut conn, inserted.id, inserted.tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)
                .and_then(TryInto::try_into)
        }
    }

    async fn get_feature(&self, id: FeatureId, tenant_id: TenantId) -> StoreResult<Feature> {
        let mut conn = self.get_conn().await?;

        let row = FeatureRow::find_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut feature: Feature = row.try_into()?;

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

    async fn update_feature(
        &self,
        id: FeatureId,
        tenant_id: TenantId,
        update: FeatureUpdate,
    ) -> StoreResult<Feature> {
        let mut conn = self.get_conn().await?;

        let patch = FeatureRowPatch {
            name: update.name,
            description: update.description,
            product_id: update.product_id,
            status: None,
            updated_at: Some(chrono::Utc::now()),
        };

        let row = FeatureRow::update(&mut conn, id, tenant_id, patch)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut feature: Feature = row.try_into()?;

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
        id: FeatureId,
        tenant_id: TenantId,
        status: FeatureStatusEnum,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        FeatureRow::set_status(&mut conn, id, tenant_id, status.into())
            .await
            .map_err(Into::into)
    }

    async fn create_entitlement(&self, entitlement: EntitlementNew) -> StoreResult<Entitlement> {
        let mut conn = self.get_conn().await?;

        let feature: Feature =
            FeatureRow::find_by_id(&mut conn, entitlement.feature_id, entitlement.tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)
                .and_then(TryInto::try_into)?;
        validate_value_matches_feature_type(&entitlement.value, &feature.feature_type)?;

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

        row.insert(&mut conn)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
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
            validate_value_matches_feature_type(value, &feature.feature_type)?;
            Some(value_to_json(value)?)
        } else {
            None
        };

        let patch = EntitlementRowPatch {
            mode: None,
            value: patch_value,
            updated_at: Some(chrono::Utc::now()),
        };

        EntitlementRow::update(&mut conn, id, tenant_id, patch)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn delete_entitlement(&self, id: EntitlementId, tenant_id: TenantId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        EntitlementRow::delete(&mut conn, id, tenant_id)
            .await
            .map_err(Into::into)
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
        let rows = build_entitlement_rows(&mut conn, specs, &target, tenant_id).await?;
        EntitlementRowNew::insert_batch_skip_conflicts(&rows, &mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(|rows| rows.into_iter().map(TryInto::try_into).collect())
    }
}

pub(crate) struct UsagePeriodBounds {
    pub(crate) start: NaiveDateTime,
    /// `None` for `Never` reset — no upper bound. When set, this is exclusive (= reset_at).
    pub(crate) end: Option<NaiveDateTime>,
}

#[derive(Copy, Clone)]
pub(crate) struct BillingCyclePeriod {
    pub(crate) period_start: NaiveDate,
    pub(crate) period_end: Option<NaiveDate>,
}

/// Compute the time window over which to count usage for a metered entitlement.
///
/// - `Never` — open-ended window starting at the customer's subscription activation date
///   (or `now` if no activation is known). `end = None`: usage is cumulative for life.
/// - `BillingCycle` — anchored on the subscription's current billing period. With no
///   active subscription, falls back to `[now, ∞)`, which surfaces as zero usage.
/// - `Calendar { unit, interval }` — fixed buckets anchored on a global epoch (the Unix
///   epoch for Hour/Day, ISO Monday 1969-12-29 for Week, calendar year zero for Month/Year).
///   Same `now` always falls into the same bucket regardless of when the subscription
///   started — useful for "calendar month" semantics shared across customers. The anchor
///   is arbitrary but deterministic: e.g. `Year { interval: 3 }` produces buckets
///   `[…, 2022-2024, 2025-2027, …]`, not buckets starting on the customer's signup year.
/// - `SlidingWindow { unit, interval }` — continuous rolling window `[now − interval, now]`.
///   Usage drops out the moment it ages past the window edge — no boundary moment, no reset.
/// - `FixedWindow { unit, interval }` — discrete, non-overlapping bucket of length
///   `interval × unit` anchored on the customer's subscription activation date. Bucket N
///   covers `[activation + N × interval, activation + (N+1) × interval)`. Usage resets to
///   zero at every bucket boundary. With no known activation, falls back to `[now, ∞)`
///   (same as `Never`) so the caller gets an empty window rather than an arbitrary anchor.
pub(crate) fn compute_usage_period(
    reset_period: &ResetPeriod,
    billing_cycle: Option<BillingCyclePeriod>,
    activation_date: Option<NaiveDateTime>,
    now: NaiveDateTime,
) -> UsagePeriodBounds {
    match reset_period {
        ResetPeriod::Never => UsagePeriodBounds {
            start: activation_date.unwrap_or(now),
            end: None,
        },
        ResetPeriod::BillingCycle => match billing_cycle {
            Some(bc) => UsagePeriodBounds {
                start: bc.period_start.and_time(NaiveTime::MIN),
                // exclusive end: day after the inclusive last day
                end: bc
                    .period_end
                    .map(|d| (d + Days::new(1)).and_time(NaiveTime::MIN)),
            },
            None => UsagePeriodBounds {
                start: now,
                end: None,
            },
        },
        ResetPeriod::Calendar { unit, interval } => calendar_period(now, unit, *interval),
        ResetPeriod::FixedWindow { unit, interval } => match activation_date {
            Some(act) => fixed_window(act, now, unit, *interval),
            None => UsagePeriodBounds {
                start: now,
                end: None,
            },
        },
        ResetPeriod::SlidingWindow { unit, interval } => UsagePeriodBounds {
            start: subtract_duration(now, unit, *interval),
            end: Some(now),
        },
    }
}

/// Find the fixed-window bucket containing `now`, with buckets of length `interval × unit`
/// anchored at `activation`. Returns `[bucket_start, bucket_end)`.
fn fixed_window(
    activation: NaiveDateTime,
    now: NaiveDateTime,
    unit: &PeriodUnit,
    interval: u32,
) -> UsagePeriodBounds {
    match unit {
        // Constant-length units → closed-form via Duration arithmetic.
        PeriodUnit::Hour | PeriodUnit::Day | PeriodUnit::Week => {
            let step_hours: i64 = match unit {
                PeriodUnit::Hour => interval as i64,
                PeriodUnit::Day => 24 * interval as i64,
                PeriodUnit::Week => 24 * 7 * interval as i64,
                _ => unreachable!(),
            };
            // If now < activation (clock skew or pre-dated activation), clamp to bucket 0.
            let elapsed = (now - activation).num_hours().max(0);
            let bucket = elapsed / step_hours;
            let start = activation + Duration::hours(bucket * step_hours);
            let end = start + Duration::hours(step_hours);
            UsagePeriodBounds {
                start,
                end: Some(end),
            }
        }
        // Variable-length units (Month/Year): walk forward in `interval`-sized steps.
        // Bounded by activation-to-now span in months; even 10 years on a 1-month reset
        // is at most ~120 iterations.
        PeriodUnit::Month | PeriodUnit::Year => {
            let step_months: u32 = match unit {
                PeriodUnit::Month => interval,
                PeriodUnit::Year => interval * 12,
                _ => unreachable!(),
            };
            let mut start = activation;
            loop {
                let next =
                    NaiveDateTime::new(start.date() + Months::new(step_months), start.time());
                if next > now {
                    return UsagePeriodBounds {
                        start,
                        end: Some(next),
                    };
                }
                start = next;
            }
        }
    }
}

fn calendar_period(now: NaiveDateTime, unit: &PeriodUnit, interval: u32) -> UsagePeriodBounds {
    match unit {
        PeriodUnit::Hour => {
            let epoch = DateTime::UNIX_EPOCH.naive_utc();
            let hours_since_epoch = (now - epoch).num_hours();
            let period_idx = hours_since_epoch / interval as i64;
            let start = epoch + Duration::hours(period_idx * interval as i64);
            let end = start + Duration::hours(interval as i64);
            UsagePeriodBounds {
                start,
                end: Some(end),
            }
        }
        PeriodUnit::Day => {
            let today = now.date();
            let epoch = DateTime::UNIX_EPOCH.date_naive();
            let days = (today - epoch).num_days();
            let period_idx = days / interval as i64;
            let start_date = epoch + Days::new((period_idx * interval as i64) as u64);
            let end_date = start_date + Days::new(interval as u64);
            UsagePeriodBounds {
                start: start_date.and_time(NaiveTime::MIN),
                end: Some(end_date.and_time(NaiveTime::MIN)),
            }
        }
        PeriodUnit::Week => {
            let today = now.date();
            let iso_epoch = NaiveDate::from_ymd_opt(1969, 12, 29).unwrap();
            let days_since_iso_epoch = (today - iso_epoch).num_days();
            let weeks_since_iso_epoch = days_since_iso_epoch / 7;
            let period_idx = weeks_since_iso_epoch / interval as i64;
            let start_date = iso_epoch + Days::new((period_idx * 7 * interval as i64) as u64);
            let end_date = start_date + Days::new(7 * interval as u64);
            UsagePeriodBounds {
                start: start_date.and_time(NaiveTime::MIN),
                end: Some(end_date.and_time(NaiveTime::MIN)),
            }
        }
        PeriodUnit::Month => {
            let today = now.date();
            let interval = interval as i64;
            let months_since_zero = today.year() as i64 * 12 + (today.month() as i64 - 1);
            let period_idx = months_since_zero.div_euclid(interval);
            let start_months = period_idx * interval;
            let start_year = start_months.div_euclid(12) as i32;
            let start_month = (start_months.rem_euclid(12) + 1) as u32;
            let start_date = NaiveDate::from_ymd_opt(start_year, start_month, 1).unwrap_or(today);
            let end_date = start_date + Months::new(interval as u32);
            UsagePeriodBounds {
                start: start_date.and_time(NaiveTime::MIN),
                end: Some(end_date.and_time(NaiveTime::MIN)),
            }
        }
        PeriodUnit::Year => {
            let today = now.date();
            let year = today.year() as i64;
            let interval = interval as i64;
            let period_idx = year.div_euclid(interval);
            let start_year = (period_idx * interval) as i32;
            let start_date = NaiveDate::from_ymd_opt(start_year, 1, 1).unwrap_or(today);
            let end_date =
                NaiveDate::from_ymd_opt(start_year.saturating_add(interval as i32), 1, 1)
                    .unwrap_or(today);
            UsagePeriodBounds {
                start: start_date.and_time(NaiveTime::MIN),
                end: Some(end_date.and_time(NaiveTime::MIN)),
            }
        }
    }
}

fn subtract_duration(now: NaiveDateTime, unit: &PeriodUnit, interval: u32) -> NaiveDateTime {
    match unit {
        PeriodUnit::Hour => now - Duration::hours(interval as i64),
        PeriodUnit::Day => NaiveDateTime::new(now.date() - Days::new(interval as u64), now.time()),
        PeriodUnit::Week => {
            NaiveDateTime::new(now.date() - Days::new(7 * interval as u64), now.time())
        }
        PeriodUnit::Month => NaiveDateTime::new(now.date() - Months::new(interval), now.time()),
        PeriodUnit::Year => NaiveDateTime::new(now.date() - Months::new(interval * 12), now.time()),
    }
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
            EntitlementEntityId::Subscription(_) | EntitlementEntityId::Quote(_) => {}
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
            EntitlementEntityId::Subscription(_) | EntitlementEntityId::Quote(_) => None,
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
async fn build_entitlement_rows(
    conn: &mut PgConn,
    specs: Vec<EntitlementSpec>,
    entity: &EntitlementEntityId,
    tenant_id: TenantId,
) -> StoreResult<Vec<EntitlementRowNew>> {
    let feature_ids: Vec<FeatureId> = specs.iter().map(|s| s.feature_id).collect();
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

    specs
        .into_iter()
        .map(|spec| {
            let feature = feature_type_map.get(&spec.feature_id).ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(format!(
                    "feature {} not found",
                    spec.feature_id
                )))
            })?;
            validate_value_matches_feature_type(&spec.value, &feature.feature_type)?;
            let json_value = value_to_json(&spec.value)?;
            Ok(EntitlementRowNew {
                id: EntitlementId::new(),
                tenant_id,
                feature_id: spec.feature_id,
                entity_id,
                entity_type,
                mode: mode.clone().into(),
                value: json_value,
            })
        })
        .collect()
}

/// Insert a batch of entitlement specs for a single entity inside an existing transaction.
/// The caller provides the entity (the thing being created), tenant/actor context, and the specs.
pub(crate) async fn insert_entitlement_specs(
    conn: &mut PgConn,
    specs: Vec<EntitlementSpec>,
    entity: EntitlementEntityId,
    tenant_id: TenantId,
) -> StoreResult<Vec<Entitlement>> {
    if specs.is_empty() {
        return Ok(vec![]);
    }

    let rows = build_entitlement_rows(conn, specs, &entity, tenant_id).await?;
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
/// 1. Load scoped features (active + disabled, not archived) for `product_ids`, including
///    globally-scoped features (`product_id IS NULL`). Product names are joined at DB level.
/// 2. Load entity-bound entitlements for every id in `chain`, optionally narrowed to
///    `filter_feature_id` at the DB level.
/// 3. Load feature-level (tenant-default) entitlements, restricted to scoped feature ids
///    to prevent cross-product leakage.
/// 4. Drop rows outside the product scope or `filter_feature_id`.
/// 5. [`resolve`]: sort by `entity.priority()` asc — `Feature(0) < Plan(1) < AddOn(2) <
///    PlanVersion(3) < Subscription(4) = Quote(4)` — then fold. Higher priority replaces
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

async fn resolve_entitlements_tx(
    conn: &mut PgConn,
    customer_id: CustomerId,
    tenant_id: TenantId,
    filter_feature_id: Option<FeatureId>,
) -> StoreResult<Vec<ResolvedEntitlement>> {
    use itertools::Itertools;

    let sub_rows = SubscriptionRow::find_active_by_customer(conn, customer_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    if sub_rows.is_empty() {
        return Ok(vec![]);
    }

    let (subscription_ids, plan_version_ids): (Vec<_>, Vec<_>) =
        sub_rows.iter().map(|r| (r.id, r.plan_version_id)).unzip();

    let plan_id_map = PlanVersionRow::get_plan_ids_by_version_ids(conn, &plan_version_ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let add_on_ids =
        SubscriptionAddOnRow::list_active_add_on_ids(conn, &subscription_ids, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

    let add_on_product_ids =
        SubscriptionAddOnRow::list_active_product_ids(conn, &subscription_ids, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

    let component_product_ids =
        SubscriptionComponentRow::list_product_ids(conn, &subscription_ids, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

    let product_ids: Vec<ProductId> = itertools::chain!(add_on_product_ids, component_product_ids)
        .unique()
        .collect();

    let chain: Vec<EntitlementEntityId> = itertools::chain!(
        plan_id_map
            .values()
            .map(|id| EntitlementEntityId::Plan(*id)),
        plan_version_ids
            .iter()
            .map(|id| EntitlementEntityId::PlanVersion(*id)),
        add_on_ids.iter().map(|id| EntitlementEntityId::AddOn(*id)),
        subscription_ids
            .iter()
            .map(|id| EntitlementEntityId::Subscription(*id)),
    )
    .collect();

    // Customer-wide effective lookup always includes tenant-global features so the
    // customer sees every entitlement that applies to them (Sub/Quote chain semantics).
    resolve_for_chain(
        conn,
        tenant_id,
        &chain,
        &product_ids,
        true,
        filter_feature_id,
    )
    .await
}

/// Per-feature fold over a pre-filtered, pre-scoped set of entitlements.
///
/// Priority ladder (low → high):
/// `Feature(0) < Plan(1) < AddOn(2) < PlanVersion(3) < Subscription(4) = Quote(4)`.
/// Subscription always wins for a customer; PlanVersion overrides its linked AddOns
/// because the plan version is the authoritative composition for the plan.
///
/// Algorithm:
/// - Skip features with status != Active (Disabled = global off-switch; Archived = hidden).
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
                    warning_threshold_pct,
                    enabled,
                },
            ) => ResolvedEntitlementValue::Metered {
                metric_id,
                limit,
                reset_period,
                overage_behavior,
                warning_threshold_pct,
                enabled,
            },
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
        // Type mismatch should never reach here (validated at insert; logged as `[BUG]`
        // by the caller). Keep `prev`'s origin to avoid spurious origin churn.
        _ => true,
    }
}

/// Combine two same-priority `EntitlementValue`s, taking the more permissive of each
/// *visible* field (algorithm rule §2):
/// - `limit`: `max` (None = unlimited absorbs any finite).
/// - `enabled`: OR — kill-switch must be set on *every* grant to disable the feature.
/// - `warning_threshold_pct`: `min` — warn at the earliest threshold any grant asks for.
///
/// `reset_period` and `overage_behavior` are not part of the "more permissive" comparison;
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
                warning_threshold_pct: wta,
                enabled: ea,
            },
            EntitlementValue::Metered {
                limit: lb,
                reset_period: rpb,
                overage_behavior: obb,
                warning_threshold_pct: wtb,
                enabled: eb,
            },
        ) => EntitlementValue::Metered {
            limit: match (la, lb) {
                (None, _) | (_, None) => None,
                (Some(x), Some(y)) => Some(x.max(y)),
            },
            reset_period: rpb,
            overage_behavior: obb,
            warning_threshold_pct: match (wta, wtb) {
                (Some(x), Some(y)) => Some(x.min(y)),
                (None, w) | (w, None) => w,
            },
            enabled: ea || eb,
        },
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
/// - `warning_threshold_pct`: take `b`'s value when set, otherwise fall back to `a`.
/// - `reset_period`, `overage_behavior`: take `b`'s value — set them consistently across the
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
                warning_threshold_pct: wta,
                enabled: ea,
            },
            EntitlementValue::Metered {
                limit: lb,
                reset_period: rpb,
                overage_behavior: obb,
                warning_threshold_pct: wtb,
                enabled: eb,
            },
        ) => EntitlementValue::Metered {
            limit: match (la, lb) {
                (None, _) | (_, None) => None,
                (Some(x), Some(y)) => Some(x + y),
            },
            reset_period: rpb,
            overage_behavior: obb,
            warning_threshold_pct: wtb.or(wta),
            enabled: ea || eb,
        },
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
    use chrono::Utc;
    use common_domain::ids::{
        AddOnId, BaseId, BillableMetricId, PlanId, PlanVersionId, QuoteId, SubscriptionId,
    };

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
            description: None,
            feature_type,
            status: FeatureStatusEnum::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
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
                    limit: Some(100.into()),
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    warning_threshold_pct: None,
                    enabled: true,
                },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::Subscription(SubscriptionId::new()),
                EntitlementModeEnum::Override,
                EntitlementValue::Metered {
                    limit: Some(500.into()),
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    warning_threshold_pct: None,
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
                    limit: Some(1000.into()),
                    reset_period: ResetPeriod::BillingCycle,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    warning_threshold_pct: Some(90),
                    enabled: true,
                },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::AddOn(aid2),
                EntitlementModeEnum::Stack,
                EntitlementValue::Metered {
                    limit: Some(1000.into()),
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    warning_threshold_pct: None,
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
            limit: Some(100.into()),
            reset_period: ResetPeriod::Never,
            overage_behavior: OverageBehavior::Block {
                grace_period_pct: None,
            },
            warning_threshold_pct: None,
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
            a.created_at = Utc::now() - chrono::Duration::seconds(60);
            let mut b = make_entitlement(
                fid,
                EntitlementEntityId::AddOn(second),
                EntitlementModeEnum::Stack,
                metered(second == aid_on),
            );
            b.created_at = Utc::now();
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
                    limit: Some(100.into()),
                    reset_period: ResetPeriod::Calendar {
                        unit: PeriodUnit::Month,
                        interval: 1,
                    },
                    overage_behavior: OverageBehavior::Allow,
                    warning_threshold_pct: Some(80),
                    enabled: true,
                },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::AddOn(AddOnId::new()),
                EntitlementModeEnum::Stack,
                EntitlementValue::Metered {
                    limit: None,
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    warning_threshold_pct: None,
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
                    limit: Some(100.into()),
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    warning_threshold_pct: Some(70),
                    enabled: true,
                },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::PlanVersion(PlanVersionId::new()),
                EntitlementModeEnum::Override,
                EntitlementValue::Metered {
                    limit: Some(200.into()),
                    reset_period: ResetPeriod::BillingCycle,
                    overage_behavior: OverageBehavior::Allow,
                    warning_threshold_pct: Some(90),
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
            warning_threshold_pct,
            ..
        } = &result[0].value
        else {
            panic!("expected metered value");
        };
        assert_eq!(*limit, Some(200.into()));
        assert!(matches!(reset_period, ResetPeriod::BillingCycle));
        assert!(matches!(overage_behavior, OverageBehavior::Allow));
        assert_eq!(*warning_threshold_pct, Some(90));
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
                    limit: Some(100.into()),
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    warning_threshold_pct: None,
                    enabled: false,
                },
            ),
            make_entitlement(
                fid,
                EntitlementEntityId::Subscription(SubscriptionId::new()),
                EntitlementModeEnum::Stack,
                EntitlementValue::Metered {
                    limit: Some(100.into()),
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    warning_threshold_pct: None,
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
                limit: Some(100.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
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
    fn usage_period_sliding_window_day() {
        let now = dt(2025, 6, 15);
        let b = compute_usage_period(
            &ResetPeriod::SlidingWindow {
                unit: PeriodUnit::Day,
                interval: 10,
            },
            None,
            None,
            now,
        );
        assert_eq!(b.start, dt(2025, 6, 5));
        assert_eq!(b.end, Some(now));
    }

    // ── compute_usage_period ──────────────────────────────────────────────────

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn dt(y: i32, m: u32, day: u32) -> NaiveDateTime {
        d(y, m, day).and_time(NaiveTime::MIN)
    }

    fn dt_hms(y: i32, m: u32, day: u32, h: u32, min: u32, s: u32) -> NaiveDateTime {
        NaiveDateTime::new(d(y, m, day), NaiveTime::from_hms_opt(h, min, s).unwrap())
    }

    #[test]
    fn usage_period_never_uses_activation_date() {
        let now = dt(2025, 6, 15);
        let activation = dt(2024, 1, 1);
        let b = compute_usage_period(&ResetPeriod::Never, None, Some(activation), now);
        assert_eq!(b.start, dt(2024, 1, 1));
        assert_eq!(b.end, None);
    }

    #[test]
    fn usage_period_never_falls_back_to_now_when_no_activation() {
        let now = dt(2025, 6, 15);
        let b = compute_usage_period(&ResetPeriod::Never, None, None, now);
        assert_eq!(b.start, now);
        assert_eq!(b.end, None);
    }

    #[test]
    fn usage_period_billing_cycle_uses_subscription_period() {
        let now = dt(2025, 6, 15);
        let bc = BillingCyclePeriod {
            period_start: d(2025, 6, 1),
            period_end: Some(d(2025, 6, 30)),
        };
        let b = compute_usage_period(&ResetPeriod::BillingCycle, Some(bc), None, now);
        assert_eq!(b.start, dt(2025, 6, 1));
        assert_eq!(b.end, Some(dt(2025, 7, 1)));
    }

    #[test]
    fn usage_period_billing_cycle_no_subscription_unbounded() {
        let now = dt(2025, 6, 15);
        let b = compute_usage_period(&ResetPeriod::BillingCycle, None, None, now);
        assert_eq!(b.start, now);
        assert_eq!(b.end, None);
    }

    #[test]
    fn usage_period_sliding_window_month() {
        // SlidingWindow always returns [now − interval, now].
        let now = dt(2025, 6, 15);
        let b = compute_usage_period(
            &ResetPeriod::SlidingWindow {
                unit: PeriodUnit::Month,
                interval: 1,
            },
            None,
            None,
            now,
        );
        assert_eq!(b.start, dt(2025, 5, 15));
        assert_eq!(b.end, Some(now));
    }

    #[test]
    fn usage_period_sliding_window_week() {
        let now = dt(2025, 6, 15);
        let b = compute_usage_period(
            &ResetPeriod::SlidingWindow {
                unit: PeriodUnit::Week,
                interval: 2,
            },
            None,
            None,
            now,
        );
        assert_eq!(b.start, dt(2025, 6, 1));
        assert_eq!(b.end, Some(now));
    }

    #[test]
    fn usage_period_sliding_window_hour() {
        let now = dt_hms(2025, 6, 15, 14, 30, 0);
        let b = compute_usage_period(
            &ResetPeriod::SlidingWindow {
                unit: PeriodUnit::Hour,
                interval: 3,
            },
            None,
            None,
            now,
        );
        assert_eq!(b.start, dt_hms(2025, 6, 15, 11, 30, 0));
        assert_eq!(b.end, Some(now));
    }

    #[test]
    fn usage_period_fixed_window_no_activation_unbounded() {
        // No anchor → can't form a fixed bucket; degrade to the same "[now, ∞)" empty
        // window that BillingCycle/Never use when their anchor is missing.
        let now = dt(2025, 6, 15);
        let b = compute_usage_period(
            &ResetPeriod::FixedWindow {
                unit: PeriodUnit::Day,
                interval: 7,
            },
            None,
            None,
            now,
        );
        assert_eq!(b.start, now);
        assert_eq!(b.end, None);
    }

    #[test]
    fn usage_period_fixed_window_day_first_bucket_starts_at_activation() {
        // Activation 2025-06-10, 7-day buckets. `now` 2025-06-13 falls in bucket 0.
        let activation = dt(2025, 6, 10);
        let now = dt(2025, 6, 13);
        let b = compute_usage_period(
            &ResetPeriod::FixedWindow {
                unit: PeriodUnit::Day,
                interval: 7,
            },
            None,
            Some(activation),
            now,
        );
        assert_eq!(b.start, dt(2025, 6, 10));
        assert_eq!(b.end, Some(dt(2025, 6, 17)));
    }

    #[test]
    fn usage_period_fixed_window_day_advances_to_current_bucket() {
        // Activation 2025-06-10, 7-day buckets. `now` 2025-07-02 → bucket 3
        // (start 2025-07-01, end 2025-07-08).
        let activation = dt(2025, 6, 10);
        let now = dt(2025, 7, 2);
        let b = compute_usage_period(
            &ResetPeriod::FixedWindow {
                unit: PeriodUnit::Day,
                interval: 7,
            },
            None,
            Some(activation),
            now,
        );
        assert_eq!(b.start, dt(2025, 7, 1));
        assert_eq!(b.end, Some(dt(2025, 7, 8)));
    }

    #[test]
    fn usage_period_fixed_window_hour_anchored_on_activation() {
        // Activation midnight 2025-06-10, 6-hour buckets. `now` 2025-06-10 14:30
        // → bucket 2 (start 12:00, end 18:00).
        let activation = dt(2025, 6, 10);
        let now = dt_hms(2025, 6, 10, 14, 30, 0);
        let b = compute_usage_period(
            &ResetPeriod::FixedWindow {
                unit: PeriodUnit::Hour,
                interval: 6,
            },
            None,
            Some(activation),
            now,
        );
        assert_eq!(b.start, dt_hms(2025, 6, 10, 12, 0, 0));
        assert_eq!(b.end, Some(dt_hms(2025, 6, 10, 18, 0, 0)));
    }

    #[test]
    fn usage_period_fixed_window_hour_anchored_on_activation_with_time() {
        let activation = dt_hms(2025, 5, 27, 14, 10, 0);
        let now = dt_hms(2025, 5, 27, 14, 20, 0);
        let b = compute_usage_period(
            &ResetPeriod::FixedWindow {
                unit: PeriodUnit::Hour,
                interval: 2,
            },
            None,
            Some(activation),
            now,
        );
        assert_eq!(b.start, dt_hms(2025, 5, 27, 14, 10, 0));
        assert_eq!(b.end, Some(dt_hms(2025, 5, 27, 16, 10, 0)));
    }

    #[test]
    fn usage_period_fixed_window_month_advances_to_current_bucket() {
        // Activation 2025-01-15, 1-month buckets. `now` 2025-06-20 → bucket starts 2025-06-15.
        let activation = dt(2025, 1, 15);
        let now = dt(2025, 6, 20);
        let b = compute_usage_period(
            &ResetPeriod::FixedWindow {
                unit: PeriodUnit::Month,
                interval: 1,
            },
            None,
            Some(activation),
            now,
        );
        assert_eq!(b.start, dt(2025, 6, 15));
        assert_eq!(b.end, Some(dt(2025, 7, 15)));
    }

    #[test]
    fn usage_period_fixed_window_year_advances_to_current_bucket() {
        // Activation 2020-03-10, 1-year buckets. `now` 2025-06-15 → bucket starts 2025-03-10.
        let activation = dt(2020, 3, 10);
        let now = dt(2025, 6, 15);
        let b = compute_usage_period(
            &ResetPeriod::FixedWindow {
                unit: PeriodUnit::Year,
                interval: 1,
            },
            None,
            Some(activation),
            now,
        );
        assert_eq!(b.start, dt(2025, 3, 10));
        assert_eq!(b.end, Some(dt(2026, 3, 10)));
    }

    #[test]
    fn calendar_period_month_interval_1_mid_month() {
        let b = calendar_period(dt(2025, 6, 15), &PeriodUnit::Month, 1);
        assert_eq!(b.start, dt(2025, 6, 1));
        assert_eq!(b.end, Some(dt(2025, 7, 1)));
    }

    #[test]
    fn calendar_period_month_interval_1_end_of_month() {
        let b = calendar_period(dt(2025, 1, 31), &PeriodUnit::Month, 1);
        assert_eq!(b.start, dt(2025, 1, 1));
        assert_eq!(b.end, Some(dt(2025, 2, 1)));
    }

    #[test]
    fn calendar_period_month_interval_3_q2() {
        let b = calendar_period(dt(2025, 6, 15), &PeriodUnit::Month, 3);
        assert_eq!(b.start, dt(2025, 4, 1));
        assert_eq!(b.end, Some(dt(2025, 7, 1)));
    }

    #[test]
    fn calendar_period_week_interval_1_wednesday() {
        let b = calendar_period(dt(2025, 6, 11), &PeriodUnit::Week, 1);
        assert_eq!(b.start, dt(2025, 6, 9));
        assert_eq!(b.end, Some(dt(2025, 6, 16)));
    }

    #[test]
    fn calendar_period_week_interval_2() {
        let b = calendar_period(dt(2025, 6, 11), &PeriodUnit::Week, 2);
        assert_eq!(b.start, dt(2025, 6, 2));
        assert_eq!(b.end, Some(dt(2025, 6, 16)));
    }

    #[test]
    fn calendar_period_year_interval_1() {
        let b = calendar_period(dt(2025, 8, 20), &PeriodUnit::Year, 1);
        assert_eq!(b.start, dt(2025, 1, 1));
        assert_eq!(b.end, Some(dt(2026, 1, 1)));
    }

    #[test]
    fn calendar_period_day_interval_1() {
        let now = dt(2025, 6, 15);
        let b = calendar_period(now, &PeriodUnit::Day, 1);
        assert_eq!(b.start, dt(2025, 6, 15));
        assert_eq!(b.end, Some(dt(2025, 6, 16)));
    }

    #[test]
    fn calendar_period_day_interval_7() {
        let now = dt(2025, 6, 15);
        let b = calendar_period(now, &PeriodUnit::Day, 7);
        assert!(b.start <= now);
        assert!(b.end.unwrap() > now);
        assert_eq!((b.end.unwrap() - b.start).num_days(), 7);
    }

    #[test]
    fn calendar_period_hour_interval_1() {
        let now = dt_hms(2025, 6, 15, 14, 30, 0);
        let b = calendar_period(now, &PeriodUnit::Hour, 1);
        assert_eq!(b.start, dt_hms(2025, 6, 15, 14, 0, 0));
        assert_eq!(b.end, Some(dt_hms(2025, 6, 15, 15, 0, 0)));
    }

    #[test]
    fn calendar_period_hour_interval_4_across_midnight() {
        let now = dt_hms(2025, 6, 15, 1, 15, 0);
        let b = calendar_period(now, &PeriodUnit::Hour, 4);
        assert_eq!(b.start, dt_hms(2025, 6, 15, 0, 0, 0));
        assert_eq!(b.end, Some(dt_hms(2025, 6, 15, 4, 0, 0)));
    }

    #[test]
    fn subtract_month() {
        assert_eq!(
            subtract_duration(dt(2025, 3, 31), &PeriodUnit::Month, 1),
            dt(2025, 2, 28)
        );
    }

    #[test]
    fn subtract_year() {
        assert_eq!(
            subtract_duration(dt(2025, 6, 15), &PeriodUnit::Year, 2),
            dt(2023, 6, 15)
        );
    }

    #[test]
    fn subtract_week() {
        assert_eq!(
            subtract_duration(dt(2025, 6, 15), &PeriodUnit::Week, 3),
            dt(2025, 5, 25)
        );
    }

    #[test]
    fn subtract_day() {
        assert_eq!(
            subtract_duration(dt(2025, 6, 15), &PeriodUnit::Day, 10),
            dt(2025, 6, 5)
        );
    }

    #[test]
    fn subtract_hour() {
        assert_eq!(
            subtract_duration(dt_hms(2025, 6, 15, 10, 30, 0), &PeriodUnit::Hour, 3),
            dt_hms(2025, 6, 15, 7, 30, 0)
        );
    }

    #[test]
    fn subtract_hour_crosses_midnight() {
        assert_eq!(
            subtract_duration(dt_hms(2025, 6, 15, 1, 0, 0), &PeriodUnit::Hour, 3),
            dt_hms(2025, 6, 14, 22, 0, 0)
        );
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
                limit: Some(200.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
                enabled: true,
            },
        );
        a.created_at = Utc::now() - chrono::Duration::seconds(60);
        let mut b = make_entitlement(
            fid,
            EntitlementEntityId::Plan(PlanId::new()),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                limit: Some(100.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
                enabled: true,
            },
        );
        b.created_at = Utc::now();

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
        a.created_at = Utc::now() - chrono::Duration::seconds(60);
        let mut b = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(AddOnId::new()),
            EntitlementModeEnum::Override,
            EntitlementValue::Boolean { enabled: false },
        );
        b.created_at = Utc::now();

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
                limit: Some(500.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
                enabled: true,
            },
        );
        winner.created_at = Utc::now() - chrono::Duration::seconds(60);
        let mut loser = make_entitlement(
            fid,
            EntitlementEntityId::Plan(losing_plan_id),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                limit: Some(100.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
                enabled: true,
            },
        );
        loser.created_at = Utc::now();
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
                limit: Some(100.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
                enabled: true,
            },
        );
        a.created_at = Utc::now() - chrono::Duration::seconds(60);
        let mut b = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(second_addon),
            EntitlementModeEnum::Stack,
            EntitlementValue::Metered {
                limit: Some(50.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
                enabled: true,
            },
        );
        b.created_at = Utc::now();
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
                limit: Some(100.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
                enabled: true,
            },
        );
        let plan_version = make_entitlement(
            fid,
            EntitlementEntityId::PlanVersion(PlanVersionId::new()),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                limit: Some(200.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
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
                limit: Some(1000.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
                enabled: true,
            },
        );
        let b = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(aid2),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                limit: Some(1000.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
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
                limit: Some(1000.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
                enabled: true,
            },
        );
        let b = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(AddOnId::new()),
            EntitlementModeEnum::Stack,
            EntitlementValue::Metered {
                limit: Some(500.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
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
                limit: Some(50.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
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
                    limit: Some(limit.into()),
                    reset_period: ResetPeriod::Never,
                    overage_behavior: OverageBehavior::Block {
                        grace_period_pct: None,
                    },
                    warning_threshold_pct: None,
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
                limit: Some(300.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
                enabled: true,
            },
        );
        let b = make_entitlement(
            fid,
            EntitlementEntityId::AddOn(AddOnId::new()),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                limit: Some(700.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
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
                limit: None,
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
                enabled: true,
            },
        );
        let plan_version = make_entitlement(
            fid,
            EntitlementEntityId::PlanVersion(PlanVersionId::new()),
            EntitlementModeEnum::Override,
            EntitlementValue::Metered {
                limit: Some(100.into()),
                reset_period: ResetPeriod::Never,
                overage_behavior: OverageBehavior::Block {
                    grace_period_pct: None,
                },
                warning_threshold_pct: None,
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
