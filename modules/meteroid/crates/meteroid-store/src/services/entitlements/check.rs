//! The basic check (ADR 0011 §3.14): values, exact app counts and read event usage, decided per
//! level; the live engine then merges its monitor usage and denials through `live_check`.
use crate::domain::entitlement_checks::{
    AppCountExplain, CheckHint, CheckLevel, CheckLevelExplain, CheckReason, CheckRequest,
    CheckResponse, CheckedFeature, LevelUsage, LevelValue, LiveCheckLevel, LiveCheckRequest,
    LiveLevelUsage, ResolvedLevel, USAGE_UNAVAILABLE_WARNING, UsageSource,
};
use crate::domain::entitlement_fold::{EvalLevel, GrantValue, LimitValue};
use crate::domain::entitlement_levels::{LevelOutcome, combined_value, decide};
use crate::domain::entitlement_policy::{AtLimit, Resets};
use crate::domain::entitlements::{Feature, FeatureKind, LimitCountedBy};
use crate::domain::resources::{ResolvedResource, ResourceInput};
use crate::errors::StoreError;
use crate::repositories::entitlements::SourceNames;
use crate::services::entitlements::limits::level_monitor_id;
use crate::services::entitlements::resolve::{UsageReading, resolve_resource_input};
use crate::store::PgConn;
use crate::{Store, StoreResult};
use chrono::{DateTime, Utc};
use common_domain::ids::{AliasOr, CustomerId, FeatureId, TenantId};
use diesel_models::entitlement_app_counts::{EntitlementAppCountOpRow, EntitlementAppCountRow};
use diesel_models::entitlements::FeatureRow;
use error_stack::bail;
use rust_decimal::Decimal;

