//! ADR 0011 §8, §3.8: limits counted by the app, against the store.
use crate::data::ids::{CUST_UBER_ID, TENANT_ID};
use crate::meteroid_it;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use chrono::{DateTime, Duration, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CustomerId, FeatureId, ResourceTypeId};
use diesel_async::RunQueryDsl;
use meteroid_store::domain::entitlement_fold::GrantValue;
use meteroid_store::domain::entitlements::{FeatureKind, FeatureNew, LimitCountedBy};
use meteroid_store::errors::{EntitlementRefusal, StoreError};
use meteroid_store::repositories::EntitlementsInterface;
use meteroid_store::repositories::ResourcesInterface;
use meteroid_store::repositories::resources::ResourceTypeNew;
use meteroid_store::services::entitlements::app_counts::{
    AppCountOp, AppCountResult, AppCountWrite,
};
use meteroid_store::{Store, StoreResult};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

/// `projects` is counted per site, `seats` for the customer.
struct Env {
    setup: MeteroidSetup,
    projects: FeatureId,
    seats: FeatureId,
}

impl Env {
    async fn new() -> Self {
        crate::helpers::init::logging();
        let database = meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
        let setup = meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await;
        let site = setup
            .store
            .create_resource_type(ResourceTypeNew {
                tenant_id: TENANT_ID,
                code: "site".into(),
                name: "Site".into(),
                plural_name: "Sites".into(),
                parent_type_id: None,
                keys: vec!["site_id".into(), "website_id".into()],
                attributes: BTreeMap::new(),
            })
            .await
            .unwrap();
        let projects = feature(&setup.store, "projects", LimitCountedBy::App, Some(site.id)).await;
        let seats = feature(&setup.store, "seats", LimitCountedBy::App, None).await;
        Self {
            setup,
            projects,
            seats,
        }
    }

    fn counted(&self, site: bool) -> FeatureId {
        if site { self.projects } else { self.seats }
    }

    fn store(&self) -> &Store {
        &self.setup.store
    }

    async fn write(
        &self,
        site: Option<(&str, &str)>,
        op: AppCountOp,
        at: Option<DateTime<Utc>>,
        key: &str,
    ) -> StoreResult<AppCountResult> {
        write(
            self.store(),
            self.counted(site.is_some()),
            site,
            op,
            at,
            key,
        )
        .await
    }

    /// The customer's total and, for a site, its own: `projects` for a site, else `seats`.
    async fn totals(&self, path: Option<&str>) -> (Decimal, Option<Decimal>) {
        let mut conn = self.store().get_conn().await.unwrap();
        let feature_id = self.counted(path.is_some());
        Store::app_count_totals(&mut conn, TENANT_ID, CUST_UBER_ID, feature_id, path)
            .await
            .unwrap()
    }
}

async fn feature(
    store: &Store,
    code: &str,
    counted_by: LimitCountedBy,
    level_type_id: Option<ResourceTypeId>,
) -> FeatureId {
    store
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: None,
                name: code.into(),
                code: code.into(),
                description: None,
                kind: FeatureKind::Limit(counted_by),
                unit: None,
                level_type_id,
                policy: None,
                default: None::<GrantValue>,
            },
        )
        .await
        .unwrap()
        .id
}

async fn write(
    store: &Store,
    feature_id: FeatureId,
    site: Option<(&str, &str)>,
    op: AppCountOp,
    at: Option<DateTime<Utc>>,
    key: &str,
) -> StoreResult<AppCountResult> {
    store
        .report_app_count(
            TENANT_ID,
            CUST_UBER_ID,
            feature_id,
            AppCountWrite {
                resources: site
                    .map(|(key, value)| BTreeMap::from([(key.to_owned(), value.to_owned())]))
                    .unwrap_or_default(),
                op,
                at,
                idempotency_key: key.to_owned(),
            },
        )
        .await
}

fn n(value: i64) -> Decimal {
    Decimal::from(value)
}

