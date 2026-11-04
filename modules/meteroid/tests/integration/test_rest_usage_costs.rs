//! `/usage/{customer,subscription}/{id}/events` and `/costs`: per-event cost for linear pricing,
//! and bucketed costs that add up to the very invoice line the invoicer prices; and the running
//! total `/usage/subscription/{id}` adds for the current period.
use crate::data::ids::{
    CUST_COMODO_ID, CUST_SPOTIFY_ID, CUST_UBER_ID, INVOICING_ENTITY_ID, METRIC_BANDWIDTH,
    METRIC_DATABASE_SIZE, PLAN_VERSION_1_LEETCODE_ID, PLAN_VERSION_PAID_FREE_TRIAL_ID,
    PLAN_VERSION_USAGE_ALPHA_ID, PLAN_VERSION_USAGE_BETA_ID, PRODUCT_FAMILY_ID, TENANT_ID,
};
use crate::helpers::credit_recompute::{
    metric_component, started_on, subscribe, subscribe_filtered, subscribe_from, subscribe_on,
};
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use chrono::{Datelike, Duration, Months, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, BillableMetricId, CustomerId, SubscriptionId, TenantId};
use error_stack::bail;
use meteroid_grpc::meteroid::api;
use meteroid_store::StoreResult;
use meteroid_store::clients::usage::{
    EventSearchOptions, EventSearchResult, GroupedUsageData, IngestEventsRequest,
    IngestEventsResult, MetricEvent, MetricEventCursor, TenantUsageGrain, UsageClient, UsageData,
    UsageSeriesPoint, WindowedUsageData,
};
use meteroid_store::domain::credit_units::{CreditUnitNew, PriceCreditUnit};
use meteroid_store::domain::enums::BillingMetricAggregateEnum;
use meteroid_store::domain::enums::TaxResolverEnum;
use meteroid_store::domain::price_components::{
    MatrixDimension, MatrixRow, PriceEntry, TierRow, UsagePricingModel,
};
use meteroid_store::domain::subscription_components::ExtraComponent;
use meteroid_store::domain::{
    BillableMetric, BillableMetricNew, Dimension, InvoicingEntityPatch, SegmentationMatrix,
    UsageFilter, UsagePeriod,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::credit_units::CreditUnitsInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::subscriptions::{
    CancellationEffectiveAt, SubscriptionInterfaceAuto,
};
use meteroid_testkit::mailer::MockMailerService;
use reqwest::StatusCode;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

struct StoredEvent {
    id: String,
    customer_id: CustomerId,
    code: String,
    at: NaiveDateTime,
    properties: HashMap<String, String>,
}

/// A metering backend over an in-memory event list, cutting windows and groups the way the
/// ClickHouse queries do.
#[derive(Clone, Default)]
struct EventStream {
    events: Arc<Mutex<Vec<StoredEvent>>>,
}

impl EventStream {
    fn add(
        &self,
        customer_id: CustomerId,
        code: &str,
        at: NaiveDateTime,
        region: &str,
        value: i64,
    ) {
        self.add_value(customer_id, code, at, region, &value.to_string());
    }

    fn add_value(
        &self,
        customer_id: CustomerId,
        code: &str,
        at: NaiveDateTime,
        region: &str,
        value: &str,
    ) {
        let mut events = self.events.lock().unwrap();
        let id = format!("evt-{:04}", events.len());
        events.push(StoredEvent {
            id,
            customer_id,
            code: code.to_owned(),
            at,
            properties: HashMap::from([
                ("value".to_owned(), value.to_owned()),
                ("region".to_owned(), region.to_owned()),
            ]),
        });
    }

    fn dimension_keys<'a>(metric: &'a BillableMetric, group_by: Option<&'a str>) -> Vec<&'a str> {
        let mut keys: Vec<&str> = group_by.into_iter().collect();
        keys.extend(metric.usage_group_key.as_deref());
        match metric.segmentation_matrix.as_ref() {
            Some(SegmentationMatrix::Single(dimension)) => keys.push(&dimension.key),
            Some(SegmentationMatrix::Double {
                dimension1,
                dimension2,
            }) => keys.extend([dimension1.key.as_str(), dimension2.key.as_str()]),
            Some(SegmentationMatrix::Linked {
                dimension1_key,
                dimension2_key,
                ..
            }) => keys.extend([dimension1_key.as_str(), dimension2_key.as_str()]),
            None => {}
        }
        keys
    }

    fn matching(
        &self,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: &UsagePeriod,
    ) -> Vec<(String, NaiveDateTime, HashMap<String, String>)> {
        let mut found: Vec<_> = self
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                event.customer_id == *customer_id
                    && event.code == metric.code
                    && event.at >= period.start
                    && event.at < period.end
                    && metric.usage_scope.contains(&event.properties, event.at)
            })
            .map(|event| (event.id.clone(), event.at, event.properties.clone()))
            .collect();
        found.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
        found
    }

    fn aggregate(
        &self,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: Option<&str>,
        period: &UsagePeriod,
        floor: impl Fn(NaiveDateTime) -> Option<NaiveDateTime>,
    ) -> Vec<(Option<NaiveDateTime>, GroupedUsageData)> {
        let keys = Self::dimension_keys(metric, group_by);
        let mut totals: BTreeMap<String, (Option<NaiveDateTime>, GroupedUsageData)> =
            BTreeMap::new();
        for (_, at, properties) in self.matching(customer_id, metric, period) {
            let value = Decimal::from_str(&properties["value"]).unwrap();
            let dimensions: HashMap<String, String> = keys
                .iter()
                .filter_map(|key| {
                    properties
                        .get(*key)
                        .map(|value| ((*key).to_owned(), value.clone()))
                })
                .collect();
            let bucket = floor(at);
            let key = format!(
                "{bucket:?}{:?}",
                dimensions.iter().collect::<BTreeMap<_, _>>()
            );
            totals
                .entry(key)
                .and_modify(|(_, total)| {
                    total.value = match metric.aggregation_type {
                        BillingMetricAggregateEnum::Max => total.value.max(value),
                        _ => total.value + value,
                    }
                })
                .or_insert((bucket, GroupedUsageData { value, dimensions }));
        }
        totals.into_values().collect()
    }
}

