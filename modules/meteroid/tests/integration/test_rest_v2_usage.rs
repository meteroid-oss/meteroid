//! Usage through the public REST API v2 (ADR-0013 §14): a customer's and a subscription's usage
//! by metric (named by code, quantities as decimal strings), the tenant's summary, raw events by
//! cursor, costs by bucket, unattributed usage, and the problems a refused read answers.
use crate::data::ids::{CUST_SPOTIFY_ID, METRIC_BANDWIDTH};
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::SeedLevel;
use crate::test_rest_v2::Fixture;
use chrono::{Duration, NaiveDateTime, SecondsFormat, Utc};
use common_domain::ids::{BillableMetricId, CustomerId, TenantId};
use meteroid_grpc::meteroid::api;
use meteroid_store::StoreResult;
use meteroid_store::clients::usage::{
    EventSearchOptions, EventSearchResult, GroupedUsageData, IngestEventsRequest,
    IngestEventsResult, MetricEvent, MetricEventCursor, MutableMockUsageClient, UsageClient,
    UsageData, WindowedUsageData,
};
use meteroid_store::domain::{BillableMetric, UsagePeriod};
use meteroid_testkit::mailer::MockMailerService;
use reqwest::StatusCode;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

/// Aggregates from the mutable mock; events of one customer from a fixed list, paged by
/// `(timestamp, id)` as the metering backend pages them.
#[derive(Clone)]
struct Metering {
    usage: MutableMockUsageClient,
    customer_id: CustomerId,
    events: Arc<Vec<MetricEvent>>,
}

#[async_trait::async_trait]
impl UsageClient for Metering {
    async fn fetch_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.usage
            .fetch_usage(tenant_id, customer_id, metric, period)
            .await
    }

    async fn fetch_usage_grouped_by(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: &str,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.usage
            .fetch_usage_grouped_by(tenant_id, customer_id, metric, group_by, period)
            .await
    }

    async fn fetch_total_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<Decimal> {
        self.usage
            .fetch_total_usage(tenant_id, customer_id, metric, period)
            .await
    }

    async fn fetch_windowed_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<WindowedUsageData> {
        self.usage
            .fetch_windowed_usage(tenant_id, customer_id, metric, period)
            .await
    }

    async fn list_metric_events(
        &self,
        _tenant_id: &TenantId,
        customer_id: &CustomerId,
        _metric: &BillableMetric,
        period: UsagePeriod,
        after: Option<MetricEventCursor>,
        limit: u32,
    ) -> StoreResult<Vec<MetricEvent>> {
        if *customer_id != self.customer_id {
            return Ok(Vec::new());
        }
        let key = |event: &MetricEvent| {
            (
                event.timestamp.and_utc().timestamp_nanos_opt().unwrap(),
                event.id.clone(),
            )
        };
        let mut events: Vec<MetricEvent> = self
            .events
            .iter()
            .filter(|event| event.timestamp >= period.start && event.timestamp < period.end)
            .filter(|event| {
                after
                    .as_ref()
                    .is_none_or(|cursor| key(event) > (cursor.timestamp_nanos, cursor.id.clone()))
            })
            .cloned()
            .collect();
        events.sort_by_key(key);
        events.truncate(limit as usize);
        Ok(events)
    }

    async fn fetch_usage_summary(
        &self,
        tenant_id: &TenantId,
        customer_id: Option<&CustomerId>,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.usage
            .fetch_usage_summary(tenant_id, customer_id, metric, period)
            .await
    }

    async fn search_events(
        &self,
        tenant_id: &TenantId,
        options: EventSearchOptions,
    ) -> StoreResult<EventSearchResult> {
        self.usage.search_events(tenant_id, options).await
    }

    async fn ingest_events(
        &self,
        tenant_id: &TenantId,
        request: IngestEventsRequest,
    ) -> StoreResult<IngestEventsResult> {
        self.usage.ingest_events(tenant_id, request).await
    }
}

fn hours_ago(hours: i64) -> NaiveDateTime {
    (Utc::now() - Duration::hours(hours)).naive_utc()
}

