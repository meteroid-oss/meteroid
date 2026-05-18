use crate::data::ids;
use crate::helpers;
use common_domain::ids::{QuoteId, SubscriptionId};
use meteroid_grpc::meteroid::api;
use meteroid_store::repositories::{EntitlementsInterface, QuotesInterface};
use std::error::Error;
use tonic::Code;

use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use meteroid_grpc::meteroid::api::quotes::v1::{QuoteStatus, RecipientDetails};
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;

struct TestContext {
    setup: MeteroidSetup,
    clients: AllClients,
}

async fn setup_test(seed_level: SeedLevel) -> Result<TestContext, Box<dyn Error>> {
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, seed_level).await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;

    let clients = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    Ok(TestContext { setup, clients })
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
                auto_advance_invoices: None,
                // Defaults to true - the quote conversion will gracefully fall back to false
                // if no payment provider is configured on the invoicing entity
                charge_automatically: None,
                invoice_memo: None,
                invoice_threshold: None,
                create_subscription_on_acceptance: None,
                add_ons: None,
                coupons: None,
                payment_methods_config: None,
                entitlements: vec![],
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
    let TestContext { setup: _, clients } = setup_test(SeedLevel::PLANS).await.unwrap();

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
    let TestContext { setup: _, clients } = setup_test(SeedLevel::PLANS).await.unwrap();

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
    let TestContext { setup, clients } = setup_test(SeedLevel::PLANS).await.unwrap();

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
    let TestContext { setup, clients } = setup_test(SeedLevel::PLANS).await.unwrap();

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
    let TestContext { setup, clients } = setup_test(SeedLevel::PLANS).await.unwrap();

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

/// Verify that inline `EntitlementSpec`s on `CreateQuote` are persisted and returned when
/// the quote detail is fetched via the store.
#[tokio::test]
async fn test_quote_with_inline_entitlements() {
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        CreateFeatureRequest, EntitlementSpec, FeatureType, entitlement_value, feature_type,
    };

    let TestContext { setup, clients } = setup_test(SeedLevel::PLANS).await.unwrap();

    // Create a boolean feature for the inline entitlement.
    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "quote-bool-feature".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    let now = chrono::offset::Local::now().date_naive();
    let plan_version_id = ids::PLAN_VERSION_NOTION_ID.as_proto();
    let customer_id = ids::CUST_SPOTIFY_ID.as_proto();
    let component_id = ids::COMP_NOTION_SEATS_ID.as_proto();

    // Create a quote with one inline boolean entitlement.
    let created_quote = clients
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
                            initial_slot_count: Some(1),
                            committed_capacity: None,
                        },
                    ],
                    ..Default::default()
                }),
                auto_advance_invoices: None,
                charge_automatically: None,
                invoice_memo: None,
                invoice_threshold: None,
                create_subscription_on_acceptance: None,
                add_ons: None,
                coupons: None,
                payment_methods_config: None,
                entitlements: vec![EntitlementSpec {
                    feature_id: feature.id.clone(),
                    value: Some(meteroid_grpc::meteroid::api::entitlements::v1::EntitlementValue {
                        value: Some(entitlement_value::Value::BooleanValue(
                            meteroid_grpc::meteroid::api::entitlements::v1::entitlement_value::BooleanValue {
                                enabled: true,
                            },
                        )),
                    }),
                }],
            }),
        }))
        .await
        .expect("Failed to create quote with entitlements")
        .into_inner()
        .quote
        .expect("Quote not returned");

    let quote_id_str = created_quote.quote.as_ref().unwrap().id.clone();
    let quote_id = QuoteId::from_proto(&quote_id_str).expect("Invalid quote ID");

    // Fetch the detailed quote via the store and verify the entitlement is present.
    let detailed_quote = setup
        .store
        .get_detailed_quote_by_id(ids::TENANT_ID, quote_id)
        .await
        .expect("Failed to fetch detailed quote");

    assert_eq!(
        detailed_quote.entitlements.len(),
        1,
        "Exactly one entitlement must be attached to the quote"
    );

    let ent = &detailed_quote.entitlements[0];
    assert_eq!(
        ent.feature_id,
        common_domain::ids::FeatureId::from_proto(&feature.id).unwrap(),
        "Entitlement must reference the expected feature"
    );

    match &ent.value {
        meteroid_store::domain::entitlements::EntitlementValue::Boolean { enabled, .. } => {
            assert!(*enabled, "Inline entitlement enabled must be true");
        }
        other => panic!("expected boolean entitlement value, got: {:?}", other),
    }
}