#[async_trait::async_trait]
impl UsageClient for EventStream {
    async fn fetch_usage(
        &self,
        _tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: meteroid_store::domain::UsagePeriod,
    ) -> StoreResult<UsageData> {
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_owned()));
        }
        Ok(UsageData {
            data: self
                .aggregate(customer_id, metric, None, &period, |_| None)
                .into_iter()
                .map(|(_, group)| group)
                .collect(),
            period,
        })
    }

    async fn fetch_usage_grouped_by(
        &self,
        _tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: &str,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_owned()));
        }
        Ok(UsageData {
            data: self
                .aggregate(customer_id, metric, Some(group_by), &period, |_| None)
                .into_iter()
                .map(|(_, group)| group)
                .collect(),
            period,
        })
    }

    async fn fetch_usage_series(
        &self,
        _tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: Option<&str>,
        grain: TenantUsageGrain,
        period: UsagePeriod,
    ) -> StoreResult<Vec<UsageSeriesPoint>> {
        let floor = |at: NaiveDateTime| match grain {
            TenantUsageGrain::Hour => Some(
                at.date()
                    .and_time(NaiveTime::from_hms_opt(chrono::Timelike::hour(&at), 0, 0).unwrap()),
            ),
            TenantUsageGrain::Day => Some(at.date().and_time(NaiveTime::MIN)),
        };
        Ok(self
            .aggregate(customer_id, metric, group_by, &period, floor)
            .into_iter()
            .map(|(bucket, group)| UsageSeriesPoint {
                window_start: bucket.unwrap(),
                value: group.value,
                dimensions: group.dimensions,
            })
            .collect())
    }

    async fn list_metric_events(
        &self,
        _tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
        after: Option<MetricEventCursor>,
        limit: u32,
    ) -> StoreResult<Vec<MetricEvent>> {
        Ok(self
            .matching(customer_id, metric, &period)
            .into_iter()
            .filter(|(id, at, _)| {
                after.as_ref().is_none_or(|cursor| {
                    (at.and_utc().timestamp_nanos_opt().unwrap(), id.as_str())
                        > (cursor.timestamp_nanos, cursor.id.as_str())
                })
            })
            .take(limit as usize)
            .map(|(id, timestamp, properties)| MetricEvent {
                id,
                timestamp,
                properties,
            })
            .collect())
    }

    async fn fetch_total_usage(
        &self,
        _tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<Decimal> {
        Ok(self
            .aggregate(customer_id, metric, None, &period, |_| None)
            .into_iter()
            .map(|(_, group)| group.value)
            .sum())
    }

    async fn fetch_windowed_usage(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        _metric: &BillableMetric,
        _period: UsagePeriod,
    ) -> StoreResult<WindowedUsageData> {
        bail!(StoreError::InvalidArgument("not read here".to_owned()))
    }

    async fn fetch_usage_summary(
        &self,
        tenant_id: &TenantId,
        customer_id: Option<&CustomerId>,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        let Some(customer_id) = customer_id else {
            bail!(StoreError::InvalidArgument("not read here".to_owned()))
        };
        self.fetch_usage(tenant_id, customer_id, metric, period)
            .await
    }

    async fn search_events(
        &self,
        _tenant_id: &TenantId,
        _options: EventSearchOptions,
    ) -> StoreResult<EventSearchResult> {
        bail!(StoreError::InvalidArgument("not read here".to_owned()))
    }

    async fn ingest_events(
        &self,
        _tenant_id: &TenantId,
        _request: IngestEventsRequest,
    ) -> StoreResult<IngestEventsResult> {
        bail!(StoreError::InvalidArgument("not read here".to_owned()))
    }
}

fn tiers() -> Vec<TierRow> {
    vec![
        TierRow {
            first_unit: 0,
            rate: dec!(0.10),
            flat_fee: None,
            flat_cap: None,
        },
        TierRow {
            first_unit: 1000,
            rate: dec!(0.05),
            flat_fee: Some(dec!(1.5)),
            flat_cap: None,
        },
    ]
}

fn matrix() -> UsagePricingModel {
    let cell = |value: &str, price: Decimal| MatrixRow {
        dimension1: MatrixDimension {
            key: "region".to_owned(),
            value: value.to_owned(),
        },
        dimension2: None,
        per_unit_price: price,
    };
    UsagePricingModel::Matrix {
        rates: vec![cell("eu", dec!(0.02)), cell("us", dec!(0.03))],
    }
}

struct Api {
    setup: MeteroidSetup,
    http: reqwest::Client,
    auth_token: String,
    api_key: String,
}

impl Api {
    async fn start(stream: &EventStream) -> Self {
        crate::helpers::init::logging();
        let database =
            crate::meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
        let setup = crate::meteroid_it::container::start_meteroid_with_clients(
            database,
            SeedLevel::PLANS,
            Arc::new(stream.clone()),
            Arc::new(MockMailerService::new()),
        )
        .await;
        let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
        let api_key = Self::create_key(&setup, auth.token.as_str(), "testslug").await;
        Self {
            setup,
            http: http_client_tracing::client(),
            auth_token: auth.token,
            api_key,
        }
    }

    async fn create_key(setup: &MeteroidSetup, token: &str, tenant_slug: &str) -> String {
        AllClients::from_channel(setup.channel.clone(), token, "TESTORG", tenant_slug)
            .api_tokens
            .clone()
            .create_api_token(api::apitokens::v1::CreateApiTokenRequest {
                name: "usage-costs".to_owned(),
            })
            .await
            .unwrap()
            .into_inner()
            .api_key
    }

