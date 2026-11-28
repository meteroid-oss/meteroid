use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::connectors::v1::connect_payment_provider_request::Credentials;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;

fn connect_mock_request() -> api::connectors::v1::ConnectPaymentProviderRequest {
    api::connectors::v1::ConnectPaymentProviderRequest {
        credentials: Some(Credentials::Mock(api::connectors::v1::MockConnector {
            alias: "mock".to_string(),
        })),
        auto_register_webhook_url: None,
    }
}

/// The test gateway: connectable on a non-production tenant
/// (the seeded one is `Development`), refused on a production tenant even with
/// the instance flag on.
#[tokio::test]
async fn test_connect_mock_is_refused_on_production_tenants() {
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

    let connected = clients
        .connectors
        .clone()
        .connect_payment_provider(connect_mock_request())
        .await
        .unwrap()
        .into_inner()
        .connector
        .unwrap();

    assert_eq!(connected.alias, "mock");
    assert_eq!(
        connected.provider,
        api::connectors::v1::ConnectorProviderEnum::Mock as i32
    );
    let capabilities = connected
        .payment_capabilities
        .expect("a payment provider advertises its capabilities");
    assert!(capabilities.supports_cards && capabilities.supports_direct_debit);

    // It is listed like any other payment provider, so it can be routed to the
    // card rail from the UI.
    let listed = clients
        .connectors
        .clone()
        .list_connectors(api::connectors::v1::ListConnectorsRequest {
            connector_type: Some(api::connectors::v1::ConnectorTypeEnum::PaymentProvider as i32),
        })
        .await
        .unwrap()
        .into_inner()
        .connectors;

    assert!(listed.iter().any(|c| c.id == connected.id));

    // The environment cannot be flipped to production underneath a connected test gateway.
    let flipped = clients
        .tenants
        .clone()
        .update_tenant(api::tenants::v1::UpdateTenantRequest {
            data: Some(api::tenants::v1::TenantUpdate {
                environment: Some(api::tenants::v1::TenantEnvironmentEnum::Production as i32),
                ..Default::default()
            }),
        })
        .await
        .expect_err("a tenant with a test gateway must stay non-production");
    assert_eq!(flipped.code(), tonic::Code::FailedPrecondition);

    let production = clients
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: "live".to_string(),
            environment: api::tenants::v1::TenantEnvironmentEnum::Production as i32,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: 0,
            copy_from: None,
        })
        .await
        .unwrap()
        .into_inner()
        .tenant
        .unwrap();

    let production_clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        production.slug.as_str(),
    );

    let refused = production_clients
        .connectors
        .clone()
        .connect_payment_provider(connect_mock_request())
        .await
        .expect_err("the mock provider must never connect to a production tenant");

    assert_eq!(refused.code(), tonic::Code::FailedPrecondition);
}

/// The reverse guard: live credentials stay on production tenants, and a production tenant
/// holding a live connection cannot be moved out of production underneath it.
#[tokio::test]
async fn test_live_gateway_is_confined_to_production_tenants() {
    use common_domain::ids::{BaseId, ConnectorId, TenantId};
    use diesel_models::connectors::ConnectorRowNew;
    use diesel_models::enums::{
        ConnectorProviderEnum, ConnectorStatusEnum, ConnectorTypeEnum, ConnectorWebhookRouteEnum,
    };
    use meteroid_store::domain::connectors::ProviderAccount;
    use meteroid_store::repositories::connectors::refuse_live_gateway_outside_production;

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

    let account = |livemode: bool| -> ProviderAccount {
        serde_json::from_value(serde_json::json!({
            "livemode": livemode,
            "checked_at": "2026-11-01T00:00:00",
        }))
        .unwrap()
    };

    let development = crate::data::ids::TENANT_ID;
    let refused = refuse_live_gateway_outside_production(&setup.store, development, &account(true))
        .await
        .expect_err("a development tenant takes test credentials only");
    assert!(matches!(
        refused.current_context(),
        meteroid_store::errors::StoreError::FailedPrecondition(_)
    ));
    refuse_live_gateway_outside_production(&setup.store, development, &account(false))
        .await
        .expect("test credentials are fine outside production");

    let production = clients
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: "live".to_string(),
            environment: api::tenants::v1::TenantEnvironmentEnum::Production as i32,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: 0,
            copy_from: None,
        })
        .await
        .unwrap()
        .into_inner()
        .tenant
        .unwrap();
    let production_id = TenantId::from_proto(&production.id).unwrap();
    refuse_live_gateway_outside_production(&setup.store, production_id, &account(true))
        .await
        .expect("production takes live credentials");

    {
        let mut conn = setup.store.pool.get().await.unwrap();
        ConnectorRowNew {
            id: ConnectorId::new(),
            tenant_id: production_id,
            alias: "stripe-live".to_string(),
            connector_type: ConnectorTypeEnum::PaymentProvider,
            provider: ConnectorProviderEnum::Stripe,
            data: None,
            sensitive: None,
            status: ConnectorStatusEnum::Active,
            account: Some(serde_json::to_value(account(true)).unwrap()),
            webhook_route: Some(ConnectorWebhookRouteEnum::ConnectorId),
        }
        .insert(&mut conn)
        .await
        .unwrap();
    }

    let production_clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        production.slug.as_str(),
    );
    let demoted = production_clients
        .tenants
        .clone()
        .update_tenant(api::tenants::v1::UpdateTenantRequest {
            data: Some(api::tenants::v1::TenantUpdate {
                environment: Some(api::tenants::v1::TenantEnvironmentEnum::Staging as i32),
                ..Default::default()
            }),
        })
        .await
        .expect_err("a tenant with a live connection must stay in production");
    assert_eq!(demoted.code(), tonic::Code::FailedPrecondition);
}