fn refusal<T: std::fmt::Debug>(result: StoreResult<T>) -> Option<EntitlementRefusal> {
    EntitlementRefusal::of(&result.expect_err("refused"))
}

#[tokio::test]
async fn sets_and_adds_read_back_at_once_and_a_late_add_is_ignored() {
    let env = Env::new().await;
    let t0 = Utc::now() - Duration::hours(1);
    let set = env
        .write(None, AppCountOp::Set(n(10)), Some(t0), "set")
        .await
        .unwrap();
    assert_eq!(set.count, n(10));
    assert_eq!(set.path, "");
    assert!(!set.replayed);
    assert_eq!(env.totals(None).await, (n(10), None), "read-your-writes");

    let added = env
        .write(None, AppCountOp::Add(n(2)), None, "add")
        .await
        .unwrap();
    assert_eq!(added.count, n(12));
    let late = env
        .write(
            None,
            AppCountOp::Add(n(5)),
            Some(t0 - Duration::minutes(1)),
            "late",
        )
        .await
        .unwrap();
    assert_eq!(late.count, n(12), "the set already reflects it");
    assert!(!late.applied && !late.replayed, "recorded, not a retry");
    assert!(added.applied);
    let at_the_set = env
        .write(None, AppCountOp::Add(n(1)), Some(t0), "tie")
        .await
        .unwrap();
    assert!(
        at_the_set.applied,
        "a set counts what happened before its `at`"
    );
    assert_eq!(at_the_set.count, n(13));

    let decrement = env
        .write(None, AppCountOp::Add(n(-20)), None, "drop")
        .await
        .unwrap();
    assert_eq!(decrement.count, n(-7));
    assert_eq!(
        env.totals(None).await.0,
        n(-7),
        "raw: the reader shows 0 and flags it"
    );

    let older_set = env
        .write(
            None,
            AppCountOp::Set(n(3)),
            Some(t0 - Duration::minutes(5)),
            "old-set",
        )
        .await
        .unwrap();
    assert_eq!(
        older_set.count,
        n(-7),
        "a set older than the last one is ignored"
    );
    let resent = env
        .write(None, AppCountOp::Set(n(10)), Some(t0), "same-instant")
        .await
        .unwrap();
    assert_eq!(
        resent.count,
        n(-7),
        "10 and the adds from its instant on, the one at it included"
    );
}

#[tokio::test]
async fn an_add_after_a_set_dated_ahead_of_the_server_counts() {
    let env = Env::new().await;
    let ahead = Utc::now() + Duration::minutes(3);
    env.write(None, AppCountOp::Set(n(10)), Some(ahead), "set")
        .await
        .unwrap();
    let added = env
        .write(None, AppCountOp::Add(n(1)), None, "add")
        .await
        .unwrap();
    assert!(added.applied);
    assert_eq!(added.count, n(11));
    assert_eq!(env.totals(None).await.0, n(11));
}

#[tokio::test]
async fn a_retried_or_resent_set_keeps_the_adds_made_since() {
    let env = Env::new().await;
    let t0 = Utc::now() - Duration::minutes(10);
    let t1 = t0 + Duration::minutes(1);
    let site = Some(("site_id", "s1"));
    env.write(site, AppCountOp::Set(n(10)), Some(t0), "B")
        .await
        .unwrap();
    let added = env
        .write(site, AppCountOp::Add(n(1)), Some(t1), "A")
        .await
        .unwrap();
    assert_eq!(added.count, n(11));

    let retried = env
        .write(site, AppCountOp::Set(n(10)), Some(t0), "B")
        .await
        .unwrap();
    assert!(retried.replayed && !retried.applied);
    assert_eq!(retried.count, n(10), "the stored result, nothing applied");
    assert_eq!(env.totals(Some("site=s1")).await.1, Some(n(11)));

    let resent = env
        .write(site, AppCountOp::Set(n(10)), Some(t0), "C")
        .await
        .unwrap();
    assert!(!resent.replayed);
    assert_eq!(resent.count, n(11), "the add after its `at` is kept");
    assert_eq!(env.totals(Some("site=s1")).await.1, Some(n(11)));
}

