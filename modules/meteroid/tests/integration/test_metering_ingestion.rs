use crate::data::ids;
use crate::metering_it;
use crate::{helpers, meteroid_it};
use backon::BlockingRetryable;
use backon::Retryable;
use chrono::Days;
use image::imageops::resize;
use metering::ingest::domain::RawEventRow;
use metering_grpc::meteroid::metering::v1::{Event, IngestRequest, event};
use meteroid::clients::usage::MeteringUsageClient;
use meteroid_grpc::meteroid::api::billablemetrics::v1::aggregation::AggregationType;
use meteroid_grpc::meteroid::api::billablemetrics::v1::segmentation_matrix::{
    Dimension, Matrix, SegmentationMatrixDouble,
};
use meteroid_grpc::meteroid::api::billablemetrics::v1::{
    Aggregation, CreateBillableMetricRequest, SegmentationMatrix,
};
use std::sync::Arc;
use std::time::Duration;
use tonic::Request;
use uuid::Uuid;

#[tokio::test]
async fn test_metering_ingestion() {
    helpers::init::logging();

    // we start pg, clickhouse, kafka

    let (_pg_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;

    let (_kafka_container, kafka_port) = metering_it::container::start_kafka()
        .await
        .expect("Could not start kafka");

    let (_clickhouse_container, clickhouse_port) = metering_it::container::start_clickhouse().await;

    metering_it::kafka::create_topic(kafka_port, "meteroid-events-raw")
        .await
        .expect("Could not create topic");

    // we start meteroid and metering

    let meteroid_port = helpers::network::free_local_port().expect("Could not get free port");
    let metering_port = helpers::network::free_local_port().expect("Could not get free port");

    let metering_config = metering_it::config::mocked_config(
        meteroid_port,
        metering_port,
        clickhouse_port,
        kafka_port,
        "meteroid-events-raw".to_string(),
    );

    let metering_setup = metering_it::container::start_metering(metering_config.clone()).await;

    let api_key = "pv_sand_9XzHg0EYO2Usy9ITU6bbhBnkYYbx/2vO7XtUUeQ7Wq9EZCAbBG";

    let mut metering_clients = metering_it::clients::AllClients::from_channel(
        metering_setup.channel.clone(),
        api_key,
        &metering_config.internal_auth,
    );

    let metering_client = MeteringUsageClient::from_channel(
        metering_setup.channel.clone(),
        &metering_config.internal_auth,
    );

    let meteroid_setup = meteroid_it::container::start_meteroid_with_port(
        meteroid_port,
        metering_port,
        postgres_connection_string,
        meteroid_it::container::SeedLevel::PRODUCT,
        Arc::new(metering_client),
    )
    .await;

    let jwt_auth = meteroid_it::svc_auth::login(meteroid_setup.channel.clone()).await;

    let mut meteroid_clients = meteroid_it::clients::AllClients::from_channel(
        meteroid_setup.channel.clone(),
        jwt_auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    let tenant_id = ids::TENANT_ID;

    let customer_1 = ids::CUST_SPOTIFY_ID;
    let customer_2 = ids::CUST_UBER_ID;

    let now = chrono::Utc::now();

    // period 2 started yesterday.
    // so after workers we expect a draft for period 2, and a finalized/issued for period 1
    // we will create the draft invoice for period 1 manually, as the draft worker only check last 7 days
    let period_2_start = now - chrono::Duration::days(1);
    let period_1_start = period_2_start.checked_sub_days(Days::new(20)).unwrap();

    // we consider a billing period 1, customer 1, inference endpoint
    let to_ingest = vec![
        Event {
            id: Uuid::new_v4().to_string(),
            code: "api_calls".to_string(),
            customer_id: Some(event::CustomerId::MeteroidCustomerId(
                customer_1.to_string(),
            )),
            timestamp: period_1_start.to_rfc3339(),
            properties: [
                ("endpoint".to_string(), "inference".to_string()),
                ("model".to_string(), "gpt_3.5".to_string()),
                ("tokens".to_string(), "20".to_string()),
            ]
            .into(),
        },
        Event {
            id: Uuid::new_v4().to_string(),
            code: "api_calls".to_string(),
            customer_id: Some(event::CustomerId::MeteroidCustomerId(
                customer_1.to_string(),
            )),
            timestamp: period_1_start
                .checked_add_days(Days::new(1))
                .unwrap()
                .to_rfc3339(),
            properties: [
                ("endpoint".to_string(), "inference".to_string()),
                ("model".to_string(), "gpt_3.5".to_string()),
                ("tokens".to_string(), "150".to_string()),
            ]
            .into(),
        },
        Event {
            id: Uuid::new_v4().to_string(),
            code: "api_calls".to_string(),
            customer_id: Some(event::CustomerId::MeteroidCustomerId(
                customer_1.to_string(),
            )),
            timestamp: period_1_start
                .checked_add_days(Days::new(10))
                .unwrap()
                .to_rfc3339(),
            properties: [
                ("endpoint".to_string(), "inference".to_string()),
                ("model".to_string(), "gpt_3.5".to_string()),
                ("tokens".to_string(), "70".to_string()),
            ]
            .into(),
        },
        Event {
            id: Uuid::new_v4().to_string(),
            code: "api_calls".to_string(),
            customer_id: Some(event::CustomerId::MeteroidCustomerId(
                customer_1.to_string(),
            )),
            timestamp: period_2_start
                .checked_sub_days(Days::new(1))
                .unwrap()
                .to_rfc3339(),
            properties: [
                ("endpoint".to_string(), "inference".to_string()),
                ("model".to_string(), "gpt_3.5".to_string()),
                ("tokens".to_string(), "9".to_string()),
            ]
            .into(),
        },
        // out of period
        Event {
            id: Uuid::new_v4().to_string(),
            code: "api_calls".to_string(),
            customer_id: Some(event::CustomerId::MeteroidCustomerId(
                customer_1.to_string(),
            )),
            timestamp: period_2_start.to_rfc3339(),
            properties: [
                ("endpoint".to_string(), "inference".to_string()),
                ("model".to_string(), "gpt_3.5".to_string()),
                ("tokens".to_string(), "25000".to_string()),
            ]
            .into(),
        },
        // other customer
        Event {
            id: Uuid::new_v4().to_string(),
            code: "api_calls".to_string(),
            customer_id: Some(event::CustomerId::MeteroidCustomerId(
                customer_2.to_string(),
            )),
            timestamp: period_1_start
                .checked_add_days(Days::new(10))
                .unwrap()
                .to_rfc3339(),
            properties: [
                ("endpoint".to_string(), "inference".to_string()),
                ("model".to_string(), "gpt_3.5".to_string()),
                ("tokens".to_string(), "25000".to_string()),
            ]
            .into(),
        },
        // other event type
        Event {
            id: Uuid::new_v4().to_string(),
            code: "api_response".to_string(),
            customer_id: Some(event::CustomerId::MeteroidCustomerId(
                customer_1.to_string(),
            )),
            timestamp: period_1_start
                .checked_add_days(Days::new(10))
                .unwrap()
                .to_rfc3339(),
            properties: [
                ("endpoint".to_string(), "inference".to_string()),
                ("model".to_string(), "gpt_3.5".to_string()),
                ("tokens".to_string(), "25000".to_string()),
            ]
            .into(),
        },
        // other endpoint
        Event {
            id: Uuid::new_v4().to_string(),
            code: "api_response".to_string(),
            customer_id: Some(event::CustomerId::MeteroidCustomerId(
                customer_1.to_string(),
            )),
            timestamp: period_1_start
                .checked_add_days(Days::new(10))
                .unwrap()
                .to_rfc3339(),
            properties: [
                ("endpoint".to_string(), "embedding".to_string()),
                ("model".to_string(), "gpt_3.5".to_string()),
                ("tokens".to_string(), "25000".to_string()),
            ]
            .into(),
        },
        // other model
        Event {
            id: Uuid::new_v4().to_string(),
            code: "api_calls".to_string(),
            customer_id: Some(event::CustomerId::MeteroidCustomerId(
                customer_1.to_string(),
            )),
            timestamp: period_1_start
                .checked_add_days(Days::new(10))
                .unwrap()
                .to_rfc3339(),
            properties: [
                ("endpoint".to_string(), "inference".to_string()),
                ("model".to_string(), "gpt_4".to_string()),
                ("tokens".to_string(), "25000".to_string()),
            ]
            .into(),
        },
    ];

    let to_ingest_len = to_ingest.len();

    // we ingest events in metering
    let ingested = metering_clients
        .events
        .ingest(Request::new(IngestRequest {
            events: to_ingest,
            allow_backfilling: true,
        }))
        .await
        .expect("Could not ingest events");

    let ingested = ingested.into_inner();

    assert_eq!(ingested.failures.len(), 0);

    let clickhouse_client = metering_it::clickhouse::get_client(clickhouse_port);

    (|| async {
        match clickhouse_client
            .query("SELECT * FROM raw_events")
            .fetch_all::<RawEventRow>()
            .await
        {
            Ok(vec) => {
                if vec.len() != to_ingest_len {
                    Err(anyhow::anyhow!("Unexpected number of rows"))
                } else {
                    Ok(vec)
                }
            }
            Err(e) => Err(anyhow::anyhow!(e)),
        }
    })
    .retry(
        backon::ConstantBuilder::default()
            .with_delay(Duration::from_millis(500))
            .with_max_times(10),
    )
    .notify(|err: &anyhow::Error, dur: Duration| {
        log::warn!(
            "Retrying to poll and assert events after {:?}, error: {}",
            dur,
            err
        );
    })
    .await
    .expect("Failed to validate events in ClickHouse");

    // todo run pgmq as BM is created async

    // // we create a meter
    // let created_metric = meteroid_clients
    //     .metrics
    //     .create_billable_metric(Request::new(CreateBillableMetricRequest {
    //         name: "inference api calls".to_string(),
    //         code: "api_calls".to_string(),
    //         description: None,
    //         aggregation: Some(Aggregation {
    //             aggregation_type: AggregationType::Sum as i32,
    //             aggregation_key: Some("tokens".to_string()),
    //             unit_conversion: None,
    //         }),
    //         segmentation_matrix: Some(SegmentationMatrix {
    //             // TODO simplify. Also, Vec<Dimension / LinkedDimension> ?
    //             matrix: Some(Matrix::Double(SegmentationMatrixDouble {
    //                 dimension1: Some(Dimension {
    //                     key: "endpoint".to_string(),
    //                     values: vec!["inference".to_string()],
    //                 }),
    //                 dimension2: Some(Dimension {
    //                     key: "model".to_string(),
    //                     values: vec!["gpt_3.5".to_string()],
    //                 }),
    //             })),
    //         }),
    //         usage_group_key: None,
    //         family_local_id: ids::PRODUCT_FAMILY_ID.as_proto(),
    //         product_id: None,
    //     }))
    //     .await
    //     .expect("Could not create meter");
    //
    // let created_metric = created_metric.into_inner();
    //
    //
    // // we validate that it was created in clickhouse
    //
    // // list all tables in db meteroid
    // let tables = clickhouse_client
    //     .query("SHOW TABLES")
    //     .fetch_all::<String>()
    //     .await
    //     .unwrap();
    //
    // let expected_table_name = metering::connectors::clickhouse::sql::get_meter_view_name(
    //     &tenant_id.to_string(),
    //     &created_metric.billable_metric.unwrap().id,
    // )
    // .split(".")
    // .collect::<Vec<&str>>()[1]
    //     .to_string();
    //
    // tables
    //     .into_iter()
    //     .find(|x| x == &expected_table_name)
    //     .expect("Could not find meter table");
}
