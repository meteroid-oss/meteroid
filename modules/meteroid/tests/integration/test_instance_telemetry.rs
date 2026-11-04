//! The anonymous instance report reads the database: a stable identity and usage counted in
//! production tenants only.

use common_config::instance_telemetry::InstanceTelemetryConfig;
use diesel_async::RunQueryDsl;
use meteroid::services::instance_telemetry::InstanceTelemetryService;
use meteroid_store::repositories::instance_telemetry::InstanceTelemetryInterface;
use meteroid_store::store::PgPool;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;

async fn scalar(pool: &PgPool, sql: &str) -> i64 {
    let mut conn = pool.get().await.unwrap();
    diesel::dsl::sql::<diesel::sql_types::BigInt>(sql)
        .get_result(&mut conn)
        .await
        .unwrap()
}

async fn execute(pool: &PgPool, sql: &str) {
    let mut conn = pool.get().await.unwrap();
    diesel::sql_query(sql).execute(&mut conn).await.unwrap();
}

#[tokio::test]
async fn test_instance_telemetry_identity_and_usage() {
    helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await;
    let store = &setup.store;
    let pool = &store.pool;

    let first = store.instance_telemetry("v1.0.0").await.unwrap();
    let again = store.instance_telemetry("v2.0.0").await.unwrap();
    assert_eq!(
        first.instance_id, again.instance_id,
        "the identity is created once"
    );
    assert_eq!(again.first_version, "v1.0.0");

    let service = InstanceTelemetryService::new(
        store.clone(),
        InstanceTelemetryConfig::default(),
        "s3://key:secret@bucket/prefix".to_string(),
        vec![],
    );

    execute(pool, "UPDATE tenant SET environment = 'SANDBOX'").await;
    let sandbox_only = store
        .instance_usage(chrono::Utc::now().date_naive())
        .await
        .unwrap();
    assert_eq!(sandbox_only.tenants_live, 0);
    assert_eq!(
        sandbox_only.customers, 0,
        "sandboxes say nothing about adoption"
    );
    assert!(sandbox_only.organizations >= 1);
    assert!(sandbox_only.users >= 1);
    assert!(sandbox_only.postgres_major >= 14);

    execute(pool, "UPDATE tenant SET environment = 'PRODUCTION'").await;
    let usage = store
        .instance_usage(chrono::Utc::now().date_naive())
        .await
        .unwrap();
    let customers = scalar(
        pool,
        "SELECT count(*) FROM customer WHERE archived_at IS NULL",
    )
    .await;
    let active = scalar(
        pool,
        "SELECT count(*) FROM subscription WHERE status IN ('ACTIVE', 'TRIAL_ACTIVE')",
    )
    .await;
    assert!(customers > 0, "the seed has customers");
    assert_eq!(usage.customers, customers as u64);
    assert_eq!(usage.subscriptions_active, active as u64);
    assert!(!usage.integrations.contains(&"mock".to_string()));

    let report = service.build(&first).await.unwrap();
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["usage"]["customers"], customers);
    assert_eq!(json["infra"]["object_storage"], "s3");
    assert!(!json.to_string().contains("secret"));

    store.record_instance_telemetry_sent().await.unwrap();
    let sent = store.instance_telemetry("v1.0.0").await.unwrap();
    assert!(sent.last_sent_at.is_some());
}
