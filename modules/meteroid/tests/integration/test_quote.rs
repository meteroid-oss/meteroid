use crate::data::ids;
use crate::helpers;
use common_domain::ids::QuoteId;
use meteroid_grpc::meteroid::api;
use meteroid_store::repositories::QuotesInterface;
use std::error::Error;
use testcontainers::{ContainerAsync, GenericImage};
use tonic::Code;

use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use meteroid_grpc::meteroid::api::quotes::v1::{QuoteStatus, RecipientDetails};
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;

struct TestContext {
    setup: MeteroidSetup,
    clients: AllClients,
    _container: ContainerAsync<GenericImage>,
}

async fn setup_test(seed_level: SeedLevel) -> Result<TestContext, Box<dyn Error>> {
    helpers::init::logging();
    let (_container, postgres_connection_string) = meteroid_it::container::start_postgres().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, seed_level).await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;

    let clients = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    Ok(TestContext {
        setup,
        clients,
        _container,
    })
}

/// Helper to create a quote for testing
async fn create_test_quote(clients: &AllClients) -> api::quotes::v1::DetailedQuote {
    let now = chrono::offset::Local::now().date_naive();

    // Use IDs from the seeded data, converted to proto format (base62)
    let plan_version_id = ids::PLAN_VERSION_NOTION_ID.as_proto();
    let customer_id = ids::CUST_SPOTIFY_ID.as_proto();
    let component_id = ids::COMP_NOTION_SEATS_ID.as_proto();

    clients
        .quotes
        .clone()
        .create_quote(tonic::Request::new(api::quotes::v1::CreateQuoteRequest {
            quote: Some(api::quotes::v1::CreateQuote {
                plan_version_id,
                customer_id,
                currency: "EUR".to_string(),
                quote_number: None,
                trial_duration: None,
                start_date: Some(now.to_string()),
                billing_start_date: None,
                end_date: None,
                billing_day_anchor: Some(1),
                activation_condition: api::subscriptions::v1::ActivationCondition::OnStart.into(),
                valid_until: None,
                expires_at: None,
                internal_notes: None,
                cover_image: None,
                overview: None,
                terms_and_services: None,
                net_terms: Some(30),
                attachments: vec![],
                recipients: vec![RecipientDetails {
                    name: "Test User".to_string(),
                    email: "test@example.test".to_string(),
                }],
                components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
                    parameterized_components: vec![
                        api::subscriptions::v1::create_subscription_components::ComponentParameterization {
                            component_id,
                            billing_period: Some(BillingPeriod::Monthly.into()),
                            initial_slot_count: Some(5),
                            committed_capacity: None,
                        },
                    ],
                    ..Default::default()
                }),
                payment_strategy: None,
                auto_advance_invoices: None,
                // Defaults to true - the quote conversion will gracefully fall back to false
                // if no payment provider is configured on the invoicing entity
                charge_automatically: None,
                invoice_memo: None,
                invoice_threshold: None,
                create_subscription_on_acceptance: None,
                add_ons: None,
                coupons: None,
            }),
        }))
        .await
        .expect("Failed to create quote")
        .into_inner()
        .quote
        .expect("Quote not returned")
}

#[tokio::test]
async fn test_quote_create() {
    let TestContext {
        setup: _,
        clients,
        _container,
    } = setup_test(SeedLevel::PLANS).await.unwrap();

    let quote = create_test_quote(&clients).await;

    assert_eq!(quote.quote.as_ref().unwrap().status(), QuoteStatus::Draft);
    assert_eq!(
        quote.quote.as_ref().unwrap().customer_id,
        ids::CUST_SPOTIFY_ID.as_proto()
    );
    assert_eq!(
        quote.quote.as_ref().unwrap().plan_version_id,
        ids::PLAN_VERSION_NOTION_ID.as_proto()
    );
}

