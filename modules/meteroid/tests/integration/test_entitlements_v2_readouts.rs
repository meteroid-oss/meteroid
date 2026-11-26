//! ADR 0011 §3.11: a subscription's readouts show the deciding value of the level it includes a
//! feature on, and its own grant when another subscription decides (review item 16).
use crate::data::ids::{
    CUST_UBER_ID, METRIC_BANDWIDTH, PLAN_LEETCODE_ID, PLAN_NOTION_ID, PLAN_VERSION_1_LEETCODE_ID,
    PLAN_VERSION_NOTION_ID, TENANT_ID,
};
use crate::meteroid_it;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use chrono::{Days, NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    AliasOr, BaseId, EntitlementEntityId, PlanId, PlanVersionId, ResourceId, SubscriptionId,
};
use diesel_async::RunQueryDsl;
use meteroid_store::domain::entitlement_fold::{GrantValue, LimitValue};
use meteroid_store::domain::entitlements::{
    EffectiveEntitlement, EffectiveEntitlementValue, EntitlementNew, EntitlementSpec, Feature,
    FeatureKind, FeatureNew, LimitCountedBy, SubscriptionGrant,
};
use meteroid_store::domain::resources::ResourceType;
use meteroid_store::repositories::resources::{ResourceTypeNew, ResourceUpsert};
use meteroid_store::repositories::{EntitlementsInterface, ResourcesInterface};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

struct Env {
    setup: MeteroidSetup,
}

fn limit(n: i64) -> GrantValue {
    GrantValue::Limit(LimitValue::Amount(Decimal::from(n)))
}

fn days_ago(days: u64) -> NaiveDate {
    Utc::now().date_naive() - Days::new(days)
}

impl Env {
    async fn new() -> Self {
        let database = meteroid_it::container::create_test_database().await;
        Self {
            setup: meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await,
        }
    }

