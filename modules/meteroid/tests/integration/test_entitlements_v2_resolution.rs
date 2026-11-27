//! ADR 0011 §8, against the store: rows (§3.1, §3.2), levels (§3.3), the deciding subscription
//! (§3.7), the etag (§3.10) and the history `as_of` would read (§3.12).
use crate::data::ids::{
    API_TOKEN_ID, CUST_COMODO_ID, CUST_SPOTIFY_ID, CUST_UBER_ID, METRIC_BANDWIDTH,
    PLAN_LEETCODE_ID, PLAN_NOTION_ID, PLAN_VERSION_1_LEETCODE_ID, PLAN_VERSION_NOTION_ID,
    TENANT_ID, USER_ID,
};
use crate::meteroid_it;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use chrono::{Duration, NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    AliasOr, BaseId, CustomerId, EntitlementEntityId, PlanId, PlanVersionId, ResourceId,
    SubscriptionId,
};
use diesel_async::RunQueryDsl;
use diesel_models::plans::PlanRow;
use meteroid_store::Store;
use meteroid_store::StoreResult;
use meteroid_store::domain::entitlement_checks::{
    CheckHint, CheckReason, CheckRequest, CheckResponse, LevelValue, ReasonType, SnapshotRead,
};
use meteroid_store::domain::entitlement_fold::{
    Condition, Effect, EvalLevel, GrantValue, LimitValue, RowLevel,
};
use meteroid_store::domain::entitlement_policy::{AtLimit, FeaturePolicy};
use meteroid_store::domain::entitlements::{
    Entitlement, EntitlementNew, EntitlementSpec, EntitlementUpdate, Feature, FeatureKind,
    FeatureNew, LimitCountedBy,
};
use meteroid_store::domain::resources::{ResourceInput, ResourceType};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::resources::{ResourceRefusal, ResourceTypeNew, ResourceUpsert};
use meteroid_store::repositories::{EntitlementsInterface, ResourcesInterface};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

struct Env {
    setup: MeteroidSetup,
}

fn limit(n: i64) -> GrantValue {
    GrantValue::Limit(LimitValue::Amount(Decimal::from(n)))
}

fn pairs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn spec(feature: &Feature, value: GrantValue) -> EntitlementSpec {
    EntitlementSpec::new(feature.id, value)
}

fn invalid<T: std::fmt::Debug>(result: StoreResult<T>) -> String {
    match result.expect_err("refused").current_context() {
        StoreError::InvalidArgument(message) => message.clone(),
        other => panic!("expected a 400, got {other:?}"),
    }
}

impl Env {
    async fn new() -> Self {
        crate::helpers::init::logging();
        let database = meteroid_it::container::create_test_database().await;
        Self {
            setup: meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await,
        }
    }

    fn store(&self) -> &Store {
        &self.setup.store
    }

    async fn feature(
        &self,
        code: &str,
        kind: FeatureKind,
        level: Option<&ResourceType>,
        default: Option<GrantValue>,
    ) -> Feature {
        self.store()
            .create_feature(
                Actor::System,
                FeatureNew {
                    tenant_id: TENANT_ID,
                    product_id: None,
                    name: code.into(),
                    code: code.into(),
                    description: None,
                    kind,
                    unit: None,
                    level_type_id: level.map(|t| t.id),
                    policy: None,
                    default,
                },
            )
            .await
            .unwrap()
    }

    async fn row(
        &self,
        entity: EntitlementEntityId,
        resource_id: Option<ResourceId>,
        spec: EntitlementSpec,
    ) -> StoreResult<Entitlement> {
        self.store()
            .create_entitlement(
                &Actor::User { id: USER_ID },
                EntitlementNew {
                    tenant_id: TENANT_ID,
                    entity,
                    resource_id,
                    spec,
                },
            )
            .await
    }

    async fn site_type(&self) -> ResourceType {
        self.store()
            .create_resource_type(ResourceTypeNew {
                tenant_id: TENANT_ID,
                code: "site".into(),
                name: "Site".into(),
                plural_name: "Sites".into(),
                parent_type_id: None,
                keys: vec!["site_id".into()],
                attributes: [(
                    "environment".to_string(),
                    vec!["production".to_string(), "staging".to_string()],
                )]
                .into(),
            })
            .await
            .unwrap()
    }

    async fn site(&self, customer_id: CustomerId, key_value: &str) -> ResourceId {
        self.store()
            .upsert_resource(ResourceUpsert {
                tenant_id: TENANT_ID,
                customer_id,
                resource_type: AliasOr::Alias("site".into()),
                key_value: key_value.into(),
                name: None,
                attributes: BTreeMap::new(),
                archived: false,
            })
            .await
            .unwrap()
            .id
    }

    /// An active subscription, written directly: resolution only reads its plan and resource.
    async fn subscribe(
        &self,
        customer_id: CustomerId,
        plan_version_id: PlanVersionId,
        start: NaiveDate,
        resource_id: Option<ResourceId>,
    ) -> SubscriptionId {
        use diesel::sql_types::{Date, Nullable, Uuid};
        let id = SubscriptionId::new();
        diesel::sql_query(
            "INSERT INTO subscription (id, customer_id, billing_day_anchor, tenant_id, start_date,
                 plan_version_id, created_at, net_terms, currency, mrr_cents, period,
                 pending_checkout, activation_condition, status, current_period_start,
                 auto_advance_invoices, charge_automatically, backdate_invoices, custom_properties,
                 tax_inclusive, lifecycle_suspended, resource_id, activated_at)
             VALUES ($1, $2, 1, $3, $4, $5, now(), 0, 'EUR', 0, 'MONTHLY', false, 'ON_START',
                 'ACTIVE', $4, true, false, false, '{}', false, false, $6, now())",
        )
        .bind::<Uuid, _>(id.as_uuid())
        .bind::<Uuid, _>(customer_id.as_uuid())
        .bind::<Uuid, _>(TENANT_ID.as_uuid())
        .bind::<Date, _>(start)
        .bind::<Uuid, _>(plan_version_id.as_uuid())
        .bind::<Nullable<Uuid>, _>(resource_id.map(|r| r.as_uuid()))
        .execute(&mut self.store().get_conn().await.unwrap())
        .await
        .unwrap();
        id
    }

    async fn check(
        &self,
        customer_id: CustomerId,
        feature: &Feature,
        resources: &[(&str, &str)],
        attributes: &[(&str, &str)],
        current_count: Option<i64>,
    ) -> CheckResponse {
        self.store()
            .check_entitlement(
                TENANT_ID,
                customer_id,
                AliasOr::Alias(feature.code.clone()),
                CheckRequest {
                    resources: pairs(resources),
                    attributes: pairs(attributes),
                    current_count: current_count.map(Decimal::from),
                    explain: true,
                    ..CheckRequest::default()
                },
            )
            .await
            .unwrap()
    }
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, d).unwrap()
}