#[tokio::test]
async fn a_reused_key_with_another_body_and_a_set_past_retention_are_conflicts() {
    let env = Env::new().await;
    env.write(None, AppCountOp::Add(n(1)), None, "k")
        .await
        .unwrap();
    let retry = env
        .write(None, AppCountOp::Add(n(1)), None, "k")
        .await
        .unwrap();
    assert!(
        retry.replayed,
        "the receive time is not part of the request"
    );
    assert_eq!(env.totals(None).await.0, n(1));

    env.write(Some(("site_id", "s1")), AppCountOp::Add(n(1)), None, "p")
        .await
        .unwrap();
    for other in [
        env.write(None, AppCountOp::Add(n(2)), None, "k").await,
        env.write(None, AppCountOp::Set(n(1)), None, "k").await,
        env.write(Some(("site_id", "s2")), AppCountOp::Add(n(1)), None, "p")
            .await,
    ] {
        assert_eq!(
            refusal(other),
            Some(EntitlementRefusal::IdempotencyKeyReused)
        );
    }

    let stale = env
        .write(
            None,
            AppCountOp::Set(n(4)),
            Some(Utc::now() - Duration::days(8)),
            "stale",
        )
        .await;
    let error = stale.expect_err("older than the 7 days of op log");
    assert!(matches!(error.current_context(), StoreError::Conflict(_)));
    assert_eq!(
        EntitlementRefusal::of(&error),
        Some(EntitlementRefusal::StaleSet)
    );
    assert_eq!(env.totals(None).await.0, n(1));
}

#[tokio::test]
async fn paths_match_whole_and_an_alias_writes_the_canonical_row() {
    let env = Env::new().await;
    env.write(Some(("site_id", "s1")), AppCountOp::Add(n(1)), None, "1")
        .await
        .unwrap();
    env.write(Some(("site_id", "s10")), AppCountOp::Add(n(10)), None, "10")
        .await
        .unwrap();
    let by_alias = env
        .write(
            Some(("website_id", "s1")),
            AppCountOp::Add(n(2)),
            None,
            "alias",
        )
        .await
        .unwrap();
    assert_eq!(by_alias.path, "site=s1");
    assert_eq!(by_alias.count, n(3));

    assert_eq!(env.totals(Some("site=s1")).await, (n(13), Some(n(3))));
    assert_eq!(env.totals(Some("site=s10")).await.1, Some(n(10)));
    assert_eq!(env.totals(Some("site=s2")).await.1, Some(Decimal::ZERO));

    let odd = env
        .write(
            Some(("site_id", "a/b=c%")),
            AppCountOp::Set(n(7)),
            None,
            "odd",
        )
        .await
        .unwrap();
    assert_eq!(odd.path, "site=a%2Fb%3Dc%25");
    assert_eq!(env.totals(Some(&odd.path)).await.1, Some(n(7)));
}

#[tokio::test]
async fn concurrent_adds_on_one_path_sum_exactly() {
    let env = Env::new().await;
    let writes = (0..100).map(|i| {
        let (store, feature_id) = (env.store().clone(), env.projects);
        tokio::spawn(async move {
            write(
                &store,
                feature_id,
                Some(("site_id", "s1")),
                AppCountOp::Add(Decimal::ONE),
                None,
                &format!("add-{i}"),
            )
            .await
            .map(|result| result.replayed)
        })
    });
    for write in futures::future::join_all(writes).await {
        assert!(!write.unwrap().unwrap());
    }
    assert_eq!(env.totals(Some("site=s1")).await, (n(100), Some(n(100))));
}

