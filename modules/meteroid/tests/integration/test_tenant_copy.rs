use crate::data::ids;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::ids::BaseId;
use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Uuid as SqlUuid};
use diesel_async::RunQueryDsl;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::tenants::v1::TenantEnvironmentEnum;
use meteroid_store::Store;

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

async fn count(store: &Store, sql: &str, tenant: uuid::Uuid) -> i64 {
    let mut conn = store.pool.get().await.unwrap();
    diesel::sql_query(sql)
        .bind::<SqlUuid, _>(tenant)
        .get_result::<Count>(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e:?}"))
        .n
}

/// Tables the seed fills, then tables it may leave empty.
const SEEDED: &[&str] = &[
    "product_family",
    "product",
    "billable_metric",
    "price",
    "plan",
    "plan_version",
    "invoicing_entity",
    "purchase_link",
];
const COPIED: &[&str] = &[
    "managed_quantity",
    "rate_card",
    "plan_version_option",
    "custom_property_definition",
    "invoice_annex_template",
    "quote_approval_rule",
    "add_on",
    "coupon",
    "feature",
    "credit_unit",
    "custom_tax",
    "payment_reminder_sequence",
    "dunning_policy",
];

/// Rows of the copy that still reference the source tenant.
const LEAKS: &[&str] = &[
    "SELECT count(*) AS n FROM purchase_link l JOIN plan p ON p.id = l.plan_id \
     WHERE l.tenant_id = $1 AND p.tenant_id <> l.tenant_id",
    "SELECT count(*) AS n FROM product p JOIN product_family f ON f.id = p.product_family_id \
     WHERE p.tenant_id = $1 AND f.tenant_id <> p.tenant_id",
    "SELECT count(*) AS n FROM plan p JOIN plan_version v ON v.id = p.active_version_id \
     WHERE p.tenant_id = $1 AND v.tenant_id <> p.tenant_id",
    "SELECT count(*) AS n FROM price_component c JOIN plan_version v ON v.id = c.plan_version_id \
     JOIN product p ON p.id = c.product_id WHERE v.tenant_id = $1 AND p.tenant_id <> v.tenant_id",
    "SELECT count(*) AS n FROM plan_component_price cp JOIN price_component c ON c.id = cp.plan_component_id \
     JOIN plan_version v ON v.id = c.plan_version_id JOIN price pr ON pr.id = cp.price_id \
     WHERE v.tenant_id = $1 AND pr.tenant_id <> v.tenant_id",
    "SELECT count(*) AS n FROM product p, billable_metric m \
     WHERE p.tenant_id = $1 AND m.tenant_id <> p.tenant_id \
       AND p.fee_structure::text LIKE '%' || m.id::text || '%'",
    "SELECT count(*) AS n FROM price_component c JOIN plan_version v ON v.id = c.plan_version_id, \
     billable_metric m WHERE v.tenant_id = $1 AND m.tenant_id <> v.tenant_id \
       AND c.legacy_fee::text LIKE '%' || m.id::text || '%'",
];

#[tokio::test]
async fn test_copy_configuration_into_a_production_tenant() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );
    let source = ids::TENANT_ID.as_uuid();
    let source_token = "S".repeat(32);
    execute(
        &setup.store,
        &format!(
            "INSERT INTO purchase_link (id, tenant_id, plan_id, name, token) \
             VALUES ('{}', '{source}', '{}', 'Buy', '{source_token}')",
            uuid::Uuid::now_v7(),
            ids::PLAN_LEETCODE_ID.as_uuid(),
        ),
    )
    .await;
    execute(
        &setup.store,
        &format!(
            "INSERT INTO dunning_policy (tenant_id, retry_days, exhausted_action) \
             VALUES ('{source}', '{{3,7}}', 'CANCEL_SUBSCRIPTION')"
        ),
    )
    .await;

    let created = clients
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: "Live".to_string(),
            environment: TenantEnvironmentEnum::Production as i32,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: 0,
            copy_from: Some(api::tenants::v1::TenantConfigurationCopy {
                tenant_id: ids::TENANT_ID.as_base62(),
                catalog: true,
                invoicing: true,
            }),
            sandbox_template: None,
        })
        .await
        .unwrap()
        .into_inner()
        .tenant
        .unwrap();
    let target = common_domain::ids::TenantId::from_proto(created.id.clone())
        .unwrap()
        .as_uuid();

    for table in SEEDED.iter().chain(COPIED) {
        let sql = format!("SELECT count(*) AS n FROM {table} WHERE tenant_id = $1");
        let copied = count(&setup.store, &sql, target).await;
        assert_eq!(copied, count(&setup.store, &sql, source).await, "{table}");
        assert!(
            copied > 0 || COPIED.contains(table),
            "{table} has nothing to copy"
        );
    }
    for table in [
        "customer",
        "subscription",
        "invoice",
        "connector",
        "api_token",
    ] {
        let sql = format!("SELECT count(*) AS n FROM {table} WHERE tenant_id = $1");
        assert_eq!(count(&setup.store, &sql, target).await, 0, "{table}");
    }
    for sql in LEAKS {
        assert_eq!(count(&setup.store, sql, target).await, 0, "{sql}");
    }
    assert_eq!(
        count(
            &setup.store,
            "SELECT count(*) AS n FROM purchase_link WHERE tenant_id = $1 \
             AND length(token) = 32 AND token NOT LIKE 'copy:%' AND token <> 'SSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSS'",
            target
        )
        .await,
        1,
        "the copy gets its own token"
    );
    assert_eq!(
        count(
            &setup.store,
            "SELECT count(*) AS n FROM invoicing_entity WHERE tenant_id = $1 AND is_default",
            target
        )
        .await,
        1
    );

    let copy_clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        &created.slug,
    );
    let plans = copy_clients
        .plans
        .clone()
        .list_plans(api::plans::v1::ListPlansRequest::default())
        .await
        .unwrap()
        .into_inner()
        .plans;
    assert!(!plans.is_empty());
}