#[tokio::test]
async fn effects_are_allowed_per_tier_and_policy_overrides_only_on_set() {
    let env = Env::new().await;
    let projects = env
        .feature(
            "projects",
            FeatureKind::Limit(LimitCountedBy::App),
            None,
            None,
        )
        .await;
    let plan = EntitlementEntityId::Plan(PLAN_LEETCODE_ID);
    let customer = EntitlementEntityId::Customer(CUST_UBER_ID);
    env.subscribe(CUST_UBER_ID, PLAN_VERSION_1_LEETCODE_ID, day(1), None)
        .await;

    let raise = |value| EntitlementSpec {
        effect: Some(Effect::RaiseTo),
        ..spec(&projects, value)
    };
    assert!(invalid(env.row(plan, None, raise(limit(10))).await).contains("not allowed"));
    let add = EntitlementSpec {
        effect: Some(Effect::Add),
        ..spec(&projects, limit(10))
    };
    assert!(invalid(env.row(customer, None, add).await).contains("not allowed"));

    let deny_new = FeaturePolicy {
        at_limit: AtLimit::DenyNew,
        resets: meteroid_store::domain::entitlement_policy::Resets::Never,
        ..FeaturePolicy::default()
    };
    let overriding_raise = EntitlementSpec {
        policy_override: Some(deny_new.clone()),
        ..raise(limit(80))
    };
    assert!(invalid(env.row(customer, None, overriding_raise).await).contains("policy override"));

    let plan_row = EntitlementSpec {
        policy_override: Some(FeaturePolicy {
            thresholds: vec![80],
            ..deny_new
        }),
        ..spec(&projects, limit(50))
    };
    env.row(plan, None, plan_row).await.unwrap();
    let raised = env
        .row(
            customer,
            None,
            EntitlementSpec {
                effect: None,
                ..raise(limit(80))
            },
        )
        .await
        .unwrap();
    assert_eq!(
        raised.effect,
        Effect::RaiseTo,
        "a customer override defaults to Raise to"
    );

    let checked = env.check(CUST_UBER_ID, &projects, &[], &[], Some(79)).await;
    assert!(checked.allowed);
    assert_eq!(checked.value, Some(limit(80)));
    let resolved = env
        .store()
        .resolve_customer_entitlements(
            &mut env.store().get_conn().await.unwrap(),
            TENANT_ID,
            CUST_UBER_ID,
            None,
            Some(std::slice::from_ref(&projects.id)),
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(
        resolved.features[0].levels[0].policy.thresholds,
        vec![80],
        "Raise to keeps the plan's policy override"
    );
    let denied = env.check(CUST_UBER_ID, &projects, &[], &[], Some(80)).await;
    assert_eq!(denied.reason, Some(CheckReason::LimitReached));
}

#[tokio::test]
async fn edits_append_history_and_open_rows_are_unique() {
    let env = Env::new().await;
    let sso = env
        .feature(
            "sso",
            FeatureKind::Boolean,
            None,
            Some(GrantValue::Boolean(false)),
        )
        .await;
    let default = EntitlementEntityId::Feature(sso.id);
    let duplicate = env
        .row(default, None, spec(&sso, GrantValue::Boolean(true)))
        .await
        .expect_err("a second open default on the same level and condition");
    assert!(matches!(
        duplicate.current_context(),
        StoreError::DuplicateValue { .. }
    ));

    let customer = EntitlementEntityId::Customer(CUST_UBER_ID);
    let row = env
        .row(customer, None, spec(&sso, GrantValue::Boolean(true)))
        .await
        .unwrap();
    assert_eq!(row.created_by, Some(Actor::User { id: USER_ID }));
    let edited = env
        .store()
        .update_entitlement(
            &Actor::ApiToken { id: API_TOKEN_ID },
            row.id,
            TENANT_ID,
            EntitlementUpdate {
                reason: Some(Some("renewal".into())),
                ..EntitlementUpdate::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(edited.id, row.id, "an edit keeps the lineage");
    assert_ne!(edited.version_id, row.version_id);
    assert_eq!(edited.lineage_created_at, row.lineage_created_at);
    assert_eq!(
        edited.created_by,
        Some(Actor::ApiToken { id: API_TOKEN_ID })
    );

    let history = env
        .store()
        .get_entitlement_history(row.id, TENANT_ID)
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert!(history[0].revoked_at.is_some());
    assert_eq!(history[0].revoked_by, Some(API_TOKEN_ID.as_uuid()));
    assert_eq!(
        history[0].reason, None,
        "the old version is never updated in place"
    );
    assert!(history[1].revoked_at.is_none());
    assert_eq!(
        env.store()
            .get_entitlement(row.id, TENANT_ID)
            .await
            .unwrap(),
        edited
    );

    env.store()
        .delete_entitlement(&Actor::System, row.id, TENANT_ID)
        .await
        .unwrap();
    assert!(
        env.store()
            .get_entitlement(row.id, TENANT_ID)
            .await
            .is_err()
    );
    assert_eq!(
        env.store()
            .get_entitlement_history(row.id, TENANT_ID)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn levels_conditions_and_a_missing_resource() {
    let env = Env::new().await;
    let site = env.site_type().await;
    let projects = env
        .feature(
            "projects",
            FeatureKind::Limit(LimitCountedBy::App),
            Some(&site),
            Some(limit(5)),
        )
        .await;
    let default = EntitlementEntityId::Feature(projects.id);
    let staging = Condition([("site.environment".to_string(), vec!["staging".to_string()])].into());
    env.row(
        default,
        None,
        EntitlementSpec {
            condition: Some(staging.clone()),
            ..spec(&projects, GrantValue::Limit(LimitValue::Unlimited))
        },
    )
    .await
    .unwrap();

    let without_site = env.check(CUST_UBER_ID, &projects, &[], &[], Some(0)).await;
    assert_eq!(without_site.reason, Some(CheckReason::ResourceRequired));
    assert_eq!(without_site.reason_type, Some(ReasonType::Misconfigured));

    let production = env
        .check(
            CUST_UBER_ID,
            &projects,
            &[("site_id", "s1")],
            &[("site.environment", "production")],
            Some(5),
        )
        .await;
    assert_eq!(production.reason, Some(CheckReason::LimitReached));
    let staged = env
        .check(
            CUST_UBER_ID,
            &projects,
            &[("site_id", "s1")],
            &[("site.environment", "staging")],
            Some(5),
        )
        .await;
    assert!(staged.allowed, "the more specific condition wins");
    let unasserted = env
        .check(CUST_UBER_ID, &projects, &[("site_id", "s1")], &[], Some(4))
        .await;
    assert_eq!(
        unasserted.value,
        Some(limit(5)),
        "a missing attribute does not match"
    );

    // Across all sites: at most 6, counted on the customer level.
    let customer = EntitlementEntityId::Customer(CUST_UBER_ID);
    env.row(
        customer,
        None,
        EntitlementSpec {
            effect: Some(Effect::CapAt),
            level: RowLevel::Customer,
            ..spec(&projects, limit(6))
        },
    )
    .await
    .unwrap();
    let capped = env
        .check(CUST_UBER_ID, &projects, &[("site_id", "s1")], &[], Some(1))
        .await;
    assert_eq!(
        capped.levels.iter().map(|l| l.level).collect::<Vec<_>>(),
        vec![EvalLevel::Type(site.id), EvalLevel::Customer]
    );
    assert_eq!(capped.levels[1].limit, Some(Decimal::from(6)));

    assert!(
        invalid(
            env.row(
                customer,
                None,
                EntitlementSpec {
                    level: RowLevel::Customer,
                    effect: Some(Effect::Set),
                    condition: Some(staging),
                    ..spec(&projects, limit(1))
                },
            )
            .await
        )
        .contains("customer level has none"),
        "a condition on the customer level names a descendant's attribute"
    );
}

/// §3.3: a feature with a product, defaults per site and across all sites, held by a subscription
/// without rows of its own. The across-all-sites default still caps the customer.
#[tokio::test]
async fn a_default_across_all_sites_caps_a_feature_held_through_its_product() {
    use meteroid_store::services::entitlements::app_counts::{AppCountOp, AppCountWrite};
    let env = Env::new().await;
    let site = env.site_type().await;
    let product = crate::data::ids::PRODUCT_SEATS_ID;
    let projects = env
        .store()
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: Some(product),
                name: "projects".into(),
                code: "projects".into(),
                description: None,
                kind: FeatureKind::Limit(LimitCountedBy::App),
                unit: None,
                level_type_id: Some(site.id),
                policy: None,
                default: Some(limit(5)),
            },
        )
        .await
        .unwrap();
    env.row(
        EntitlementEntityId::Feature(projects.id),
        None,
        EntitlementSpec {
            level: RowLevel::Customer,
            ..spec(&projects, limit(12))
        },
    )
    .await
    .unwrap();
    let subscription = env
        .subscribe(CUST_UBER_ID, PLAN_VERSION_1_LEETCODE_ID, day(1), None)
        .await;
    diesel::sql_query(
        "INSERT INTO subscription_component (id, name, subscription_id, product_id, period)
         VALUES (gen_random_uuid(), 'Seats', $1, $2, 'MONTHLY')",
    )
    .bind::<diesel::sql_types::Uuid, _>(subscription.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(product.as_uuid())
    .execute(&mut env.store().get_conn().await.unwrap())
    .await
    .unwrap();
    for (key, n) in [("s1", 4), ("s2", 4), ("s3", 4)] {
        env.store()
            .report_app_count(
                TENANT_ID,
                CUST_UBER_ID,
                projects.id,
                AppCountWrite {
                    resources: pairs(&[("site_id", key)]),
                    op: AppCountOp::Set(Decimal::from(n)),
                    at: None,
                    idempotency_key: format!("set-{key}"),
                },
            )
            .await
            .unwrap();
    }

    let checked = env
        .check(CUST_UBER_ID, &projects, &[("site_id", "s1")], &[], None)
        .await;
    assert_eq!(
        checked.levels.iter().map(|l| l.limit).collect::<Vec<_>>(),
        vec![Some(Decimal::from(5)), Some(Decimal::from(12))],
        "{checked:?}"
    );
    assert!(!checked.allowed, "the customer holds 12 of 12: {checked:?}");
    assert_eq!(checked.reason, Some(CheckReason::LimitReached));
}

#[tokio::test]
async fn the_customer_level_answers_when_the_own_level_is_empty() {
    let env = Env::new().await;
    let site = env.site_type().await;
    let sso = env
        .feature("sso", FeatureKind::Boolean, Some(&site), None)
        .await;
    env.row(
        EntitlementEntityId::Customer(CUST_UBER_ID),
        None,
        EntitlementSpec {
            level: RowLevel::Customer,
            effect: Some(Effect::Set),
            ..spec(&sso, GrantValue::Boolean(true))
        },
    )
    .await
    .unwrap();
    let checked = env.check(CUST_UBER_ID, &sso, &[], &[], None).await;
    assert!(checked.allowed, "no resource is required: {checked:?}");

    // A customer-level off denies even when the site says on.
    let s1 = env.site(CUST_SPOTIFY_ID, "s1").await;
    let customer = EntitlementEntityId::Customer(CUST_SPOTIFY_ID);
    env.row(
        customer,
        Some(s1),
        EntitlementSpec {
            effect: Some(Effect::Set),
            ..spec(&sso, GrantValue::Boolean(true))
        },
    )
    .await
    .unwrap();
    env.row(
        customer,
        None,
        EntitlementSpec {
            level: RowLevel::Customer,
            effect: Some(Effect::Set),
            ..spec(&sso, GrantValue::Boolean(false))
        },
    )
    .await
    .unwrap();
    let denied = env
        .check(CUST_SPOTIFY_ID, &sso, &[("site_id", "s1")], &[], None)
        .await;
    assert_eq!(denied.reason, Some(CheckReason::NotEntitled));
    assert_eq!(denied.levels[0].value, Some(GrantValue::Boolean(true)));
}

#[tokio::test]
async fn the_deciding_subscription_is_the_nearest_then_the_best_then_the_oldest() {
    let env = Env::new().await;
    env.site_type().await;
    let seats = env
        .feature("seats", FeatureKind::Limit(LimitCountedBy::App), None, None)
        .await;
    let starter = EntitlementEntityId::Plan(PLAN_LEETCODE_ID);
    let pro = EntitlementEntityId::Plan(PLAN_NOTION_ID);
    env.row(starter, None, spec(&seats, limit(10)))
        .await
        .unwrap();
    env.row(pro, None, spec(&seats, limit(20))).await.unwrap();

    // Only a site subscription includes it: the check must name a site of that type.
    let s1 = env.site(CUST_UBER_ID, "s1").await;
    env.site(CUST_UBER_ID, "s2").await;
    let on_s1 = env
        .subscribe(CUST_UBER_ID, PLAN_VERSION_NOTION_ID, day(1), Some(s1))
        .await;
    let required = env.check(CUST_UBER_ID, &seats, &[], &[], Some(0)).await;
    assert_eq!(required.reason, Some(CheckReason::ResourceRequired));
    let other_site = env
        .check(CUST_UBER_ID, &seats, &[("site_id", "s2")], &[], Some(0))
        .await;
    assert_eq!(other_site.reason, Some(CheckReason::NotSubscribed));
    assert_eq!(other_site.reason_type, Some(ReasonType::Denied));
    let on_site = env
        .check(CUST_UBER_ID, &seats, &[("site_id", "s1")], &[], Some(0))
        .await;
    assert_eq!(on_site.levels[0].deciding_subscription, Some(on_s1));

    // Customer subscriptions: the better value wins over the older one, and a check without a
    // resource answered by them while a site subscription also includes it carries a hint.
    let older = env
        .subscribe(CUST_UBER_ID, PLAN_VERSION_1_LEETCODE_ID, day(1), None)
        .await;
    let newer = env
        .subscribe(CUST_UBER_ID, PLAN_VERSION_NOTION_ID, day(5), None)
        .await;
    let customer_wide = env.check(CUST_UBER_ID, &seats, &[], &[], Some(0)).await;
    assert_eq!(customer_wide.levels[0].deciding_subscription, Some(newer));
    assert_eq!(customer_wide.value, Some(limit(20)));
    assert_eq!(customer_wide.hint, Some(CheckHint::ResourceOmitted));
    let site_first = env
        .check(CUST_UBER_ID, &seats, &[("site_id", "s1")], &[], Some(0))
        .await;
    assert_eq!(site_first.levels[0].deciding_subscription, Some(on_s1));
    assert_ne!(site_first.levels[0].deciding_subscription, Some(older));

    // Ties go to the oldest.
    let tie = env
        .subscribe(CUST_COMODO_ID, PLAN_VERSION_NOTION_ID, day(9), None)
        .await;
    let oldest = env
        .subscribe(CUST_COMODO_ID, PLAN_VERSION_NOTION_ID, day(2), None)
        .await;
    let tied = env.check(CUST_COMODO_ID, &seats, &[], &[], Some(0)).await;
    assert_eq!(tied.levels[0].deciding_subscription, Some(oldest));
    assert_ne!(tied.levels[0].deciding_subscription, Some(tie));
}

#[tokio::test]
async fn without_a_subscription_a_billing_cycle_limit_resets_each_calendar_month() {
    let env = Env::new().await;
    let api_calls = env
        .feature(
            "api_calls",
            FeatureKind::Limit(LimitCountedBy::Events(METRIC_BANDWIDTH)),
            None,
            Some(limit(1000)),
        )
        .await;
    let resolved = env
        .store()
        .resolve_customer_entitlements(
            &mut env.store().get_conn().await.unwrap(),
            TENANT_ID,
            CUST_COMODO_ID,
            None,
            None,
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(resolved.features[0].feature.id, api_calls.id);
    let level = &resolved.features[0].levels[0];
    assert_eq!(level.deciding_subscription, None);
    let window = level.window.as_ref().expect("an events limit has a window");
    assert_eq!(
        window.reset_period,
        meteroid_store::domain::entitlements::ResetPeriod::Calendar {
            unit: meteroid_store::domain::entitlements::PeriodUnit::Month,
            interval: 1,
        }
    );
    assert_eq!(window.start.date_naive().format("%d").to_string(), "01");
}

/// A fixed or never-ending window with no subscription to anchor on starts at the row's lineage,
/// not at the instant of the read (which would count nothing, forever).
#[tokio::test]
async fn without_an_activation_a_window_starts_at_the_row() {
    use meteroid_store::domain::entitlement_policy::{Period, Resets};
    use meteroid_store::domain::entitlements::PeriodUnit;
    let env = Env::new().await;
    let customer = EntitlementEntityId::Customer(CUST_COMODO_ID);
    for (code, resets, period) in [
        (
            "weekly_calls",
            Resets::Fixed,
            Some(Period {
                unit: PeriodUnit::Week,
                interval: 1,
            }),
        ),
        ("lifetime_calls", Resets::Never, None),
    ] {
        let feature = env
            .store()
            .create_feature(
                Actor::System,
                FeatureNew {
                    tenant_id: TENANT_ID,
                    product_id: None,
                    name: code.into(),
                    code: code.into(),
                    description: None,
                    kind: FeatureKind::Limit(LimitCountedBy::Events(METRIC_BANDWIDTH)),
                    unit: None,
                    level_type_id: None,
                    policy: Some(FeaturePolicy {
                        resets,
                        period,
                        ..FeaturePolicy::default()
                    }),
                    default: None,
                },
            )
            .await
            .unwrap();
        let row = env
            .row(customer, None, spec(&feature, limit(100)))
            .await
            .unwrap();
        let resolved = env
            .store()
            .resolve_customer_entitlements(
                &mut env.store().get_conn().await.unwrap(),
                TENANT_ID,
                CUST_COMODO_ID,
                None,
                Some(&[feature.id]),
                Utc::now() + Duration::days(1),
            )
            .await
            .unwrap();
        let window = resolved.features[0].levels[0]
            .window
            .clone()
            .expect("an events limit has a window");
        assert_eq!(window.start, row.lineage_created_at, "{code}");
    }
}

/// An edit moving `valid_from`, to now or back in time, leaves a never-ending window where it
/// started, at the lineage's creation: the limit keeps counting what it had counted, no more.
#[tokio::test]
async fn an_edit_keeps_the_window_start() {
    use meteroid_store::domain::entitlement_policy::Resets;
    let env = Env::new().await;
    let feature = env
        .store()
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: None,
                name: "edited_calls".into(),
                code: "edited_calls".into(),
                description: None,
                kind: FeatureKind::Limit(LimitCountedBy::Events(METRIC_BANDWIDTH)),
                unit: None,
                level_type_id: None,
                policy: Some(FeaturePolicy {
                    resets: Resets::Never,
                    ..FeaturePolicy::default()
                }),
                default: None,
            },
        )
        .await
        .unwrap();
    let window_start = || {
        let (env, feature_id) = (&env, feature.id);
        async move {
            let resolved = env
                .store()
                .resolve_customer_entitlements(
                    &mut env.store().get_conn().await.unwrap(),
                    TENANT_ID,
                    CUST_COMODO_ID,
                    None,
                    Some(&[feature_id]),
                    Utc::now() + Duration::minutes(1),
                )
                .await
                .unwrap();
            resolved.features[0].levels[0]
                .window
                .clone()
                .expect("an events limit has a window")
                .start
        }
    };
    let row = env
        .row(
            EntitlementEntityId::Customer(CUST_COMODO_ID),
            None,
            spec(&feature, limit(100)),
        )
        .await
        .unwrap();
    assert_eq!(window_start().await, row.lineage_created_at);

    env.store()
        .update_entitlement(
            &Actor::System,
            row.id,
            TENANT_ID,
            EntitlementUpdate {
                value: Some(limit(200)),
                valid_from: Some(Some(Utc::now())),
                ..EntitlementUpdate::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(window_start().await, row.lineage_created_at);

    env.store()
        .update_entitlement(
            &Actor::System,
            row.id,
            TENANT_ID,
            EntitlementUpdate {
                valid_from: Some(Some(Utc::now() - Duration::days(30))),
                ..EntitlementUpdate::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        window_start().await,
        row.lineage_created_at,
        "a backdated edit does not reach back either"
    );
}

/// Imported data keeps its business dates: a window without an activation starts at the deciding
/// subscription's start date, or at the deciding row's `valid_from`, not when the import ran.
#[tokio::test]
async fn without_an_activation_a_window_starts_at_the_business_dates() {
    use meteroid_store::domain::entitlement_policy::Resets;
    let env = Env::new().await;
    let lifetime = |code: &str| FeatureNew {
        tenant_id: TENANT_ID,
        product_id: None,
        name: code.into(),
        code: code.into(),
        description: None,
        kind: FeatureKind::Limit(LimitCountedBy::Events(METRIC_BANDWIDTH)),
        unit: None,
        level_type_id: None,
        policy: Some(FeaturePolicy {
            resets: Resets::Never,
            ..FeaturePolicy::default()
        }),
        default: None,
    };
    let window_start = |feature: Feature| {
        let env = &env;
        async move {
            let resolved = env
                .store()
                .resolve_customer_entitlements(
                    &mut env.store().get_conn().await.unwrap(),
                    TENANT_ID,
                    CUST_COMODO_ID,
                    None,
                    Some(&[feature.id]),
                    Utc::now(),
                )
                .await
                .unwrap();
            let level = resolved.features[0].levels[0].clone();
            let window = level.window.expect("an events limit has a window");
            (level.deciding_subscription, window.start)
        }
    };

    let overridden = env
        .store()
        .create_feature(Actor::System, lifetime("imported_override_calls"))
        .await
        .unwrap();
    let valid_from =
        chrono::DateTime::from_timestamp(Utc::now().timestamp() - 400 * 86_400, 0).unwrap();
    let mut backdated = spec(&overridden, limit(100));
    backdated.valid_from = Some(valid_from);
    env.row(
        EntitlementEntityId::Customer(CUST_COMODO_ID),
        None,
        backdated,
    )
    .await
    .unwrap();
    assert_eq!(window_start(overridden).await, (None, valid_from));

    let planned = env
        .store()
        .create_feature(Actor::System, lifetime("imported_plan_calls"))
        .await
        .unwrap();
    env.row(
        EntitlementEntityId::Plan(PLAN_LEETCODE_ID),
        None,
        spec(&planned, limit(100)),
    )
    .await
    .unwrap();
    let imported = env
        .subscribe(CUST_COMODO_ID, PLAN_VERSION_1_LEETCODE_ID, day(5), None)
        .await;
    // A checkout trial: the only live status without an activation, which `ACTIVE` stamps.
    diesel::sql_query(
        "UPDATE subscription SET activated_at = NULL, status = 'TRIAL_ACTIVE',
             activation_condition = 'ON_CHECKOUT' WHERE id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(imported.as_uuid())
    .execute(&mut env.store().get_conn().await.unwrap())
    .await
    .unwrap();
    assert_eq!(
        window_start(planned).await,
        (
            Some(imported),
            day(5).and_hms_opt(0, 0, 0).unwrap().and_utc()
        )
    );
}

#[tokio::test]
async fn write_guardrails() {
    let env = Env::new().await;
    let site = env.site_type().await;

    let per_site_events = env
        .store()
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: None,
                name: "api_calls".into(),
                code: "api_calls".into(),
                description: None,
                kind: FeatureKind::Limit(LimitCountedBy::Events(METRIC_BANDWIDTH)),
                unit: None,
                level_type_id: Some(site.id),
                policy: None,
                default: None,
            },
        )
        .await;
    assert!(invalid(per_site_events).contains("attributed"));

    let projects = env
        .feature(
            "projects",
            FeatureKind::Limit(LimitCountedBy::App),
            Some(&site),
            None,
        )
        .await;
    PlanRow::set_sold_for_resource_type(
        &mut env.store().get_conn().await.unwrap(),
        TENANT_ID,
        PLAN_LEETCODE_ID,
        Some(site.id),
    )
    .await
    .unwrap();
    let across_all_sites = env
        .row(
            EntitlementEntityId::Plan(PLAN_LEETCODE_ID),
            None,
            EntitlementSpec {
                level: RowLevel::Customer,
                ..spec(&projects, limit(50))
            },
        )
        .await;
    assert!(invalid(across_all_sites).contains("sold for"));

    env.store()
        .update_resource_type(
            TENANT_ID,
            AliasOr::Alias("site".into()),
            meteroid_store::repositories::resources::ResourceTypePatch {
                archived: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let archived = env
        .row(
            EntitlementEntityId::Customer(CUST_UBER_ID),
            None,
            spec(&projects, limit(3)),
        )
        .await
        .expect_err("an archived type takes no new rows");
    assert_eq!(
        ResourceRefusal::of(&archived),
        Some(ResourceRefusal::TypeArchived)
    );
}

#[tokio::test]
async fn the_etag_follows_values_and_validity() {
    let env = Env::new().await;
    let sso = env
        .feature(
            "sso",
            FeatureKind::Boolean,
            None,
            Some(GrantValue::Boolean(true)),
        )
        .await;
    let store = env.store();
    let snapshot = |if_none_match: Option<String>| async move {
        store
            .entitlement_snapshot(
                TENANT_ID,
                CUST_UBER_ID,
                &ResourceInput::default(),
                if_none_match.as_deref(),
            )
            .await
            .unwrap()
    };
    let etag = |read: SnapshotRead| match read {
        SnapshotRead::Modified(snapshot) => snapshot.etag,
        SnapshotRead::NotModified { etag } => panic!("unexpected 304 for {etag}"),
    };
    let first = etag(snapshot(None).await);
    assert_eq!(
        first,
        etag(snapshot(None).await),
        "equal values, equal etags"
    );
    assert!(matches!(
        snapshot(Some(format!("\"{first}\""))).await,
        SnapshotRead::NotModified { .. }
    ));

    let until = Utc::now() + Duration::seconds(2);
    let row = env
        .row(
            EntitlementEntityId::Customer(CUST_UBER_ID),
            None,
            EntitlementSpec {
                effect: Some(Effect::Set),
                valid_until: Some(until),
                ..spec(&sso, GrantValue::Boolean(false))
            },
        )
        .await
        .unwrap();
    let scheduled = etag(snapshot(None).await);
    assert_ne!(scheduled, first);
    let SnapshotRead::Modified(read) = snapshot(None).await else {
        panic!("modified");
    };
    assert_eq!(
        read.features[0].levels[0].value,
        LevelValue::Value(GrantValue::Boolean(false))
    );

    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    let expired = etag(snapshot(None).await);
    assert_ne!(expired, scheduled, "a passed valid_until changes the etag");
    assert_eq!(expired, first, "the values are the default's again");

    // The history stays readable for a later `as_of` (§3.12): the row was never revoked.
    let history = env
        .store()
        .get_entitlement_history(row.id, TENANT_ID)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].valid_until, row.valid_until);
}

/// A negative `requested` would lower `used + requested` under any ceiling: refused before any
/// read, on DENY and DENY_NEW alike; zero asks for nothing and is answered.
#[tokio::test]
async fn test_a_full_entitlement_refuses_and_negative_requests_are_invalid() {
    let env = Env::new().await;
    let calls = env
        .feature(
            "calls",
            FeatureKind::Limit(LimitCountedBy::Events(METRIC_BANDWIDTH)),
            None,
            Some(limit(0)),
        )
        .await;
    let seats = env
        .store()
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: None,
                name: "seats".into(),
                code: "seats".into(),
                description: None,
                kind: FeatureKind::Limit(LimitCountedBy::App),
                unit: None,
                level_type_id: None,
                policy: Some(FeaturePolicy {
                    at_limit: AtLimit::DenyNew,
                    resets: meteroid_store::domain::entitlement_policy::Resets::Never,
                    ..FeaturePolicy::default()
                }),
                default: Some(limit(2)),
            },
        )
        .await
        .unwrap();
    let check = |feature: &Feature, requested: i64, current_count: Option<i64>| {
        env.store().check_entitlement(
            TENANT_ID,
            CUST_UBER_ID,
            AliasOr::Alias(feature.code.clone()),
            CheckRequest {
                requested: Some(Decimal::from(requested)),
                current_count: current_count.map(Decimal::from),
                ..CheckRequest::default()
            },
        )
    };
    for (feature, count) in [(&calls, None), (&seats, Some(2))] {
        assert!(invalid(check(feature, -1, count).await).contains("negative"));
        assert!(
            check(feature, 0, count).await.unwrap().allowed,
            "{}",
            feature.code
        );
        let full = check(feature, 1, count).await.unwrap();
        assert!(!full.allowed, "{}", feature.code);
        assert_eq!(full.reason, Some(CheckReason::LimitReached));
    }
}

/// ADR 0011 §3.3: a per-site feature held only through a site override still needs a site.
#[tokio::test]
async fn an_override_only_own_level_requires_a_resource() {
    let env = Env::new().await;
    let site = env.site_type().await;
    let s1 = env.site(CUST_UBER_ID, "s1").await;
    let projects = env
        .feature(
            "projects",
            FeatureKind::Limit(LimitCountedBy::App),
            Some(&site),
            None,
        )
        .await;
    env.row(
        EntitlementEntityId::Customer(CUST_UBER_ID),
        Some(s1),
        spec(&projects, limit(3)),
    )
    .await
    .unwrap();
    let without_site = env.check(CUST_UBER_ID, &projects, &[], &[], Some(0)).await;
    assert_eq!(without_site.reason, Some(CheckReason::ResourceRequired));
    let on_s1 = env
        .check(CUST_UBER_ID, &projects, &[("site_id", "s1")], &[], Some(2))
        .await;
    assert!(on_s1.allowed, "{on_s1:?}");
}

/// A legacy row whose value doesn't fit the feature's kind grants nothing, as in v1.
#[tokio::test]
async fn a_mismatched_legacy_row_grants_nothing() {
    let env = Env::new().await;
    let seats = env
        .feature("seats", FeatureKind::Limit(LimitCountedBy::App), None, None)
        .await;
    let row = env
        .row(
            EntitlementEntityId::Customer(CUST_UBER_ID),
            None,
            spec(&seats, limit(3)),
        )
        .await
        .unwrap();
    diesel::sql_query(
        "UPDATE entitlement SET value = '{\"type\": \"boolean\", \"enabled\": true}' WHERE id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(row.id.as_uuid())
    .execute(&mut env.store().get_conn().await.unwrap())
    .await
    .unwrap();
    let checked = env.check(CUST_UBER_ID, &seats, &[], &[], Some(0)).await;
    assert!(!checked.allowed, "{checked:?}");
}

/// A feature's rows and app counts were written for its level: while it has any, the level stays.
#[tokio::test]
async fn a_feature_with_rows_or_counts_keeps_its_level() {
    let env = Env::new().await;
    let site = env.site_type().await;
    let update = || meteroid_store::domain::entitlements::FeatureUpdate {
        level_type_id: Some(None),
        ..Default::default()
    };
    let with_row = env
        .feature(
            "with_row",
            FeatureKind::Limit(LimitCountedBy::App),
            Some(&site),
            Some(limit(5)),
        )
        .await;
    let refused = env
        .store()
        .update_feature(with_row.id, TENANT_ID, update())
        .await;
    assert!(invalid(refused).contains("rows"));

    let counted = env
        .feature(
            "counted",
            FeatureKind::Limit(LimitCountedBy::App),
            Some(&site),
            None,
        )
        .await;
    diesel::sql_query(
        "INSERT INTO entitlement_app_count (tenant_id, customer_id, feature_id, path, count)
         VALUES ($1, $2, $3, 'site=s1', 2)",
    )
    .bind::<diesel::sql_types::Uuid, _>(TENANT_ID.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(CUST_UBER_ID.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(counted.id.as_uuid())
    .execute(&mut env.store().get_conn().await.unwrap())
    .await
    .unwrap();
    let refused = env
        .store()
        .update_feature(counted.id, TENANT_ID, update())
        .await;
    assert!(invalid(refused).contains("counts"));

    let bare = env
        .feature(
            "bare",
            FeatureKind::Limit(LimitCountedBy::App),
            Some(&site),
            None,
        )
        .await;
    let moved = env
        .store()
        .update_feature(bare.id, TENANT_ID, update())
        .await
        .unwrap();
    assert_eq!(
        moved.level_type_id, None,
        "nothing written for its level yet"
    );
}

/// ADR 0011 §3.9: an active trial reads its trialing plan's rows on the plan tier, which mark
/// the trialing subscriptions when they change; its own plan's once the trial converts.
#[tokio::test]
async fn a_trial_reads_its_trialing_plans_limits() {
    let owned = Env::new().await;
    let env = &owned;
    let seats = env
        .feature("seats", FeatureKind::Limit(LimitCountedBy::App), None, None)
        .await;
    let free = EntitlementEntityId::Plan(PLAN_LEETCODE_ID);
    let pro = EntitlementEntityId::Plan(PLAN_NOTION_ID);
    env.row(free, None, spec(&seats, limit(2))).await.unwrap();
    let sub = env
        .subscribe(CUST_UBER_ID, PLAN_VERSION_1_LEETCODE_ID, day(1), None)
        .await;
    let sql = |query: &'static str| async move {
        diesel::sql_query(query)
            .bind::<diesel::sql_types::Uuid, _>(sub.as_uuid())
            .bind::<diesel::sql_types::Uuid, _>(PLAN_NOTION_ID.as_uuid())
            .bind::<diesel::sql_types::Uuid, _>(PLAN_VERSION_1_LEETCODE_ID.as_uuid())
            .execute(&mut env.store().get_conn().await.unwrap())
            .await
            .unwrap();
    };
    let value = || async {
        env.check(CUST_UBER_ID, &seats, &[], &[], Some(0))
            .await
            .value
    };

    sql("UPDATE subscription SET status = 'TRIAL_ACTIVE' WHERE id = $1 AND $2 = $2 AND $3 = $3")
        .await;
    assert_eq!(
        value().await,
        Some(limit(2)),
        "no trialing plan: its own plan"
    );

    sql(
        "INSERT INTO subscription_trial (subscription_id, tenant_id, from_date, until_date, free,
             created_at, trial_plan_id)
         SELECT id, tenant_id, start_date, start_date + 14, true, now(), $2 FROM subscription
         WHERE id = $1 AND $3 = $3",
    )
    .await;
    sql("DELETE FROM credit_recompute_mark WHERE $1 = $1 AND $2 = $2 AND $3 = $3").await;
    env.row(pro, None, spec(&seats, limit(20))).await.unwrap();
    assert_eq!(value().await, Some(limit(20)), "the trialing plan's limit");
    let reports = env
        .feature("reports", FeatureKind::Boolean, None, None)
        .await;
    env.row(pro, None, spec(&reports, GrantValue::Boolean(true)))
        .await
        .unwrap();
    let readout = env
        .store()
        .resolve_entitlements_for_entity(
            &mut env.store().get_conn().await.unwrap(),
            TENANT_ID,
            meteroid_store::repositories::entitlements::ResolveTarget::Subscription(sub),
        )
        .await
        .unwrap();
    assert!(
        readout.iter().any(|e| e.feature.code == "reports"),
        "the subscription's readout reads the trialing plan too: {readout:?}"
    );
    let marked: i64 = diesel::select(diesel::dsl::sql::<diesel::sql_types::BigInt>(
        "(SELECT count(*) FROM credit_recompute_mark)",
    ))
    .get_result(&mut env.store().get_conn().await.unwrap())
    .await
    .unwrap();
    assert!(marked > 0, "a change to the trialing plan marks its trials");

    sql("UPDATE subscription SET status = 'ACTIVE' WHERE id = $1 AND $2 = $2 AND $3 = $3").await;
    assert_eq!(value().await, Some(limit(2)), "converted: its own plan");
}

/// Rows of one key clash only when their validity overlaps, an undated one starting when created:
/// an expired grant doesn't block a new one, and 100 until a date then 200 from it line up.
#[tokio::test]
async fn rows_of_one_key_clash_only_when_their_validity_overlaps() {
    let env = Env::new().await;
    let seats = env
        .feature("seats", FeatureKind::Limit(LimitCountedBy::App), None, None)
        .await;
    let customer = EntitlementEntityId::Customer(CUST_UBER_ID);
    let now = Utc::now();
    let dated = |n: i64, from: Option<i64>, until: Option<i64>| EntitlementSpec {
        valid_from: from.map(|d| now + Duration::days(d)),
        valid_until: until.map(|d| now + Duration::days(d)),
        ..spec(&seats, limit(n))
    };

    env.row(customer, None, dated(50, Some(-10), Some(-1)))
        .await
        .unwrap();
    env.row(customer, None, dated(100, None, Some(30)))
        .await
        .expect("a grant after an expired one");
    env.row(customer, None, dated(200, Some(30), None))
        .await
        .expect("200 from the day 100 ends");
    let clash = env
        .row(customer, None, dated(300, Some(20), Some(40)))
        .await;
    assert!(
        matches!(
            clash.expect_err("overlaps both").current_context(),
            StoreError::DuplicateValue { .. }
        ),
        "an overlap is a conflict"
    );
    let now_value = env
        .check(CUST_UBER_ID, &seats, &[], &[], Some(0))
        .await
        .value;
    assert_eq!(now_value, Some(limit(100)));
}

/// Several rows on one owner are written together or not at all.
#[tokio::test]
async fn several_rows_on_one_owner_are_all_or_nothing() {
    let env = Env::new().await;
    let seats = env
        .feature("seats", FeatureKind::Limit(LimitCountedBy::App), None, None)
        .await;
    let projects = env
        .feature(
            "projects",
            FeatureKind::Limit(LimitCountedBy::App),
            None,
            None,
        )
        .await;
    let customer = EntitlementEntityId::Customer(CUST_UBER_ID);
    env.row(customer, None, spec(&projects, limit(5)))
        .await
        .unwrap();
    let refused = env
        .store()
        .create_entitlements(
            &Actor::System,
            TENANT_ID,
            customer,
            None,
            vec![spec(&seats, limit(10)), spec(&projects, limit(6))],
        )
        .await;
    assert!(refused.is_err(), "projects already has an open row");
    let rows = env
        .store()
        .list_entitlements_by_entity(customer, TENANT_ID)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "seats wasn't kept: {rows:?}");
}

/// Several trials of one customer, loaded together: each reads its own trialing plan (or its own
/// plan without one), and the best of them answers.
#[tokio::test]
async fn several_trials_each_read_their_own_trialing_plan() {
    let owned = Env::new().await;
    let env = &owned;
    let seats = env
        .feature("seats", FeatureKind::Limit(LimitCountedBy::App), None, None)
        .await;
    for (plan, n) in [
        (PLAN_LEETCODE_ID, 2),
        (PLAN_NOTION_ID, 20),
        (crate::data::ids::PLAN_STARTER_ID, 5),
        (crate::data::ids::PLAN_PRO_ID, 50),
    ] {
        env.row(
            EntitlementEntityId::Plan(plan),
            None,
            spec(&seats, limit(n)),
        )
        .await
        .unwrap();
    }
    let leetcode = env
        .subscribe(
            CUST_UBER_ID,
            crate::data::ids::PLAN_VERSION_1_LEETCODE_ID,
            day(1),
            None,
        )
        .await;
    let starter = env
        .subscribe(
            CUST_UBER_ID,
            crate::data::ids::PLAN_VERSION_STARTER_ID,
            day(1),
            None,
        )
        .await;
    let sql = |query: String| async move {
        diesel::sql_query(query)
            .execute(&mut env.store().get_conn().await.unwrap())
            .await
            .unwrap();
    };
    let value = || async {
        env.check(CUST_UBER_ID, &seats, &[], &[], Some(0))
            .await
            .value
    };
    for sub in [leetcode, starter] {
        sql(format!(
            "UPDATE subscription SET status = 'TRIAL_ACTIVE' WHERE id = '{}'",
            sub.as_uuid()
        ))
        .await;
    }
    let trial_of = |sub: SubscriptionId, plan: PlanId| {
        format!(
            "INSERT INTO subscription_trial (subscription_id, tenant_id, from_date, until_date,
                 free, created_at, trial_plan_id)
             SELECT id, tenant_id, start_date, start_date + 14, true, now(), '{}'
             FROM subscription WHERE id = '{}'",
            plan.as_uuid(),
            sub.as_uuid()
        )
    };
    sql(trial_of(leetcode, crate::data::ids::PLAN_NOTION_ID)).await;
    assert_eq!(
        value().await,
        Some(limit(20)),
        "leetcode's trial reads notion"
    );

    sql(trial_of(starter, crate::data::ids::PLAN_PRO_ID)).await;
    assert_eq!(value().await, Some(limit(50)), "starter's trial reads pro");

    sql(format!(
        "UPDATE subscription SET status = 'ACTIVE' WHERE id = '{}'",
        starter.as_uuid()
    ))
    .await;
    assert_eq!(
        value().await,
        Some(limit(20)),
        "starter converted: its own plan"
    );
    sql(format!(
        "UPDATE subscription SET status = 'ACTIVE' WHERE id = '{}'",
        leetcode.as_uuid()
    ))
    .await;
    assert_eq!(
        value().await,
        Some(limit(5)),
        "both converted: their own plans"
    );
}

/// An add-on row dated from the next period, its subscription's period not renewed past yet: the
/// subscription reads its rows on its read day, the add-on's as its components', so it grants
/// nothing yet.
#[tokio::test]
async fn an_add_on_of_the_next_period_grants_nothing_before_the_renewal() {
    use diesel::sql_types::Uuid as SqlUuid;
    #[derive(diesel::QueryableByName)]
    struct Id {
        #[diesel(sql_type = SqlUuid)]
        id: uuid::Uuid,
    }
    let env = Env::new().await;
    let today = Utc::now().date_naive();
    let beta = env
        .feature(
            "beta",
            FeatureKind::Boolean,
            None,
            Some(GrantValue::Boolean(false)),
        )
        .await;
    let sub = env
        .subscribe(
            CUST_UBER_ID,
            PLAN_VERSION_1_LEETCODE_ID,
            today - Duration::days(30),
            None,
        )
        .await;
    let mut conn = env.store().get_conn().await.unwrap();
    diesel::sql_query("UPDATE subscription SET current_period_end = current_date WHERE id = $1")
        .bind::<SqlUuid, _>(sub.as_uuid())
        .execute(&mut conn)
        .await
        .unwrap();
    let add_on: Id = diesel::sql_query(
        "INSERT INTO add_on (id, name, tenant_id, product_id, price_id, fee_structure)
         SELECT gen_random_uuid(), 'Beta pack', tenant_id, product_id, id,
                COALESCE((SELECT pr.fee_structure FROM product pr WHERE pr.id = price.product_id), (SELECT pc.fee_structure FROM price_component pc JOIN plan_component_price pcp ON pcp.plan_component_id = pc.id WHERE pcp.price_id = price.id LIMIT 1), '{\"type\": \"Rate\"}'::jsonb)
         FROM price WHERE tenant_id = $1 LIMIT 1 RETURNING id",
    )
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .get_result(&mut conn)
    .await
    .unwrap();
    diesel::sql_query(
        "INSERT INTO subscription_add_on (id, name, subscription_id, add_on_id, period,
             effective_from, created_at)
         VALUES (gen_random_uuid(), 'Beta pack', $1, $2, 'MONTHLY', current_date, now())",
    )
    .bind::<SqlUuid, _>(sub.as_uuid())
    .bind::<SqlUuid, _>(add_on.id)
    .execute(&mut conn)
    .await
    .unwrap();
    env.row(
        EntitlementEntityId::AddOn(add_on.id.into()),
        None,
        spec(&beta, GrantValue::Boolean(true)),
    )
    .await
    .unwrap();

    let answer = env.check(CUST_UBER_ID, &beta, &[], &[], None).await;
    assert!(!answer.allowed, "{answer:?}");
}

/// Resolved at another time, a subscription's components are read on that time's read day, as
/// its add-ons are: a component starting tomorrow holds its product tomorrow, not today.
#[tokio::test]
async fn components_are_read_on_the_resolutions_day() {
    let env = Env::new().await;
    let product = crate::data::ids::PRODUCT_SEATS_ID;
    let reports = env
        .store()
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: Some(product),
                name: "reports".into(),
                code: "reports".into(),
                description: None,
                kind: FeatureKind::Boolean,
                unit: None,
                level_type_id: None,
                policy: None,
                default: Some(GrantValue::Boolean(true)),
            },
        )
        .await
        .unwrap();
    let today = Utc::now().date_naive();
    let subscription = env
        .subscribe(
            CUST_UBER_ID,
            PLAN_VERSION_1_LEETCODE_ID,
            today - Duration::days(30),
            None,
        )
        .await;
    diesel::sql_query(
        "INSERT INTO subscription_component (id, name, subscription_id, product_id, period,
             effective_from)
         VALUES (gen_random_uuid(), 'Seats', $1, $2, 'MONTHLY', current_date + 1)",
    )
    .bind::<diesel::sql_types::Uuid, _>(subscription.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(product.as_uuid())
    .execute(&mut env.store().get_conn().await.unwrap())
    .await
    .unwrap();
    let granted = |now: chrono::DateTime<Utc>| {
        let env = &env;
        let reports = reports.id;
        async move {
            env.store()
                .resolve_customer_entitlements(
                    &mut env.store().get_conn().await.unwrap(),
                    TENANT_ID,
                    CUST_UBER_ID,
                    None,
                    Some(&[reports]),
                    now,
                )
                .await
                .unwrap()
                .features
                .iter()
                .any(|f| {
                    f.levels
                        .iter()
                        .any(|l| l.value == LevelValue::Value(GrantValue::Boolean(true)))
                })
        }
    };
    assert!(
        !granted(Utc::now()).await,
        "the component isn't in force today"
    );
    assert!(
        granted(Utc::now() + Duration::days(1)).await,
        "held through tomorrow's component"
    );
}

