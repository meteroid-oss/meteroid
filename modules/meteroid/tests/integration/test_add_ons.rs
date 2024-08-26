use crate::meteroid_it::container::SeedLevel;
use crate::{helpers, meteroid_it};
use meteroid_grpc::meteroid::api;

#[tokio::test]
async fn test_add_ons_basic() {
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

    let one_time_fee = api::components::v1::Fee {
        fee_type: Some(api::components::v1::fee::FeeType::OneTime(
            api::components::v1::fee::OneTimeFee {
                unit_price: "10".into(),
                quantity: 5,
            },
        )),
    };

    // create add-on
    let created = clients
        .add_ons
        .clone()
        .create_add_on(api::addons::v1::CreateAddOnRequest {
            name: "test-add-on".into(),
            fee: Some(one_time_fee.clone()),
        })
        .await
        .unwrap()
        .into_inner()
        .add_on
        .unwrap();

    assert_eq!(created.name.as_str(), "test-add-on");
    assert_eq!(created.fee.as_ref(), Some(one_time_fee).as_ref());

    // list add-ons
    let add_ons = clients
        .add_ons
        .clone()
        .list_add_ons(api::addons::v1::ListAddOnRequest {})
        .await
        .unwrap()
        .into_inner()
        .add_ons;

    assert_eq!(add_ons.len(), 1);
    assert_eq!(add_ons.first(), Some(&created));

    // edit add-on
    let to_edit = api::addons::v1::AddOn {
        name: "edited-add-on".into(),
        ..created
    };

    let edited = clients
        .add_ons
        .clone()
        .edit_add_on(api::addons::v1::EditAddOnRequest {
            add_on: Some(to_edit.clone()),
        })
        .await
        .unwrap()
        .into_inner()
        .add_on
        .unwrap();

    assert_eq!(&edited, &to_edit);

    // remove add-on
    clients
        .add_ons
        .clone()
        .remove_add_on(api::addons::v1::RemoveAddOnRequest {
            add_on_id: edited.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();

    let add_ons = clients
        .add_ons
        .clone()
        .list_add_ons(api::addons::v1::ListAddOnRequest {})
        .await
        .unwrap()
        .into_inner()
        .add_ons;

    assert_eq!(add_ons.len(), 0);
}