#[tokio::test]
async fn test_quote_conversion_requires_accepted_status() {
    let TestContext {
        setup: _,
        clients,
        _container,
    } = setup_test(SeedLevel::PLANS).await.unwrap();

    let quote = create_test_quote(&clients).await;
    let quote_id = quote.quote.as_ref().unwrap().id.clone();

    // Attempting to convert a draft quote should fail
    let result = clients
        .quotes
        .clone()
        .convert_quote_to_subscription(tonic::Request::new(
            api::quotes::v1::ConvertQuoteToSubscriptionRequest {
                quote_id: quote_id.clone(),
            },
        ))
        .await;

    assert!(result.is_err());
    let err = result.err().unwrap();
    // Should fail because quote is in DRAFT status, not ACCEPTED
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn test_quote_conversion_happy_path() {
    let TestContext {
        setup,
        clients,
        _container,
    } = setup_test(SeedLevel::PLANS).await.unwrap();

    // Create quote
    let quote = create_test_quote(&clients).await;
    let quote_id_proto = quote.quote.as_ref().unwrap().id.clone();

    // Publish the quote first (DRAFT -> PENDING)
    let published = clients
        .quotes
        .clone()
        .publish_quote(tonic::Request::new(api::quotes::v1::PublishQuoteRequest {
            id: quote_id_proto.clone(),
        }))
        .await
        .expect("Failed to publish quote")
        .into_inner()
        .quote
        .expect("Quote not returned");

    assert_eq!(
        published.quote.as_ref().unwrap().status(),
        QuoteStatus::Pending
    );

    // To convert, the quote must be ACCEPTED. This typically requires signatures.
    // For this test, we directly update the quote status via the store.
    // Note: In a real scenario, this would go through the signature flow.
    let quote_id = QuoteId::from_proto(&quote_id_proto).expect("Invalid quote ID");

    setup
        .store
        .accept_quote(quote_id, ids::TENANT_ID)
        .await
        .expect("Failed to accept quote");

    // Now convert the accepted quote to subscription
    let conversion_result = clients
        .quotes
        .clone()
        .convert_quote_to_subscription(tonic::Request::new(
            api::quotes::v1::ConvertQuoteToSubscriptionRequest {
                quote_id: quote_id_proto.clone(),
            },
        ))
        .await
        .expect("Failed to convert quote")
        .into_inner();

    // Verify the subscription was created
    assert!(conversion_result.subscription.is_some());
    let subscription = conversion_result.subscription.unwrap();
    assert!(!subscription.id.is_empty());

    // Verify the quote was updated with conversion info
    let updated_quote = conversion_result.quote.expect("Quote not returned");
    assert!(updated_quote.converted_to_subscription_id.is_some());
    assert_eq!(
        updated_quote.converted_to_subscription_id.unwrap(),
        subscription.id
    );
    assert!(updated_quote.converted_at.is_some());
}

#[tokio::test]
async fn test_quote_conversion_already_converted_fails() {
    let TestContext {
        setup,
        clients,
        _container,
    } = setup_test(SeedLevel::PLANS).await.unwrap();

    // Create and accept quote
    let quote = create_test_quote(&clients).await;
    let quote_id_proto = quote.quote.as_ref().unwrap().id.clone();

    // Publish the quote
    clients
        .quotes
        .clone()
        .publish_quote(tonic::Request::new(api::quotes::v1::PublishQuoteRequest {
            id: quote_id_proto.clone(),
        }))
        .await
        .expect("Failed to publish quote");

    // Accept the quote
    let quote_id = QuoteId::from_proto(&quote_id_proto).expect("Invalid quote ID");
    setup
        .store
        .accept_quote(quote_id, ids::TENANT_ID)
        .await
        .expect("Failed to accept quote");

    // First conversion should succeed
    let first_conversion = clients
        .quotes
        .clone()
        .convert_quote_to_subscription(tonic::Request::new(
            api::quotes::v1::ConvertQuoteToSubscriptionRequest {
                quote_id: quote_id_proto.clone(),
            },
        ))
        .await;

    assert!(
        first_conversion.is_ok(),
        "First conversion should succeed: {:?}",
        first_conversion.err()
    );

    // Second conversion attempt should fail (race condition protection)
    let second_conversion = clients
        .quotes
        .clone()
        .convert_quote_to_subscription(tonic::Request::new(
            api::quotes::v1::ConvertQuoteToSubscriptionRequest {
                quote_id: quote_id_proto.clone(),
            },
        ))
        .await;

    assert!(second_conversion.is_err(), "Second conversion should fail");
    let err = second_conversion.err().unwrap();
    // Should fail because quote is already converted
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(
        err.message().contains("already been converted"),
        "Error message should mention already converted: {}",
        err.message()
    );
}

#[tokio::test]
async fn test_quote_conversion_falls_back_charge_automatically_without_payment_provider() {
    let TestContext {
        setup,
        clients,
        _container,
    } = setup_test(SeedLevel::PLANS).await.unwrap();

    // Create quote with charge_automatically=true (the default)
    let quote = create_test_quote(&clients).await;
    let quote_id_proto = quote.quote.as_ref().unwrap().id.clone();

    // Verify the quote was created with charge_automatically=true
    assert!(
        quote.quote.as_ref().unwrap().charge_automatically,
        "Quote should have charge_automatically=true by default"
    );

    // Publish the quote
    clients
        .quotes
        .clone()
        .publish_quote(tonic::Request::new(api::quotes::v1::PublishQuoteRequest {
            id: quote_id_proto.clone(),
        }))
        .await
        .expect("Failed to publish quote");

    // Accept the quote
    let quote_id = QuoteId::from_proto(&quote_id_proto).expect("Invalid quote ID");
    setup
        .store
        .accept_quote(quote_id, ids::TENANT_ID)
        .await
        .expect("Failed to accept quote");

    // Convert the quote - this should succeed even though charge_automatically=true
    // and no payment provider is configured, because the conversion gracefully
    // falls back to charge_automatically=false
    let conversion_result = clients
        .quotes
        .clone()
        .convert_quote_to_subscription(tonic::Request::new(
            api::quotes::v1::ConvertQuoteToSubscriptionRequest {
                quote_id: quote_id_proto.clone(),
            },
        ))
        .await
        .expect("Quote conversion should succeed with graceful fallback");

    // Verify the subscription was created
    let subscription = conversion_result
        .into_inner()
        .subscription
        .expect("Subscription should be created");
    assert!(!subscription.id.is_empty());

    // Verify the subscription has charge_automatically=false (the fallback)
    // We need to fetch the subscription details to check this
    let subscription_details = clients
        .subscriptions
        .clone()
        .get_subscription_details(tonic::Request::new(
            api::subscriptions::v1::GetSubscriptionDetailsRequest {
                subscription_id: subscription.id.clone(),
            },
        ))
        .await
        .expect("Failed to get subscription details")
        .into_inner()
        .subscription
        .expect("Subscription details should be returned");

    assert!(
        !subscription_details.charge_automatically,
        "Subscription should have charge_automatically=false after fallback"
    );
}