impl Store {
    pub async fn check_entitlement(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        feature: AliasOr<FeatureId>,
        request: CheckRequest,
    ) -> StoreResult<CheckResponse> {
        let now = common_clock::now();
        let requested = request.requested.unwrap_or(Decimal::ONE);
        if requested < Decimal::ZERO {
            bail!(StoreError::InvalidArgument(
                "requested must not be negative".to_owned()
            ));
        }
        let input = ResourceInput {
            resources: request.resources.clone(),
            attributes: request.attributes.clone(),
        };
        let (feature, resolved, resource) = {
            let mut conn = self.get_conn().await?;
            let feature: Feature = FeatureRow::find_by_id_or_code(&mut conn, feature, tenant_id)
                .await?
                .try_into()?;
            let resource =
                resolve_resource_input(&mut conn, tenant_id, customer_id, &input).await?;
            log_asserted(
                tenant_id,
                customer_id,
                resource.as_ref().map(|(r, _)| r),
                &request,
            );
            let resolved = self
                .resolve_customer_entitlements(
                    &mut conn,
                    tenant_id,
                    customer_id,
                    resource.as_ref().map(|(r, _)| r),
                    Some(std::slice::from_ref(&feature.id)),
                    now,
                )
                .await?;
            (feature, resolved.features.into_iter().next(), resource)
        };
        let mut response = CheckResponse {
            feature: CheckedFeature {
                id: feature.id,
                code: feature.code.clone(),
            },
            allowed: false,
            reason: Some(CheckReason::NotEntitled),
            reason_type: Some(CheckReason::NotEntitled.reason_type()),
            value: None,
            levels: vec![],
            near_limit: false,
            retry_after: None,
            hint: None,
            attributes_ignored: resource
                .as_ref()
                .map(|(r, _)| r.attributes_ignored.clone())
                .unwrap_or_default(),
            warnings: request
                .subscription_id
                .map(|_| "subscription_id_ignored".to_owned())
                .into_iter()
                .collect(),
            usage_source: UsageSource::Read,
            usage_as_of: None,
        };
        let Some(resolved) = resolved else {
            return Ok(response);
        };

        // The live engine answers first: a level it reads from a fresh monitor needs no read here.
        let live_request = live_request(
            &feature,
            customer_id,
            &resolved.levels,
            resource.as_ref().map(|(r, _)| r),
            requested,
        );
        let live = self
            .hooks
            .credit
            .live_check(
                &mut self.get_conn().await?,
                tenant_id,
                customer_id,
                &live_request,
                now,
            )
            .await?;
        if live.is_some() {
            response.usage_source = UsageSource::Monitor;
        }
        let reading = UsageReading::check();
        let mut used = Vec::with_capacity(resolved.levels.len());
        let mut unavailable = false;
        for level in &resolved.levels {
            let of = |readings: &[LiveLevelUsage]| {
                readings
                    .iter()
                    .find(|usage| usage.level == level.level)
                    .cloned()
            };
            let monitored = live.as_ref().and_then(|live| of(&live.levels));
            let read = match (monitored, request.current_count, &feature.kind) {
                (Some(monitor), _, _) => LevelUsage::Read(Some(monitor.used)),
                (None, Some(count), FeatureKind::Limit(LimitCountedBy::App)) if level.own => {
                    LevelUsage::Read(Some(count.max(Decimal::ZERO)))
                }
                _ => {
                    self.level_usage_read(
                        tenant_id,
                        customer_id,
                        &feature,
                        level,
                        resource.as_ref(),
                        now,
                        None,
                        &reading,
                    )
                    .await?
                }
            };
            // Metering unavailable: the monitor's last reading answers, whatever its age (§3.14).
            let read = match read {
                LevelUsage::Unavailable => match live.as_ref().and_then(|live| of(&live.last)) {
                    Some(last) => {
                        response.usage_source = UsageSource::Monitor;
                        response.usage_as_of = Some(
                            response
                                .usage_as_of
                                .map_or(last.as_of, |as_of| as_of.min(last.as_of)),
                        );
                        LevelUsage::Read(Some(last.used))
                    }
                    None => {
                        unavailable = true;
                        LevelUsage::Unavailable
                    }
                },
                read => read,
            };
            used.push(read);
        }
        if unavailable {
            response.warnings.push(USAGE_UNAVAILABLE_WARNING.to_owned());
        }
        let mut conn = self.get_conn().await?;

        let outcomes: Vec<LevelOutcome> = resolved
            .levels
            .iter()
            .zip(&used)
            .map(|(level, used)| outcome(level, *used))
            .collect();
        let decision = decide(&outcomes, requested, now);
        let live_denial = live.and_then(|l| l.denial).filter(|_| decision.allowed);
        response.allowed = decision.allowed && live_denial.is_none();
        response.reason = decision.reason.or(live_denial);
        response.reason_type = response.reason.map(CheckReason::reason_type);
        response.near_limit = decision.near_limit;
        response.retry_after = decision.retry_after;
        response.value = combined_value(&feature.value_order(), &outcomes);
        response.hint = resolved
            .levels
            .iter()
            .any(|l| l.resource_omitted)
            .then_some(CheckHint::ResourceOmitted);

        let mut levels = Vec::with_capacity(outcomes.len());
        for (level, outcome) in resolved.levels.iter().zip(&outcomes) {
            let assessment = outcome.assess(requested);
            let limit = match &outcome.value {
                LevelValue::Value(GrantValue::Limit(LimitValue::Amount(n))) => Some(*n),
                _ => None,
            };
            let explain = if request.explain {
                Some(CheckLevelExplain {
                    contributions: level.explain.clone(),
                    candidates: level.candidates.clone(),
                    app_counts: app_count_explain(
                        &mut conn,
                        tenant_id,
                        customer_id,
                        &feature,
                        level,
                    )
                    .await?,
                })
            } else {
                None
            };
            levels.push(CheckLevel {
                level: level.level,
                path: level.path.clone(),
                value: match &level.value {
                    LevelValue::Value(v) => Some(v.clone()),
                    _ => None,
                },
                limit,
                used: outcome.usage.used(),
                near_limit: assessment.near_limit,
                reset_at: outcome.reset_at,
                window: level.window.clone(),
                deciding_subscription: level.deciding_subscription,
                denial: assessment.denial,
                in_overage: outcome.policy.at_limit == AtLimit::AllowAndBill
                    && limit
                        .zip(outcome.usage.used())
                        .is_some_and(|(limit, used)| {
                            used.checked_add(requested)
                                .is_none_or(|total| total > limit)
                        }),
                explain,
            });
        }
        if request.explain {
            let resource_id = resource.as_ref().and_then(|(r, _)| r.reference.stored_id());
            let explained = levels.iter().filter_map(|l| l.explain.as_ref());
            let names = SourceNames::for_contributions(
                &mut conn,
                tenant_id,
                explained.flat_map(|e| &e.contributions),
                resource_id,
            )
            .await?;
            for explain in levels.iter_mut().filter_map(|l| l.explain.as_mut()) {
                names.name(&mut explain.contributions, resource_id);
            }
        }
        response.levels = levels;
        Ok(response)
    }
}

