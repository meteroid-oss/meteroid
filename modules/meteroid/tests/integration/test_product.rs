use crate::data::ids::{PRODUCT_FAMILY_ID, PRODUCT_PLATFORM_FEE_ID, TENANT_ID};
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::test_env;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId, ProductId};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_grpc::meteroid::api;
use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum};
use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::products::ProductNew;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::plans::PlansInterface;
use meteroid_store::repositories::products::ProductInterface;
use rust_decimal_macros::dec;

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
            icon: None,
            color: None,
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

/// Where a product is sold: live plans' active and draft versions, and live add-ons.
#[tokio::test]
async fn product_usage_lists_live_plan_versions_and_add_ons() {
    let env = test_env().await;
    let store = env.store();
    let product = store
        .create_product(
            Actor::System,
            ProductNew {
                name: "Support".to_string(),
                description: None,
                tenant_id: TENANT_ID,
                family_id: PRODUCT_FAMILY_ID,
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
                catalog: true,
                tax_category_id: None,
                icon: None,
                color: None,
            },
        )
        .await
        .expect("a product")
        .id;
    let support = |amount| {
        SeedComp::rate(
            PriceComponentId::new(),
            "Support",
            product,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            amount,
        )
    };
    let platform = || {
        SeedComp::rate(
            PriceComponentId::new(),
            "Platform",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            dec!(20),
        )
    };
    let mut conn = env.conn().await;

    // Sold in USD, while another charge of the version is priced in EUR too: EUR is a gap.
    let basic = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Usage Basic", basic)
        .currency("USD")
        .components(vec![support(dec!(12))])
        .seed(&mut conn)
        .await
        .expect("basic");
    PlanSeed::seed_components(&mut conn, basic, &[platform()], "EUR")
        .await
        .expect("basic in EUR");

    // Two options, the price in each.
    let team = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Usage Team", team)
        .currency("USD")
        .options(&[
            ("Monthly", DieselBillingPeriodEnum::Monthly),
            ("Annual", DieselBillingPeriodEnum::Annual),
        ])
        .components(vec![support(dec!(15))])
        .seed(&mut conn)
        .await
        .expect("team");

    // Only its draft sells the product.
    let (draft_plan, draft) = (PlanId::new(), PlanVersionId::new());
    PlanSeed::new(draft_plan, "Usage Draft", PlanVersionId::new())
        .currency("USD")
        .draft(draft, 2)
        .components(vec![platform()])
        .seed(&mut conn)
        .await
        .expect("draft plan");
    PlanSeed::seed_components(&mut conn, draft, &[support(dec!(9))], "USD")
        .await
        .expect("draft components");

    // An old version, neither active nor draft, and an archived plan: not where it is sold.
    let old_plan = PlanId::new();
    PlanSeed::new(old_plan, "Usage Old", PlanVersionId::new())
        .currency("USD")
        .components(vec![platform()])
        .seed(&mut conn)
        .await
        .expect("old plan");
    PlanSeed::seed_version(
        &mut conn,
        old_plan,
        PlanVersionId::new(),
        0,
        "USD",
        &[support(dec!(5))],
    )
    .await
    .expect("old version");
    let archived_plan = PlanId::new();
    PlanSeed::new(archived_plan, "Usage Archived", PlanVersionId::new())
        .currency("USD")
        .components(vec![support(dec!(7))])
        .seed(&mut conn)
        .await
        .expect("archived plan");
    store
        .archive_plan(Actor::System, archived_plan, TENANT_ID)
        .await
        .expect("archived");

    let add_on = |name: &str| {
        store.create_add_on_with_prices(
            name.to_string(),
            ProductRef::Existing(product),
            vec![PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "USD".to_string(),
                pricing: Pricing::Rate { rate: dec!(30) },
                credit_unit: None,
            })],
            None,
            true,
            None,
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
    };
    let extra = add_on("Extra support").await.expect("an add-on");
    let retired = add_on("Retired support").await.expect("another add-on");
    store
        .archive_add_on(Actor::System, retired.id, TENANT_ID)
        .await
        .expect("archived add-on");

    let usage = store
        .product_usage(TENANT_ID, product)
        .await
        .expect("the product's usage");

    let plans: Vec<(&str, i32, bool)> = usage
        .plans
        .iter()
        .map(|p| (p.plan_name.as_str(), p.version, p.is_draft))
        .collect();
    assert_eq!(
        plans,
        [
            ("Usage Basic", 1, false),
            ("Usage Draft", 2, true),
            ("Usage Team", 1, false)
        ]
    );

    let basic_usage = &usage.plans[0];
    assert_eq!(basic_usage.plan_version_id, basic);
    assert_eq!(basic_usage.currencies, ["EUR", "USD"]);
    assert_eq!(basic_usage.components.len(), 1, "the product's charge only");
    let charge = &basic_usage.components[0];
    assert_eq!(charge.product_id, Some(product));
    assert_eq!(charge.prices.len(), 1);
    assert_eq!(charge.prices[0].currency, "USD");
    assert!(matches!(charge.prices[0].pricing, Pricing::Rate { rate } if rate == dec!(12)));

    let team_usage = &usage.plans[2];
    let options: Vec<&str> = team_usage.options.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(options, ["Monthly", "Annual"]);
    let links = &team_usage.components[0].option_prices;
    assert_eq!(links.len(), 2, "its price in each option");
    assert!(
        team_usage
            .options
            .iter()
            .all(|o| links.iter().any(|l| l.option_id == o.id))
    );

    assert_eq!(usage.plans[1].plan_version_id, draft);

    let add_ons: Vec<_> = usage.add_ons.iter().map(|a| a.id).collect();
    assert_eq!(add_ons, [extra.id], "the live add-on only");
    assert_eq!(usage.add_ons[0].catalog_prices().len(), 1);

    let unused = store
        .product_usage(TENANT_ID, ProductId::new())
        .await
        .expect("an unknown product's usage");
    assert!(unused.plans.is_empty() && unused.add_ons.is_empty());
}
