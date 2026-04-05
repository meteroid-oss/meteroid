use crate::data::ids;
use crate::metering_it;
use crate::metering_it::clients::TestLayeredClientService;
use crate::{helpers, meteroid_it};
use backon::Retryable;
use chrono::Days;
use common_domain::ids::{BillableMetricId, TenantId};
use itertools::Itertools;
use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;
use metering_grpc::meteroid::metering::v1::{Event, IngestRequest, QueryRawEventsRequest, event};
use meteroid::clients::usage::MeteringUsageClient;
use meteroid::workers::pgmq::processors::run_outbox_dispatch;
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
        tokio::join!(run_outbox_dispatch(store_clone.clone()),);
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

    let unique_events = to_ingest.clone();

    // simulate duplicate events
    to_ingest.extend(to_ingest.clone());

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

    log::info!("Validating raw events via gRPC...");
    let grpc_events = get_eventually_grpc_raw_events(
        metering_clients.usage.clone(),
        ids::TENANT_ID,
        period_1_start - chrono::Duration::days(1),
        now + chrono::Duration::days(1),
        unique_events.len(),
    )
    .await
    .expect("Failed to validate raw events via gRPC");

    assert_raw_events_eq(&unique_events, &grpc_events);
    log::info!("Raw events via gRPC validated!");

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

fn assert_raw_events_eq(ingested: &[Event], returned: &[Event]) {
    let mut sorted_ingested = ingested.to_vec();
    sorted_ingested.sort_by(|a, b| a.id.cmp(&b.id));
    let mut sorted_returned = returned.to_vec();
    sorted_returned.sort_by(|a, b| a.id.cmp(&b.id));

    assert_eq!(sorted_ingested.len(), sorted_returned.len());

    for (left, right) in sorted_ingested.iter().zip(sorted_returned.iter()) {
        assert_eq!(left.id, right.id);
        assert_eq!(left.code, right.code);
        assert_eq!(left.customer_id, right.customer_id);
        assert_eq!(
            left.properties
                .iter()
                .sorted_by_key(|(k, _)| k.as_str())
                .collect_vec(),
            right
                .properties
                .iter()
                .sorted_by_key(|(k, _)| k.as_str())
                .collect_vec(),
        );
    }
}

async fn get_eventually_grpc_raw_events(
    client: UsageQueryServiceClient<TestLayeredClientService>,
    tenant_id: TenantId,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    expected_count: usize,
) -> anyhow::Result<Vec<Event>> {
    (|| async {
        let response = client
            .clone()
            .query_raw_events(Request::new(QueryRawEventsRequest {
                tenant_id: tenant_id.as_proto(),
                from: Some(prost_types::Timestamp {
                    seconds: from.timestamp(),
                    nanos: 0,
                }),
                to: Some(prost_types::Timestamp {
                    seconds: to.timestamp(),
                    nanos: 0,
                }),
                limit: 1000,
                offset: 0,
                search: None,
                event_codes: vec![],
                customer_ids: vec![],
                sort_order: 0,
            }))
            .await
            .map_err(|e| anyhow::anyhow!(e))?
            .into_inner();

        if response.events.len() != expected_count {
            Err(anyhow::anyhow!(
                "Expected {expected_count} but got {} raw events",
                response.events.len()
            ))
        } else {
            Ok(response.events)
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
