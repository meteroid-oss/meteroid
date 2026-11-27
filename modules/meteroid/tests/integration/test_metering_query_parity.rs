//! Exact meter values against a real ClickHouse: events are inserted straight into the raw
//! table and read back through the metering connector and its gRPC conversion.

use crate::helpers;
use crate::metering_it;
use chrono::{DateTime, Duration, TimeZone, Utc};
use common_domain::ids::{BaseId, CustomerId, TenantId};
use metering::connectors::Connector;
use metering::connectors::clickhouse::ClickhouseConnector;
use metering::domain::{MeterAggregation, MeterValue, QueryMeterParams};
use metering_grpc::meteroid::metering::v1::QueryMeterRequest;
use metering_grpc::meteroid::metering::v1::meter::AggregationType;
use metering_grpc::meteroid::metering::v1::query_meter_request::QueryWindowSize;
use metering_grpc::meteroid::metering::v1::usage_query_service_server::UsageQueryService as _;
use metering_server::query::service::UsageQueryService;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::str::FromStr;
use std::sync::Arc;
use testcontainers::{ContainerAsync, GenericImage};
use uuid::Uuid;

struct Env {
    _container: ContainerAsync<GenericImage>,
    client: clickhouse::Client,
    connector: Arc<ClickhouseConnector>,
    tenant_id: TenantId,
    customer_id: CustomerId,
}

struct Row<'a> {
    id: String,
    code: &'a str,
    value: &'a str,
    timestamp: DateTime<Utc>,
    ingested_at: DateTime<Utc>,
}

fn at(seconds: i64, nanos: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 10, 12, 0, 0).unwrap()
        + Duration::seconds(seconds)
        + Duration::nanoseconds(nanos as i64)
}

fn row<'a>(code: &'a str, value: &'a str, timestamp: DateTime<Utc>) -> Row<'a> {
    Row {
        id: Uuid::now_v7().to_string(),
        code,
        value,
        timestamp,
        ingested_at: timestamp,
    }
}

fn ch_time(t: DateTime<Utc>) -> String {
    format!(
        "toDateTime64('{}', 9, 'UTC')",
        t.format("%Y-%m-%d %H:%M:%S%.9f")
    )
}

async fn setup() -> Env {
    helpers::init::logging();
    let (container, http_port, tcp_port) = metering_it::container::start_clickhouse().await;
    let config =
        metering_it::config::mocked_config(0, 0, http_port, tcp_port, 0, "unused".to_string());
    let connector = ClickhouseConnector::init(
        &config.clickhouse,
        &config.consumer.tables,
        &config.kafka,
        vec![],
    )
    .await
    .expect("connector init");
    let client = clickhouse::Client::default()
        .with_url(&config.clickhouse.http_address)
        .with_user(&config.clickhouse.username)
        .with_password(&config.clickhouse.password)
        .with_database(&config.clickhouse.database);
    Env {
        _container: container,
        client,
        connector: Arc::new(connector),
        tenant_id: TenantId::from(Uuid::now_v7()),
        customer_id: CustomerId::from(Uuid::now_v7()),
    }
}

