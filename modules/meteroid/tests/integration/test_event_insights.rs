//! EventInsightsService over a real metering stack: metric usage as billing reads it (filters,
//! unit conversion), customer top-N, and a customer's usage.
use crate::data::ids::{
    CUST_COMODO_ID, CUST_SPOTIFY_ID, CUST_UBER_ID, PRODUCT_FAMILY_ID, TENANT_ID,
};
use crate::{helpers, metering_it, meteroid_it};
use backon::Retryable;
use chrono::{Duration, NaiveDateTime, NaiveTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::CustomerId;
use metering_grpc::meteroid::metering::v1::{Event, IngestRequest, event};
use meteroid::clients::usage::MeteringUsageClient;
use meteroid_grpc::meteroid::api::billablemetrics::v1 as metrics;
use meteroid_grpc::meteroid::api::eventinsights::v1 as grpc;
use meteroid_grpc::meteroid::api::eventinsights::v1::query_metric_usage_request::{
    Breakdown, Grain, Metric,
};
use meteroid_mailer::config::MailerConfig;
use meteroid_store::domain::BillableMetricNew;
use meteroid_store::domain::billable_metrics::{MetricFilter, MetricFilterOp, MetricFilters};
use meteroid_store::domain::enums::BillingMetricAggregateEnum;
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::Request;
use uuid::Uuid;

const CODE: &str = "insight_calls";

fn event_at(n: u128, customer: CustomerId, at: NaiveDateTime, region: &str, tokens: &str) -> Event {
    Event {
        id: Uuid::from_u128(n).to_string(),
        code: CODE.to_owned(),
        customer_id: Some(event::CustomerId::MeteroidCustomerId(customer.to_string())),
        timestamp: at.and_utc().to_rfc3339(),
        properties: [
            ("region".to_owned(), region.to_owned()),
            ("tokens".to_owned(), tokens.to_owned()),
        ]
        .into(),
    }
}

fn timestamp(at: NaiveDateTime) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: at.and_utc().timestamp(),
        nanos: 0,
    }
}

/// tokens over the EU events only, billed per thousand.
fn draft(aggregation: metrics::aggregation::AggregationType) -> Metric {
    Metric::Draft(grpc::MetricDefinition {
        code: CODE.to_owned(),
        aggregation: Some(metrics::Aggregation {
            aggregation_type: aggregation.into(),
            aggregation_key: Some("tokens".to_owned()),
            unit_conversion: Some(metrics::aggregation::UnitConversion {
                factor: 1000.0,
                rounding: metrics::aggregation::unit_conversion::UnitConversionRounding::Up.into(),
            }),
        }),
        segmentation_matrix: None,
        filters: vec![metrics::MetricFilter {
            property: "region".to_owned(),
            op: metrics::metric_filter::Operator::In.into(),
            values: vec!["eu".to_owned()],
        }],
    })
}

fn values(
    points: &[grpc::MetricUsagePoint],
    key: impl Fn(&grpc::MetricUsagePoint) -> String,
) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    for point in points {
        assert!(
            out.insert(key(point), point.value).is_none(),
            "one point per series"
        );
    }
    out
}

