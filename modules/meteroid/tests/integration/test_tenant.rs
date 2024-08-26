use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::tenants::v1::tenant_billing_configuration::{
    BillingConfigOneof, Stripe,
};
use meteroid_grpc::meteroid::api::tenants::v1::{
    ConfigureTenantBillingRequest, TenantBillingConfiguration,
};

#[tokio::test]
async fn test_tenants_basic() {
    // Generic setup
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;
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

    // configure tenant billing
    let cfg = TenantBillingConfiguration {
        billing_config_oneof: Some(BillingConfigOneof::Stripe(Stripe {
            api_secret: "api_secret".into(),
            webhook_secret: "webhook_secret".into(),
        })),
    };

    let cfg_res = clients
        .tenants
        .clone()
        .configure_tenant_billing(ConfigureTenantBillingRequest {
            billing_config: Some(cfg.clone()),
        })
        .await
        .unwrap()
        .into_inner()
        .billing_config
        .unwrap();

    assert_eq!(&cfg_res, &cfg);

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}
