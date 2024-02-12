use chrono::DateTime;
use meteroid::eventbus::webhook_handler::WebhookHandler;
use meteroid::eventbus::{Event, EventHandler};
use std::str::FromStr;
use testcontainers::clients::Cli;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use crate::meteroid_it::db::seed::{CUSTOMER_UBER_ID, SUBSCRIPTION_SPORTIFY_ID1, TENANT_ID};
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::users::v1::UserRole;

#[tokio::test]
async fn test_webhook_endpoint_out() {
    // Generic setup
    helpers::init::logging();
    let docker = Cli::default();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres(&docker);
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    assert_eq!(auth.user.unwrap().role, UserRole::Admin as i32);

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "a712afi5lzhk",
    );

    let events_to_listen = vec![
        api::webhooks::out::v1::WebhookEventType::CustomerCreated as i32,
        api::webhooks::out::v1::WebhookEventType::SubscriptionCreated as i32,
    ];

    // create endpoint 1
    let created = clients
        .webhooks_out
        .clone()
        .create_webhook_endpoint(api::webhooks::out::v1::CreateWebhookEndpointRequest {
            url: "https://example.com".to_string(),
            description: Some("Test".to_string()),
            events_to_listen: events_to_listen.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .endpoint
        .unwrap();

    assert_eq!(created.url.as_str(), "https://example.com");
    assert_eq!(created.description, Some("Test".to_string()));
    assert_eq!(created.events_to_listen, events_to_listen.clone());
    assert!(created.enabled);
    assert!(meteroid::webhook::Webhook::new(created.secret.as_str()).is_ok());

    // list endpoints
    let listed = clients
        .webhooks_out
        .clone()
        .list_webhook_endpoints(api::webhooks::out::v1::ListWebhookEndpointsRequest {})
        .await
        .unwrap()
        .into_inner()
        .endpoints;

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], created);
    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}

#[tokio::test]
async fn test_webhook_out_handler() {
    // Generic setup
    helpers::init::logging();
    let docker = Cli::default();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres(&docker);
    let setup = meteroid_it::container::start_meteroid(
        postgres_connection_string,
        SeedLevel::SUBSCRIPTIONS,
    )
    .await;

    let mut endpoint_server1 = mockito::Server::new();
    let endpoint_url1 = endpoint_server1.url();

    let mut endpoint_server2 = mockito::Server::new();
    let endpoint_url2 = endpoint_server2.url();

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    assert_eq!(auth.user.unwrap().role, UserRole::Admin as i32);

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "a712afi5lzhk",
    );

    // create endpoint 1
    let _ = clients
        .webhooks_out
        .clone()
        .create_webhook_endpoint(api::webhooks::out::v1::CreateWebhookEndpointRequest {
            url: endpoint_url1,
            description: Some("Test".to_string()),
            events_to_listen: vec![
                api::webhooks::out::v1::WebhookEventType::CustomerCreated as i32,
                api::webhooks::out::v1::WebhookEventType::SubscriptionCreated as i32,
            ],
        })
        .await
        .unwrap()
        .into_inner()
        .endpoint
        .unwrap();

    // create endpoint 2
    let _ = clients
        .webhooks_out
        .clone()
        .create_webhook_endpoint(api::webhooks::out::v1::CreateWebhookEndpointRequest {
            url: endpoint_url2,
            description: Some("Test".to_string()),
            events_to_listen: vec![
                api::webhooks::out::v1::WebhookEventType::CustomerCreated as i32,
                api::webhooks::out::v1::WebhookEventType::SubscriptionCreated as i32,
            ],
        })
        .await
        .unwrap()
        .into_inner()
        .endpoint
        .unwrap();

    let handler = WebhookHandler::new(
        setup.pool.clone(),
        setup.config.secrets_crypt_key.clone(),
        false,
    );

    test_webhook_subscription_created_handler(
        &mut endpoint_server1,
        &mut endpoint_server2,
        &handler,
    )
    .await;

    test_webhook_customer_created_handler(&mut endpoint_server1, &mut endpoint_server2, &handler)
        .await;

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}

