use meteroid_grpc::meteroid::api;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;

#[tokio::test]
async fn test_billable_metrics_basic() {
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

    let metric_name = "friends and co".to_owned();
    let metric_code = "vvv".to_owned();

    // create family
    let _family = clients
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

    // create metric
    let created = clients
        .metrics
        .clone()
        .create_billable_metric(api::billablemetrics::v1::CreateBillableMetricRequest {
            name: metric_name.to_string(),
            code: metric_code.to_string(),
            description: None,
            aggregation: Some(api::billablemetrics::v1::Aggregation {
                aggregation_type: api::billablemetrics::v1::aggregation::AggregationType::Sum as i32,
                aggregation_key: Some("aggregation_key".to_string()),
                unit_conversion: Some(api::billablemetrics::v1::aggregation::UnitConversion {
                    factor: 1.0,
                    rounding: api::billablemetrics::v1::aggregation::unit_conversion::UnitConversionRounding::Nearest as i32,
                }),
            }),
            segmentation_matrix: None, // todo add
            usage_group_key: Some("usage".to_string()),
            family_local_id: "product_family_local_id".to_string(),
            product_id: None,
        })
        .await
        .unwrap()
        .into_inner()
        .billable_metric
        .unwrap();

    assert_eq!(created.name, metric_name.clone());
    assert_eq!(created.code, metric_code.clone());

    // get billable metric
    let get_by_id = clients
        .metrics
        .clone()
        .get_billable_metric(api::billablemetrics::v1::GetBillableMetricRequest {
            id: created.id.to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .billable_metric
        .unwrap();

    assert_eq!(get_by_id.id, created.id.clone());
    assert_eq!(get_by_id.name, metric_name.clone());

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}