#[tokio::test]
async fn only_a_limit_counted_by_the_app_takes_counts() {
    let env = Env::new().await;
    let api_calls = feature(
        env.store(),
        "api-calls",
        LimitCountedBy::Events(crate::data::ids::METRIC_BANDWIDTH),
        None,
    )
    .await;
    let refused = write(
        env.store(),
        api_calls,
        None,
        AppCountOp::Add(n(1)),
        None,
        "k",
    )
    .await;
    assert!(matches!(
        refused.expect_err("counted from events").current_context(),
        StoreError::InvalidArgument(_)
    ));
    let ahead = Some(Utc::now() + Duration::minutes(10));
    for refused in [
        env.write(None, AppCountOp::Set(n(-1)), None, "negative")
            .await,
        env.write(None, AppCountOp::Add(n(1)), None, " ").await,
        env.write(None, AppCountOp::Add(n(1)), ahead, "ahead").await,
        write(
            env.store(),
            env.seats,
            Some(("site_id", "s1")),
            AppCountOp::Add(n(1)),
            None,
            "seat-on-a-site",
        )
        .await,
    ] {
        assert!(matches!(
            refused.expect_err("invalid").current_context(),
            StoreError::InvalidArgument(_)
        ));
    }
    let without_a_site = write(
        env.store(),
        env.projects,
        None,
        AppCountOp::Add(n(1)),
        None,
        "project-without-a-site",
    )
    .await;
    assert_eq!(
        refusal(without_a_site),
        Some(EntitlementRefusal::ResourceRequired)
    );
    let elsewhere = env
        .store()
        .report_app_count(
            TENANT_ID,
            CustomerId::new(),
            env.seats,
            AppCountWrite {
                resources: BTreeMap::new(),
                op: AppCountOp::Add(n(1)),
                at: None,
                idempotency_key: "elsewhere".into(),
            },
        )
        .await;
    assert!(matches!(
        elsewhere.expect_err("not the tenant's").current_context(),
        StoreError::ValueNotFound(_)
    ));
}

#[tokio::test]
async fn ops_past_retention_are_purged() {
    let env = Env::new().await;
    env.write(
        None,
        AppCountOp::Add(n(1)),
        Some(Utc::now() - Duration::days(9)),
        "old",
    )
    .await
    .unwrap();
    env.write(None, AppCountOp::Add(n(1)), None, "recent")
        .await
        .unwrap();
    assert_eq!(
        env.store().purge_app_count_ops(Utc::now()).await.unwrap(),
        0
    );

    diesel::sql_query(
        "UPDATE entitlement_app_count_op SET created_at = now() - interval '9 days'
         WHERE idempotency_key = 'old'",
    )
    .execute(&mut env.store().get_conn().await.unwrap())
    .await
    .unwrap();
    assert_eq!(
        env.store().purge_app_count_ops(Utc::now()).await.unwrap(),
        1
    );
    assert_eq!(env.totals(None).await.0, n(2), "the counts stay");
}

async fn backdate_creation(env: &Env, key: &str) {
    diesel::sql_query(
        "UPDATE entitlement_app_count_op SET created_at = at WHERE idempotency_key = $1",
    )
    .bind::<diesel::sql_types::Text, _>(key)
    .execute(&mut env.store().get_conn().await.unwrap())
    .await
    .unwrap();
}

