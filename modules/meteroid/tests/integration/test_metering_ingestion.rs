use crate::data::ids;
use crate::metering_it;
use crate::{helpers, meteroid_it};
use backon::Retryable;
use chrono::Days;
use itertools::Itertools;
use metering::ingest::domain::RawEventRow;
use metering_grpc::meteroid::metering::v1::{Event, IngestRequest, event};
use meteroid::clients::usage::MeteringUsageClient;
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

    let _meteroid_setup = meteroid_it::container::start_meteroid_with_port(
        meteroid_port,
        metering_port,
        postgres_connection_string,
        meteroid_it::container::SeedLevel::PRODUCT,
        Arc::new(metering_client),
    )
    .await;

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

    let clickhouse_client = metering_it::clickhouse::get_client(clickhouse_port);

    let events = (|| async {
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

    assert_events_eq(&to_ingest, &events);
}

fn assert_events_eq(left: &[Event], right: &[RawEventRow]) {
    fn sort_by<T, F>(items: &[T], sort_fn: F) -> Vec<T>
    where
        T: Clone,
        F: Fn(&T) -> &str,
    {
        let mut vec: Vec<T> = items.iter().cloned().collect();
        vec.sort_by(|a, b| sort_fn(a).cmp(sort_fn(b)));
        vec
    }

    let sorted_left = sort_by(left, |a| &a.id);
    let sorted_right = sort_by(right, |a| &a.id);

    assert_eq!(sorted_left.len(), sorted_right.len());

    for (event, right_event) in sorted_left.iter().zip(sorted_right.iter()) {
        assert_event_eq(event, right_event);
    }
}

fn assert_event_eq(left: &Event, right: &RawEventRow) {
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
            .sorted_by(|a, b| a.0.cmp(&b.0))
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