async fn test_webhook_subscription_created_handler(
    endpoint_server1: &mut mockito::Server,
    endpoint_server2: &mut mockito::Server,
    handler: &WebhookHandler,
) {
    let event = Event {
        event_id: uuid::Uuid::from_str("de88623b-a85b-48a6-8720-bc656a9107c7").unwrap(),
        event_timestamp: DateTime::parse_from_rfc3339("2024-01-01T23:22:15Z")
            .unwrap()
            .to_utc(),
        event_data: meteroid::eventbus::EventData::SubscriptionCreated(
            meteroid::eventbus::TenantEventDataDetails {
                tenant_id: TENANT_ID,
                entity_id: SUBSCRIPTION_SPORTIFY_ID1,
            },
        ),
    };

    fn endpoint_mock(endpoint_server: &mut mockito::Server) -> mockito::Mock {
        endpoint_server
            .mock("POST", "/")
            .match_header("content-type", "application/json")
            .match_header("webhook-id", "de88623b-a85b-48a6-8720-bc656a9107c7")
            .match_header("webhook-timestamp", "1704151335")
            .match_header("webhook-signature", mockito::Matcher::Regex(r"v1,.*".to_string()))
            .match_body(mockito::Matcher::JsonString(
                r#"{"type":"subscription.created","timestamp":"2024-01-01T23:22:15Z","data":{"billing_day":1,"billing_end_date":null,"billing_start_date":"2023-11-04","currency":"EUR","customer_name":"Sportify","net_terms":0}}"#.to_string(),
            ))
            .with_status(201)
            .create()
    }

    let endpoint_mock1 = endpoint_mock(endpoint_server1);
    let endpoint_mock2 = endpoint_mock(endpoint_server2);

    let _ = handler.handle(event).await.unwrap();

    endpoint_mock1.assert();
    endpoint_mock1.remove();

    endpoint_mock2.assert();
    endpoint_mock2.remove();
}

async fn test_webhook_customer_created_handler(
    endpoint_server1: &mut mockito::Server,
    endpoint_server2: &mut mockito::Server,
    handler: &WebhookHandler,
) {
    let event = Event {
        event_id: uuid::Uuid::from_str("de88623b-a85b-48a6-8720-bc656a9107c8").unwrap(),
        event_timestamp: DateTime::parse_from_rfc3339("2024-02-01T23:22:15Z")
            .unwrap()
            .to_utc(),
        event_data: meteroid::eventbus::EventData::CustomerCreated(
            meteroid::eventbus::TenantEventDataDetails {
                tenant_id: TENANT_ID,
                entity_id: CUSTOMER_UBER_ID,
            },
        ),
    };

    fn endpoint_mock(endpoint_server: &mut mockito::Server) -> mockito::Mock {
        endpoint_server
            .mock("POST", "/")
            .match_header("content-type", "application/json")
            .match_header("webhook-id", "de88623b-a85b-48a6-8720-bc656a9107c8")
            .match_header("webhook-timestamp", "1706829735")
            .match_header("webhook-signature", mockito::Matcher::Regex(r"v1,.*".to_string()))
            .match_body(mockito::Matcher::JsonString(
                r#"{"type":"customer.created","timestamp":"2024-02-01T23:22:15Z","data":{"balance_value_cents":0,"email":null,"invoicing_email":null,"name":"Uber","phone":null}}"#.to_string(),
            ))
            .with_status(201)
            .create()
    }

    let endpoint_mock1 = endpoint_mock(endpoint_server1);
    let endpoint_mock2 = endpoint_mock(endpoint_server2);

    let _ = handler.handle(event).await.unwrap();

    endpoint_mock1.assert();
    endpoint_mock1.remove();

    endpoint_mock2.assert();
    endpoint_mock2.remove();
}
