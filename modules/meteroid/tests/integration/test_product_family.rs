use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::users::v1::UserRole;

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
    assert_eq!(auth.user.unwrap().role, UserRole::Admin as i32);

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "a712afi5lzhk",
    );

    // create product family
    let created = clients
        .product_families
        .clone()
        .create_product_family(api::productfamilies::v1::CreateProductFamilyRequest {
            name: "product_family_name".into(),
            external_id: "product_family_external_id".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .product_family
        .unwrap();

    assert_eq!(created.name.as_str(), "product_family_name");
    assert_eq!(created.external_id.as_str(), "product_family_external_id");

    // product family by external_id
    let by_external_id = clients
        .product_families
        .clone()
        .get_product_family_by_external_id(
            api::productfamilies::v1::GetProductFamilyByExternalIdRequest {
                external_id: "product_family_external_id".into(),
            },
        )
        .await
        .unwrap()
        .into_inner()
        .product_family
        .unwrap();

    assert_eq!(&by_external_id, &created);

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
