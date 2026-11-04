use crate::{helpers, metering_it};
use chrono::{Duration, Utc};
use common_domain::ids::TenantId;
use metering::connectors::Connector;
use metering::connectors::clickhouse::ClickhouseConnector;
use metering::domain::{EventSortOrder, QueryRawEventsParams};
use uuid::Uuid;

/// The metric preview's "Raw events" tab: limit 10 over the Distributed table, which a
/// single-shard cluster serves from the local replica — the shape lazy materialization breaks.
#[tokio::test]
async fn test_raw_events_listing_survives_small_limits() {
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
    let connector = ClickhouseConnector::init(&config.clickhouse, &config.kafka, vec![])
        .await
        .expect("connector init");

    let tenant_id = Uuid::now_v7();
    let ch = metering_it::clickhouse::get_client(ch_http_port);
    ch.query(
        "INSERT INTO raw_events_local_v2 (id, code, customer_id, tenant_id, timestamp, ingested_at, properties) \
         SELECT toString(number), 'api_calls', generateUUIDv4(), ?, now64(9) - number * 60, \
                now64(9) - number, map('n', toString(number)) \
         FROM numbers(50)",
    )
    .bind(tenant_id)
    .execute()
    .await
    .expect("seed raw events");

    for sort_order in [EventSortOrder::TimestampDesc, EventSortOrder::IngestedDesc] {
        let result = connector
            .query_raw_events(QueryRawEventsParams {
                tenant_id: TenantId::from(tenant_id),
                from: Utc::now() - Duration::days(1),
                to: Some(Utc::now() + Duration::minutes(1)),
                limit: 10,
                offset: 0,
                search: None,
                event_codes: vec![],
                customer_ids: vec![],
                filters: vec![],
                sort_order: sort_order.clone(),
                after: None,
                keep_last_events: false,
            })
            .await
            .unwrap_or_else(|err| panic!("{sort_order:?} listing failed: {err:?}"));

        let ids: Vec<_> = result.events.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, (0..10).map(|n| n.to_string()).collect::<Vec<_>>());
    }

    let alive: u8 = ch
        .query("SELECT 1")
        .fetch_one()
        .await
        .expect("ClickHouse is still up");
    assert_eq!(alive, 1);
}
