//! A ClickHouse left by a release before 1.0.0-rc4 (0.19.x, rc1 to rc3): its history ends at V4,
//! with checksums the current V1 to V4 no longer produce, and its events carry base62 ids.
use crate::{helpers, metering_it};
use clickhouse::Row;
use common_domain::ids::{BaseId, CustomerId, TenantId};
use metering::connectors::clickhouse::ClickhouseConnector;
use serde::Deserialize;
use uuid::Uuid;

const PRE_RC4_LAYOUT: &[&str] = &[
    "CREATE TABLE refinery_schema_history (version INT, name String, applied_on String, checksum String) \
     ENGINE = MergeTree ORDER BY version",
    "INSERT INTO refinery_schema_history VALUES \
     (1, 'initial', '2026-01-13T10:00:00Z', '11'), \
     (2, 'preprocessed_events', '2026-01-13T10:00:00Z', '22'), \
     (3, 'reset_replicated_and_kafka_auth', '2026-01-13T10:00:00Z', '33'), \
     (4, 'remove_preprocessed_events', '2026-01-13T10:00:00Z', '44')",
    "CREATE TABLE raw_events (id String, code String, customer_id String, tenant_id String, \
     timestamp DateTime64(9, 'UTC'), ingested_at DateTime64(9, 'UTC'), properties Map(String, String)) \
     ENGINE = MergeTree PARTITION BY toYYYYMM(timestamp) ORDER BY (tenant_id, timestamp, code, customer_id)",
];

#[derive(Row, Deserialize)]
struct Base62Totals {
    customer_id: String,
    events: u64,
    total: u64,
}

#[derive(Row, Deserialize)]
struct Copied {
    #[serde(with = "clickhouse::serde::uuid")]
    tenant_id: Uuid,
    #[serde(with = "clickhouse::serde::uuid")]
    customer_id: Uuid,
    events: u64,
    total: u64,
}

#[tokio::test]
async fn test_pre_rc4_history_keeps_its_usage_events() {
    helpers::init::logging();

    let (_clickhouse_container, ch_http_port, ch_tcp_port) =
        metering_it::container::start_clickhouse().await;
    let config = metering_it::config::mocked_config(
        0,
        0,
        ch_http_port,
        ch_tcp_port,
        0,
        "unused".to_string(),
    );
    let ch = metering_it::clickhouse::get_client(ch_http_port);
    for statement in PRE_RC4_LAYOUT {
        ch.query(statement).execute().await.expect("pre-rc4 layout");
    }

    let tenant = TenantId::new();
    let (first, second) = (CustomerId::new(), CustomerId::new());
    ch.query(
        "INSERT INTO raw_events SELECT concat('evt-', toString(number)), 'api_calls', \
         if(number % 2 = 0, ?, ?), ?, now64(9) - number * 60, now64(9) - number, \
         map('value', toString(number % 7)) FROM numbers(3000)",
    )
    .bind(first.to_string())
    .bind(second.to_string())
    .bind(tenant.to_string())
    .execute()
    .await
    .expect("base62 events");
    ch.query(
        "INSERT INTO raw_events VALUES ('evt-alias', 'api_calls', 'acme', ?, now64(9), now64(9), map())",
    )
    .bind(tenant.to_string())
    .execute()
    .await
    .expect("unconvertible event");

    let totals = "count() AS events, sum(toUInt64OrZero(properties['value'])) AS total";
    let mut expected: Vec<(Uuid, u64, u64)> = ch
        .query(&format!(
            "SELECT customer_id, {totals} FROM raw_events WHERE customer_id != 'acme' GROUP BY customer_id"
        ))
        .fetch_all::<Base62Totals>()
        .await
        .expect("source totals")
        .into_iter()
        .map(|t| {
            let id: CustomerId = t.customer_id.parse().expect("base62 customer id");
            (*id, t.events, t.total)
        })
        .collect();
    expected.sort();
    assert_eq!(expected.len(), 2);

    for _ in 0..2 {
        ClickhouseConnector::init(&config.clickhouse, &config.kafka, vec![])
            .await
            .expect("migrations run over a pre-rc4 history");
    }

    let copied: Vec<Copied> = ch
        .query(&format!(
            "SELECT tenant_id, customer_id, {totals} FROM raw_events_v2 GROUP BY tenant_id, customer_id"
        ))
        .fetch_all()
        .await
        .expect("copied events");
    assert!(copied.iter().all(|c| c.tenant_id == *tenant));
    let mut copied: Vec<_> = copied
        .into_iter()
        .map(|c| (c.customer_id, c.events, c.total))
        .collect();
    copied.sort();
    assert_eq!(copied, expected);

    let unconverted: u64 = ch
        .query("SELECT count() FROM raw_events_unconverted")
        .fetch_one()
        .await
        .expect("the previous layout is kept when an event cannot be copied");
    assert_eq!(unconverted, 3001);

    let versions: Vec<i32> = ch
        .query("SELECT version FROM refinery_schema_history ORDER BY version")
        .fetch_all()
        .await
        .expect("history");
    assert_eq!(versions, (1..=10).collect::<Vec<_>>());
}
