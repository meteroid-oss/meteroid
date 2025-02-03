use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid_grpc::meteroid::api;

#[tokio::test]
async fn test_product_families_basic() {
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

    // create product family
    let created = clients
        .product_families
        .clone()
        .create_product_family(api::productfamilies::v1::CreateProductFamilyRequest {
            name: "product_family_name".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .product_family
        .unwrap();

    assert_eq!(created.name.as_str(), "product_family_name");

    // product family by local_id
    let by_local_id = clients
        .product_families
        .clone()
        .get_product_family_by_local_id(
            api::productfamilies::v1::GetProductFamilyByLocalIdRequest {
                local_id: created.local_id.clone(),
            },
        )
        .await
        .unwrap()
        .into_inner()
        .product_family
        .unwrap();

    assert_eq!(&by_local_id, &created);

    // list product families
    let listed = clients
        .product_families
        .clone()
        .list_product_families(api::productfamilies::v1::ListProductFamiliesRequest {})
        .await
        .unwrap()
        .into_inner()
        .product_families;

    assert_eq!(listed.len(), 1);
    assert_eq!(listed.first(), Some(created).as_ref());

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}
