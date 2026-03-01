use crate::meteroid_it::container::SeedLevel;
use crate::{helpers, meteroid_it};
use meteroid_grpc::meteroid::api;

#[tokio::test]
async fn test_add_ons_basic() {
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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

    // Create add-on with a new product + rate pricing
    let created = clients
        .add_ons
        .clone()
        .create_add_on(api::addons::v1::CreateAddOnRequest {
            name: "test-add-on".into(),
            product: Some(api::components::v1::ProductRef {
                r#ref: Some(api::components::v1::product_ref::Ref::NewProduct(
                    api::components::v1::NewProduct {
                        name: "Test Add-on Product".into(),
                        fee_type: api::prices::v1::FeeType::Rate.into(),
                        fee_structure: Some(api::prices::v1::FeeStructure {
                            structure: Some(api::prices::v1::fee_structure::Structure::Rate(
                                api::prices::v1::fee_structure::RateStructure {},
                            )),
                        }),
                    },
                )),
            }),
            price: Some(api::components::v1::PriceEntry {
                entry: Some(api::components::v1::price_entry::Entry::NewPrice(
                    api::components::v1::PriceInput {
                        cadence: api::shared::v1::BillingPeriod::Monthly.into(),
                        currency: "USD".into(),
                        pricing: Some(api::components::v1::price_input::Pricing::RatePricing(
                            api::prices::v1::RatePricing {
                                rate: "9.99".into(),
                            },
                        )),
                    },
                )),
            }),
            description: Some("A test add-on".into()),
            self_serviceable: true,
            max_instances_per_subscription: Some(3),
            product_family_local_id: None,
        })
        .await
        .unwrap()
        .into_inner()
        .add_on
        .unwrap();

    assert_eq!(created.name.as_str(), "test-add-on");
    assert_eq!(created.description.as_deref(), Some("A test add-on"));
    assert!(created.self_serviceable);
    assert_eq!(created.max_instances_per_subscription, Some(3));
    assert!(!created.product_id.is_empty());
    assert!(created.price.is_some());
    assert!(created.archived_at.is_none());

    // List add-ons
    let add_ons = clients
        .add_ons
        .clone()
        .list_add_ons(api::addons::v1::ListAddOnRequest {
            plan_version_id: None,
            search: None,
            pagination: None,
            currency: None,
        })
        .await
        .unwrap()
        .into_inner()
        .add_ons;

    assert_eq!(add_ons.len(), 1);
    assert_eq!(add_ons[0].id, created.id);

    // Get add-on by id
    let fetched = clients
        .add_ons
        .clone()
        .get_add_on(api::addons::v1::GetAddOnRequest {
            add_on_id: created.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .add_on
        .unwrap();

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.name.as_str(), "test-add-on");

    // Edit add-on
    let edited = clients
        .add_ons
        .clone()
        .edit_add_on(api::addons::v1::EditAddOnRequest {
            add_on_id: created.id.clone(),
            name: "edited-add-on".into(),
            price: None,
            description: Some("Updated description".into()),
            self_serviceable: Some(false),
            max_instances_per_subscription: None,
        })
        .await
        .unwrap()
        .into_inner()
        .add_on
        .unwrap();

    assert_eq!(edited.name.as_str(), "edited-add-on");
    assert_eq!(edited.description.as_deref(), Some("Updated description"));
    assert!(!edited.self_serviceable);

    // Archive add-on (RemoveAddOn now archives instead of deleting)
    clients
        .add_ons
        .clone()
        .remove_add_on(api::addons::v1::RemoveAddOnRequest {
            add_on_id: edited.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();

    // Archived add-on should not appear in list
    let add_ons = clients
        .add_ons
        .clone()
        .list_add_ons(api::addons::v1::ListAddOnRequest {
            plan_version_id: None,
            search: None,
            pagination: None,
            currency: None,
        })
        .await
        .unwrap()
        .into_inner()
        .add_ons;

    assert_eq!(add_ons.len(), 0);
}
