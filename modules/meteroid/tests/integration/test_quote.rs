use crate::data::ids;
use crate::helpers;
use common_domain::ids::{BaseId, QuoteId, SubscriptionId};
use meteroid_grpc::meteroid::api;
use meteroid_store::domain::SubscriptionFee;
use meteroid_store::domain::customer_emails::EmailKind;
use meteroid_store::domain::enums::QuoteStatusEnum;
use meteroid_store::domain::quotes::QuoteSignatureNew;
use meteroid_store::repositories::customer_emails::CustomerEmailsInterface;
use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;
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
    try_create_test_quote(clients, None)
        .await
        .expect("Failed to create quote")
}

fn test_quote_payload(
    slot_count: u32,
    custom_properties: Option<String>,
) -> api::quotes::v1::CreateQuote {
    let now = chrono::Utc::now().date_naive();

    // Use IDs from the seeded data, converted to proto format (base62)
    let plan_version_id = ids::PLAN_VERSION_NOTION_ID.as_proto();
    let customer_id = ids::CUST_SPOTIFY_ID.as_proto();
    let component_id = ids::COMP_NOTION_SEATS_ID.as_proto();

    api::quotes::v1::CreateQuote {
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
        usage_examples: vec![],
        minimum_commitment_override: None,
        custom_properties,
        recipients: vec![RecipientDetails {
            name: "Test User".to_string(),
            email: "test@example.test".to_string(),
        }],
        components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
            parameterized_components: vec![
                api::subscriptions::v1::create_subscription_components::ComponentParameterization {
                    component_id,
                    billing_period: Some(BillingPeriod::Monthly.into()),
                    initial_slot_count: Some(slot_count),
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
    }
}

/// Creates a quote with the given JSON-encoded custom properties.
async fn try_create_test_quote(
    clients: &AllClients,
    custom_properties: Option<String>,
) -> Result<api::quotes::v1::DetailedQuote, tonic::Status> {
    clients
        .quotes
        .clone()
        .create_quote(tonic::Request::new(api::quotes::v1::CreateQuoteRequest {
            quote: Some(test_quote_payload(5, custom_properties)),
        }))
        .await
        .map(|r| r.into_inner().quote.expect("Quote not returned"))
}

#[tokio::test]
async fn test_quote_create() {
    let TestContext {
        setup: _setup,
        clients,
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

fn seat_count(quote: &api::quotes::v1::DetailedQuote) -> u32 {
    let seats_id = ids::COMP_NOTION_SEATS_ID.as_proto();
    let seats = quote
        .components
        .iter()
        .find(|c| c.price_component_id.as_deref() == Some(seats_id.as_str()))
        .expect("seats component missing");
    match seats.fee.as_ref().and_then(|f| f.fee.as_ref()) {
        Some(api::subscriptions::v1::subscription_fee::Fee::Slot(slot)) => slot.initial_slots,
        other => panic!("expected slot fee, got {other:?}"),
    }
}

#[tokio::test]
async fn test_quote_update() {
    let TestContext { setup, clients } = setup_test(SeedLevel::PLANS).await.unwrap();

    let created = create_test_quote(&clients).await;
    let created_quote = created.quote.as_ref().unwrap();
    assert_eq!(seat_count(&created), 5);

    let mut payload = test_quote_payload(10, None);
    payload.net_terms = Some(45);
    payload.overview = Some("Updated overview".to_string());
    payload.expires_at = Some("2030-12-31".to_string());

    let updated = clients
        .quotes
        .clone()
        .update_quote(tonic::Request::new(api::quotes::v1::UpdateQuoteRequest {
            id: created_quote.id.clone(),
            quote: Some(payload),
        }))
        .await
        .expect("Failed to update quote")
        .into_inner()
        .quote
        .unwrap();
    let updated_quote = updated.quote.as_ref().unwrap();

    assert_eq!(updated_quote.id, created_quote.id);
    assert_eq!(updated_quote.status(), QuoteStatus::Draft);
    assert_eq!(updated_quote.quote_number, created_quote.quote_number);
    assert_eq!(updated_quote.net_terms, 45);
    assert_eq!(updated_quote.overview.as_deref(), Some("Updated overview"));
    assert!(
        updated_quote
            .expires_at
            .as_deref()
            .is_some_and(|d| d.starts_with("2030-12-31"))
    );
    assert_eq!(updated.components.len(), created.components.len());

    assert_eq!(seat_count(&updated), 10);

    clients
        .quotes
        .clone()
        .publish_quote(tonic::Request::new(api::quotes::v1::PublishQuoteRequest {
            id: created_quote.id.clone(),
        }))
        .await
        .expect("Failed to publish quote");

    let update = |slots: u32| {
        let mut quotes = clients.quotes.clone();
        let id = created_quote.id.clone();
        async move {
            quotes
                .update_quote(tonic::Request::new(api::quotes::v1::UpdateQuoteRequest {
                    id,
                    quote: Some(test_quote_payload(slots, None)),
                }))
                .await
        }
    };

    let pending = update(3)
        .await
        .expect("A pending quote nobody signed must stay editable")
        .into_inner()
        .quote
        .unwrap();
    assert_eq!(
        pending.quote.as_ref().unwrap().status(),
        QuoteStatus::Pending
    );
    assert_eq!(seat_count(&pending), 3);

    let quote_id = QuoteId::from_proto(&created_quote.id).unwrap();
    setup
        .store
        .insert_quote_signature(
            common_domain::actor::Actor::System,
            QuoteSignatureNew {
                quote_id,
                signed_by_name: "Jane Doe".to_string(),
                signed_by_email: "jane@spotify.com".to_string(),
                signed_by_title: None,
                signature_data: None,
                signature_method: "typed".to_string(),
                ip_address: None,
                user_agent: None,
                verification_token: None,
            },
            ids::TENANT_ID,
        )
        .await
        .expect("Failed to sign quote");

    let err = update(4)
        .await
        .expect_err("Updating a quote someone signed must fail");
    assert_eq!(err.code(), Code::InvalidArgument);

    setup
        .store
        .accept_quote(
            common_domain::actor::Actor::System,
            quote_id,
            ids::TENANT_ID,
        )
        .await
        .expect("Failed to accept quote");

    let err = update(4)
        .await
        .expect_err("Updating an accepted quote must fail");
    assert_eq!(err.code(), Code::FailedPrecondition);
}

#[tokio::test]
async fn test_quote_conversion_requires_accepted_status() {
    let TestContext {
        setup: _setup,
        clients,
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
        .accept_quote(
            common_domain::actor::Actor::System,
            quote_id,
            ids::TENANT_ID,
        )
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

    let detailed = setup
        .store
        .get_detailed_quote_by_id(ids::TENANT_ID, quote_id)
        .await
        .expect("Failed to load quote");
    let quoted_seats = detailed
        .components
        .iter()
        .find_map(|c| match &c.fee {
            SubscriptionFee::Slot { initial_slots, .. } => Some(*initial_slots),
            _ => None,
        })
        .expect("Quote has no slot component");
    assert_eq!(quoted_seats, 5);

    let subscription_id = SubscriptionId::from_proto(&subscription.id).unwrap();
    let active_seats = setup
        .store
        .get_active_slots_value(ids::TENANT_ID, subscription_id, "Seats".to_string(), None)
        .await
        .expect("Failed to read active seats");
    assert_eq!(active_seats, 5);
}

#[tokio::test]
async fn test_send_quote_requires_quote_ready_email() {
    let TestContext { setup, clients } = setup_test(SeedLevel::PLANS).await.unwrap();

    let quote = create_test_quote(&clients).await;
    let quote_id_proto = quote.quote.as_ref().unwrap().id.clone();
    let quote_id = QuoteId::from_proto(&quote_id_proto).unwrap();
    let entity_id = setup
        .store
        .get_detailed_quote_by_id(ids::TENANT_ID, quote_id)
        .await
        .unwrap()
        .invoicing_entity
        .id;

    let get_email_enabled = || async {
        clients
            .quotes
            .clone()
            .get_quote(tonic::Request::new(api::quotes::v1::GetQuoteRequest {
                id: quote_id_proto.clone(),
            }))
            .await
            .unwrap()
            .into_inner()
            .quote
            .unwrap()
            .quote_email_enabled
    };
    assert_eq!(get_email_enabled().await, Some(true));

    setup
        .store
        .set_email_kind_enabled(
            common_domain::actor::Actor::System,
            ids::TENANT_ID,
            entity_id,
            EmailKind::QuoteReady,
            false,
        )
        .await
        .unwrap();
    assert_eq!(get_email_enabled().await, Some(false));

    let err = clients
        .quotes
        .clone()
        .send_quote(tonic::Request::new(api::quotes::v1::SendQuoteRequest {
            id: quote_id_proto.clone(),
            ..Default::default()
        }))
        .await
        .expect_err("sending must be refused while the Quote ready email is off");
    assert_eq!(err.code(), Code::InvalidArgument);

    let status = setup
        .store
        .get_detailed_quote_by_id(ids::TENANT_ID, quote_id)
        .await
        .unwrap()
        .quote
        .status;
    assert_eq!(status, QuoteStatusEnum::Draft);
}

/// The quote's minimum replaces the plan's on the subscription it converts to, and is the floor
/// its MRR books from creation: 5 seats at €10.00 under a €200.00 commitment earn €200.00.
#[tokio::test]
async fn test_quote_minimum_floors_the_converted_mrr() {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::bi_mrr_movement_log::dsl as m;
    use meteroid_store::domain::plan_minimums::CreateMinimumScope;
    use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;

    let TestContext { setup, clients } = setup_test(SeedLevel::PLANS).await.unwrap();

    let quote_id_proto = create_test_quote(&clients)
        .await
        .quote
        .expect("Quote not returned")
        .id;
    // The core edition refuses a new minimum through the API (the commitments module's gate);
    // the store seeds it, as a quote created before a downgrade would have it.
    setup
        .store
        .set_quote_minimum(
            ids::TENANT_ID,
            QuoteId::from_proto(&quote_id_proto).unwrap(),
            20_000,
            CreateMinimumScope::AllComponents,
        )
        .await
        .expect("quote minimum");
    clients
        .quotes
        .clone()
        .publish_quote(tonic::Request::new(api::quotes::v1::PublishQuoteRequest {
            id: quote_id_proto.clone(),
        }))
        .await
        .expect("Failed to publish quote");
    setup
        .store
        .accept_quote(
            common_domain::actor::Actor::System,
            QuoteId::from_proto(&quote_id_proto).unwrap(),
            ids::TENANT_ID,
        )
        .await
        .expect("Failed to accept quote");
    let subscription = clients
        .quotes
        .clone()
        .convert_quote_to_subscription(tonic::Request::new(
            api::quotes::v1::ConvertQuoteToSubscriptionRequest {
                quote_id: quote_id_proto,
            },
        ))
        .await
        .expect("Failed to convert quote")
        .into_inner()
        .subscription
        .expect("Subscription not returned");
    let subscription_id = SubscriptionId::from_proto(&subscription.id).unwrap();

    let details = setup
        .store
        .get_subscription_details(ids::TENANT_ID, subscription_id)
        .await
        .expect("subscription details");
    let minimum = details.minimum.expect("the quote's minimum");
    assert_eq!(
        (minimum.amount_subunit, minimum.is_override),
        (20_000, true)
    );
    assert_eq!(details.subscription.mrr_cents, 20_000);

    let movements: Vec<i64> = m::bi_mrr_movement_log
        .filter(m::subscription_id.eq(subscription_id))
        .select(m::net_mrr_change)
        .load(&mut setup.store.pool.get().await.unwrap())
        .await
        .unwrap();
    assert_eq!(movements, vec![20_000], "booked once, at the quote's floor");
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
        .accept_quote(
            common_domain::actor::Actor::System,
            quote_id,
            ids::TENANT_ID,
        )
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
        .accept_quote(
            common_domain::actor::Actor::System,
            quote_id,
            ids::TENANT_ID,
        )
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
            code: "quote-bool-feature".into(),
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

    let now = chrono::Utc::now().date_naive();
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
                usage_examples: vec![],
                minimum_commitment_override: None,
                custom_properties: None,
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
            code: "conv-bool-feature".into(),
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

    let now = chrono::Utc::now().date_naive();
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
                usage_examples: vec![],
                minimum_commitment_override: None,
                custom_properties: None,
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
        .accept_quote(
            common_domain::actor::Actor::System,
            quote_id,
            ids::TENANT_ID,
        )
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

#[tokio::test]
async fn test_quote_custom_properties_and_list_filters() {
    use api::quotes::v1::ListQuotesRequest;
    use common_grpc::meteroid::common::v1::{Filter, FilterOperator};
    use meteroid_store::domain::custom_property_definitions::{
        CustomPropertyDefinitionNew, PropertyConfig, SelectOption,
    };
    use meteroid_store::domain::enums::{CustomPropertyEntityType, CustomPropertyType};
    use meteroid_testkit::custom_properties::create_custom_property_definition;

    let TestContext { setup, clients } = setup_test(SeedLevel::PLANS).await.unwrap();

    for (key, property_type, options) in [
        (
            "deal_stage",
            CustomPropertyType::SingleSelect,
            vec!["discovery", "negotiation"],
        ),
        ("owner", CustomPropertyType::Text, vec![]),
    ] {
        create_custom_property_definition(
            &setup.store,
            CustomPropertyDefinitionNew {
                tenant_id: ids::TENANT_ID,
                entity_type: CustomPropertyEntityType::Quote,
                key: key.to_string(),
                name: key.to_string(),
                description: None,
                property_type,
                config: PropertyConfig {
                    options: (!options.is_empty()).then(|| {
                        options
                            .iter()
                            .map(|o| SelectOption {
                                value: o.to_string(),
                                label: None,
                            })
                            .collect()
                    }),
                    ..Default::default()
                },
                required: false,
                default_value: None,
                display_order: 0,
            },
        )
        .await;
    }

    let q1 = try_create_test_quote(
        &clients,
        Some(r#"{"deal_stage":"negotiation","owner":"Alice"}"#.to_string()),
    )
    .await
    .expect("create q1");
    let q2 = try_create_test_quote(&clients, Some(r#"{"deal_stage":"discovery"}"#.to_string()))
        .await
        .expect("create q2");
    let q3 = try_create_test_quote(&clients, None)
        .await
        .expect("create q3");

    // Values round-trip, and a quote created without them gets an empty bag.
    let props = |q: &api::quotes::v1::DetailedQuote| -> serde_json::Value {
        serde_json::from_str(
            q.quote
                .as_ref()
                .unwrap()
                .custom_properties
                .as_deref()
                .unwrap_or("null"),
        )
        .unwrap()
    };
    assert_eq!(
        props(&q1),
        serde_json::json!({ "deal_stage": "negotiation", "owner": "Alice" })
    );
    assert_eq!(props(&q3), serde_json::json!({}));

    // Values are validated against the quote definitions.
    for bad in [
        r#"{"deal_stage":"closed"}"#,
        r#"{"unknown_key":"x"}"#,
        r#"not json"#,
    ] {
        let err = try_create_test_quote(&clients, Some(bad.to_string()))
            .await
            .expect_err("invalid custom properties must be rejected");
        assert_eq!(err.code(), Code::InvalidArgument, "{bad}: {err:?}");
    }

    let id = |q: &api::quotes::v1::DetailedQuote| q.quote.as_ref().unwrap().id.clone();
    let list = |filters: Vec<Filter>| {
        let mut quotes = clients.quotes.clone();
        async move {
            quotes
                .list_quotes(tonic::Request::new(ListQuotesRequest {
                    order_by: None,
                    search: None,
                    pagination: None,
                    status: None,
                    customer_id: None,
                    filters,
                }))
                .await
                .map(|r| {
                    r.into_inner()
                        .quotes
                        .into_iter()
                        .map(|q| q.id)
                        .collect::<std::collections::BTreeSet<_>>()
                })
        }
    };
    let filter = |field: &str, op: FilterOperator, values: &[&str]| Filter {
        field: field.to_string(),
        op: op.into(),
        values: values.iter().map(|v| v.to_string()).collect(),
    };

    let all = list(vec![]).await.unwrap();
    assert!([id(&q1), id(&q2), id(&q3)].iter().all(|q| all.contains(q)));
    assert_eq!(
        list(vec![filter(
            "cp:deal_stage",
            FilterOperator::Equals,
            &["negotiation"]
        )])
        .await
        .unwrap(),
        [id(&q1)].into_iter().collect()
    );
    assert_eq!(
        list(vec![filter("cp:deal_stage", FilterOperator::IsSet, &[])])
            .await
            .unwrap(),
        [id(&q1), id(&q2)].into_iter().collect()
    );
    assert_eq!(
        list(vec![filter("cp:owner", FilterOperator::Contains, &["ali"])])
            .await
            .unwrap(),
        [id(&q1)].into_iter().collect()
    );
    let err = list(vec![filter("cp:nope", FilterOperator::IsSet, &[])])
        .await
        .expect_err("unknown property");
    assert_eq!(err.code(), Code::InvalidArgument);

    // Built-in fields: all three quotes are EUR, for Spotify, on the Notion plan, without expiry.
    let ours: std::collections::BTreeSet<_> = [id(&q1), id(&q2), id(&q3)].into_iter().collect();
    for f in [
        filter("currency", FilterOperator::Equals, &["EUR"]),
        filter(
            "customer",
            FilterOperator::In,
            &[&ids::CUST_SPOTIFY_ID.as_base62()],
        ),
        filter(
            "plan",
            FilterOperator::Equals,
            &[&ids::PLAN_NOTION_ID.as_base62()],
        ),
        filter("expires_at", FilterOperator::IsEmpty, &[]),
        filter("created_at", FilterOperator::After, &["2000-01-01"]),
    ] {
        let got = list(vec![f.clone()]).await.unwrap();
        assert!(
            ours.is_subset(&got),
            "{f:?} should match all created quotes"
        );
    }
    for f in [
        filter("currency", FilterOperator::Equals, &["USD"]),
        filter("created_at", FilterOperator::Before, &["2000-01-01"]),
    ] {
        let got = list(vec![f.clone()]).await.unwrap();
        assert!(ours.is_disjoint(&got), "{f:?} should match none of them");
    }
    let err = list(vec![filter("status", FilterOperator::Equals, &["DRAFT"])])
        .await
        .expect_err("status is a dedicated argument, not a filter field");
    assert_eq!(err.code(), Code::InvalidArgument);
}

pub(crate) fn quote_granting(feature_id: &str) -> api::quotes::v1::CreateQuoteRequest {
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        EntitlementSpec, EntitlementValue, entitlement_value,
    };

    api::quotes::v1::CreateQuoteRequest {
        quote: Some(api::quotes::v1::CreateQuote {
            plan_version_id: ids::PLAN_VERSION_NOTION_ID.as_proto(),
            customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
            currency: "EUR".to_string(),
            quote_number: None,
            trial_duration: None,
            start_date: Some(chrono::Utc::now().date_naive().to_string()),
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
            usage_examples: vec![],
            minimum_commitment_override: None,
            custom_properties: None,
            recipients: vec![RecipientDetails {
                name: "Test User".to_string(),
                email: "test@example.test".to_string(),
            }],
            components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
                parameterized_components: vec![
                    api::subscriptions::v1::create_subscription_components::ComponentParameterization {
                        component_id: ids::COMP_NOTION_SEATS_ID.as_proto(),
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
                feature_id: feature_id.to_string(),
                value: Some(EntitlementValue {
                    value: Some(entitlement_value::Value::BooleanValue(
                        entitlement_value::BooleanValue { enabled: true },
                    )),
                }),
            }],
        }),
    }
}

/// A quote agreed before its feature is archived still converts while it is archived, and the
/// subscription is made from the visible grants only, so restoring the feature does not add it
/// there. A quote created while archived skips it.
#[tokio::test]
async fn test_quote_conversion_leaves_out_hidden_feature_entitlement() {
    use common_domain::ids::EntitlementEntityId;
    use meteroid_grpc::meteroid::api::entitlements::v1::{
        CreateFeatureRequest, FeatureStatus, FeatureType, GetEffectiveEntitlementsRequest,
        SetFeatureStatusRequest, feature_type,
    };

    let TestContext { setup, clients } = setup_test(SeedLevel::PLANS).await.unwrap();

    let feature = clients
        .entitlements
        .clone()
        .create_feature(CreateFeatureRequest {
            name: "quoted-then-archived".into(),
            code: "quoted-then-archived".into(),
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

    let agreed_quote = || async {
        let id = clients
            .quotes
            .clone()
            .create_quote(tonic::Request::new(quote_granting(&feature.id)))
            .await
            .expect("Failed to create quote")
            .into_inner()
            .quote
            .unwrap()
            .quote
            .unwrap()
            .id;
        clients
            .quotes
            .clone()
            .publish_quote(tonic::Request::new(api::quotes::v1::PublishQuoteRequest {
                id: id.clone(),
            }))
            .await
            .expect("Failed to publish quote");
        setup
            .store
            .accept_quote(
                common_domain::actor::Actor::System,
                QuoteId::from_proto(&id).unwrap(),
                ids::TENANT_ID,
            )
            .await
            .expect("Failed to accept quote");
        id
    };
    let convert = |quote_id: String| {
        let mut client = clients.quotes.clone();
        async move {
            client
                .convert_quote_to_subscription(tonic::Request::new(
                    api::quotes::v1::ConvertQuoteToSubscriptionRequest { quote_id },
                ))
                .await
                .map(|r| r.into_inner().subscription.unwrap().id)
        }
    };
    let subscription_grants = |subscription_id: String| {
        let store = setup.store.clone();
        async move {
            store
                .list_entitlements_by_entity(
                    EntitlementEntityId::Subscription(
                        SubscriptionId::from_proto(&subscription_id).unwrap(),
                    ),
                    ids::TENANT_ID,
                )
                .await
                .unwrap()
        }
    };
    let quote_id_proto = agreed_quote().await;
    let set_status = |status: FeatureStatus| {
        let mut client = clients.entitlements.clone();
        let id = feature.id.clone();
        async move {
            client
                .set_feature_status(SetFeatureStatusRequest {
                    id,
                    status: status.into(),
                })
                .await
                .unwrap();
        }
    };
    let resolves_for_customer = || {
        let mut client = clients.entitlements.clone();
        let feature_id = feature.id.clone();
        async move {
            client
                .get_effective_entitlements(GetEffectiveEntitlementsRequest {
                    customer_id: ids::CUST_SPOTIFY_ID.as_proto(),
                })
                .await
                .unwrap()
                .into_inner()
                .entitlements
                .iter()
                .any(|e| e.feature.as_ref().is_some_and(|f| f.id == feature_id))
        }
    };
    let quote_grants = |quote_id: &str| {
        let store = setup.store.clone();
        let quote_id = QuoteId::from_proto(quote_id).unwrap();
        async move {
            store
                .list_entitlements_by_entity(EntitlementEntityId::Quote(quote_id), ids::TENANT_ID)
                .await
                .unwrap()
        }
    };

    set_status(FeatureStatus::Archived).await;

    let new_quote_id = clients
        .quotes
        .clone()
        .create_quote(tonic::Request::new(quote_granting(&feature.id)))
        .await
        .expect("granting an archived feature does not fail a new quote")
        .into_inner()
        .quote
        .unwrap()
        .quote
        .unwrap()
        .id;
    assert!(
        quote_grants(&new_quote_id).await.is_empty(),
        "a new quote skips the archived feature's grant"
    );

    let subscription_id = convert(quote_id_proto.clone())
        .await
        .expect("a quote agreed before the archive still converts");
    assert!(
        subscription_grants(subscription_id.clone())
            .await
            .is_empty(),
        "the hidden grant is not on the subscription"
    );
    assert!(!resolves_for_customer().await);

    set_status(FeatureStatus::Active).await;
    assert!(
        subscription_grants(subscription_id).await.is_empty(),
        "a subscription converted while archived stays as converted"
    );
    assert_eq!(
        quote_grants(&quote_id_proto).await.len(),
        1,
        "the agreed quote's own grant comes back"
    );
    assert!(!resolves_for_customer().await);
    assert!(
        quote_grants(&new_quote_id).await.is_empty(),
        "a grant skipped while archived stays skipped"
    );
}