impl Env {
    async fn insert(&self, rows: &[Row<'_>]) {
        let values = rows
            .iter()
            .map(|r| {
                format!(
                    "('{}', '{}', toUUID('{}'), toUUID('{}'), {}, {}, map('v', '{}'))",
                    r.id,
                    r.code,
                    self.customer_id.as_uuid(),
                    self.tenant_id.as_uuid(),
                    ch_time(r.timestamp),
                    ch_time(r.ingested_at),
                    r.value
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        self.client
            .query(&format!(
                "INSERT INTO raw_events_v2 (id, code, customer_id, tenant_id, timestamp, ingested_at, properties) \
                 SETTINGS insert_distributed_sync = 1 VALUES {values}"
            ))
            .execute()
            .await
            .expect("insert");
    }

    fn params(&self, code: &str, aggregation: MeterAggregation) -> QueryMeterParams {
        QueryMeterParams {
            aggregation,
            tenant_id: self.tenant_id,
            code: code.to_string(),
            value_property: Some("v".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            merge_segments: false,
            attribution: None,
            origin: metering::domain::OriginFilter::All,
            group_by_customer: false,
            filters: vec![],
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            from: at(-3600, 0),
            to: Some(at(3600, 0)),
            keep_last_events: false,
        }
    }

    async fn value(&self, code: &str, aggregation: MeterAggregation) -> (MeterValue, u64) {
        let usage = self
            .connector
            .query_meter(self.params(code, aggregation))
            .await
            .expect("query_meter");
        assert_eq!(usage.len(), 1, "{usage:?}");
        (usage[0].value.clone(), usage[0].decimal_fallbacks)
    }

    async fn grpc_value(
        &self,
        code: &str,
        aggregation: AggregationType,
    ) -> Result<Decimal, tonic::Status> {
        let service = UsageQueryService::new(self.connector.clone());
        let mut request = QueryMeterRequest {
            tenant_id: self.tenant_id.as_proto(),
            code: code.to_string(),
            value_property: Some("v".to_string()),
            from: Some(prost_types::Timestamp {
                seconds: at(-3600, 0).timestamp(),
                nanos: 0,
            }),
            to: Some(prost_types::Timestamp {
                seconds: at(3600, 0).timestamp(),
                nanos: 0,
            }),
            ..Default::default()
        };
        request.set_meter_aggregation_type(aggregation);
        request.set_window_size(QueryWindowSize::AggregateAll);
        let response = service
            .query_meter(tonic::Request::new(request))
            .await?
            .into_inner();
        let value = response.usage[0].value.clone().expect("value");
        Ok(Decimal::from_str(&value.value).unwrap())
    }
}

#[tokio::test]
async fn test_metering_query_parity() {
    let env = setup().await;

    // Ten 0.1 sum to exactly 1 and average to 0.1; the mixed sets sum the same in any order.
    env.insert(
        &(0..10)
            .map(|i| row("tenths", "0.1", at(i, 0)))
            .collect::<Vec<_>>(),
    )
    .await;
    let mixed = ["0.1", "0.7", "0.2", "1000000.000000001", "-0.3"];
    for (code, order) in [
        ("mixed_a", [0, 1, 2, 3, 4]),
        ("mixed_b", [4, 3, 2, 1, 0]),
        ("mixed_c", [3, 0, 4, 2, 1]),
    ] {
        for (i, idx) in order.into_iter().enumerate() {
            env.insert(&[row(code, mixed[idx], at(i as i64, 0))]).await;
        }
    }
    assert_eq!(
        env.value("tenths", MeterAggregation::Sum).await,
        (MeterValue::Exact(dec!(1)), 0)
    );
    assert_eq!(
        env.grpc_value("tenths", AggregationType::Mean)
            .await
            .unwrap(),
        dec!(0.1)
    );
    for code in ["mixed_a", "mixed_b", "mixed_c"] {
        assert_eq!(
            env.value(code, MeterAggregation::Sum).await.0,
            MeterValue::Exact(dec!(1000000.700000001)),
            "{code}"
        );
    }

    // Non-finite, absurd and unparsable values are skipped; exponent notation counts.
    env.insert(
        &["inf", "nan", "1e40", "1e18", "abc", "1e3", "2"]
            .into_iter()
            .enumerate()
            .map(|(i, v)| row("guard", v, at(i as i64, 0)))
            .collect::<Vec<_>>(),
    )
    .await;
    // 25.6 and 25.8 parse exponents as decimals: no Float64 fallback is needed.
    assert_eq!(
        env.value("guard", MeterAggregation::Sum).await,
        (MeterValue::Exact(dec!(1002)), 0)
    );
    assert_eq!(
        env.value("guard", MeterAggregation::Count).await.0,
        MeterValue::Exact(dec!(7))
    );

    // Max returns the winning string exactly, not its Float64 or 9-decimal reading.
    env.insert(&[
        row("max", "0.3", at(0, 0)),
        row("max", "0.30000000000000004", at(1, 0)),
    ])
    .await;
    assert_eq!(
        env.value("max", MeterAggregation::Max).await.0,
        MeterValue::Exact(dec!(0.30000000000000004))
    );
    assert_eq!(
        env.value("max", MeterAggregation::Min).await.0,
        MeterValue::Exact(dec!(0.3))
    );

    // Latest: sub-second order, then ingested_at, then id.
    env.insert(&[
        row("latest_subsecond", "2", at(5, 900_000_000)),
        row("latest_subsecond", "1", at(5, 100_000_000)),
    ])
    .await;
    assert_eq!(
        env.value("latest_subsecond", MeterAggregation::Latest)
            .await
            .0,
        MeterValue::Exact(dec!(2))
    );
    let same = at(7, 0);
    env.insert(&[
        Row {
            ingested_at: same + Duration::seconds(2),
            ..row("latest_ingested", "20", same)
        },
        Row {
            ingested_at: same + Duration::seconds(1),
            ..row("latest_ingested", "10", same)
        },
    ])
    .await;
    assert_eq!(
        env.value("latest_ingested", MeterAggregation::Latest)
            .await
            .0,
        MeterValue::Exact(dec!(20))
    );
    env.insert(&[
        Row {
            id: "b".to_string(),
            ..row("latest_id", "200", same)
        },
        Row {
            id: "a".to_string(),
            ..row("latest_id", "100", same)
        },
    ])
    .await;
    assert_eq!(
        env.value("latest_id", MeterAggregation::Latest).await.0,
        MeterValue::Exact(dec!(200))
    );

    // A sum beyond rust_decimal's 28 digits is an error, never a silent None or zero.
    env.insert(
        &(0..101)
            .map(|i| row("overflow", "999999999999999000.000000001", at(i, 0)))
            .collect::<Vec<_>>(),
    )
    .await;
    assert!(
        env.connector
            .query_meter(env.params("overflow", MeterAggregation::Sum))
            .await
            .is_err()
    );
    assert!(
        env.grpc_value("overflow", AggregationType::Sum)
            .await
            .is_err()
    );
}

/// A keep-last tenant's filters judge the version it keeps: a re-send that changes the filtered
/// property moves the event, and one that makes its value unparsable drops it.
#[tokio::test]
async fn test_a_keep_last_resend_is_filtered_on_its_last_version() {
    use metering::domain::{
        EventCursor, EventSortOrder, PropertyFilter, PropertyFilterOp, QueryRawEventsParams,
        SegmentationFilter,
    };
    let env = setup().await;
    let resend = |id: &str, v: &str, region: &str, ingested: i64| {
        format!(
            "('{id}', 'resend', toUUID('{}'), toUUID('{}'), {}, {}, map('v', '{v}', 'region', '{region}'))",
            env.customer_id.as_uuid(),
            env.tenant_id.as_uuid(),
            ch_time(at(0, 0)),
            ch_time(at(ingested, 0)),
        )
    };
    let values = [
        resend("moved", "10", "eu", 0),
        resend("moved", "10", "us", 1),
        resend("unparsable", "7", "eu", 0),
        resend("unparsable", "abc", "eu", 1),
    ]
    .join(", ");
    env.client
        .query(&format!(
            "INSERT INTO raw_events_v2 (id, code, customer_id, tenant_id, timestamp, ingested_at, properties) \
             SETTINGS insert_distributed_sync = 1 VALUES {values}"
        ))
        .execute()
        .await
        .expect("insert");

    let filter = |op, region: &str| PropertyFilter {
        property: "region".to_string(),
        op,
        values: vec![region.to_string()],
    };
    let total = |aggregation,
                 keep_last_events,
                 filters: Vec<PropertyFilter>,
                 segment: Option<&str>| {
        let mut params = env.params("resend", aggregation);
        params.keep_last_events = keep_last_events;
        params.filters = filters;
        params.segmentation_filter = segment.map(|region| {
            SegmentationFilter::Independent(vec![("region".to_string(), vec![region.to_string()])])
        });
        let connector = env.connector.clone();
        async move {
            connector
                .query_meter(params)
                .await
                .expect("query_meter")
                .into_iter()
                .map(|usage| match usage.value {
                    MeterValue::Exact(value) => value,
                    other => panic!("{other:?}"),
                })
                .sum::<Decimal>()
        }
    };
    let (sum, count) = (MeterAggregation::Sum, MeterAggregation::Count);
    let eu = || vec![filter(PropertyFilterOp::In, "eu")];
    assert_eq!(
        total(sum.clone(), true, eu(), None).await,
        Decimal::ZERO,
        "both eu copies are superseded"
    );
    assert_eq!(
        total(
            sum.clone(),
            true,
            vec![filter(PropertyFilterOp::In, "us")],
            None
        )
        .await,
        dec!(10),
        "the moved event is counted once"
    );
    assert_eq!(
        total(count, true, eu(), None).await,
        dec!(1),
        "only the unparsable one is still eu"
    );
    assert_eq!(
        total(sum.clone(), true, vec![], None).await,
        dec!(10),
        "the unparsable copy is dropped"
    );
    assert_eq!(
        total(
            sum.clone(),
            true,
            vec![filter(PropertyFilterOp::NotIn, "us")],
            None
        )
        .await,
        Decimal::ZERO
    );
    assert_eq!(
        total(sum.clone(), true, vec![], Some("eu")).await,
        Decimal::ZERO,
        "segments too"
    );
    assert_eq!(
        total(sum, false, vec![], None).await,
        dec!(27),
        "a default tenant counts every copy"
    );

    let found = env
        .connector
        .query_raw_events(QueryRawEventsParams {
            tenant_id: env.tenant_id,
            from: at(-3600, 0),
            to: Some(at(3600, 0)),
            limit: 10,
            offset: 0,
            search: Some("eu".to_string()),
            event_codes: vec!["resend".to_string()],
            customer_ids: vec![],
            filters: vec![filter(PropertyFilterOp::In, "eu")],
            sort_order: EventSortOrder::TimestampAsc,
            after: Some(EventCursor {
                timestamp_nanos: at(-1, 0).timestamp_nanos_opt().unwrap(),
                id: String::new(),
            }),
            keep_last_events: true,
        })
        .await
        .expect("query_raw_events")
        .events;
    let found: Vec<_> = found
        .iter()
        .map(|e| (e.id.as_str(), e.properties["v"].as_str()))
        .collect();
    assert_eq!(
        found,
        vec![("unparsable", "abc")],
        "the explorer lists last copies only"
    );
}
