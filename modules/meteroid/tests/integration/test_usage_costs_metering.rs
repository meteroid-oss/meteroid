//! The usage-costs reads against a real metering stack: bucketed series and keyset-paged events.
use crate::data::ids::{CUST_SPOTIFY_ID, CUST_UBER_ID, PRODUCT_FAMILY_ID, TENANT_ID};
use crate::{helpers, metering_it, meteroid_it};
use backon::Retryable;
use chrono::{Duration, NaiveDateTime, NaiveTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::CustomerId;
use metering_grpc::meteroid::metering::v1::{Event, IngestRequest, event};
use meteroid::clients::usage::MeteringUsageClient;
use meteroid_mailer::config::MailerConfig;
use meteroid_store::clients::usage::{
    MetricEvent, MetricEventCursor, TenantUsageGrain, UsageClient,
};
use meteroid_store::domain::enums::BillingMetricAggregateEnum;
use meteroid_store::domain::{BillableMetric, BillableMetricNew, UsagePeriod};
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::BTreeMap;
use std::sync::Arc;
use tonic::Request;
use uuid::Uuid;

const CODE: &str = "cost_calls";

fn event_at(
    n: u128,
    customer: CustomerId,
    code: &str,
    at: NaiveDateTime,
    region: &str,
    tokens: &str,
) -> Event {
    Event {
        id: Uuid::from_u128(n).to_string(),
        code: code.to_owned(),
        customer_id: Some(event::CustomerId::MeteroidCustomerId(customer.to_string())),
        timestamp: at.and_utc().to_rfc3339(),
        properties: [
            ("region".to_owned(), region.to_owned()),
            ("tokens".to_owned(), tokens.to_owned()),
        ]
        .into(),
    }
}

async fn eventually_events(
    client: &MeteringUsageClient,
    metric: &BillableMetric,
    period: &UsagePeriod,
    expected: usize,
) -> Vec<MetricEvent> {
    (|| async {
        let events = client
            .list_metric_events(
                &TENANT_ID,
                &CUST_SPOTIFY_ID,
                metric,
                period.clone(),
                None,
                1000,
            )
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        if events.len() == expected {
            Ok(events)
        } else {
            Err(anyhow::anyhow!("{} of {expected} events", events.len()))
        }
    })
    .retry(
        backon::ConstantBuilder::default()
            .with_delay(std::time::Duration::from_millis(200))
            .with_max_times(100),
    )
    .await
    .expect("the events reach the store")
}

#[tokio::test]
async fn test_usage_series_and_event_paging_over_clickhouse() {
    helpers::init::logging();
    let postgres = meteroid_it::container::create_seeded_test_database(
        meteroid_it::container::SeedLevel::PRODUCT,
    )
    .await;
    let ((_kafka, kafka_port), (_clickhouse, ch_http_port, ch_tcp_port)) =
        metering_it::container::start_kafka_and_clickhouse().await;
    metering_it::kafka::create_topic(kafka_port, "meteroid-events-raw")
        .await
        .expect("Could not create topic");

    let meteroid_listener = meteroid_it::container::bind_meteroid_grpc().await;
    let meteroid_port = meteroid_listener.local_addr().unwrap().port();
    let metering_listener = metering_it::container::bind_metering_grpc().await;
    let metering_port = metering_listener.local_addr().unwrap().port();
    let metering_config = metering_it::config::mocked_config(
        meteroid_port,
        metering_port,
        ch_http_port,
        ch_tcp_port,
        kafka_port,
        "meteroid-events-raw".to_owned(),
    );
    let metering =
        metering_it::container::start_metering(metering_config.clone(), metering_listener).await;
    let mut ingest_clients = metering_it::clients::AllClients::from_channel(
        metering.channel.clone(),
        "pv_sand_9XzHg0EYO2Usy9ITU6bbhBnkYYbx/2vO7XtUUeQ7Wq9EZCAbBG",
        &metering_config.internal_auth,
    );
    let usage_client = Arc::new(MeteringUsageClient::from_channel(
        metering.channel.clone(),
        &metering_config.internal_auth,
    ));
    let meteroid = meteroid_it::container::start_meteroid_with_grpc_listener(
        meteroid_listener,
        metering_port,
        postgres,
        meteroid_it::container::SeedLevel::PRODUCT,
        usage_client.clone(),
        meteroid_mailer::service::mailer_service(MailerConfig::dummy()),
    )
    .await;

    let metric = meteroid
        .store
        .insert_billable_metric(
            Actor::System,
            BillableMetricNew {
                name: "Cost calls".to_owned(),
                description: None,
                code: CODE.to_owned(),
                aggregation_type: BillingMetricAggregateEnum::Sum,
                aggregation_key: Some("tokens".to_owned()),
                unit_conversion_factor: None,
                unit_conversion_rounding: None,
                segmentation_matrix: None,
                filters: None,
                usage_group_key: None,
                unit_label: None,
                tenant_id: TENANT_ID,
                product_family_id: PRODUCT_FAMILY_ID,
                product_id: None,
            },
        )
        .await
        .unwrap();

    let day0 = (Utc::now().date_naive() - Duration::days(6)).and_time(NaiveTime::MIN);
    let at = |days: i64, hours: i64, minutes: i64| {
        day0 + Duration::days(days) + Duration::hours(hours) + Duration::minutes(minutes)
    };
    let range_end = at(3, 0, 0);
    let events = vec![
        event_at(1, CUST_SPOTIFY_ID, CODE, at(0, 1, 10), "eu", "10.5"),
        event_at(2, CUST_SPOTIFY_ID, CODE, at(0, 1, 40), "us", "20"),
        event_at(3, CUST_SPOTIFY_ID, CODE, at(0, 5, 0), "eu", "1"),
        event_at(4, CUST_SPOTIFY_ID, CODE, at(1, 23, 59), "eu", "7"),
        // Three events share one instant, so paging must tie-break on the id.
        event_at(5, CUST_SPOTIFY_ID, CODE, at(2, 8, 0), "us", "100"),
        event_at(6, CUST_SPOTIFY_ID, CODE, at(2, 8, 0), "us", "200"),
        event_at(7, CUST_SPOTIFY_ID, CODE, at(2, 8, 0), "eu", "300"),
        // Outside the range, of another customer, and of another metric.
        event_at(8, CUST_SPOTIFY_ID, CODE, range_end, "eu", "9000"),
        event_at(9, CUST_UBER_ID, CODE, at(0, 1, 10), "eu", "9000"),
        event_at(
            10,
            CUST_SPOTIFY_ID,
            "other_code",
            at(0, 1, 10),
            "eu",
            "9000",
        ),
    ];
    let failures = ingest_clients
        .events
        .ingest(Request::new(IngestRequest {
            events,
            allow_backfilling: true,
        }))
        .await
        .expect("Could not ingest events")
        .into_inner()
        .failures;
    assert!(failures.is_empty());

    let period = UsagePeriod {
        start: day0,
        end: range_end,
    };
    let stored = eventually_events(&usage_client, &metric, &period, 7).await;

    let series = |grain, group_by: Option<&'static str>, period: UsagePeriod| {
        let usage_client = usage_client.clone();
        let metric = metric.clone();
        async move {
            let mut points: BTreeMap<(NaiveDateTime, Option<String>), Decimal> = BTreeMap::new();
            for point in usage_client
                .fetch_usage_series(
                    &TENANT_ID,
                    &CUST_SPOTIFY_ID,
                    &metric,
                    group_by,
                    grain,
                    period,
                )
                .await
                .unwrap()
            {
                let group = group_by.and_then(|key| point.dimensions.get(key).cloned());
                assert!(
                    points
                        .insert((point.window_start, group), point.value)
                        .is_none(),
                    "one point per bucket and group"
                );
            }
            points
        }
    };

    let daily = series(TenantUsageGrain::Day, None, period.clone()).await;
    assert_eq!(
        daily,
        BTreeMap::from([
            ((at(0, 0, 0), None), dec!(31.5)),
            ((at(1, 0, 0), None), dec!(7)),
            ((at(2, 0, 0), None), dec!(600)),
        ]),
        "the event at the end of the range is left out"
    );

    let daily_by_region = series(TenantUsageGrain::Day, Some("region"), period.clone()).await;
    assert_eq!(
        daily_by_region,
        BTreeMap::from([
            ((at(0, 0, 0), Some("eu".to_owned())), dec!(11.5)),
            ((at(0, 0, 0), Some("us".to_owned())), dec!(20)),
            ((at(1, 0, 0), Some("eu".to_owned())), dec!(7)),
            ((at(2, 0, 0), Some("eu".to_owned())), dec!(300)),
            ((at(2, 0, 0), Some("us".to_owned())), dec!(300)),
        ])
    );

    let hourly = series(TenantUsageGrain::Hour, None, period.clone()).await;
    assert_eq!(
        hourly,
        BTreeMap::from([
            ((at(0, 1, 0), None), dec!(30.5)),
            ((at(0, 5, 0), None), dec!(1)),
            ((at(1, 23, 0), None), dec!(7)),
            ((at(2, 8, 0), None), dec!(600)),
        ])
    );

    let mid_day = series(
        TenantUsageGrain::Hour,
        None,
        UsagePeriod {
            start: at(0, 1, 30),
            end: at(0, 6, 0),
        },
    )
    .await;
    assert_eq!(
        mid_day,
        BTreeMap::from([
            ((at(0, 1, 0), None), dec!(20)),
            ((at(0, 5, 0), None), dec!(1))
        ]),
        "a range starting mid-hour counts only the events inside it"
    );

    let expected_order: Vec<String> = (1..=7).map(|n| Uuid::from_u128(n).to_string()).collect();
    assert_eq!(
        stored.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        expected_order,
        "oldest first, ties broken by id"
    );

    for page_size in [1_u32, 2, 3, 7] {
        let mut seen = Vec::new();
        let mut after: Option<MetricEventCursor> = None;
        loop {
            let page = usage_client
                .list_metric_events(
                    &TENANT_ID,
                    &CUST_SPOTIFY_ID,
                    &metric,
                    period.clone(),
                    after.clone(),
                    page_size,
                )
                .await
                .unwrap();
            let Some(last) = page.last() else { break };
            after = Some(MetricEventCursor {
                timestamp_nanos: last.timestamp.and_utc().timestamp_nanos_opt().unwrap(),
                id: last.id.clone(),
            });
            seen.extend(page.iter().map(|e| e.id.clone()));
        }
        assert_eq!(seen, expected_order, "page size {page_size}");
    }
}