#[tokio::test]
async fn test_copy_refused_from_another_organization_tenant_or_into_a_sandbox() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );
    let request = |tenant_id: String, environment: TenantEnvironmentEnum| {
        api::tenants::v1::CreateTenantRequest {
            name: "Copy".to_string(),
            environment: environment as i32,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: 0,
            copy_from: Some(api::tenants::v1::TenantConfigurationCopy {
                tenant_id,
                catalog: true,
                invoicing: false,
            }),
            sandbox_template: None,
        }
    };

    let unknown = common_domain::ids::TenantId::new().as_base62();
    assert!(
        clients
            .tenants
            .clone()
            .create_tenant(request(unknown, TenantEnvironmentEnum::Development))
            .await
            .is_err()
    );
    assert!(
        clients
            .tenants
            .clone()
            .create_tenant(request(
                ids::TENANT_ID.as_base62(),
                TenantEnvironmentEnum::Sandbox
            ))
            .await
            .is_err()
    );
}

async fn execute(store: &Store, sql: &str) {
    let mut conn = store.pool.get().await.unwrap();
    diesel::sql_query(sql).execute(&mut conn).await.unwrap();
}

#[tokio::test]
async fn test_promotion_in_place() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );
    let tenant = ids::TENANT_ID.as_uuid();
    let production =
        "SELECT count(*) AS n FROM tenant WHERE id = $1 AND environment = 'PRODUCTION'";
    let set_environment =
        |environment: TenantEnvironmentEnum| api::tenants::v1::UpdateTenantRequest {
            data: Some(api::tenants::v1::TenantUpdate {
                environment: Some(environment as i32),
                ..Default::default()
            }),
        };

    assert!(
        clients
            .tenants
            .clone()
            .update_tenant(set_environment(TenantEnvironmentEnum::Sandbox))
            .await
            .is_err(),
        "a tenant cannot become a sandbox"
    );
    clients
        .tenants
        .clone()
        .update_tenant(set_environment(TenantEnvironmentEnum::Production))
        .await
        .unwrap();
    assert_eq!(count(&setup.store, production, tenant).await, 1);
    clients
        .tenants
        .clone()
        .update_tenant(set_environment(TenantEnvironmentEnum::Staging))
        .await
        .unwrap();
    assert_eq!(count(&setup.store, production, tenant).await, 0);
}

#[tokio::test]
async fn test_copy_refused_while_an_audience_names_customers() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );
    execute(
        &setup.store,
        &format!(
            "INSERT INTO email_kind_audience (invoicing_entity_id, tenant_id, kind, mode, audience) \
             VALUES ('{}', '{}', 'invoice_ready', 'EXCLUDE', \
             '{{\"predicates\": [{{\"customer\": {{\"customer_ids\": [\"{}\"]}}}}]}}')",
            ids::INVOICING_ENTITY_ID.as_uuid(),
            ids::TENANT_ID.as_uuid(),
            uuid::Uuid::now_v7(),
        ),
    )
    .await;
    let copy = |invoicing: bool| api::tenants::v1::CreateTenantRequest {
        name: format!("Copy {invoicing}"),
        environment: TenantEnvironmentEnum::Development as i32,
        disable_emails: None,
        email_delivery_mode: None,
        sandbox_preset: 0,
        copy_from: Some(api::tenants::v1::TenantConfigurationCopy {
            tenant_id: ids::TENANT_ID.as_base62(),
            catalog: true,
            invoicing,
        }),
        sandbox_template: None,
    };

    let refused = clients
        .tenants
        .clone()
        .create_tenant(copy(true))
        .await
        .unwrap_err();
    assert!(
        refused.message().contains("\"invoice ready\" emails"),
        "{refused:?}"
    );
    clients
        .tenants
        .clone()
        .create_tenant(copy(false))
        .await
        .unwrap();
}

#[tokio::test]
async fn test_a_tenant_change_reaches_every_cache() {
    use meteroid_store::repositories::TenantInterface;

    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;
    let environment = || async {
        setup
            .store
            .find_tenant_by_id(ids::TENANT_ID)
            .await
            .unwrap()
            .tenant
            .environment
    };
    assert_eq!(
        environment().await,
        meteroid_store::domain::TenantEnvironmentEnum::Development
    );

    // As another replica would: no local invalidation, only the notification.
    execute(
        &setup.store,
        &format!(
            "UPDATE tenant SET environment = 'STAGING' WHERE id = '{}'",
            ids::TENANT_ID.as_uuid()
        ),
    )
    .await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while environment().await != meteroid_store::domain::TenantEnvironmentEnum::Staging {
        assert!(
            std::time::Instant::now() < deadline,
            "the cached tenant never saw the change"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
