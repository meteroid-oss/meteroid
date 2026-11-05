use crate::data::ids;
use crate::metering_it;
use crate::metering_it::clients::TestLayeredClientService;
use crate::{helpers, meteroid_it};
use backon::Retryable;
use chrono::{Days, NaiveTime};
use common_domain::ids::{BillableMetricId, TenantId};
use itertools::Itertools;
use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;
use metering_grpc::meteroid::metering::v1::{Event, IngestRequest, QueryRawEventsRequest, event};
use meteroid::clients::usage::MeteringUsageClient;
use meteroid_grpc::meteroid::api;
use meteroid_mailer::config::MailerConfig;
use meteroid_store::Store;
use meteroid_store::clients::pending_usage::{
    OriginFilter, PendingUsageRequest, PendingUsageSource,
};
use meteroid_store::clients::usage::{EventSearchOptions, UsageClient, UsageData};
use meteroid_store::domain::{Period, UsagePeriod};
use meteroid_store::repositories::TenantInterface;
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

    let postgres_connection_string = meteroid_it::container::create_seeded_test_database(
        meteroid_it::container::SeedLevel::PRODUCT,
    )
    .await;

    let ((_kafka_container, kafka_port), (_clickhouse_container, ch_http_port, ch_tcp_port)) =
        metering_it::container::start_kafka_and_clickhouse().await;

    metering_it::kafka::create_topic(kafka_port, "meteroid-events-raw")
        .await
        .expect("Could not create topic");

    // we start meteroid and metering

    // Hold the meteroid gRPC listener from the moment its port is chosen so the
    // port can't be stolen by a parallel test before meteroid binds it. Its port
    // is cross-wired into the metering config below.
    let meteroid_grpc_listener = meteroid_it::container::bind_meteroid_grpc().await;
    let meteroid_port = meteroid_grpc_listener
        .local_addr()
        .expect("Could not get meteroid gRPC addr")
        .port();
    let metering_listener = metering_it::container::bind_metering_grpc().await;
    let metering_port = metering_listener.local_addr().unwrap().port();

    let metering_config = metering_it::config::mocked_config(
        meteroid_port,
        metering_port,
        ch_http_port,
        ch_tcp_port,
        kafka_port,
        "meteroid-events-raw".to_string(),
    );

    let metering_setup =
        metering_it::container::start_metering(metering_config.clone(), metering_listener).await;

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

    let meteroid_setup = meteroid_it::container::start_meteroid_with_grpc_listener(
        meteroid_grpc_listener,
        metering_port,
        postgres_connection_string,
        meteroid_it::container::SeedLevel::PRODUCT,
        metering_client.clone(),
        meteroid_mailer::service::mailer_service(MailerConfig::dummy()),
    )
    .await;

    metering_client.bind_tenant_lookup(meteroid_setup.store.keep_last_events_lookup());

    let meteroid_auth = meteroid_it::svc_auth::login(meteroid_setup.channel.clone()).await;

    let meteroid_clients = meteroid_it::clients::AllClients::from_channel(
        meteroid_setup.channel.clone(),
        meteroid_auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    let store = Arc::new(meteroid_setup.store.clone());

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
            unit_label: None,
            family_local_id: family.local_id.clone(),
            product_id: None,
            filters: vec![],
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

    // Usage periods are dates widened to midnight, so this instant is both period 1's end and
    // period 2's start.
    let period_boundary = period_2_start
        .date_naive()
        .and_time(NaiveTime::MIN)
        .and_utc();

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
        // exactly on the period boundary: period 1 ends where period 2 starts, so this belongs
        // to period 2 only. An inclusive upper bound would rate it on both invoices.
        Event {
            id: Uuid::now_v7().to_string(),
            code: "api_calls".to_string(),
            customer_id: Some(event::CustomerId::MeteroidCustomerId(
                customer_1.to_string(),
            )),
            timestamp: period_boundary.to_rfc3339(),
            properties: [
                ("endpoint".to_string(), "inference".to_string()),
                ("model".to_string(), "gpt_3.5".to_string()),
                ("tokens".to_string(), "500".to_string()),
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
        BillableMetricId::from_proto(&created_metric.id).unwrap(),
        &metering_client,
        store.clone(),
        Period {
            start: period_1_start.date_naive(),
            end: period_2_start.date_naive(),
        },
    )
    .await
    .unwrap();

    // 20 + 150 + 70 + 9 gpt_3.5 + 25000 gpt_4. The 500-token event sitting exactly on
    // period_2_start is excluded: the upper bound is exclusive, so it rates in period 2 only.
    assert_eq!(usage_data.data.first().unwrap().value, Decimal::from(25249));

    let period_2_usage = get_eventually_usage(
        BillableMetricId::from_proto(&created_metric.id).unwrap(),
        &metering_client,
        store.clone(),
        Period {
            start: period_2_start.date_naive(),
            end: period_2_start.date_naive() + chrono::Duration::days(1),
        },
    )
    .await
    .unwrap();

    // The boundary event (500) plus the one ingested at period_2_start itself (25000).
    assert_eq!(
        period_2_usage.data.first().unwrap().value,
        Decimal::from(25500)
    );

    log::info!("Clickhouse usage data validated!");

    // ---- Activity signal: fired by the events just ingested through Kafka into ClickHouse ----
    // A materialized view writes it within the events' INSERT, as a part committed after theirs:
    // the events can be read before their activity.
    let seen_since = now - chrono::Duration::hours(1);
    let mut expected = vec![
        (ids::TENANT_ID.to_string(), customer_1.to_string()),
        (ids::TENANT_ID.to_string(), customer_2.to_string()),
    ];
    expected.sort();
    let active = (|| async {
        let rows = metering_client
            .poll_usage_activity(seen_since, None, 100)
            .await
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        assert!(
            rows.iter().all(|r| r.min_event_at <= r.last_seen_at),
            "the oldest event of a customer is never after its latest activity"
        );
        let mut active = rows
            .into_iter()
            .map(|r| (r.tenant_id.to_string(), r.customer_id.to_string()))
            .collect::<Vec<_>>();
        active.sort();
        if active.len() < expected.len() {
            Err(anyhow::anyhow!(
                "{} of {} active customers",
                active.len(),
                expected.len()
            ))
        } else {
            Ok(active)
        }
    })
    .retry(
        backon::ConstantBuilder::default()
            .with_delay(Duration::from_millis(100))
            .with_max_times(50),
    )
    .await
    .unwrap();
    assert_eq!(active, expected, "each active customer appears once");
    let later = metering_client
        .poll_usage_activity(now + chrono::Duration::hours(1), None, 100)
        .await
        .unwrap();
    assert!(later.is_empty(), "nothing is active after the last event");

    // ---- Usage since a base, for a batch of customers, equals the per-customer period query ----
    let metric = store
        .find_billable_metric_by_id(
            BillableMetricId::from_proto(&created_metric.id).unwrap(),
            ids::TENANT_ID,
        )
        .await
        .unwrap();
    let batch = metering_client
        .usage_since(PendingUsageRequest {
            tenant_id: &ids::TENANT_ID,
            keep_last_events: store.keep_last_events(ids::TENANT_ID).await.unwrap(),
            customer_ids: &[customer_1, customer_2],
            metric: &metric,
            group_by: None,
            since: period_1_start.date_naive().and_time(NaiveTime::MIN),
            until: period_2_start.date_naive().and_time(NaiveTime::MIN),
            origin: OriginFilter::All,
        })
        .await
        .unwrap();
    let total = |customer| {
        batch
            .iter()
            .filter(|u| u.customer_id == customer)
            .flat_map(|u| u.groups.iter())
            .map(|g| g.value)
            .sum::<Decimal>()
    };
    assert_eq!(total(customer_1), Decimal::from(25249));
    assert_eq!(total(customer_2), Decimal::from(25000));

    // ---- Pre-aggregation property filters must change the rated sum end-to-end ----
    // Reuse the already-ingested events: customer_1 has four gpt_3.5 api_calls in period
    // (20+150+70+9 = 249) plus one gpt_4 event of 25000 that dominates the 25249 total above.
    // Updating the metric's `filters` exercises the whole path — gRPC validation + mapping,
    // JSONB persistence + read-back, the UI→engine operator collapse, and the ClickHouse SQL.
    use api::billablemetrics::v1::{
        MetricFilter, MetricFilterList, UpdateBillableMetricRequest, metric_filter::Operator,
    };

    let filter_on_model = |op: Operator| UpdateBillableMetricRequest {
        id: created_metric.id.clone(),
        name: None,
        description: None,
        unit_conversion: None,
        segmentation_matrix_values: None,
        filters: Some(MetricFilterList {
            filters: vec![MetricFilter {
                property: "model".to_string(),
                op: op as i32,
                values: vec!["gpt_4".to_string()],
            }],
        }),
        unit_label: None,
    };

    let period = Period {
        start: period_1_start.date_naive(),
        end: period_2_start.date_naive(),
    };

    // NOT_IN gpt_4: presence-required negation drops the single gpt_4 event.
    meteroid_clients
        .metrics
        .clone()
        .update_billable_metric(filter_on_model(Operator::NotIn))
        .await
        .unwrap();

    let filtered = get_eventually_usage(
        BillableMetricId::from_proto(&created_metric.id).unwrap(),
        &metering_client,
        store.clone(),
        period.clone(),
    )
    .await
    .unwrap();
    assert_eq!(filtered.data.first().unwrap().value, Decimal::from(249));

    // IN gpt_4: keeps only the gpt_4 event → its lone 25000.
    meteroid_clients
        .metrics
        .clone()
        .update_billable_metric(filter_on_model(Operator::In))
        .await
        .unwrap();

    let only_gpt4 = get_eventually_usage(
        BillableMetricId::from_proto(&created_metric.id).unwrap(),
        &metering_client,
        store.clone(),
        period,
    )
    .await
    .unwrap();
    assert_eq!(only_gpt4.data.first().unwrap().value, Decimal::from(25000));

    log::info!("Clickhouse filtered usage data validated!");

    // ---- The first copy of an id wins, keep-last tenants store every copy ----
    let topic = "meteroid-events-raw";
    let mut moved = unique_events[0].clone();
    moved.timestamp = (period_2_start + chrono::Duration::hours(1)).to_rfc3339();
    moved
        .properties
        .insert("tokens".to_string(), "1".to_string());
    ingest(&mut metering_clients, vec![moved.clone()]).await;
    metering_it::kafka::wait_until_consumed(kafka_port, topic)
        .await
        .unwrap();

    assert_eq!(
        stored_copies(ch_http_port, Some(&moved.id)).await,
        1,
        "a re-send with a moved timestamp is not stored again"
    );
    assert_eq!(
        stored_copies(ch_http_port, None).await,
        unique_events.len() as u64,
        "copies sent in one request are stored once"
    );

    // 500 ids of 765 bytes: one unchunked lookup would exceed ClickHouse's max_query_size.
    let long_ids: Vec<Event> = (0..500)
        .map(|i| {
            let mut event = unique_events[1].clone();
            event.id = format!("{i:03}{}", "€".repeat(252));
            event.code = "long_ids".to_string();
            event
        })
        .collect();
    ingest(&mut metering_clients, long_ids.clone()).await;
    metering_it::kafka::wait_until_consumed(kafka_port, topic)
        .await
        .unwrap();
    ingest(&mut metering_clients, long_ids).await;
    metering_it::kafka::wait_until_consumed(kafka_port, topic)
        .await
        .unwrap();
    assert_eq!(
        stored_with_code(ch_http_port, "long_ids").await,
        500,
        "long ids are looked up and their re-sends dropped"
    );

    // No longer offered over the API: a tenant that had it keeps it.
    meteroid_setup
        .store
        .update_tenant(
            common_domain::actor::Actor::System,
            meteroid_store::domain::TenantUpdate {
                keep_last_events: Some(true),
                ..Default::default()
            },
            ids::ORGANIZATION_ID,
            ids::TENANT_ID,
        )
        .await
        .unwrap();

    // A new key: the seeded one is cached by metering with the flag off.
    let keep_last_key = meteroid_clients
        .api_tokens
        .clone()
        .create_api_token(api::apitokens::v1::CreateApiTokenRequest {
            name: "keep-last".to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .api_key;
    let mut keep_last_clients = metering_it::clients::AllClients::from_channel(
        metering_setup.channel.clone(),
        &keep_last_key,
        &metering_config.internal_auth,
    );

    let copy = |tokens: &str| {
        let mut event = unique_events[1].clone();
        event.id = "keep-last-id".to_string();
        event
            .properties
            .insert("tokens".to_string(), tokens.to_string());
        event
    };
    ingest(&mut keep_last_clients, vec![copy("1"), copy("2")]).await;
    let mut moved_copy = copy("3");
    moved_copy.timestamp = (period_2_start + chrono::Duration::hours(1)).to_rfc3339();
    ingest(&mut keep_last_clients, vec![moved_copy]).await;
    metering_it::kafka::wait_until_consumed(kafka_port, topic)
        .await
        .unwrap();

    assert_eq!(
        stored_copies(ch_http_port, Some("keep-last-id")).await,
        3,
        "a keep-last tenant stores every copy"
    );

    let rest_copy = |tokens: &str| {
        let mut properties = unique_events[1].properties.clone();
        properties.insert("tokens".to_string(), tokens.to_string());
        serde_json::json!({
            "event_id": "keep-last-rest-id",
            "code": unique_events[1].code,
            "customer_id": customer_1.to_string(),
            "timestamp": unique_events[1].timestamp,
            "properties": properties,
        })
    };
    let response = http_client_tracing::client()
        .post(format!(
            "{}/api/v1/events/ingest",
            meteroid_setup.config.rest_api_external_url
        ))
        .bearer_auth(&keep_last_key)
        .json(&serde_json::json!({
            "events": [rest_copy("5"), rest_copy("7")],
            "allow_backfilling": true,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    metering_it::kafka::wait_until_consumed(kafka_port, topic)
        .await
        .unwrap();
    assert_eq!(
        stored_copies(ch_http_port, Some("keep-last-rest-id")).await,
        2,
        "the REST API carries the tenant's keep-last flag"
    );

    // ---- A keep-last tenant reads the last copy ingested within the window ----
    meteroid_clients
        .metrics
        .clone()
        .update_billable_metric(filter_on_model(Operator::NotIn))
        .await
        .unwrap();
    let usage_data = get_eventually_usage(
        BillableMetricId::from_proto(&created_metric.id).unwrap(),
        &metering_client,
        store.clone(),
        Period {
            start: period_1_start.date_naive(),
            end: period_2_start.date_naive(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        usage_data.data.first().unwrap().value,
        Decimal::from(249 + 2 + 7),
        "copy 2 wins over copy 1 at the same timestamp, the moved copy 3 rates in period 2, \
         the last REST copy wins"
    );
    let metric = store
        .find_billable_metric_by_id(
            BillableMetricId::from_proto(&created_metric.id).unwrap(),
            ids::TENANT_ID,
        )
        .await
        .unwrap();
    let pending = |keep_last_events| {
        let metric = &metric;
        let metering_client = &metering_client;
        async move {
            metering_client
                .usage_since(PendingUsageRequest {
                    tenant_id: &ids::TENANT_ID,
                    keep_last_events,
                    customer_ids: &[customer_1],
                    metric,
                    group_by: None,
                    since: period_1_start.date_naive().and_time(NaiveTime::MIN),
                    until: period_2_start.date_naive().and_time(NaiveTime::MIN),
                    origin: OriginFilter::All,
                })
                .await
                .unwrap()
                .iter()
                .flat_map(|usage| usage.groups.iter())
                .map(|group| group.value)
                .sum::<Decimal>()
        }
    };
    assert_eq!(
        pending(true).await,
        Decimal::from(249 + 2 + 7),
        "the credit engine's batch read carries the tenant's keep-last flag"
    );
    assert_ne!(
        pending(false).await,
        Decimal::from(249 + 2 + 7),
        "the flag in the request is what metering reads by"
    );

    let found = metering_client
        .search_events(
            &ids::TENANT_ID,
            EventSearchOptions {
                from: Some(prost_types::Timestamp {
                    seconds: period_1_start.timestamp(),
                    nanos: 0,
                }),
                to: Some(prost_types::Timestamp {
                    seconds: period_2_start.timestamp(),
                    nanos: 0,
                }),
                limit: 10,
                offset: 0,
                search: Some("keep-last-id".to_string()),
                event_codes: vec![],
                customer_ids: vec![],
                filters: vec![],
                sort_order: 0,
            },
        )
        .await
        .unwrap();
    let tokens = found
        .events
        .iter()
        .map(|e| e.properties["tokens"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        tokens,
        vec!["2"],
        "the explorer shows the last copy ingested"
    );
}

async fn ingest(clients: &mut metering_it::clients::AllClients, events: Vec<Event>) {
    let response = clients
        .events
        .ingest(Request::new(IngestRequest {
            events,
            allow_backfilling: true,
        }))
        .await
        .expect("Could not ingest events")
        .into_inner();
    assert!(response.failures.is_empty(), "{:?}", response.failures);
}

async fn stored_copies(port: metering_it::clickhouse::HttpPort, id: Option<&str>) -> u64 {
    metering_it::clickhouse::get_client(port)
        .query("SELECT count() FROM raw_events_v2 WHERE tenant_id = ? AND (? = '' OR id = ?)")
        .bind((*ids::TENANT_ID).to_string())
        .bind(id.unwrap_or_default())
        .bind(id.unwrap_or_default())
        .fetch_one::<u64>()
        .await
        .unwrap()
}

async fn stored_with_code(port: metering_it::clickhouse::HttpPort, code: &str) -> u64 {
    metering_it::clickhouse::get_client(port)
        .query("SELECT count() FROM raw_events_v2 WHERE tenant_id = ? AND code = ?")
        .bind((*ids::TENANT_ID).to_string())
        .bind(code)
        .fetch_one::<u64>()
        .await
        .unwrap()
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
                filters: vec![],
                after_timestamp_nanos: None,
                after_id: None,
                keep_last_events: false,
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
                UsagePeriod {
                    start: period_start.and_time(NaiveTime::MIN),
                    end: period_end.and_time(NaiveTime::MIN),
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

/// A broker whose host port is taken between being picked and Docker binding it starts on the
/// next port instead of failing the test.
#[tokio::test]
async fn kafka_starts_on_the_next_port_when_its_port_is_taken() {
    helpers::init::logging();
    let taken = std::net::TcpListener::bind("0.0.0.0:0").expect("a port to take");
    let taken_port = taken.local_addr().expect("the taken port").port();
    let free = helpers::network::free_local_port().expect("a free port");

    let (_kafka, port) = metering_it::container::start_kafka_on([taken_port, free])
        .await
        .expect("Kafka starts on the next port");

    assert_eq!(port, free);
    metering_it::kafka::create_topic(port, "port-retry")
        .await
        .expect("the broker answers on its port");
    drop(taken);
}

/// The second metering server of a process once migrated with the first one's config: its
/// ClickHouse consumed the first server's topic in the same consumer group, taking its events.
#[tokio::test]
async fn metering_servers_in_one_process_each_migrate_against_their_own_broker() {
    helpers::init::logging();
    let ((_a, a_http, a_tcp), (_b, b_http, b_tcp)) = tokio::join!(
        metering_it::container::start_clickhouse(),
        metering_it::container::start_clickhouse()
    );
    let a = metering_it::config::mocked_config(1, 1, a_http, a_tcp, 11, "events-a".to_owned());
    let b = metering_it::config::mocked_config(1, 1, b_http, b_tcp, 22, "events-b".to_owned());
    metering::migrations::run(&a.clickhouse, &a.kafka)
        .await
        .unwrap();
    metering::migrations::run(&b.clickhouse, &b.kafka)
        .await
        .unwrap();
    let v1_checksum = |port| async move {
        metering_it::clickhouse::get_client(port)
            .query("SELECT checksum FROM refinery_schema_history WHERE version = 1")
            .fetch_one::<String>()
            .await
            .unwrap()
    };
    assert_ne!(
        v1_checksum(a_http).await,
        v1_checksum(b_http).await,
        "B's Kafka engine is created on B's broker and topic"
    );
}
