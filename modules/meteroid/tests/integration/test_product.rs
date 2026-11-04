use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid_grpc::meteroid::api;

#[tokio::test]
async fn test_products_basic() {
    // Generic setup
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

    let family = clients
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

    // create product
    let created = clients
        .products
        .clone()
        .create_product(api::products::v1::CreateProductRequest {
            name: "product_name".into(),
            description: Some("product_description".into()),
            family_local_id: family.local_id.clone(),
            fee_type: api::prices::v1::FeeType::Rate.into(),
            fee_structure: Some(api::prices::v1::FeeStructure {
                structure: Some(api::prices::v1::fee_structure::Structure::Rate(
                    api::prices::v1::fee_structure::RateStructure {},
                )),
            }),
            catalog: None,
            tax_category_id: None,
        })
        .await
        .unwrap()
        .into_inner()
        .product
        .unwrap();

    assert_eq!(created.name.as_str(), "product_name");
    assert_eq!(created.description, Some("product_description".to_string()));

    // product family by local_id
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
            family_local_id: Some(family.local_id.clone()),
            pagination: None,
            catalog_only: None,
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
            family_local_id: Some(family.local_id.clone()),
            query: Some("_nAm".to_string()),
            pagination: None,
            catalog_only: None,
            order_by: None,
            archived: None,
        })
        .await
        .unwrap()
        .into_inner()
        .products;

    assert_eq!(searched.len(), 1);
    assert_eq!(searched.first().unwrap().id, created.id.clone());

    let archived = clients
        .products
        .clone()
        .archive_product(api::products::v1::ArchiveProductRequest {
            product_id: created.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .product
        .unwrap();
    assert!(archived.archived_at.is_some());

    let search = |archived: bool| {
        let mut client = clients.products.clone();
        let family_local_id = family.local_id.clone();
        async move {
            client
                .search_products(api::products::v1::SearchProductsRequest {
                    family_local_id: Some(family_local_id),
                    query: None,
                    pagination: None,
                    catalog_only: None,
                    order_by: None,
                    archived: Some(archived),
                })
                .await
                .unwrap()
                .into_inner()
                .products
        }
    };

    assert!(search(false).await.is_empty());
    let archived_list = search(true).await;
    assert_eq!(archived_list.len(), 1);
    assert!(archived_list.first().unwrap().archived_at.is_some());

    let unarchived = clients
        .products
        .clone()
        .unarchive_product(api::products::v1::UnarchiveProductRequest {
            product_id: created.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .product
        .unwrap();
    assert!(unarchived.archived_at.is_none());
    assert_eq!(search(false).await.len(), 1);
    assert!(search(true).await.is_empty());

    // teardown
    // meteroid_it::container::terminate_meteroid(setup.token, &setup.join_handle).await
}