/// Verify that inline `EntitlementSpec`s on a quote are carried over to the resulting
/// subscription when the quote is converted via `convert_quote_to_subscription`.
#[tokio::test]
async fn test_quote_conversion_carries_entitlements() {
    use common_domain::ids::EntitlementEntityId;
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        CreateFeatureRequest, EntitlementSpec, FeatureType, entitlement_value, feature_type,
    };

    let TestContext { setup, clients } = setup_test(SeedLevel::PLANS).await.unwrap();

    // Create a boolean feature to attach as an inline entitlement on the quote.
    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "conv-bool-feature".into(),
            description: None,
            product_id: None,
            feature_type: Some(FeatureType {
                inner: Some(feature_type::Inner::Boolean(
                    feature_type::BooleanFeature {},
                )),
            }),
            entitlement: None,
        })
        .await
        .unwrap()
        .into_inner()
        .feature
        .unwrap();

    let now = chrono::offset::Local::now().date_naive();
    let plan_version_id = ids::PLAN_VERSION_NOTION_ID.as_proto();
    let customer_id = ids::CUST_SPOTIFY_ID.as_proto();
    let component_id = ids::COMP_NOTION_SEATS_ID.as_proto();

    // Create a quote with one inline boolean entitlement.
    let created_quote = clients
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
                            initial_slot_count: Some(1),
                            committed_capacity: None,
                        },
                    ],
                    ..Default::default()
                }),
                auto_advance_invoices: None,
                charge_automatically: None,
                invoice_memo: None,
                invoice_threshold: None,
                create_subscription_on_acceptance: None,
                add_ons: None,
                coupons: None,
                payment_methods_config: None,
                entitlements: vec![EntitlementSpec {
                    feature_id: feature.id.clone(),
                    value: Some(meteroid_grpc::meteroid::api::entitlements::v1::EntitlementValue {
                        value: Some(entitlement_value::Value::BooleanValue(
                            meteroid_grpc::meteroid::api::entitlements::v1::entitlement_value::BooleanValue {
                                enabled: true,
                            },
                        )),
                    }),
                }],
            }),
        }))
        .await
        .expect("Failed to create quote with entitlements")
        .into_inner()
        .quote
        .expect("Quote not returned");

    let quote_id_proto = created_quote.quote.as_ref().unwrap().id.clone();
    let quote_id = QuoteId::from_proto(&quote_id_proto).expect("Invalid quote ID");

    // Publish the quote (DRAFT -> PENDING).
    clients
        .quotes
        .clone()
        .publish_quote(tonic::Request::new(api::quotes::v1::PublishQuoteRequest {
            id: quote_id_proto.clone(),
        }))
        .await
        .expect("Failed to publish quote");

    // Accept the quote (PENDING -> ACCEPTED) via the store shortcut used by other tests.
    setup
        .store
        .accept_quote(quote_id, ids::TENANT_ID)
        .await
        .expect("Failed to accept quote");

    // Convert the accepted quote to a subscription.
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

    let subscription_proto = conversion_result
        .subscription
        .expect("Subscription should be present after conversion");
    let subscription_id =
        SubscriptionId::from_proto(&subscription_proto.id).expect("Invalid subscription ID");

    // Fetch the entitlements attached to the resulting subscription and verify the carry-over.
    let subscription_entitlements = setup
        .store
        .list_entitlements_by_entity(
            EntitlementEntityId::Subscription(subscription_id),
            ids::TENANT_ID,
        )
        .await
        .expect("Failed to list entitlements by subscription");

    assert_eq!(
        subscription_entitlements.len(),
        1,
        "Exactly one entitlement must be carried over from the quote to the subscription"
    );

    let ent = &subscription_entitlements[0];
    assert_eq!(
        ent.feature_id,
        common_domain::ids::FeatureId::from_proto(&feature.id).unwrap(),
        "Subscription entitlement must reference the same feature as the quote entitlement"
    );

    match &ent.value {
        meteroid_store::domain::entitlements::EntitlementValue::Boolean { enabled, .. } => {
            assert!(
                *enabled,
                "Subscription entitlement must carry over enabled=true from the quote"
            );
        }
        other => panic!(
            "expected boolean entitlement value on subscription, got: {:?}",
            other
        ),
    }
}