    async fn feature(&self, code: &str, level: Option<&ResourceType>) -> Feature {
        self.setup
            .store
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
                    level_type_id: level.map(|t| t.id),
                    policy: None,
                    default: None,
                },
            )
            .await
            .unwrap()
    }

    async fn include(&self, plan: PlanId, feature: &Feature, value: i64) {
        self.setup
            .store
            .create_entitlement(
                &Actor::System,
                EntitlementNew {
                    tenant_id: TENANT_ID,
                    entity: EntitlementEntityId::Plan(plan),
                    resource_id: None,
                    spec: EntitlementSpec::new(feature.id, limit(value)),
                },
            )
            .await
            .unwrap();
    }

    async fn sites(&self) -> ResourceType {
        let sites = self
            .setup
            .store
            .create_resource_type(ResourceTypeNew {
                tenant_id: TENANT_ID,
                code: "site".into(),
                name: "Site".into(),
                plural_name: "Sites".into(),
                parent_type_id: None,
                keys: vec!["site_id".into()],
                attributes: BTreeMap::new(),
            })
            .await
            .unwrap();
        diesel::sql_query("UPDATE billable_metric SET attributed_to_type_id = $1 WHERE id = $2")
            .bind::<diesel::sql_types::Uuid, _>(sites.id.as_uuid())
            .bind::<diesel::sql_types::Uuid, _>(METRIC_BANDWIDTH.as_uuid())
            .execute(&mut self.setup.store.get_conn().await.unwrap())
            .await
            .unwrap();
        sites
    }

    async fn site(&self, key_value: &str) -> ResourceId {
        self.setup
            .store
            .upsert_resource(ResourceUpsert {
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
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

    async fn subscribe(
        &self,
        version: PlanVersionId,
        start: NaiveDate,
        resource_id: Option<ResourceId>,
    ) -> SubscriptionId {
        use diesel::sql_types::{Date, Nullable, Uuid};
        let id = SubscriptionId::new();
        diesel::sql_query(
            "INSERT INTO subscription (id, customer_id, billing_day_anchor, tenant_id, start_date,
                 plan_version_id, created_at, net_terms, currency, mrr_cents, period,
                 pending_checkout, activation_condition, status, current_period_start,
                 current_period_end, auto_advance_invoices, charge_automatically,
                 backdate_invoices, custom_properties, tax_inclusive, lifecycle_suspended,
                 resource_id, activated_at)
             VALUES ($1, $2, 1, $3, $4, $5, now(), 0, 'EUR', 0, 'MONTHLY', false, 'ON_START',
                 'ACTIVE', $4, $4 + 30, true, false, false, '{}', false, false, $6, $4)",
        )
        .bind::<Uuid, _>(id.as_uuid())
        .bind::<Uuid, _>(CUST_UBER_ID.as_uuid())
        .bind::<Uuid, _>(TENANT_ID.as_uuid())
        .bind::<Date, _>(start)
        .bind::<Uuid, _>(version.as_uuid())
        .bind::<Nullable<Uuid>, _>(resource_id.map(|r| r.as_uuid()))
        .execute(&mut self.setup.store.get_conn().await.unwrap())
        .await
        .unwrap();
        id
    }

    async fn readout(&self, subscription_id: SubscriptionId) -> Vec<EffectiveEntitlement> {
        self.setup
            .services
            .get_effective_entitlements_for_subscription(subscription_id, TENANT_ID)
            .await
            .unwrap()
    }
}

fn limit_of(readout: &[EffectiveEntitlement], feature: &Feature) -> Option<Decimal> {
    readout
        .iter()
        .find(|e| e.feature.id == feature.id)
        .and_then(|e| match &e.value {
            EffectiveEntitlementValue::Metered { limit, .. } => *limit,
            _ => None,
        })
}

fn grant_of(readout: &[EffectiveEntitlement], feature: &Feature) -> Option<SubscriptionGrant> {
    readout
        .iter()
        .find(|e| e.feature.id == feature.id)?
        .own_grant
        .clone()
}

#[tokio::test]
async fn a_subscription_reads_the_deciding_value_and_its_own_grant() {
    let env = Env::new().await;
    let api_calls = env.feature("api_calls", None).await;
    env.include(PLAN_LEETCODE_ID, &api_calls, 1_000).await;
    env.include(PLAN_NOTION_ID, &api_calls, 10_000).await;
    let starter = env
        .subscribe(PLAN_VERSION_1_LEETCODE_ID, days_ago(20), None)
        .await;
    let pro = env
        .subscribe(PLAN_VERSION_NOTION_ID, days_ago(5), None)
        .await;

    let on_starter = env.readout(starter).await;
    assert_eq!(
        limit_of(&on_starter, &api_calls),
        Some(Decimal::from(10_000))
    );
    assert_eq!(
        grant_of(&on_starter, &api_calls),
        Some(SubscriptionGrant {
            value: Some(limit(1_000)),
            deciding_subscription_id: pro,
            deciding_plan_name: Some("Notion".into()),
            deciding_resource_path: None,
        }),
        "grants 1k; Notion on the customer decides 10k"
    );
    let on_pro = env.readout(pro).await;
    assert_eq!(limit_of(&on_pro, &api_calls), Some(Decimal::from(10_000)));
    assert_eq!(grant_of(&on_pro, &api_calls), None, "it decides");

    let usage = env
        .setup
        .services
        .get_subscription_usage(TENANT_ID, starter, Utc::now().naive_utc())
        .await
        .unwrap();
    let bar = usage
        .usage
        .iter()
        .find(|u| u.feature_id == api_calls.id)
        .expect("the summary lists it");
    assert_eq!(bar.limit, Some(Decimal::from(10_000)));
    assert_eq!(bar.period_start, Some(days_ago(5)), "the deciding cycle");
    assert_eq!(
        bar.own_grant.as_ref().map(|g| g.deciding_subscription_id),
        Some(pro)
    );

    let resolved = env
        .setup
        .store
        .subscription_entitlements(TENANT_ID, starter)
        .await
        .unwrap();
    let (_, grant) = resolved
        .iter()
        .find(|(e, _)| e.feature.id == api_calls.id)
        .unwrap();
    assert_eq!(
        grant.as_ref().map(|g| g.value.clone()),
        Some(Some(limit(1_000)))
    );
}

#[tokio::test]
async fn a_site_subscription_reads_its_site_and_a_customer_one_the_sites_without_their_own() {
    let env = Env::new().await;
    let sites = env.sites().await;
    let s1 = env.site("s1").await;
    let pages = env.feature("pages", Some(&sites)).await;
    env.include(PLAN_LEETCODE_ID, &pages, 5).await;
    env.include(PLAN_NOTION_ID, &pages, 50).await;
    let business = env
        .subscribe(PLAN_VERSION_1_LEETCODE_ID, days_ago(20), None)
        .await;
    let on_s1 = env
        .subscribe(PLAN_VERSION_NOTION_ID, days_ago(5), Some(s1))
        .await;

    let site = env.readout(on_s1).await;
    assert_eq!(limit_of(&site, &pages), Some(Decimal::from(50)));
    assert_eq!(grant_of(&site, &pages), None);
    let customer = env.readout(business).await;
    assert_eq!(
        limit_of(&customer, &pages),
        Some(Decimal::from(5)),
        "per site, on the sites it decides"
    );
    assert_eq!(grant_of(&customer, &pages), None);

    let usage = env
        .setup
        .services
        .get_subscription_usage(TENANT_ID, on_s1, Utc::now().naive_utc())
        .await
        .unwrap();
    assert_eq!(usage.usage.len(), 1);
    assert_eq!(usage.usage[0].limit, Some(Decimal::from(50)));
}

/// Many site subscriptions of one customer, their resources loaded together: each reads its own
/// site's plan.
#[tokio::test]
async fn several_site_subscriptions_each_read_their_own_site() {
    let env = Env::new().await;
    let sites = env.sites().await;
    let pages = env.feature("pages", Some(&sites)).await;
    env.include(PLAN_LEETCODE_ID, &pages, 5).await;
    env.include(PLAN_NOTION_ID, &pages, 50).await;
    let mut subscriptions = Vec::new();
    for i in 0..8 {
        let site = env.site(&format!("s{i}")).await;
        let (version, limit) = if i % 2 == 0 {
            (PLAN_VERSION_NOTION_ID, 50)
        } else {
            (PLAN_VERSION_1_LEETCODE_ID, 5)
        };
        subscriptions.push((env.subscribe(version, days_ago(5), Some(site)).await, limit));
    }
    for (subscription, limit) in subscriptions {
        assert_eq!(
            limit_of(&env.readout(subscription).await, &pages),
            Some(Decimal::from(limit))
        );
    }
}
