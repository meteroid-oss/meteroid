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