    /// An API key of a new tenant of the same organization.
    async fn other_tenant_key(
        &self,
        environment: api::tenants::v1::TenantEnvironmentEnum,
    ) -> String {
        let tenant = AllClients::from_channel(
            self.setup.channel.clone(),
            self.auth_token.as_str(),
            "TESTORG",
            "testslug",
        )
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: "Other tenant".to_owned(),
            environment: environment as i32,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: 0,
        })
        .await
        .unwrap()
        .into_inner()
        .tenant
        .unwrap();
        Self::create_key(&self.setup, self.auth_token.as_str(), tenant.slug.as_str()).await
    }

    async fn insert_metric(
        &self,
        code: &str,
        aggregation_type: BillingMetricAggregateEnum,
        usage_group_key: Option<&str>,
        segmentation_matrix: Option<SegmentationMatrix>,
    ) -> BillableMetricId {
        self.setup
            .store
            .insert_billable_metric(
                Actor::System,
                BillableMetricNew {
                    name: code.to_owned(),
                    description: None,
                    code: code.to_owned(),
                    aggregation_type,
                    aggregation_key: Some("value".to_owned()),
                    unit_conversion_factor: None,
                    unit_conversion_rounding: None,
                    segmentation_matrix,
                    filters: None,
                    usage_group_key: usage_group_key.map(str::to_owned),
                    unit_label: None,
                    tenant_id: TENANT_ID,
                    product_family_id: PRODUCT_FAMILY_ID,
                    product_id: None,
                },
            )
            .await
            .unwrap()
            .id
    }

    fn url(&self, customer: &str, suffix: &str, query: &str) -> String {
        format!(
            "{}/api/v1/usage/customer/{customer}/{suffix}?{query}",
            self.setup.config.rest_api_external_url
        )
    }

    fn subscription_url(&self, subscription: SubscriptionId, suffix: &str, query: &str) -> String {
        format!(
            "{}/api/v1/usage/subscription/{}/{suffix}?{query}",
            self.setup.config.rest_api_external_url,
            subscription.as_base62()
        )
    }

    async fn subscription_events(&self, subscription: SubscriptionId, metric: &str) -> Value {
        let query = format!(
            "metric={metric}&from={}&to={}",
            ts(start()),
            ts(window_end())
        );
        let (status, body) = self
            .get(self.subscription_url(subscription, "events", &query))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn subscription_costs_raw(
        &self,
        subscription: SubscriptionId,
        metric: &str,
    ) -> (StatusCode, Value) {
        let query = format!(
            "metric={metric}&granularity=DAY&from={}&to={}",
            ts(start()),
            ts(window_end())
        );
        self.get(self.subscription_url(subscription, "costs", &query))
            .await
    }

    async fn subscription_costs(&self, subscription: SubscriptionId, metric: &str) -> Value {
        let (status, body) = self.subscription_costs_raw(subscription, metric).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn running_total(&self, subscription: SubscriptionId, query: &str) -> Value {
        let url = format!(
            "{}/api/v1/usage/subscription/{}?{query}",
            self.setup.config.rest_api_external_url,
            subscription.as_base62()
        );
        let (status, body) = self.get(url).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn get_as(&self, url: String, key: Option<&str>) -> (StatusCode, Value) {
        let mut request = self.http.get(url);
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        (
            status,
            serde_json::from_str(&body).unwrap_or(Value::String(body)),
        )
    }

    async fn get(&self, url: String) -> (StatusCode, Value) {
        self.get_as(url, Some(&self.api_key)).await
    }

    async fn events(&self, customer: CustomerId, metric: &str, extra: &str) -> (StatusCode, Value) {
        let query = format!(
            "metric={metric}&from={}&to={}{extra}",
            ts(start()),
            ts(window_end())
        );
        self.get(self.url(&customer.as_base62(), "events", &query))
            .await
    }

    async fn costs(
        &self,
        customer: CustomerId,
        metric: &str,
        granularity: &str,
        group_by: Option<&str>,
    ) -> Value {
        let (status, body) = self
            .costs_raw(customer, metric, granularity, group_by)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn costs_raw(
        &self,
        customer: CustomerId,
        metric: &str,
        granularity: &str,
        group_by: Option<&str>,
    ) -> (StatusCode, Value) {
        self.costs_between(
            customer,
            metric,
            granularity,
            group_by,
            (start(), window_end()),
        )
        .await
    }

    async fn costs_between(
        &self,
        customer: CustomerId,
        metric: &str,
        granularity: &str,
        group_by: Option<&str>,
        (from, to): (NaiveDateTime, NaiveDateTime),
    ) -> (StatusCode, Value) {
        let mut query = format!(
            "metric={metric}&granularity={granularity}&from={}&to={}",
            ts(from),
            ts(to)
        );
        if let Some(group_by) = group_by {
            query.push_str(&format!("&group_by={group_by}"));
        }
        self.get(self.url(&customer.as_base62(), "costs", &query))
            .await
    }

    async fn invoiced(&self, subscription_id: SubscriptionId, metric: BillableMetricId) -> Decimal {
        let total = self
            .invoiced_usage_where(subscription_id, |id| id == metric)
            .await;
        assert!(total > Decimal::ZERO, "the invoice bills the metric");
        total
    }

    /// What the usage lines of the subscription's upcoming invoice add up to, in EUR.
    async fn invoiced_usage_where(
        &self,
        subscription_id: SubscriptionId,
        metric: impl Fn(BillableMetricId) -> bool,
    ) -> Decimal {
        Decimal::new(self.invoiced_minor(subscription_id, metric).await, 2)
    }

    /// The same, in the invoice currency's minor units.
    async fn invoiced_minor(
        &self,
        subscription_id: SubscriptionId,
        metric: impl Fn(BillableMetricId) -> bool,
    ) -> i64 {
        self.upcoming_usage_lines(subscription_id)
            .await
            .iter()
            .filter(|line| line.metric_id.is_some_and(&metric))
            .map(|line| line.amount_subtotal)
            .sum()
    }

    async fn upcoming_usage_lines(
        &self,
        subscription_id: SubscriptionId,
    ) -> Vec<meteroid_store::domain::LineItem> {
        let mut conn = self.setup.store.get_conn().await.unwrap();
        let details = self
            .setup
            .store
            .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
            .await
            .unwrap();
        let lines = self
            .setup
            .services
            .compute_upcoming_invoice(&details)
            .await
            .unwrap()
            .invoice_lines;
        lines
            .into_iter()
            .filter(|line| line.metric_id.is_some())
            .collect()
    }
}

struct Env {
    api: Api,
    regional: BillableMetricId,
    per_unit: SubscriptionId,
    tiered: SubscriptionId,
    volume: SubscriptionId,
    matrix: SubscriptionId,
}

fn start() -> NaiveDateTime {
    started_on().and_time(NaiveTime::MIN)
}

impl Env {
    async fn new() -> Self {
        let stream = EventStream::default();
        let api = Api::start(&stream).await;
        let setup = &api.setup;
        let regional = api
            .insert_metric(
                "regional_traffic",
                BillingMetricAggregateEnum::Sum,
                None,
                Some(SegmentationMatrix::Single(Dimension {
                    key: "region".to_owned(),
                    values: vec!["eu".to_owned(), "us".to_owned()],
                })),
            )
            .await;

        let at = |days: i64, hours: u32, minutes: u32| {
            start() + Duration::days(days) + Duration::minutes(i64::from(hours * 60 + minutes))
        };
        for customer in [CUST_UBER_ID, CUST_SPOTIFY_ID, CUST_COMODO_ID] {
            stream.add(customer, "bandwidth", at(0, 1, 0), "eu", 400);
            stream.add(customer, "bandwidth", at(0, 5, 0), "us", 300);
            stream.add(customer, "bandwidth", at(1, 2, 0), "eu", 900);
            stream.add(customer, "bandwidth", at(2, 3, 0), "us", 500);
            stream.add(customer, "bandwidth", at(2, 3, 30), "eu", 100);
        }
        stream.add(CUST_UBER_ID, "regional_traffic", at(0, 1, 0), "eu", 1000);
        stream.add(CUST_UBER_ID, "regional_traffic", at(0, 2, 0), "us", 500);
        stream.add(CUST_UBER_ID, "regional_traffic", at(1, 0, 0), "eu", 250);

        let bandwidth = |name: &str, model| metric_component(name, METRIC_BANDWIDTH, model);
        let per_unit = subscribe(
            setup,
            CUST_UBER_ID,
            vec![bandwidth(
                "Per unit",
                UsagePricingModel::PerUnit { rate: dec!(0.07) },
            )],
        )
        .await;
        let matrix_subscription = subscribe(
            setup,
            CUST_UBER_ID,
            vec![metric_component("Matrix", regional, matrix())],
        )
        .await;
        let tiered = subscribe(
            setup,
            CUST_SPOTIFY_ID,
            vec![bandwidth(
                "Tiered",
                UsagePricingModel::Tiered {
                    tiers: tiers(),
                    block_size: None,
                },
            )],
        )
        .await;
        let volume = subscribe(
            setup,
            CUST_COMODO_ID,
            vec![bandwidth(
                "Volume",
                UsagePricingModel::Volume {
                    tiers: tiers(),
                    block_size: None,
                },
            )],
        )
        .await;

        Self {
            api,
            regional,
            per_unit,
            tiered,
            volume,
            matrix: matrix_subscription,
        }
    }
}

impl std::ops::Deref for Env {
    type Target = Api;

    fn deref(&self) -> &Api {
        &self.api
    }
}

fn window_end() -> NaiveDateTime {
    start() + Duration::days(35)
}

fn ts(at: NaiveDateTime) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn amount(value: &Value) -> Decimal {
    Decimal::from_str(value.as_str().expect("a decimal string")).unwrap()
}

fn total_cost(body: &Value) -> Decimal {
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| amount(&row["cost"]["amount"]))
        .sum()
}

fn total_quantity(body: &Value) -> Decimal {
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| amount(&row["quantity"]))
        .sum()
}

#[tokio::test]
async fn test_events_carry_their_exact_cost_for_per_unit_and_matrix_pricing() {
    let env = Env::new().await;

    let (status, body) = env.events(CUST_UBER_ID, "bandwidth", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["data"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    assert!(body.get("next_cursor").is_none());
    for row in rows {
        let quantity = amount(&row["quantity"]);
        assert_eq!(
            quantity.to_string(),
            row["properties"]["value"].as_str().unwrap()
        );
        assert_eq!(amount(&row["cost"]["amount"]), quantity * dec!(0.07));
        assert_eq!(row["cost"]["currency"], "EUR");
    }
    let times: Vec<_> = rows
        .iter()
        .map(|row| row["timestamp"].as_str().unwrap())
        .collect();
    let mut sorted = times.clone();
    sorted.sort();
    assert_eq!(times, sorted, "oldest first");

    let (status, body) = env.events(CUST_UBER_ID, "regional_traffic", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let per_region: Vec<_> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["properties"]["region"].as_str().unwrap().to_owned(),
                amount(&row["quantity"]),
                amount(&row["cost"]["amount"]),
            )
        })
        .collect();
    assert_eq!(
        per_region,
        vec![
            ("eu".to_owned(), dec!(1000), dec!(20.00)),
            ("us".to_owned(), dec!(500), dec!(15.00)),
            ("eu".to_owned(), dec!(250), dec!(5.00)),
        ]
    );

    let metric_id = env.regional.as_base62();
    let (status, by_id) = env.events(CUST_UBER_ID, &metric_id, "").await;
    assert_eq!(status, StatusCode::OK, "{by_id}");
    assert_eq!(by_id, body, "a metric is addressed by id as by code");
}

#[tokio::test]
async fn test_events_cost_is_null_when_the_price_depends_on_the_period() {
    let env = Env::new().await;
    for customer in [CUST_SPOTIFY_ID, CUST_COMODO_ID] {
        let (status, body) = env.events(customer, "bandwidth", "").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let rows = body["data"].as_array().unwrap();
        assert_eq!(rows.len(), 5);
        for row in rows {
            assert!(row["cost"].is_null(), "{row}");
            assert!(row["quantity"].is_string());
        }
    }
}

#[tokio::test]
async fn test_events_are_paged_by_cursor_without_gaps_or_repeats() {
    let env = Env::new().await;
    let mut seen = Vec::new();
    let mut extra = "&limit=2".to_owned();
    let mut pages = 0;
    loop {
        let (status, body) = env.events(CUST_UBER_ID, "bandwidth", &extra).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        pages += 1;
        seen.extend(
            body["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap().to_owned()),
        );
        match body["next_cursor"].as_str() {
            Some(cursor) => extra = format!("&limit=2&cursor={cursor}"),
            None => break,
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(
        seen,
        (0..5).map(|i| format!("evt-{i:04}")).collect::<Vec<_>>()
    );

    let (status, _) = env
        .events(CUST_UBER_ID, "bandwidth", "&cursor=not-a-cursor")
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_linear_bucket_costs_add_up_to_the_invoice_line_and_are_exact() {
    let env = Env::new().await;
    let expected = env.invoiced(env.per_unit, METRIC_BANDWIDTH).await;
    for granularity in ["HOUR", "DAY", "MONTH"] {
        let body = env
            .costs(CUST_UBER_ID, "bandwidth", granularity, None)
            .await;
        let rows = body["data"].as_array().unwrap();
        assert!(rows.iter().all(|row| row["exact"] == true), "{body}");
        assert!(rows.iter().all(|row| row["cost"]["currency"] == "EUR"));
        assert_eq!(total_cost(&body), expected, "{granularity}");
        assert_eq!(total_quantity(&body), dec!(2200), "{granularity}");
    }

    let daily = env.costs(CUST_UBER_ID, "bandwidth", "DAY", None).await;
    let rows = daily["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(amount(&rows[0]["cost"]["amount"]), dec!(49.00));
    assert!(rows[0].get("group").is_none());

    let grouped = env
        .costs(CUST_UBER_ID, "bandwidth", "DAY", Some("region"))
        .await;
    assert_eq!(total_cost(&grouped), expected);
    assert!(
        grouped["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["exact"] == true && row["group"].is_string())
    );
}

#[tokio::test]
async fn test_matrix_costs_follow_the_dimension_and_add_up_to_the_invoice_line() {
    let env = Env::new().await;
    let expected = env.invoiced(env.matrix, env.regional).await;
    assert_eq!(expected, dec!(40.00));

    for group_by in [None, Some("region")] {
        let body = env
            .costs(CUST_UBER_ID, "regional_traffic", "DAY", group_by)
            .await;
        assert_eq!(total_cost(&body), expected, "{group_by:?}");
        assert!(
            body["data"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["exact"] == true),
            "{body}"
        );
    }

    let body = env
        .costs(CUST_UBER_ID, "regional_traffic", "MONTH", Some("region"))
        .await;
    // The window starts 3 days ago, so it can span two calendar months: sum each group's buckets.
    let mut by_group: BTreeMap<String, Decimal> = BTreeMap::new();
    for row in body["data"].as_array().unwrap() {
        *by_group
            .entry(row["group"].as_str().unwrap().to_owned())
            .or_default() += amount(&row["cost"]["amount"]);
    }
    assert_eq!(
        by_group,
        BTreeMap::from([
            ("eu".to_owned(), dec!(25.00)),
            ("us".to_owned(), dec!(15.00))
        ])
    );
}

/// Day-by-day, per-group matrix costs are exact cell by cell, not only summed to the invoice line
/// or summed per group across the whole month.
#[tokio::test]
async fn test_matrix_costs_by_day_and_region_are_exact_cell_by_cell() {
    let env = Env::new().await;
    let body = env
        .costs(CUST_UBER_ID, "regional_traffic", "DAY", Some("region"))
        .await;
    let rows = body["data"].as_array().unwrap();
    let cell_of = |row: &Value| -> chrono::DateTime<Utc> {
        row["bucket_start"]
            .as_str()
            .unwrap()
            .parse::<chrono::DateTime<Utc>>()
            .unwrap()
    };
    let cells: BTreeMap<(chrono::DateTime<Utc>, String), Decimal> = rows
        .iter()
        .map(|row| {
            (
                (cell_of(row), row["group"].as_str().unwrap().to_owned()),
                amount(&row["cost"]["amount"]),
            )
        })
        .collect();
    assert_eq!(
        cells,
        BTreeMap::from([
            ((start().and_utc(), "eu".to_owned()), dec!(20.00)),
            ((start().and_utc(), "us".to_owned()), dec!(15.00)),
            (
                ((start() + Duration::days(1)).and_utc(), "eu".to_owned()),
                dec!(5.00)
            ),
        ]),
        "day0/eu=1000*0.02, day0/us=500*0.03, day1/eu=250*0.02; day1/us has no events"
    );
    assert!(
        rows.iter()
            .all(|row| row["exact"] == true && row["cost"]["currency"] == "EUR"),
        "{body}"
    );
}

#[tokio::test]
async fn test_tiered_and_volume_bucket_costs_add_up_to_the_invoice_line_and_are_not_exact() {
    let env = Env::new().await;
    for (customer, subscription) in [(CUST_SPOTIFY_ID, env.tiered), (CUST_COMODO_ID, env.volume)] {
        let expected = env.invoiced(subscription, METRIC_BANDWIDTH).await;
        for granularity in ["HOUR", "DAY", "MONTH"] {
            for group_by in [None, Some("region")] {
                let body = env
                    .costs(customer, "bandwidth", granularity, group_by)
                    .await;
                let rows = body["data"].as_array().unwrap();
                assert!(!rows.is_empty());
                assert!(rows.iter().all(|row| row["exact"] == false), "{body}");
                assert_eq!(
                    total_cost(&body),
                    expected,
                    "{customer} {granularity} {group_by:?}"
                );
                assert_eq!(total_quantity(&body), dec!(2200));
            }
        }
    }

    let daily = env.costs(CUST_SPOTIFY_ID, "bandwidth", "DAY", None).await;
    let rows = daily["data"].as_array().unwrap();
    assert_eq!(
        amount(&rows[0]["cost"]["amount"]),
        dec!(70.00),
        "the first 700 units sit in the first tier"
    );
    assert!(
        rows.iter()
            .all(|row| amount(&row["cost"]["amount"]) >= Decimal::ZERO)
    );
}

#[tokio::test]
async fn test_ranges_and_metrics_are_validated() {
    let env = Env::new().await;
    let uber = CUST_UBER_ID.as_base62();

    let too_wide = format!(
        "metric=bandwidth&granularity=DAY&from={}&to={}",
        ts(start()),
        ts(start() + Duration::days(94))
    );
    let (status, body) = env.get(env.url(&uber, "costs", &too_wide)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let widest = too_wide.replace(
        &ts(start() + Duration::days(94)),
        &ts(start() + Duration::days(93)),
    );
    let (status, body) = env.get(env.url(&uber, "costs", &widest)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let inverted = format!(
        "metric=bandwidth&from={}&to={}",
        ts(start() + Duration::days(1)),
        ts(start())
    );
    let (status, _) = env.get(env.url(&uber, "events", &inverted)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = env
        .get(env.url(&uber, "events", &too_wide.replace("&granularity=DAY", "")))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = env
        .get(env.url(
            &uber,
            "costs",
            &format!(
                "metric=bandwidth&granularity=WEEK&from={}&to={}",
                ts(start()),
                ts(window_end())
            ),
        ))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let year_ago = start() - Duration::days(400);
    let outside = format!(
        "metric=bandwidth&granularity=DAY&from={}&to={}",
        ts(year_ago),
        ts(year_ago + Duration::days(30))
    );
    let (status, body) = env.get(env.url(&uber, "costs", &outside)).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "outside any billing period: {body}"
    );
}

#[tokio::test]
async fn test_unknown_customers_and_metrics_are_not_found_and_calls_are_authenticated() {
    let env = Env::new().await;

    let (status, _) = env.events(CUST_UBER_ID, "no_such_metric", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = env
        .costs_raw(CUST_UBER_ID, "no_such_metric", "DAY", None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = env.events(CustomerId::new(), "bandwidth", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = env
        .costs_raw(CustomerId::new(), "bandwidth", "DAY", None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let query = format!(
        "metric=bandwidth&granularity=DAY&from={}&to={}",
        ts(start()),
        ts(window_end())
    );
    let uber = CUST_UBER_ID.as_base62();
    for suffix in ["events", "costs"] {
        let (status, _) = env.get_as(env.url(&uber, suffix, &query), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = env
            .get_as(env.url(&uber, suffix, &query), Some("not-a-key"))
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn test_linear_bucket_costs_reconcile_with_the_invoice_in_minor_units() {
    let stream = EventStream::default();
    let api = Api::start(&stream).await;
    let calls = api
        .insert_metric(
            "calls",
            BillingMetricAggregateEnum::Sum,
            Some("region"),
            None,
        )
        .await;
    let subscription = subscribe(
        &api.setup,
        CUST_UBER_ID,
        vec![metric_component(
            "Calls",
            calls,
            UsagePricingModel::PerUnit { rate: dec!(0.0015) },
        )],
    )
    .await;

    let day = |days: i64, hours: i64| start() + Duration::days(days) + Duration::hours(hours);
    let events = [
        (day(0, 1), "eu", "1000.3"),
        (day(1, 2), "eu", "2000.4"),
        (day(2, 3), "eu", "333.3"),
        (day(0, 5), "us", "501.7"),
        (day(2, 4), "us", "0.6"),
        (day(1, 9), "sa", "7.77"),
        (day(0, 6), "ap", "40"),
        (day(1, 6), "ap", "-100"),
        (day(2, 6), "ap", "30"),
    ];
    for (at, region, value) in events {
        stream.add_value(CUST_UBER_ID, "calls", at, region, value);
    }

    let expected = api.invoiced(subscription, calls).await;
    assert_eq!(
        expected,
        dec!(5.76),
        "each region is rounded to cents on its own; a net-negative region is not billed"
    );

    for granularity in ["HOUR", "DAY", "MONTH"] {
        for group_by in [None, Some("region")] {
            let (status, body) = api
                .costs_raw(CUST_UBER_ID, "calls", granularity, group_by)
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let rows = body["data"].as_array().unwrap();
            assert!(rows.iter().all(|row| row["exact"] == true), "{body}");
            assert_eq!(total_cost(&body), expected, "{granularity} {group_by:?}");
            let usage: Decimal = events
                .iter()
                .map(|(_, _, value)| Decimal::from_str(value).unwrap())
                .sum();
            assert_eq!(total_quantity(&body), usage, "{granularity} {group_by:?}");
        }
    }

    let (status, body) = api.events(CUST_UBER_ID, "calls", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for row in body["data"].as_array().unwrap() {
        assert_eq!(
            amount(&row["cost"]["amount"]),
            amount(&row["quantity"]) * dec!(0.0015),
            "an event's cost is not rounded"
        );
    }
}

#[tokio::test]
async fn test_period_priced_costs_do_not_depend_on_where_the_range_starts() {
    let env = Env::new().await;
    let cases = [
        ("DAY", start() + Duration::days(1) + Duration::hours(6)),
        (
            "HOUR",
            start() + Duration::days(1) + Duration::hours(2) + Duration::minutes(30),
        ),
    ];
    for customer in [CUST_SPOTIFY_ID, CUST_COMODO_ID] {
        for (granularity, from) in cases {
            for group_by in [None, Some("region")] {
                let (status, full) = env
                    .costs_between(
                        customer,
                        "bandwidth",
                        granularity,
                        group_by,
                        (start(), window_end()),
                    )
                    .await;
                assert_eq!(status, StatusCode::OK, "{full}");
                let (status, partial) = env
                    .costs_between(
                        customer,
                        "bandwidth",
                        granularity,
                        group_by,
                        (from, window_end()),
                    )
                    .await;
                assert_eq!(status, StatusCode::OK, "{partial}");

                let first = partial["data"][0]["bucket_start"].as_str().unwrap();
                let expected: Vec<_> = full["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|row| row["bucket_start"].as_str().unwrap() >= first)
                    .collect();
                let got: Vec<_> = partial["data"].as_array().unwrap().iter().collect();
                assert!(got.len() < full["data"].as_array().unwrap().len());
                assert_eq!(got, expected, "{customer} {granularity} {group_by:?}");
            }
        }
    }
}

#[tokio::test]
async fn test_monthly_buckets_of_a_non_additive_metric_span_anniversary_billing_periods() {
    let stream = EventStream::default();
    let api = Api::start(&stream).await;
    let peak = api
        .insert_metric("peak", BillingMetricAggregateEnum::Max, None, None)
        .await;

    let this_month = Utc::now().date_naive().with_day(1).unwrap();
    let day = |months_back: u32, day_of_month: u32| {
        (this_month - Months::new(months_back))
            .with_day(day_of_month)
            .unwrap()
    };
    let subscription_start = day(2, 15);
    subscribe_from(
        &api.setup,
        CUST_UBER_ID,
        vec![metric_component(
            "Peak",
            peak,
            UsagePricingModel::PerUnit { rate: dec!(1) },
        )],
        subscription_start,
    )
    .await;
    for _ in 0..3 {
        api.setup
            .services
            .get_and_process_cycle_transitions()
            .await
            .unwrap();
        api.setup
            .services
            .get_and_process_due_events()
            .await
            .unwrap();
    }

    let at = |date: NaiveDate| date.and_time(NaiveTime::MIN) + Duration::hours(3);
    stream.add(CUST_UBER_ID, "peak", at(day(2, 20)), "eu", 7);
    stream.add(CUST_UBER_ID, "peak", at(day(1, 10)), "eu", 30);
    stream.add(CUST_UBER_ID, "peak", at(day(1, 20)), "eu", 12);

    let (status, body) = api
        .costs_between(
            CUST_UBER_ID,
            "peak",
            "MONTH",
            None,
            (
                subscription_start.and_time(NaiveTime::MIN),
                this_month.and_time(NaiveTime::MIN),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["data"].as_array().unwrap();
    let quantities: Vec<_> = rows.iter().map(|row| amount(&row["quantity"])).collect();
    assert_eq!(
        quantities,
        vec![dec!(7), dec!(30)],
        "the month that spans two billing periods is one maximum"
    );
    assert!(rows.iter().all(|row| row["cost"].is_null()), "{body}");
}

#[tokio::test]
async fn test_metrics_sharing_a_code_are_priced_by_the_metric_the_customer_is_billed_for() {
    let stream = EventStream::default();
    let api = Api::start(&stream).await;
    let billed = api
        .insert_metric("shared", BillingMetricAggregateEnum::Sum, None, None)
        .await;
    let unrelated = api
        .insert_metric("shared", BillingMetricAggregateEnum::Sum, None, None)
        .await;
    let subscription = subscribe(
        &api.setup,
        CUST_UBER_ID,
        vec![metric_component(
            "Shared",
            billed,
            UsagePricingModel::PerUnit { rate: dec!(0.07) },
        )],
    )
    .await;
    stream.add(
        CUST_UBER_ID,
        "shared",
        start() + Duration::hours(1),
        "eu",
        400,
    );
    stream.add(
        CUST_UBER_ID,
        "shared",
        start() + Duration::days(1),
        "us",
        300,
    );

    let expected = api.invoiced(subscription, billed).await;
    assert_eq!(expected, dec!(49.00));

    for reference in [billed.as_base62(), "shared".to_owned()] {
        let (status, body) = api.costs_raw(CUST_UBER_ID, &reference, "DAY", None).await;
        assert_eq!(status, StatusCode::OK, "{reference}: {body}");
        assert_eq!(total_cost(&body), expected, "{reference}");
    }

    let (status, body) = api
        .costs_raw(CUST_UBER_ID, &unrelated.as_base62(), "DAY", None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn test_another_tenants_customers_are_not_found() {
    let env = Env::new().await;
    let other_key = env
        .other_tenant_key(api::tenants::v1::TenantEnvironmentEnum::Development)
        .await;

    let query = format!(
        "metric=bandwidth&granularity=DAY&from={}&to={}",
        ts(start()),
        ts(window_end())
    );
    let uber = CUST_UBER_ID.as_base62();
    for suffix in ["events", "costs"] {
        let (status, body) = env.get(env.url(&uber, suffix, &query)).await;
        assert_eq!(status, StatusCode::OK, "{suffix}: {body}");
        let (status, body) = env
            .get_as(env.url(&uber, suffix, &query), Some(&other_key))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{suffix}: {body}");
    }
}

async fn split_by_region() -> (Api, SubscriptionId, SubscriptionId) {
    let stream = EventStream::default();
    let api = Api::start(&stream).await;
    let at = |days: i64, hours: i64| start() + Duration::days(days) + Duration::hours(hours);
    stream.add(CUST_UBER_ID, "bandwidth", at(0, 1), "eu", 400);
    stream.add(CUST_UBER_ID, "bandwidth", at(0, 5), "us", 300);
    stream.add(CUST_UBER_ID, "bandwidth", at(1, 2), "eu", 900);
    stream.add(CUST_UBER_ID, "bandwidth", at(2, 3), "us", 500);
    stream.add(CUST_UBER_ID, "bandwidth", at(2, 4), "eu", 100);
    let bandwidth = |name: &str, rate| {
        metric_component(name, METRIC_BANDWIDTH, UsagePricingModel::PerUnit { rate })
    };
    let europe = subscribe_filtered(
        &api.setup,
        CUST_UBER_ID,
        vec![bandwidth("Europe", dec!(0.07))],
        started_on(),
        Some(UsageFilter::new("region".to_owned(), "eu".to_owned()).unwrap()),
    )
    .await;
    let elsewhere = subscribe(
        &api.setup,
        CUST_UBER_ID,
        vec![bandwidth("Elsewhere", dec!(0.10))],
    )
    .await;
    (api, europe, elsewhere)
}

fn regions(body: &Value) -> Vec<String> {
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["properties"]["region"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn test_a_subscription_reads_only_the_events_its_usage_filter_bills() {
    let (api, europe, elsewhere) = split_by_region().await;

    let events = api.subscription_events(europe, "bandwidth").await;
    assert_eq!(regions(&events), vec!["eu", "eu", "eu"]);
    for row in events["data"].as_array().unwrap() {
        assert_eq!(
            amount(&row["cost"]["amount"]),
            amount(&row["quantity"]) * dec!(0.07)
        );
    }
    let events = api.subscription_events(elsewhere, "bandwidth").await;
    assert_eq!(regions(&events), vec!["us", "us"]);

    let europe_costs = api.subscription_costs(europe, "bandwidth").await;
    assert_eq!(total_cost(&europe_costs), dec!(98.00));
    assert_eq!(
        total_cost(&europe_costs),
        api.invoiced(europe, METRIC_BANDWIDTH).await
    );
    let elsewhere_costs = api.subscription_costs(elsewhere, "bandwidth").await;
    assert_eq!(total_cost(&elsewhere_costs), dec!(80.00));
    assert_eq!(
        total_cost(&elsewhere_costs),
        api.invoiced(elsewhere, METRIC_BANDWIDTH).await
    );

    let customer = api.costs(CUST_UBER_ID, "bandwidth", "DAY", None).await;
    assert_eq!(total_cost(&customer), dec!(178.00));
    assert_eq!(total_quantity(&customer), dec!(2200));
}

#[tokio::test]
async fn test_a_subscription_reads_only_metrics_it_bills_and_its_own_tenants() {
    let (api, europe, _) = split_by_region().await;
    api.insert_metric("unbilled", BillingMetricAggregateEnum::Sum, None, None)
        .await;

    let (status, body) = api.subscription_costs_raw(europe, "unbilled").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = api
        .subscription_costs_raw(SubscriptionId::new(), "bandwidth")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let other_key = api
        .other_tenant_key(api::tenants::v1::TenantEnvironmentEnum::Development)
        .await;
    let query = format!(
        "metric=bandwidth&granularity=DAY&from={}&to={}",
        ts(start()),
        ts(window_end())
    );
    for suffix in ["events", "costs"] {
        let (status, body) = api
            .get_as(
                api.subscription_url(europe, suffix, &query),
                Some(&other_key),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{suffix}: {body}");
        let (status, _) = api
            .get_as(api.subscription_url(europe, suffix, &query), None)
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{suffix}");
    }
}

fn metric_usage<'a>(body: &'a Value, code: &str) -> &'a Value {
    body["usage"]
        .as_array()
        .unwrap()
        .iter()
        .find(|metric| metric["metric_code"] == code)
        .unwrap_or_else(|| panic!("{code} in {body}"))
}

#[tokio::test]
async fn test_the_current_period_reading_carries_the_running_total_of_the_next_invoice() {
    let env = Env::new().await;

    for (subscription, quantity) in [(env.per_unit, dec!(2200)), (env.tiered, dec!(2200))] {
        let expected = env.invoiced(subscription, METRIC_BANDWIDTH).await;
        let body = env.running_total(subscription, "").await;
        let bandwidth = metric_usage(&body, "bandwidth");
        assert_eq!(amount(&bandwidth["total_value"]), quantity);
        let costs = bandwidth["costs"].as_array().unwrap();
        assert_eq!(costs.len(), 1, "{body}");
        assert_eq!(amount(&costs[0]["amount"]), expected);
        assert_eq!(costs[0]["currency"], "EUR");
        assert_eq!(costs[0]["unit"], "CURRENCY");
        assert_eq!(body["totals"], serde_json::json!(costs));
        assert!(body["computed_at"].is_string());

        let again = env.running_total(subscription, "").await;
        assert_eq!(
            again["computed_at"], body["computed_at"],
            "served from the cache"
        );
    }

    let filtered = env
        .running_total(
            env.per_unit,
            &format!("metric_id={}", env.regional.as_base62()),
        )
        .await;
    assert!(
        filtered["usage"].as_array().unwrap().is_empty(),
        "{filtered}"
    );
    assert_eq!(filtered["totals"], serde_json::json!([]));
    let bandwidth_only = env
        .running_total(
            env.per_unit,
            &format!("metric_id={}", METRIC_BANDWIDTH.as_base62()),
        )
        .await;
    assert_eq!(
        bandwidth_only["totals"],
        metric_usage(&bandwidth_only, "bandwidth")["costs"]
    );
    assert_eq!(
        amount(&bandwidth_only["totals"][0]["amount"]),
        env.invoiced(env.per_unit, METRIC_BANDWIDTH).await
    );

    let custom = env
        .running_total(
            env.per_unit,
            &format!(
                "start_date={}&end_date={}",
                started_on(),
                started_on() + chrono::Days::new(35)
            ),
        )
        .await;
    assert!(custom.get("totals").is_none(), "{custom}");
    assert!(custom.get("computed_at").is_none());
    assert!(metric_usage(&custom, "bandwidth").get("costs").is_none());
    assert_eq!(
        amount(&metric_usage(&custom, "bandwidth")["total_value"]),
        dec!(2200)
    );
}

fn running_amount(costs: &Value) -> Decimal {
    let costs = costs.as_array().expect("costs");
    assert!(costs.len() <= 1, "one currency: {costs:?}");
    costs
        .first()
        .map_or(Decimal::ZERO, |cost| amount(&cost["amount"]))
}

#[tokio::test]
async fn test_a_cancelled_subscription_adds_nothing_and_a_scheduled_one_bills_to_its_end() {
    let env = Env::new().await;
    let tomorrow = Utc::now().date_naive() + chrono::Days::new(1);
    env.setup
        .services
        .cancel_subscription(
            Actor::System,
            env.per_unit,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(tomorrow),
        )
        .await
        .unwrap();
    let scheduled = env.running_total(env.per_unit, "").await;
    assert_eq!(
        running_amount(&scheduled["totals"]),
        dec!(154.00),
        "{scheduled}"
    );

    env.setup
        .services
        .cancel_subscription(
            Actor::System,
            env.tiered,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await
        .unwrap();
    env.setup
        .services
        .get_and_process_due_events()
        .await
        .unwrap();
    let cancelled = env.running_total(env.tiered, "").await;
    assert_eq!(cancelled["totals"], serde_json::json!([]), "{cancelled}");
    assert_eq!(
        metric_usage(&cancelled, "bandwidth")["costs"],
        serde_json::json!([])
    );
}

#[tokio::test]
async fn test_a_metric_a_plan_change_removed_still_adds_to_the_running_total() {
    let stream = EventStream::default();
    let api = Api::start(&stream).await;
    let at = |days: i64| start() + Duration::days(days) + Duration::hours(1);
    stream.add(CUST_UBER_ID, "bandwidth", at(0), "eu", 400);
    stream.add(CUST_UBER_ID, "db_size", at(1), "eu", 40);
    let subscription = subscribe_on(
        &api.setup,
        CUST_UBER_ID,
        PLAN_VERSION_USAGE_BETA_ID,
        vec![],
        started_on(),
    )
    .await;
    api.setup
        .services
        .apply_plan_change_immediate_at(
            Actor::System,
            subscription,
            TENANT_ID,
            PLAN_VERSION_USAGE_ALPHA_ID,
            vec![],
            Utc::now().date_naive(),
        )
        .await
        .unwrap();

    let body = api.running_total(subscription, "").await;
    let db_size = metric_usage(&body, "db_size");
    assert_eq!(running_amount(&db_size["costs"]), dec!(20.00), "{body}");
    let listed: Decimal = body["usage"]
        .as_array()
        .unwrap()
        .iter()
        .map(|metric| running_amount(&metric["costs"]))
        .sum();
    assert_eq!(running_amount(&body["totals"]), listed);
    assert_eq!(
        running_amount(&body["totals"]),
        api.invoiced_usage_where(subscription, |_| true).await
    );

    let only = api
        .running_total(
            subscription,
            &format!("metric_id={}", METRIC_DATABASE_SIZE.as_base62()),
        )
        .await;
    assert_eq!(only["totals"], db_size["costs"]);
    let (status, costs) = api.subscription_costs_raw(subscription, "db_size").await;
    assert_eq!(status, StatusCode::OK, "{costs}");
}

#[tokio::test]
async fn test_credit_priced_usage_is_reported_in_its_credit_unit() {
    let stream = EventStream::default();
    let api = Api::start(&stream).await;
    stream.add(
        CUST_UBER_ID,
        "bandwidth",
        start() + Duration::hours(1),
        "eu",
        400,
    );
    let unit = api
        .setup
        .store
        .create_credit_unit(
            CreditUnitNew::new(TENANT_ID, "CREDITS", "Credit", "Credits", None, 0).unwrap(),
        )
        .await
        .unwrap();
    let mut component = metric_component(
        "Calls",
        METRIC_BANDWIDTH,
        UsagePricingModel::PerUnit { rate: dec!(2) },
    );
    let PriceEntry::New(price) = &mut component.price_entry else {
        unreachable!("metric_component prices a new entry")
    };
    price.credit_unit = Some(PriceCreditUnit {
        unit_id: unit.id,
        rate: dec!(0.01),
    });
    let subscription = subscribe(&api.setup, CUST_UBER_ID, vec![component]).await;

    let body = api.running_total(subscription, "").await;
    assert_eq!(
        body["totals"],
        serde_json::json!([{ "amount": "800", "currency": "CREDITS", "unit": "CREDIT_UNIT" }]),
        "{body}"
    );
}

/// Bandwidth of 2200 units over the period's first days, on CUST_UBER.
async fn api_with_bandwidth() -> Api {
    let stream = EventStream::default();
    let api = Api::start(&stream).await;
    let at = |days: i64, hours: i64| start() + Duration::days(days) + Duration::hours(hours);
    stream.add(CUST_UBER_ID, "bandwidth", at(0, 1), "eu", 400);
    stream.add(CUST_UBER_ID, "bandwidth", at(1, 2), "eu", 1300);
    stream.add(CUST_UBER_ID, "bandwidth", at(2, 3), "us", 500);
    api
}

fn per_unit_bandwidth(rate: Decimal, currency: &str) -> ExtraComponent {
    let mut component = metric_component(
        "Bandwidth",
        METRIC_BANDWIDTH,
        UsagePricingModel::PerUnit { rate },
    );
    let PriceEntry::New(price) = &mut component.price_entry else {
        unreachable!("metric_component prices a new entry")
    };
    price.currency = currency.to_owned();
    component
}

#[tokio::test]
async fn test_a_free_trial_adds_nothing_as_its_invoice_bills_no_usage() {
    let api = api_with_bandwidth().await;
    let subscription = subscribe_on(
        &api.setup,
        CUST_UBER_ID,
        PLAN_VERSION_PAID_FREE_TRIAL_ID,
        vec![per_unit_bandwidth(dec!(0.07), "EUR")],
        started_on(),
    )
    .await;
    let details = api
        .setup
        .store
        .get_subscription_details(TENANT_ID, subscription)
        .await
        .unwrap();
    assert_eq!(
        details.subscription.status,
        meteroid_store::domain::enums::SubscriptionStatusEnum::TrialActive
    );

    let body = api.running_total(subscription, "").await;
    assert_eq!(body["totals"], serde_json::json!([]), "{body}");
    let bandwidth = metric_usage(&body, "bandwidth");
    assert_eq!(amount(&bandwidth["total_value"]), dec!(2200));
    assert_eq!(bandwidth["costs"], serde_json::json!([]));
    assert_eq!(api.invoiced_minor(subscription, |_| true).await, 0);
}

#[tokio::test]
async fn test_tax_inclusive_usage_is_reported_as_its_line_quotes_it() {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;

    let api = api_with_bandwidth().await;
    {
        use diesel_models::schema::customer::dsl as c;
        use diesel_models::schema::plan_version::dsl as pv;
        let mut conn = api.setup.store.get_conn().await.unwrap();
        diesel::update(pv::plan_version.filter(pv::id.eq(PLAN_VERSION_1_LEETCODE_ID)))
            .set(pv::tax_inclusive.eq(true))
            .execute(&mut conn)
            .await
            .unwrap();
        let paris = serde_json::json!({
            "line1": "12 rue de Rivoli", "line2": null, "city": "Paris", "country": "FR",
            "state": null, "zip_code": "75004"
        });
        diesel::update(c::customer.filter(c::id.eq(CUST_UBER_ID)))
            .set(c::billing_address.eq(Some(paris)))
            .execute(&mut conn)
            .await
            .unwrap();
    }
    api.setup
        .store
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();
    let subscription = subscribe(
        &api.setup,
        CUST_UBER_ID,
        vec![per_unit_bandwidth(dec!(0.07), "EUR")],
    )
    .await;

    let body = api.running_total(subscription, "").await;
    assert_eq!(
        running_amount(&body["totals"]),
        dec!(154.00),
        "the quoted price, tax included: {body}"
    );
    let lines = api.upcoming_usage_lines(subscription).await;
    let gross: i64 = lines.iter().map(|line| line.amount_total).sum();
    let net: i64 = lines.iter().map(|line| line.amount_subtotal).sum();
    assert_eq!(running_amount(&body["totals"]), Decimal::new(gross, 2));
    assert_eq!(
        net, 12_833,
        "the invoice states the line net of its 20 % VAT"
    );
}

/// A published JPY plan with no fee of its own.
async fn jpy_plan_version(setup: &MeteroidSetup) -> common_domain::ids::PlanVersionId {
    use common_domain::ids::{PlanId, PlanVersionId};
    use diesel_models::enums::{PlanStatusEnum, PlanTypeEnum};
    use diesel_models::plan_versions::PlanVersionRowNew;
    use diesel_models::plans::{PlanRowNew, PlanRowPatch};

    let (plan_id, plan_version_id) = (PlanId::new(), PlanVersionId::new());
    let mut conn = setup.store.get_conn().await.unwrap();
    PlanRowNew {
        id: plan_id,
        name: "Yen plan".to_owned(),
        description: None,
        tenant_id: TENANT_ID,
        product_family_id: PRODUCT_FAMILY_ID,
        plan_type: PlanTypeEnum::Standard,
        status: PlanStatusEnum::Active,
    }
    .insert(&mut conn)
    .await
    .unwrap();
    PlanVersionRowNew {
        id: plan_version_id,
        is_draft_version: false,
        plan_id,
        version: 1,
        trial_duration_days: None,
        tenant_id: TENANT_ID,
        period_start_day: None,
        net_terms: 0,
        currency: "JPY".to_owned(),
        billing_cycles: None,
        trialing_plan_id: None,
        trial_is_free: false,
        uses_product_pricing: true,
        tax_inclusive: false,
        credit_exhausted_rule: None,
        credit_exhausted_caps: serde_json::json!({}),
    }
    .insert(&mut conn)
    .await
    .unwrap();
    PlanRowPatch {
        id: plan_id,
        tenant_id: TENANT_ID,
        name: None,
        description: None,
        active_version_id: Some(Some(plan_version_id)),
        draft_version_id: None,
        self_service_rank: None,
        issues_license: None,
    }
    .update(&mut conn)
    .await
    .unwrap();
    plan_version_id
}

#[tokio::test]
async fn test_a_zero_decimal_currency_is_reported_in_whole_units() {
    let api = api_with_bandwidth().await;
    let plan_version = jpy_plan_version(&api.setup).await;
    let subscription = subscribe_on(
        &api.setup,
        CUST_UBER_ID,
        plan_version,
        vec![per_unit_bandwidth(dec!(3), "JPY")],
        started_on(),
    )
    .await;

    let body = api.running_total(subscription, "").await;
    assert_eq!(
        body["totals"],
        serde_json::json!([{ "amount": "6600", "currency": "JPY", "unit": "CURRENCY" }]),
        "{body}"
    );
    assert_eq!(api.invoiced_minor(subscription, |_| true).await, 6_600);
}