/// The v2 fixture over `metering`: 1234.5 of bandwidth in `eu` on any window, three events of
/// Spotify's in the last hours.
async fn usage_fixture() -> Fixture {
    crate::helpers::init::logging();
    let usage = MutableMockUsageClient::new();
    usage.set_usage_for_any_nonempty_window(
        METRIC_BANDWIDTH,
        UsageData {
            data: vec![GroupedUsageData {
                value: dec!(1234.5),
                dimensions: HashMap::from([("region".to_owned(), "eu".to_owned())]),
            }],
            period: UsagePeriod {
                start: hours_ago(24 * 30),
                end: hours_ago(0),
            },
        },
    );
    let events = (1..=3)
        .map(|index| MetricEvent {
            id: format!("evt-{index}"),
            timestamp: hours_ago(4 - index),
            properties: HashMap::from([
                ("value".to_owned(), index.to_string()),
                ("region".to_owned(), "eu".to_owned()),
            ]),
        })
        .collect();
    let metering = Metering {
        usage,
        customer_id: CUST_SPOTIFY_ID,
        events: Arc::new(events),
    };

    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = meteroid_it::container::start_meteroid_with_clients(
        database,
        SeedLevel::PLANS,
        Arc::new(metering),
        Arc::new(MockMailerService::new()),
    )
    .await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let api_key = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    )
    .api_tokens
    .clone()
    .create_api_token(api::apitokens::v1::CreateApiTokenRequest {
        name: "api-v2-usage".to_owned(),
    })
    .await
    .unwrap()
    .into_inner()
    .api_key;
    Fixture {
        setup,
        client: http_client_tracing::client(),
        api_key,
    }
}

fn metric<'a>(usage: &'a Value, code: &str) -> &'a Value {
    usage["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|metric| metric["metric"]["code"] == code)
        .unwrap_or_else(|| panic!("no metric {code}: {usage:#}"))
}

