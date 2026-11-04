use crate::data::ids;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid_grpc::meteroid::api;
use meteroid_store::repositories::TenantInterface;

#[tokio::test]
async fn test_tenants_basic() {
    // Generic setup
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

    let tenant_name = "meter_me";
    let tenant_slug = "prod";
    let organization_currency = "EUR"; // organization has "FR" country so tenant is created with "EUR"

    // create tenant
    let created = clients
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: tenant_name.to_string(),
            environment: 0,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: 0,
        })
        .await
        .unwrap()
        .into_inner()
        .tenant
        .unwrap();

    assert_eq!(created.reporting_currency.as_str(), organization_currency);
    assert_eq!(created.name, tenant_name);
    assert_eq!(created.slug, tenant_slug);

    // tenant by id
    let by_id = clients
        .tenants
        .clone()
        .get_tenant_by_id(api::tenants::v1::GetTenantByIdRequest {
            tenant_id: created.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .tenant
        .unwrap();

    assert_eq!(by_id.reporting_currency.as_str(), organization_currency);
    assert_eq!(by_id.name, tenant_name);
    assert_eq!(by_id.slug, tenant_slug);

    // active tenant
    let active = clients
        .tenants
        .clone()
        .active_tenant(api::tenants::v1::ActiveTenantRequest {})
        .await
        .unwrap()
        .into_inner()
        .tenant
        .unwrap();

    assert_ne!(&active, &created);

    // list tenants
    let listed = clients
        .tenants
        .clone()
        .list_tenants(api::tenants::v1::ListTenantsRequest {})
        .await
        .unwrap()
        .into_inner()
        .tenants;

    let listed_created = listed.iter().find(|x| *x == &created);

    assert_eq!(listed.len(), 2);
    assert_eq!(listed_created, Some(created).as_ref());

    // teardown
    // meteroid_it::container::terminate_meteroid(setup.token, &setup.join_handle).await
}

#[tokio::test]
async fn test_tenant_keep_last_events_setting() {
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

    let update = |keep_last_events: Option<bool>| {
        let mut tenants = clients.tenants.clone();
        async move {
            tenants
                .update_tenant(api::tenants::v1::UpdateTenantRequest {
                    data: Some(api::tenants::v1::TenantUpdate {
                        keep_last_events,
                        // An update with no field set is rejected as an empty changeset.
                        name: Some("Keep last".to_string()),
                        ..Default::default()
                    }),
                })
                .await
                .map(|r| r.into_inner().tenant.unwrap())
        }
    };

    // No longer offered: turning it on is refused.
    let refused = update(Some(true)).await.unwrap_err();
    assert_eq!(refused.code(), tonic::Code::FailedPrecondition);
    assert!(!setup.store.keep_last_events(ids::TENANT_ID).await.unwrap());

    // A tenant that had it keeps it, and may resend it unchanged.
    setup
        .store
        .update_tenant(
            common_domain::actor::Actor::System,
            meteroid_store::domain::TenantUpdate {
                keep_last_events: Some(true),
                ..Default::default()
            },
            ids::ORGANIZATION_ID,
            ids::TENANT_ID,
        )
        .await
        .unwrap();
    assert!(update(Some(true)).await.unwrap().keep_last_events);
    assert!(
        update(None).await.unwrap().keep_last_events,
        "omitted leaves it unchanged"
    );

    // Turning it off is for good.
    assert!(!update(Some(false)).await.unwrap().keep_last_events);
    assert!(update(Some(true)).await.is_err());
    assert!(!setup.store.keep_last_events(ids::TENANT_ID).await.unwrap());
}