fn outcome(level: &ResolvedLevel, usage: LevelUsage) -> LevelOutcome {
    let resets = level.policy.resets != Resets::Sliding;
    LevelOutcome {
        level: level.level,
        own: level.own,
        value: level.value.clone(),
        policy: level.policy.clone(),
        usage,
        reset_at: level.window.as_ref().and_then(|w| w.end).filter(|_| resets),
    }
}

/// The levels the live engine monitors: limits counted from events, with their window.
fn live_request(
    feature: &Feature,
    customer_id: CustomerId,
    levels: &[ResolvedLevel],
    resource: Option<&ResolvedResource>,
    requested: Decimal,
) -> LiveCheckRequest {
    let monitored = matches!(feature.kind, FeatureKind::Limit(LimitCountedBy::Events(_)));
    LiveCheckRequest {
        feature_id: feature.id,
        requested,
        levels: levels
            .iter()
            .filter(|l| monitored && matches!(l.value, LevelValue::Value(_)))
            .map(|l| LiveCheckLevel {
                level: l.level,
                path: l.path.clone(),
                monitor_id: level_monitor_id(feature.id, customer_id, l.level),
                limit: match &l.value {
                    LevelValue::Value(GrantValue::Limit(LimitValue::Amount(n))) => Some(*n),
                    _ => None,
                },
                policy: l.policy.clone(),
                window: l.window.clone(),
                deciding_subscription: l.deciding_subscription,
            })
            .collect(),
        deciding_subscription: levels
            .iter()
            .find(|l| l.own)
            .and_then(|l| l.deciding_subscription),
        resource: resource.cloned(),
    }
}

/// The exact count per path of a limit counted by the app, its last set and the adds since.
async fn app_count_explain(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    feature: &Feature,
    level: &ResolvedLevel,
) -> StoreResult<Vec<AppCountExplain>> {
    if !feature.kind.counted_by_app() {
        return Ok(vec![]);
    }
    let counts = match level.level {
        EvalLevel::Customer => {
            EntitlementAppCountRow::list_for_customer(conn, tenant_id, customer_id, feature.id)
                .await?
        }
        EvalLevel::Type(_) => {
            EntitlementAppCountRow::find(conn, tenant_id, customer_id, feature.id, &level.path)
                .await?
                .into_iter()
                .collect()
        }
    };
    let mut explained = Vec::with_capacity(counts.len());
    for count in counts {
        let since = count.last_set_at.unwrap_or(DateTime::<Utc>::MIN_UTC);
        let adds = EntitlementAppCountOpRow::adds_since(
            conn,
            tenant_id,
            customer_id,
            feature.id,
            &count.path,
            since,
        )
        .await?;
        explained.push(AppCountExplain {
            path: count.path,
            count: count.count,
            last_set_value: count.last_set_value,
            last_set_at: count.last_set_at,
            last_set_key: count.last_set_key,
            adds_since: adds.count,
            adds_sum: adds.sum.unwrap_or(Decimal::ZERO),
            last_add_at: adds.last_at,
        });
    }
    Ok(explained)
}

/// The security trail of asserted attributes (§3.4): one line each, no table.
fn log_asserted(
    tenant_id: TenantId,
    customer_id: CustomerId,
    resource: Option<&ResolvedResource>,
    request: &CheckRequest,
) {
    let path = resource.map_or("", |r| r.path.as_str());
    for (attribute, value) in &request.attributes {
        tracing::info!(
            %tenant_id,
            %customer_id,
            resource = path,
            attribute = attribute.as_str(),
            value = value.as_str(),
            api_key_id = ?request.api_key_id,
            "entitlement check asserted an attribute"
        );
    }
}
