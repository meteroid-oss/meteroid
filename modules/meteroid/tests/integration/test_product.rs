use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::users::v1::UserRole;

#[tokio::test]
async fn test_products_basic() {
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

    let family = clients
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

    // create product
    let created = clients
        .products
        .clone()
        .create_product(api::products::v1::CreateProductRequest {
            name: "product_name".into(),
            description: Some("product_description".into()),
            family_external_id: family.external_id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .product
        .unwrap();

    assert_eq!(created.name.as_str(), "product_name");
    assert_eq!(created.description, Some("product_description".to_string()));

    // product family by external_id
    let by_id = clients
        .products
        .clone()
        .get_product(api::products::v1::GetProductRequest {
            product_id: created.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .product
        .unwrap();

    assert_eq!(&by_id, &created);

    // list products
    let listed = clients
        .products
        .clone()
        .list_products(api::products::v1::ListProductsRequest {
            family_external_id: family.external_id.clone(),
            pagination: None,
        })
        .await
        .unwrap()
        .into_inner()
        .products;

    assert_eq!(listed.len(), 1);
    assert_eq!(listed.first().unwrap().id, created.id.clone());

    // search products
    let searched = clients
        .products
        .clone()
        .search_products(api::products::v1::SearchProductsRequest {
            family_external_id: family.external_id.clone(),
            query: Some("_nAm".to_string()),
            pagination: None,
        })
        .await
        .unwrap()
        .into_inner()
        .products;

    assert_eq!(searched.len(), 1);
    assert_eq!(searched.first().unwrap().id, created.id.clone());

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}
