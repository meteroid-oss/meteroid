use crate::data::ids;
use crate::metering_it;
use crate::{helpers, meteroid_it};
use backon::Retryable;
use chrono::Days;
use common_domain::ids::BillableMetricId;
use itertools::Itertools;
use metering::connectors::clickhouse::sql::get_meter_view_name;
use metering::ingest::domain::RawEventRow;
use metering_grpc::meteroid::metering::v1::{Event, IngestRequest, event};
use meteroid::clients::usage::MeteringUsageClient;
use meteroid::workers::pgmq::processors::{run_metric_sync, run_outbox_dispatch};
use meteroid_grpc::meteroid::api;
use meteroid_mailer::config::MailerConfig;
use meteroid_store::Store;
use meteroid_store::clients::usage::{UsageClient, UsageData};
use meteroid_store::domain::Period;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use rust_decimal::Decimal;
use std::sync::Arc;
use std::time::Duration;
use tonic::Request;
use uuid::Uuid;

#[tokio::test]
async fn test_metering_ingestion() {
    helpers::init::logging();

    // we start pg, clickhouse, kafka

    let postgres_connection_string = meteroid_it::container::create_test_database().await;

    let (_kafka_container, kafka_port) = metering_it::container::start_kafka()
        .await
        .expect("Could not start kafka");

    let (_clickhouse_container, ch_http_port, ch_tcp_port) =
        metering_it::container::start_clickhouse().await;

    metering_it::kafka::create_topic(kafka_port, "meteroid-events-raw")
        .await
        .expect("Could not create topic");

    // we start meteroid and metering

    let meteroid_port = helpers::network::free_local_port().expect("Could not get free port");
    let metering_port = helpers::network::free_local_port().expect("Could not get free port");

    let metering_config = metering_it::config::mocked_config(
        meteroid_port,
        metering_port,
        ch_http_port,
        ch_tcp_port,
        kafka_port,
        "meteroid-events-raw".to_string(),
        "meteroid-events-preprocessed".to_string(),
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

    let metering_client = Arc::new(metering_client);
    let metering_client_clone = metering_client.clone();

    let meteroid_setup = meteroid_it::container::start_meteroid_with_port(
        meteroid_port,
        metering_port,
        postgres_connection_string,
        meteroid_it::container::SeedLevel::PRODUCT,
        metering_client.clone(),
        meteroid_mailer::service::mailer_service(MailerConfig::dummy()),
    )
    .await;

    let meteroid_auth = meteroid_it::svc_auth::login(meteroid_setup.channel.clone()).await;

    let meteroid_clients = meteroid_it::clients::AllClients::from_channel(
        meteroid_setup.channel.clone(),
        meteroid_auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    let store = Arc::new(meteroid_setup.store.clone());
    let store_clone = store.clone();

    let pgmq_handle = tokio::spawn(async move {
        tokio::join!(
            run_outbox_dispatch(store_clone.clone()),
            run_metric_sync(store_clone.clone(), metering_client_clone)
        );
    });

    let family = meteroid_clients
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

    let created_metric = meteroid_clients
        .metrics
        .clone()
        .create_billable_metric(api::billablemetrics::v1::CreateBillableMetricRequest {
            name: "best metric ever".to_string(),
            code: "api_calls".to_string(),
            description: None,
            aggregation: Some(api::billablemetrics::v1::Aggregation {
                aggregation_type: api::billablemetrics::v1::aggregation::AggregationType::Sum as i32,
                aggregation_key: Some("tokens".to_string()),
                unit_conversion: Some(api::billablemetrics::v1::aggregation::UnitConversion {
                    factor: 1.0,
                    rounding: api::billablemetrics::v1::aggregation::unit_conversion::UnitConversionRounding::Nearest as i32,
                }),
            }),
            segmentation_matrix: Some(api::billablemetrics::v1::SegmentationMatrix {
                matrix: Some(api::billablemetrics::v1::segmentation_matrix::Matrix::Single(
                    api::billablemetrics::v1::segmentation_matrix::SegmentationMatrixSingle{
                        dimension: Some(api::billablemetrics::v1::segmentation_matrix::Dimension {
                            key: "endpoint".to_string(),
                            values: vec!["inference".to_string(), "embedding".to_string()],
                        }),
                    }
                ))
            }),
            usage_group_key: Some("usage".to_string()),
            family_local_id: family.local_id.clone(),
            product_id: None,
        })
        .await
        .unwrap()
        .into_inner()
        .billable_metric
        .unwrap();

    let clickhouse_client = metering_it::clickhouse::get_client(ch_http_port);

    // todo remove me after the raw aggregation is tested enough
    wait_for_clichouse_meter(&created_metric.id, &clickhouse_client).await;

    let customer_1 = ids::CUST_SPOTIFY_ID;
    let customer_2 = ids::CUST_UBER_ID;

    let now = chrono::Utc::now();

    // period 2 started yesterday.
    // so after workers we expect a draft for period 2, and a finalized/issued for period 1
    // we will create the draft invoice for period 1 manually, as the draft worker only check last 7 days
    let period_2_start = now - chrono::Duration::days(1);
    let period_1_start = period_2_start.checked_sub_days(Days::new(20)).unwrap();

    // we consider a billing period 1, customer 1, inference endpoint
    let mut to_ingest = vec![
        Event {
            id: Uuid::now_v7().to_string(),
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
            id: Uuid::now_v7().to_string(),
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
            id: Uuid::now_v7().to_string(),
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
            id: Uuid::now_v7().to_string(),
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
            id: Uuid::now_v7().to_string(),
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
            id: Uuid::now_v7().to_string(),
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
            id: Uuid::now_v7().to_string(),
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
            id: Uuid::now_v7().to_string(),
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
            id: Uuid::now_v7().to_string(),
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

    // simulate duplicate events
    to_ingest.extend(to_ingest.clone());

    let to_ingest_len = to_ingest.len();

    // we ingest events in metering
    let ingested = metering_clients
        .events
        .ingest(Request::new(IngestRequest {
            events: to_ingest.clone(),
            allow_backfilling: true,
        }))
        .await
        .expect("Could not ingest events");

    let ingested = ingested.into_inner();

    assert_eq!(ingested.failures.len(), 0);

    log::info!("Validating raw clickhouse events...");
    let raw_events = get_eventually_raw_events(&clickhouse_client, to_ingest_len)
        .await
        .expect("Failed to validate raw events in ClickHouse");

    assert_raw_events_eq(&to_ingest, &raw_events);
    log::info!("Raw clickhouse events validated!");

    log::info!("Validating clickhouse usage data...");

    let usage_data = get_eventually_usage(
        BillableMetricId::from_proto(created_metric.id).unwrap(),
        &metering_client,
        store.clone(),
        Period {
            start: period_1_start.date_naive(),
            end: period_2_start.date_naive(),
        },
    )
    .await
    .unwrap();

    assert_eq!(usage_data.data.first().unwrap().value, Decimal::from(25249));

    log::info!("Clickhouse usage data validated!");

    pgmq_handle.abort()
}

fn assert_raw_events_eq(left: &[Event], right: &[RawEventRow]) {
    fn sort_by<T, F>(items: &[T], sort_fn: F) -> Vec<T>
    where
        T: Clone,
        F: Fn(&T) -> &str,
    {
        let mut vec: Vec<T> = items.to_vec();
        vec.sort_by(|a, b| sort_fn(a).cmp(sort_fn(b)));
        vec
    }

    let sorted_left = sort_by(left, |a| &a.id);
    let sorted_right = sort_by(right, |a| &a.id);

    assert_eq!(sorted_left.len(), sorted_right.len());

    for (event, right_event) in sorted_left.iter().zip(sorted_right.iter()) {
        assert_raw_event_eq(event, right_event);
    }
}

fn assert_raw_event_eq(left: &Event, right: &RawEventRow) {
    assert_eq!(left.id, right.id);
    assert_eq!(left.code, right.code);

    let left_customer_id = match left.customer_id.as_ref().unwrap() {
        event::CustomerId::MeteroidCustomerId(id) => id,
        _ => panic!("Unexpected customer_id type"),
    };

    assert_eq!(left_customer_id, &right.customer_id);
    assert_eq!(left.timestamp, right.timestamp.to_rfc3339());
    assert_eq!(
        left.properties
            .iter()
            .sorted_by(|a, b| a.0.cmp(b.0))
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect_vec(),
        right
            .properties
            .iter()
            .sorted_by(|a, b| a.0.cmp(&b.0))
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect_vec(),
    );
}

async fn wait_for_clichouse_meter(bm_id: &str, ch_client: &clickhouse::Client) {
    let view_name = get_meter_view_name(ids::TENANT_ID.to_string().as_str(), bm_id);
    let view_name = view_name.strip_prefix("meteroid.").unwrap();

    log::info!("Waiting for meter {} to be created...", view_name);
    (|| async {
        match ch_client
            .query(
                format!("SELECT count(*) FROM system.tables WHERE name = '{view_name}'").as_str(),
            )
            .fetch_one::<u64>()
            .await
        {
            Ok(cnt) => {
                if cnt != 1 {
                    Err(anyhow::anyhow!(
                        "Expected 1 but got {cnt} views for {view_name}"
                    ))
                } else {
                    Ok(())
                }
            }
            Err(e) => Err(anyhow::anyhow!(e)),
        }
    })
    .retry(
        backon::ConstantBuilder::default()
            .with_delay(Duration::from_millis(100))
            .with_max_times(50),
    )
    .await
    .expect("Timeout waiting for view in ClickHouse");

    // TODO for some reason the MVw is not immediately ready so give it some time before publishing events
    tokio::time::sleep(Duration::from_millis(1500)).await;

    log::info!("Meter {view_name} is ready in ClickHouse");
}

async fn get_eventually_raw_events(
    ch_client: &clickhouse::Client,
    expected_count: usize,
) -> anyhow::Result<Vec<RawEventRow>> {
    (|| async {
        match ch_client
            .query("SELECT * FROM raw_events")
            .fetch_all::<RawEventRow>()
            .await
        {
            Ok(vec) => {
                if vec.len() != expected_count {
                    Err(anyhow::anyhow!(
                        "Expected {expected_count} but got {} raw events",
                        vec.len()
                    ))
                } else {
                    Ok(vec)
                }
            }
            Err(e) => Err(anyhow::anyhow!(e)),
        }
    })
    .retry(
        backon::ConstantBuilder::default()
            .with_delay(Duration::from_millis(100))
            .with_max_times(60),
    )
    .await
}

async fn get_eventually_usage(
    metric_id: BillableMetricId,
    metering_client: &MeteringUsageClient,
    store: Arc<Store>,
    period: Period,
) -> anyhow::Result<UsageData> {
    let bm = &store
        .clone()
        .find_billable_metric_by_id(metric_id, ids::TENANT_ID)
        .await
        .unwrap();

    let period_start = period.start;
    let period_end = period.end;

    (|| async {
        let usage = metering_client
            .fetch_usage(
                &ids::TENANT_ID,
                &ids::CUST_SPOTIFY_ID,
                bm,
                Period {
                    start: period_start,
                    end: period_end,
                },
            )
            .await
            .unwrap();

        if usage.data.len() != 1 {
            Err(anyhow::anyhow!(
                "Expected 1 (or TEMPORARY 0) usage records but got {}",
                usage.data.len()
            ))
        } else {
            Ok(usage)
        }
    })
    .retry(
        backon::ConstantBuilder::default()
            .with_delay(Duration::from_millis(100))
            .with_max_times(50),
    )
    .await
}