#[tokio::test]
async fn test_event_insights_read_usage_like_billing() {
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
        usage_client,
        meteroid_mailer::service::mailer_service(MailerConfig::dummy()),
    )
    .await;
    let auth = meteroid_it::svc_auth::login(meteroid.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        meteroid.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );

    let day0 = (Utc::now().date_naive() - Duration::days(6)).and_time(NaiveTime::MIN);
    let at = |days: i64, hours: i64| day0 + Duration::days(days) + Duration::hours(hours);
    let range_end = at(3, 0);
    let failures = ingest_clients
        .events
        .ingest(Request::new(IngestRequest {
            events: vec![
                event_at(1, CUST_SPOTIFY_ID, at(0, 1), "eu", "1000"),
                event_at(2, CUST_SPOTIFY_ID, at(0, 2), "us", "3000"),
                event_at(3, CUST_SPOTIFY_ID, at(1, 1), "eu", "2000"),
                event_at(4, CUST_UBER_ID, at(0, 3), "eu", "500"),
                event_at(5, CUST_UBER_ID, at(2, 3), "eu", "700"),
                event_at(6, CUST_COMODO_ID, at(1, 4), "eu", "100"),
                // At the exclusive end of the range.
                event_at(7, CUST_SPOTIFY_ID, range_end, "eu", "99000"),
            ],
            allow_backfilling: true,
        }))
        .await
        .expect("Could not ingest events")
        .into_inner()
        .failures;
    assert!(failures.is_empty());

    let query = |metric: Metric, grain: Grain, breakdown: Breakdown, top_n: u32| {
        let mut client = clients.event_insights.clone();
        async move {
            let mut request = grpc::QueryMetricUsageRequest {
                metric: Some(metric),
                from: Some(timestamp(day0)),
                to: Some(timestamp(range_end)),
                top_n,
                ..Default::default()
            };
            request.set_grain(grain);
            request.set_breakdown(breakdown);
            client
                .query_metric_usage(Request::new(request))
                .await
                .map(|r| r.into_inner().points)
        }
    };

    // The EU events in range, per thousand tokens: Spotify 3, Uber 1.2, Comodo 0.1.
    let total = (|| async {
        let points = query(
            draft(metrics::aggregation::AggregationType::Sum),
            Grain::Total,
            Breakdown::None,
            0,
        )
        .await
        .map_err(|status| anyhow::anyhow!("{status:?}"))?;
        match points.as_slice() {
            [point] if point.value == 4.3 => Ok(points),
            _ => Err(anyhow::anyhow!("not every event is in yet: {points:?}")),
        }
    })
    .retry(
        backon::ConstantBuilder::default()
            .with_delay(std::time::Duration::from_millis(200))
            .with_max_times(100),
    )
    .await
    .expect("the events reach the store");
    assert_eq!(total[0].window_start, day0.date().to_string());
    assert_eq!(total[0].window_end, range_end.date().to_string());
    assert!(!total[0].other);

    let daily = query(
        draft(metrics::aggregation::AggregationType::Sum),
        Grain::Day,
        Breakdown::None,
        0,
    )
    .await
    .unwrap();
    assert_eq!(
        values(&daily, |p| p.window_start.clone()),
        HashMap::from([
            (at(0, 0).date().to_string(), 1.5),
            (at(1, 0).date().to_string(), 2.1),
            (at(2, 0).date().to_string(), 0.7),
        ]),
        "per thousand tokens, unrounded, without the US event"
    );

    let names: HashMap<CustomerId, String> = meteroid
        .store
        .list_customers_by_ids(TENANT_ID, vec![CUST_SPOTIFY_ID, CUST_UBER_ID])
        .await
        .unwrap()
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect();

    let summed = query(
        draft(metrics::aggregation::AggregationType::Sum),
        Grain::Total,
        Breakdown::Customer,
        2,
    )
    .await
    .unwrap();
    assert_eq!(
        values(&summed, |p| p.customer_id.clone()),
        HashMap::from([
            (CUST_SPOTIFY_ID.as_proto(), 3.0),
            (CUST_UBER_ID.as_proto(), 1.2),
            (String::new(), 0.1),
        ]),
        "the customers beyond the top 2 sum into Other"
    );
    for point in &summed {
        let label = point.dimensions.get("customer").map(String::as_str);
        if point.customer_id.is_empty() {
            assert!(point.other);
            assert_eq!(label, Some("Other"));
        } else {
            assert!(!point.other);
            let id = CustomerId::from_proto(&point.customer_id).unwrap();
            assert_eq!(label, names.get(&id).map(String::as_str));
        }
    }

    let peaks = query(
        draft(metrics::aggregation::AggregationType::Max),
        Grain::Total,
        Breakdown::Customer,
        2,
    )
    .await
    .unwrap();
    assert_eq!(
        values(&peaks, |p| p.customer_id.clone()),
        HashMap::from([
            (CUST_SPOTIFY_ID.as_proto(), 2.0),
            (CUST_UBER_ID.as_proto(), 0.7),
        ]),
        "a maximum keeps the top 2 and has no Other"
    );

    let metric = meteroid
        .store
        .insert_billable_metric(
            Actor::System,
            BillableMetricNew {
                name: "Insight calls".to_owned(),
                description: None,
                code: CODE.to_owned(),
                aggregation_type: BillingMetricAggregateEnum::Sum,
                aggregation_key: Some("tokens".to_owned()),
                unit_conversion_factor: Some(1000),
                unit_conversion_rounding: None,
                segmentation_matrix: None,
                filters: Some(MetricFilters(vec![MetricFilter {
                    property: "region".to_owned(),
                    op: MetricFilterOp::Equal,
                    values: vec!["eu".to_owned()],
                }])),
                usage_group_key: None,
                unit_label: None,
                tenant_id: TENANT_ID,
                product_family_id: PRODUCT_FAMILY_ID,
                product_id: None,
            },
        )
        .await
        .unwrap();

    let saved = query(
        Metric::MetricId(metric.id.as_proto()),
        Grain::Total,
        Breakdown::None,
        0,
    )
    .await
    .unwrap();
    assert_eq!(saved.iter().map(|p| p.value).collect::<Vec<_>>(), vec![4.3]);

    let usage = clients
        .event_insights
        .clone()
        .get_customer_usage(Request::new(grpc::GetCustomerUsageRequest {
            customer_id: CUST_SPOTIFY_ID.as_proto(),
            from: day0.date().to_string(),
            to: at(2, 0).date().to_string(),
        }))
        .await
        .unwrap()
        .into_inner();
    let ours = usage
        .metrics
        .iter()
        .find(|m| m.metric_id == metric.id.as_proto())
        .expect("the metric is summarised");
    assert_eq!(ours.total, 3.0, "the US event is filtered out");
    assert_eq!(ours.aggregation, "sum");
    assert_eq!(
        values(&ours.trend, |p| p.window_start.clone()),
        HashMap::from([
            (at(0, 0).date().to_string(), 1.0),
            (at(1, 0).date().to_string(), 2.0),
        ])
    );
}