/// An ordered select whose rows need the order (a Raise to here) keeps it until they're revoked.
#[tokio::test]
async fn a_select_keeps_its_order_while_a_row_needs_it() {
    use meteroid_store::domain::entitlements::{FeatureUpdate, ValueType};
    let env = Env::new().await;
    let tier = env
        .feature(
            "support_tier",
            FeatureKind::Value(ValueType::Select {
                options: vec!["basic".into(), "priority".into()],
                ordered: true,
            }),
            None,
            None,
        )
        .await;
    let raise = env
        .row(
            EntitlementEntityId::Customer(CUST_UBER_ID),
            None,
            EntitlementSpec {
                effect: Some(Effect::RaiseTo),
                ..spec(&tier, GrantValue::Text("priority".into()))
            },
        )
        .await
        .unwrap();
    let unorder = || FeatureUpdate {
        select_ordered: Some(false),
        ..Default::default()
    };

    let refused = invalid(
        env.store()
            .update_feature(tier.id, TENANT_ID, unorder())
            .await,
    );
    assert!(refused.contains("1 row(s) need an order"), "{refused}");

    env.store()
        .delete_entitlement(&Actor::User { id: USER_ID }, raise.id, TENANT_ID)
        .await
        .unwrap();
    let unordered = env
        .store()
        .update_feature(tier.id, TENANT_ID, unorder())
        .await
        .unwrap();
    assert!(unordered.value_order().is_unordered());
}