fn moment(at: NaiveDateTime) -> String {
    at.and_utc().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Rules: ADR-0013 §14
#[tokio::test]
async fn test_rest_v2_customer_and_tenant_usage() {
    let f = usage_fixture().await;
    let bandwidth: BillableMetricId = METRIC_BANDWIDTH;
    assert!(bandwidth.to_string().starts_with("bm_"));

    // By metric code, quantities as decimal strings, split by dimension; dates read as midnight.
    let usage = f
        .get("/api/v2/customers/ext:spotify/usage?from=2026-11-01&to=2026-12-01&metric=bandwidth")
        .await;
    assert_eq!(usage.status, StatusCode::OK, "{:#}", usage.body);
    assert!(
        usage.body["from"]
            .as_str()
            .unwrap()
            .starts_with("2026-11-01T00:00:00"),
        "{:#}",
        usage.body
    );
    assert_eq!(usage.body["metrics"].as_array().unwrap().len(), 1);
    let read = metric(&usage.body, "bandwidth");
    assert_eq!(read["quantity"], "1234.5");
    assert_eq!(read["groups"][0]["dimensions"]["region"], "eu");
    assert_eq!(read["groups"][0]["quantity"], "1234.5");
    assert!(read.get("metric_id").is_none() && read.get("costs").is_none());
    assert!(usage.body.get("totals").is_none());

    // Every live metric when none is named; by the customer's id as by its alias.
    let all = f
        .get(&format!(
            "/api/v2/customers/{CUST_SPOTIFY_ID}/usage?from=2026-11-01T00:00:00Z&to=2026-12-01T00:00:00Z"
        ))
        .await;
    assert_eq!(all.status, StatusCode::OK, "{:#}", all.body);
    assert_eq!(metric(&all.body, "bandwidth")["quantity"], "1234.5");

    // The tenant's summary.
    let summary = f
        .get("/api/v2/usage/summary?from=2026-11-01&to=2026-12-01&metric=bandwidth")
        .await;
    assert_eq!(summary.status, StatusCode::OK, "{:#}", summary.body);
    assert_eq!(metric(&summary.body, "bandwidth")["quantity"], "1234.5");

    // Refused reads answer problems at the parameter.
    let backwards = f
        .get("/api/v2/customers/ext:spotify/usage?from=2026-12-01&to=2026-11-01")
        .await;
    assert_eq!(backwards.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        backwards.header("content-type"),
        Some("application/problem+json")
    );
    assert_eq!(backwards.body["errors"][0]["path"], "to");
    let unknown = f
        .get("/api/v2/usage/summary?from=2026-11-01&to=2026-12-01&metric=no_such_metric")
        .await;
    assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(unknown.body["errors"][0]["path"], "metric");
    assert_eq!(unknown.body["errors"][0]["code"], "not_found");
    // A plain usage read covers a year and a day at most, refused at `to`.
    for path in [
        "/api/v2/customers/ext:spotify/usage?from=2025-01-01&to=2026-12-01",
        "/api/v2/usage/summary?from=2025-01-01&to=2026-12-01",
    ] {
        let too_long = f.get(path).await;
        assert_eq!(
            too_long.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{path}: {:#}",
            too_long.body
        );
        assert_eq!(too_long.body["errors"][0]["path"], "to", "{path}");
    }
    let undated = f
        .get("/api/v2/customers/ext:spotify/usage?to=2026-12-01")
        .await;
    assert_eq!(undated.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(undated.body["code"], "validation_failed");
    assert_eq!(undated.body["errors"][0]["path"], "from");
    let nobody = f
        .get("/api/v2/customers/ext:nobody/usage?from=2026-11-01&to=2026-12-01")
        .await;
    assert_eq!(nobody.status, StatusCode::NOT_FOUND);
    assert_eq!(nobody.body["code"], "not_found");

    // Unattributed usage: the current month by default.
    let unattributed = f
        .get("/api/v2/customers/ext:spotify/usage/unattributed")
        .await;
    assert_eq!(
        unattributed.status,
        StatusCode::OK,
        "{:#}",
        unattributed.body
    );
    assert!(unattributed.body["metrics"].is_array());
    for stretch in unattributed.body["metrics"].as_array().unwrap() {
        for subscription in stretch["subscriptions"].as_array().unwrap() {
            assert!(subscription["id"].as_str().unwrap().starts_with("sub_"));
        }
    }
}

/// Rules: ADR-0013 §14
#[tokio::test]
async fn test_rest_v2_usage_events_and_costs() {
    let f = usage_fixture().await;
    let (from, to) = (moment(hours_ago(48)), moment(hours_ago(-24)));

    // Raw events oldest first, by cursor.
    let first = f
        .get(&format!(
            "/api/v2/customers/ext:spotify/usage/events?metric=bandwidth&from={from}&to={to}&limit=2"
        ))
        .await;
    assert_eq!(first.status, StatusCode::OK, "{:#}", first.body);
    let page = first.body["data"].as_array().unwrap();
    assert_eq!(page.len(), 2, "{:#}", first.body);
    assert_eq!(page[0]["id"], "evt-1");
    assert_eq!(page[1]["id"], "evt-2");
    assert_eq!(page[0]["properties"]["region"], "eu");
    assert_eq!(first.body["has_more"], true);
    let cursor = first.body["next_cursor"].as_str().unwrap().to_owned();
    let second = f
        .get(&format!(
            "/api/v2/customers/ext:spotify/usage/events?metric=bandwidth&from={from}&to={to}&limit=2&after={cursor}"
        ))
        .await;
    assert_eq!(second.status, StatusCode::OK, "{:#}", second.body);
    assert_eq!(second.body["data"].as_array().unwrap().len(), 1);
    assert_eq!(second.body["data"][0]["id"], "evt-3");
    assert_eq!(second.body["has_more"], false);
    assert!(second.body["next_cursor"].is_null());

    // Refused: a cursor of another list, a range longer than 93 days, a limit past 100.
    let forged = f
        .get(&format!(
            "/api/v2/customers/ext:spotify/usage/events?metric=bandwidth&from={from}&to={to}&after=forged"
        ))
        .await;
    assert_eq!(forged.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(forged.body["errors"][0]["path"], "after");
    let too_long = f
        .get("/api/v2/customers/ext:spotify/usage/events?metric=bandwidth&from=2026-01-01&to=2026-06-01")
        .await;
    assert_eq!(too_long.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(too_long.body["errors"][0]["path"], "to");
    let too_many = f
        .get(&format!(
            "/api/v2/customers/ext:spotify/usage/events?metric=bandwidth&from={from}&to={to}&limit=500"
        ))
        .await;
    assert_eq!(too_many.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(too_many.body["errors"][0]["path"], "limit");

    // Costs by bucket: the granularity in snake_case; amounts, when priced, in minor units.
    let shouting = f
        .get(&format!(
            "/api/v2/customers/ext:spotify/usage/costs?metric=bandwidth&granularity=DAY&from={from}&to={to}"
        ))
        .await;
    assert_eq!(shouting.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(shouting.body["errors"][0]["path"], "granularity");
    let costs = f
        .get(&format!(
            "/api/v2/customers/ext:spotify/usage/costs?metric=bandwidth&granularity=day&from={from}&to={to}"
        ))
        .await;
    match costs.status {
        StatusCode::OK => {
            assert_eq!(costs.body["granularity"], "day");
            assert_eq!(costs.body["metric"]["code"], "bandwidth");
            for bucket in costs.body["buckets"].as_array().unwrap() {
                assert!(bucket["quantity"].is_string(), "{bucket:#}");
                if let Some(cost) = bucket.get("cost") {
                    assert!(cost["amount"].is_i64(), "{cost:#}");
                }
            }
        }
        // Spotify has no billing period pricing bandwidth in the range.
        StatusCode::UNPROCESSABLE_ENTITY => {
            assert_eq!(costs.body["code"], "validation_failed", "{:#}", costs.body)
        }
        other => panic!("costs answered {other}: {:#}", costs.body),
    }
}

/// Rules: ADR-0013 §14
#[tokio::test]
async fn test_rest_v2_subscription_usage() {
    let f = usage_fixture().await;
    crate::test_rest_v2_subscriptions::seed(&f).await;
    let created = f
        .post_once(
            "/api/v2/subscriptions",
            "usage-subscription",
            json!({
                "customer": "ext:spotify",
                "plan": "studio",
                "option": "monthly",
                "external_id": "usage-sub-1",
                "quantities": {"users": "3"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);

    // Its current period, by external id: the metrics it bills.
    let current = f.get("/api/v2/subscriptions/usage-sub-1/usage").await;
    assert_eq!(current.status, StatusCode::OK, "{:#}", current.body);
    let read = metric(&current.body, "bandwidth");
    assert_eq!(read["quantity"], "1234.5");
    if let Some(costs) = read.get("costs") {
        for cost in costs.as_array().unwrap() {
            assert!(cost["amount"].is_i64(), "{cost:#}");
            assert!(cost.get("currency").is_some() || cost.get("credit_unit").is_some());
        }
    }

    // Over a range, narrowed to one metric.
    let ranged = f
        .get("/api/v2/subscriptions/usage-sub-1/usage?from=2026-11-01&to=2026-12-01&metric=bandwidth")
        .await;
    assert_eq!(ranged.status, StatusCode::OK, "{:#}", ranged.body);
    assert_eq!(ranged.body["metrics"].as_array().unwrap().len(), 1);
    assert!(ranged.body.get("computed_at").is_none());

    // Its events: those its periods bill.
    let (from, to) = (moment(hours_ago(48)), moment(hours_ago(-24)));
    let events = f
        .get(&format!(
            "/api/v2/subscriptions/usage-sub-1/usage/events?metric=bandwidth&from={from}&to={to}"
        ))
        .await;
    assert_eq!(events.status, StatusCode::OK, "{:#}", events.body);
    assert!(events.body["data"].is_array());

    // A subscription that is none.
    let missing = f.get("/api/v2/subscriptions/no-such-sub/usage").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{:#}", missing.body);
    assert_eq!(missing.body["code"], "not_found");
}