async fn set_retention(env: &Env, days: i32) {
    diesel::sql_query("UPDATE tenant SET entitlement_op_retention_days = $1 WHERE id = $2")
        .bind::<diesel::sql_types::Integer, _>(days)
        .bind::<diesel::sql_types::Uuid, _>(TENANT_ID.as_uuid())
        .execute(&mut env.store().get_conn().await.unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_set_older_than_the_purged_horizon_stays_stale_whatever_the_retention() {
    let env = Env::new().await;
    let days_ago = |days| Some(Utc::now() - Duration::days(days));
    env.write(None, AppCountOp::Add(n(3)), days_ago(20), "purged-add")
        .await
        .unwrap();
    backdate_creation(&env, "purged-add").await;
    let past_retention = Some(Utc::now() - Duration::hours(7 * 24 + 12));
    env.write(
        None,
        AppCountOp::Add(n(1)),
        past_retention,
        "add-at-the-edge",
    )
    .await
    .unwrap();
    backdate_creation(&env, "add-at-the-edge").await;
    assert_eq!(
        env.store().purge_app_count_ops(Utc::now()).await.unwrap(),
        1,
        "a day past the retention is kept, for a set racing the purge"
    );

    set_retention(&env, 30).await;
    let after_raise = env
        .write(
            None,
            AppCountOp::Set(n(10)),
            days_ago(25),
            "set-past-the-purge",
        )
        .await;
    assert_eq!(refusal(after_raise), Some(EntitlementRefusal::StaleSet));
    let within = env
        .write(None, AppCountOp::Set(n(10)), days_ago(6), "set-within")
        .await
        .unwrap();
    assert_eq!(within.count, n(10));

    set_retention(&env, 2).await;
    backdate_creation(&env, "set-within").await;
    assert_eq!(
        env.store().purge_app_count_ops(Utc::now()).await.unwrap(),
        2
    );
    let after_lower = env
        .write(
            None,
            AppCountOp::Set(n(10)),
            days_ago(4),
            "set-past-the-shorter-retention",
        )
        .await;
    assert_eq!(refusal(after_lower), Some(EntitlementRefusal::StaleSet));
    set_retention(&env, 30).await;
    let raised_again = env
        .write(
            None,
            AppCountOp::Set(n(10)),
            days_ago(4),
            "set-after-raising-again",
        )
        .await;
    assert_eq!(refusal(raised_again), Some(EntitlementRefusal::StaleSet));
    env.write(
        None,
        AppCountOp::Set(n(4)),
        days_ago(1),
        "set-after-the-horizon",
    )
    .await
    .unwrap();
}

/// A rollback deletes the limits the app counts: the previous release reads a metered feature
/// by its metric and fails every resolution of the tenant on one without.
#[tokio::test]
async fn a_rollback_leaves_no_feature_the_previous_release_cannot_read() {
    use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
    #[derive(diesel::QueryableByName)]
    struct Count {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        n: i64,
    }
    const METERED_WITHOUT_METRIC: &str = "SELECT count(*) AS n FROM feature \
         WHERE feature_type = 'METERED' AND metric_id IS NULL";
    let env = Env::new().await;
    let mut conn = env.store().get_conn().await.unwrap();
    let before: Count = diesel::sql_query(METERED_WITHOUT_METRIC)
        .get_result(&mut conn)
        .await
        .unwrap();
    assert_eq!(before.n, 2, "projects and seats");
    conn.batch_execute("BEGIN").await.unwrap();
    for down in [
        include_str!("../../migrations/diesel/2026-11-26-011852_entitlements_v2_billing/down.sql"),
        include_str!("../../migrations/diesel/2026-11-26-011437_entitlements_v2/down.sql"),
    ] {
        conn.batch_execute(down).await.unwrap();
    }
    let after: Count = diesel::sql_query(METERED_WITHOUT_METRIC)
        .get_result(&mut conn)
        .await
        .unwrap();
    conn.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(after.n, 0);
}

/// A rollback runs with every reference to what it drops in place: a per-unit limit and the
/// app's counts on the limits it deletes, a customer row on a resource, a metric attributed to a
/// type and a plan sold for one.
#[tokio::test]
async fn a_rollback_runs_with_references_to_what_it_drops() {
    use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
    #[derive(diesel::QueryableByName)]
    struct Count {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        n: i64,
    }
    let env = Env::new().await;
    env.write(
        Some(("site_id", "s1")),
        AppCountOp::Set(n(3)),
        None,
        "before-the-rollback",
    )
    .await
    .unwrap();
    let mut conn = env.store().get_conn().await.unwrap();
    conn.batch_execute("BEGIN").await.unwrap();
    conn.batch_execute(&format!(
        "INSERT INTO managed_quantity (id, tenant_id, code, name)
         VALUES ('{quantity}', '{tenant}', 'rollback_seats', 'Seats');
         INSERT INTO feature_quantity_limit (feature_id, managed_quantity_id)
         VALUES ('{seats}', '{quantity}');
         INSERT INTO entitlement (id, tenant_id, feature_id, entity_id, entity_type, mode, value,
                                  lineage_id, lineage_created_at, lineage_anchor, effect, resource_id)
         SELECT i.id, '{tenant}', '{projects}', '{customer}', 'CUSTOMER', 'OVERRIDE',
                '{{\"type\": \"metered\", \"limit\": \"5\"}}', i.id, now(), now(), 'SET', r.id
         FROM (SELECT gen_random_uuid() AS id) i,
              resource r
         WHERE r.tenant_id = '{tenant}' AND r.key_value = 's1';
         UPDATE billable_metric SET attributed_to_type_id = t.id
         FROM resource_type t WHERE t.tenant_id = '{tenant}' AND t.code = 'site'
           AND billable_metric.tenant_id = '{tenant}';
         UPDATE plan SET sold_for_resource_type_id = t.id
         FROM resource_type t WHERE t.tenant_id = '{tenant}' AND t.code = 'site'
           AND plan.tenant_id = '{tenant}';",
        quantity = uuid::Uuid::now_v7(),
        tenant = TENANT_ID.as_uuid(),
        seats = env.seats.as_uuid(),
        projects = env.projects.as_uuid(),
        customer = CUST_UBER_ID.as_uuid(),
    ))
    .await
    .unwrap();
    for down in [
        include_str!(
            "../../migrations/diesel/2026-11-26-012219_entitlement_feature_owner/down.sql"
        ),
        include_str!("../../migrations/diesel/2026-11-26-011852_entitlements_v2_billing/down.sql"),
        include_str!("../../migrations/diesel/2026-11-26-011437_entitlements_v2/down.sql"),
    ] {
        conn.batch_execute(down).await.expect("the rollback");
    }
    let limits: Count = diesel::sql_query("SELECT count(*) AS n FROM feature_quantity_limit")
        .get_result(&mut conn)
        .await
        .unwrap();
    conn.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(limits.n, 0, "with the limit it counted per unit of");
}

/// A v1 threshold that isn't a whole number stops the conversion on its row, named.
#[tokio::test]
async fn a_v1_term_that_is_not_a_whole_number_stops_the_migration_on_its_row() {
    use diesel_async::SimpleAsyncConnection;
    let env = Env::new().await;
    let mut conn = env.store().get_conn().await.unwrap();
    conn.batch_execute("BEGIN").await.unwrap();
    for down in [
        include_str!("../../migrations/diesel/2026-11-26-011852_entitlements_v2_billing/down.sql"),
        include_str!("../../migrations/diesel/2026-11-26-011437_entitlements_v2/down.sql"),
    ] {
        conn.batch_execute(down).await.unwrap();
    }
    let feature = uuid::Uuid::now_v7();
    let row = uuid::Uuid::now_v7();
    conn.batch_execute(&format!(
        "INSERT INTO feature (id, tenant_id, name, code, feature_type)
         VALUES ('{feature}', '{tenant}', 'calls', 'calls_v1', 'METERED');
         INSERT INTO entitlement (id, tenant_id, feature_id, entity_id, entity_type, mode, value)
         VALUES ('{row}', '{tenant}', '{feature}', '{feature}', 'FEATURE', 'OVERRIDE',
                 '{{\"type\": \"metered\", \"limit\": \"5\", \"thresholds\": [80.5]}}');",
        tenant = TENANT_ID.as_uuid(),
    ))
    .await
    .unwrap();
    let migrated = conn
        .batch_execute(include_str!(
            "../../migrations/diesel/2026-11-26-011437_entitlements_v2/up.sql"
        ))
        .await;
    conn.batch_execute("ROLLBACK").await.unwrap();
    let error = migrated.expect_err("the migration stops").to_string();
    assert!(error.contains(&row.to_string()), "{error}");
}