/// A row is live from when the app wrote it, by the app's clock: here an hour behind the database's.
#[tokio::test]
async fn a_row_is_live_by_the_apps_clock() {
    let env = Env::new().await;
    common_clock::test::set(Utc::now() - Duration::hours(1));
    let beta = env.feature("beta", FeatureKind::Boolean, None, None).await;
    let row = env
        .row(
            EntitlementEntityId::Customer(CUST_UBER_ID),
            None,
            spec(&beta, GrantValue::Boolean(true)),
        )
        .await
        .unwrap();
    assert!(row.created_at <= common_clock::now());
    let answer = env.check(CUST_UBER_ID, &beta, &[], &[], None).await;
    assert!(answer.allowed, "{answer:?}");
}

/// An edit that waited on another edit of the same row finds the row edited: a 409, not a 404.
#[tokio::test]
async fn an_edit_raced_by_another_is_a_conflict() {
    use diesel_async::SimpleAsyncConnection;
    let env = Env::new().await;
    let beta = env.feature("beta", FeatureKind::Boolean, None, None).await;
    let row = env
        .row(
            EntitlementEntityId::Customer(CUST_UBER_ID),
            None,
            spec(&beta, GrantValue::Boolean(true)),
        )
        .await
        .unwrap();
    let mut first = env.store().get_conn().await.unwrap();
    first
        .batch_execute(&format!(
            "BEGIN; SELECT id FROM entitlement WHERE id = '{}' FOR UPDATE;",
            row.version_id.as_uuid()
        ))
        .await
        .unwrap();

    let store = env.store().clone();
    let second = tokio::spawn(async move {
        store
            .update_entitlement(
                &Actor::User { id: USER_ID },
                row.id,
                TENANT_ID,
                EntitlementUpdate {
                    reason: Some(Some("second".into())),
                    ..EntitlementUpdate::default()
                },
            )
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    first
        .batch_execute(&format!(
            "UPDATE entitlement SET revoked_at = now() WHERE id = '{id}';
             INSERT INTO entitlement
             SELECT (jsonb_populate_record(NULL::entitlement, to_jsonb(e)
                 || jsonb_build_object('id', gen_random_uuid(), 'revoked_at', NULL))).*
             FROM entitlement e WHERE id = '{id}';
             COMMIT;",
            id = row.version_id.as_uuid()
        ))
        .await
        .unwrap();

    let raced = second.await.unwrap().expect_err("raced");
    assert!(
        matches!(raced.current_context(), StoreError::Conflict(_)),
        "{raced:?}"
    );
}

/// A batch locks its keys in order whatever order it lists them in: waiting on its first key, it
/// holds none, so two batches never wait on each other.
#[tokio::test]
async fn a_batch_locks_its_keys_in_order() {
    use diesel::sql_types::BigInt;
    use diesel_async::SimpleAsyncConnection;
    #[derive(diesel::QueryableByName)]
    struct Locks {
        #[diesel(sql_type = BigInt)]
        held: i64,
        #[diesel(sql_type = BigInt)]
        waiting: i64,
    }
    let env = Env::new().await;
    let a = env.feature("alpha", FeatureKind::Boolean, None, None).await;
    let b = env.feature("bravo", FeatureKind::Boolean, None, None).await;
    let key = |f: &Feature| {
        format!(
            "entitlement:{}:{}:Feature:None:None:",
            f.id,
            CUST_UBER_ID.as_uuid()
        )
    };
    let (first, last) = if key(&a) < key(&b) {
        (&a, &b)
    } else {
        (&b, &a)
    };

    let mut holder = env.store().get_conn().await.unwrap();
    holder
        .batch_execute(&format!(
            "BEGIN; SELECT pg_advisory_xact_lock(hashtextextended('{}', 0));",
            key(first)
        ))
        .await
        .unwrap();
    let store = env.store().clone();
    let specs = vec![
        spec(last, GrantValue::Boolean(true)),
        spec(first, GrantValue::Boolean(true)),
    ];
    let batch = tokio::spawn(async move {
        store
            .create_entitlements(
                &Actor::User { id: USER_ID },
                TENANT_ID,
                EntitlementEntityId::Customer(CUST_UBER_ID),
                None,
                specs,
            )
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let mut observer = env.store().get_conn().await.unwrap();
    let locks: Locks = diesel::sql_query(
        "SELECT count(*) FILTER (WHERE granted) AS held, count(*) FILTER (WHERE NOT granted) AS waiting
         FROM pg_locks
         WHERE locktype = 'advisory'
           AND database = (SELECT oid FROM pg_database WHERE datname = current_database())",
    )
    .get_result(&mut observer)
    .await
    .unwrap();
    assert_eq!(
        (locks.held, locks.waiting),
        (1, 1),
        "the holder's key, the batch waiting"
    );

    holder.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(batch.await.unwrap().unwrap().len(), 2);
}
