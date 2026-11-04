//! Sandbox presets billed through the engine, over an in-memory metering log.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use common_domain::ids::{BaseId, CustomerId, TenantId};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use error_stack::bail;
use metering_grpc::meteroid::metering::v1::event::CustomerId as EventCustomerId;
use meteroid_seeder::presets;
use meteroid_store::StoreResult;
use meteroid_store::clients::usage::{
    EventSearchOptions, EventSearchResult, GroupedUsageData, IngestEventsRequest,
    IngestEventsResult, UsageClient, UsageData, WindowedUsageData,
};
use meteroid_store::domain::{BillableMetric, SegmentationMatrix, UsagePeriod};
use meteroid_store::errors::StoreError;
use rust_decimal::Decimal;

use crate::data::ids::ORGANIZATION_ID;
use crate::harness::{TestEnv, test_env_with_usage};

struct LoggedEvent {
    customer: String,
    code: String,
    at: NaiveDateTime,
    properties: HashMap<String, String>,
}

/// Sums ingested events per customer like the metering backend, ignoring ids it already saw.
#[derive(Default)]
struct EventLog {
    events: Mutex<Vec<LoggedEvent>>,
    seen: Mutex<HashSet<(String, String)>>,
}

impl EventLog {
    fn aggregate(
        &self,
        customer: &CustomerId,
        metric: &BillableMetric,
        group_by: Option<&str>,
        since: NaiveDateTime,
        until: NaiveDateTime,
    ) -> Vec<GroupedUsageData> {
        let customer = customer.as_base62();
        let value_key = metric.aggregation_key.as_deref().unwrap_or_default();
        let mut kept: Vec<&str> = group_by.into_iter().collect();
        kept.extend(metric.usage_group_key.as_deref());
        if let Some(SegmentationMatrix::Single(dimension)) = &metric.segmentation_matrix {
            kept.push(&dimension.key);
        }
        let mut totals: BTreeMap<Vec<(String, String)>, Decimal> = BTreeMap::new();
        for event in self.events.lock().unwrap().iter() {
            if event.customer != customer
                || event.code != metric.code
                || event.at < since
                || event.at >= until
            {
                continue;
            }
            let Some(value) = event
                .properties
                .get(value_key)
                .and_then(|v| Decimal::from_str(v).ok())
            else {
                continue;
            };
            let mut dimensions: Vec<_> = kept
                .iter()
                .filter_map(|key| {
                    event
                        .properties
                        .get(*key)
                        .map(|v| ((*key).to_owned(), v.clone()))
                })
                .collect();
            dimensions.sort();
            dimensions.dedup();
            *totals.entry(dimensions).or_default() += value;
        }
        totals
            .into_iter()
            .map(|(dimensions, value)| GroupedUsageData {
                value,
                dimensions: dimensions.into_iter().collect(),
            })
            .collect()
    }

    fn read(
        &self,
        customer: &CustomerId,
        metric: &BillableMetric,
        group_by: Option<&str>,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_owned()));
        }
        Ok(UsageData {
            data: self.aggregate(customer, metric, group_by, period.start, period.end),
            period,
        })
    }
}

#[async_trait::async_trait]
impl UsageClient for EventLog {
    async fn fetch_usage(
        &self,
        _tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.read(customer_id, metric, None, period)
    }

    async fn fetch_usage_grouped_by(
        &self,
        _tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: &str,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.read(customer_id, metric, Some(group_by), period)
    }

    async fn fetch_total_usage(
        &self,
        _tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<Decimal> {
        Ok(self
            .read(customer_id, metric, None, period)?
            .data
            .into_iter()
            .map(|g| g.value)
            .sum())
    }

    async fn fetch_windowed_usage(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        _metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<WindowedUsageData> {
        Ok(WindowedUsageData {
            data: vec![],
            period,
        })
    }

    async fn fetch_usage_summary(
        &self,
        _tenant_id: &TenantId,
        customer_id: Option<&CustomerId>,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        let Some(customer_id) = customer_id else {
            bail!(StoreError::InvalidArgument(
                "the log only sums per customer".to_owned()
            ));
        };
        self.read(customer_id, metric, None, period)
    }

    async fn search_events(
        &self,
        _tenant_id: &TenantId,
        _options: EventSearchOptions,
    ) -> StoreResult<EventSearchResult> {
        Ok(EventSearchResult { events: vec![] })
    }

    async fn ingest_events(
        &self,
        _tenant_id: &TenantId,
        request: IngestEventsRequest,
    ) -> StoreResult<IngestEventsResult> {
        let mut events = self.events.lock().unwrap();
        let mut seen = self.seen.lock().unwrap();
        for event in request.events {
            let Some(EventCustomerId::MeteroidCustomerId(customer)) = event.customer_id else {
                continue;
            };
            if !seen.insert((customer.clone(), event.id)) {
                continue;
            }
            events.push(LoggedEvent {
                customer,
                code: event.code,
                at: DateTime::parse_from_rfc3339(&event.timestamp)
                    .expect("generated events carry RFC 3339 timestamps")
                    .naive_utc(),
                properties: event.properties.into_iter().collect(),
            });
        }
        Ok(IngestEventsResult { failures: vec![] })
    }
}

async fn customer_ids(env: &TestEnv, tenant_id: TenantId) -> HashMap<String, CustomerId> {
    use diesel_models::schema::customer::dsl as c;
    c::customer
        .filter(c::tenant_id.eq(tenant_id))
        .select((c::name, c::id))
        .load::<(String, CustomerId)>(&mut env.conn().await)
        .await
        .unwrap()
        .into_iter()
        .collect()
}

/// The default sandbox's future-start subscription (Halvorsen Freight) is activated by the
/// lifecycle into cycle 0 and renews into cycle 1, billing its first period's usage once.
#[tokio::test]
async fn pulse_future_start_subscription_renews_into_cycle_one() {
    use chrono::{Datelike, Months, NaiveDate, NaiveTime};
    use diesel_async::SimpleAsyncConnection;
    use diesel_models::enums::SubscriptionStatusEnum;
    use diesel_models::schema::{billable_metric::dsl as m, subscription::dsl as s};
    use meteroid_store::domain::LineItem;
    use meteroid_store::services::InvoiceDelivery;

    let log = Arc::new(EventLog::default());
    let env = test_env_with_usage(log.clone()).await;
    let tenant = presets::run_preset_at(
        env.store(),
        env.services(),
        presets::default_scenario(),
        ORGANIZATION_ID,
        Some("Pulse".to_string()),
        (Utc::now() - Duration::minutes(5)).naive_utc(),
    )
    .await
    .unwrap();
    let customer = customer_ids(&env, tenant.id).await["Halvorsen Freight"];
    let (subscription_id, status, cycle): (
        common_domain::ids::SubscriptionId,
        SubscriptionStatusEnum,
        Option<i32>,
    ) = s::subscription
        .filter(s::customer_id.eq(customer))
        .select((s::id, s::status, s::cycle_index))
        .first(&mut env.conn().await)
        .await
        .unwrap();
    assert_eq!(status, SubscriptionStatusEnum::PendingActivation);
    assert_eq!(cycle, None);

    // Its start (and its components') moved two months back, with 1,000 AI actions used in its
    // first period.
    let today = Utc::now().date_naive();
    let start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap() - Months::new(2);
    let renewal = start + Months::new(1);
    env.conn()
        .await
        .batch_execute(&format!(
            "UPDATE subscription SET start_date = '{start}', billing_start_date = '{start}',
                current_period_start = '{start}', current_period_end = '{start}',
                billing_day_anchor = 1, activated_at = '{start} 00:00:00',
                created_at = '{created} 09:00:00'
             WHERE id = '{id}';
             UPDATE invoice SET invoice_date = '{start}', billing_period_start = '{start}'
             WHERE subscription_id = '{id}';
             UPDATE subscription_component SET effective_from = '{start}'
             WHERE subscription_id = '{id}';",
            created = start - Duration::days(7),
            id = subscription_id.as_uuid()
        ))
        .await
        .unwrap();
    let metric: String = m::billable_metric
        .filter(m::tenant_id.eq(tenant.id))
        .select(m::code)
        .first(&mut env.conn().await)
        .await
        .unwrap();
    log.events.lock().unwrap().push(LoggedEvent {
        customer: customer.as_base62(),
        code: metric,
        at: (start + Duration::days(5)).and_time(NaiveTime::from_hms_opt(12, 0, 0).unwrap()),
        properties: HashMap::from([("value".to_string(), "1000".to_string())]),
    });

    let noon = |day: NaiveDate| {
        day.and_time(NaiveTime::from_hms_opt(12, 0, 0).unwrap())
            .and_utc()
    };
    let usage_cents = |lines: &[LineItem]| -> i64 {
        lines
            .iter()
            .filter(|line| line.metric_id.is_some())
            .map(|line| line.amount_subtotal)
            .sum()
    };
    let cycle_of = || async {
        s::subscription
            .filter(s::id.eq(subscription_id))
            .select(s::cycle_index)
            .first::<Option<i32>>(&mut env.conn().await)
            .await
            .unwrap()
    };

    let activation = env
        .services()
        .bill_subscription_period_at(
            tenant.id,
            subscription_id,
            noon(start),
            InvoiceDelivery::Suppressed,
        )
        .await
        .unwrap()
        .expect("the first period is invoiced");
    assert_eq!(cycle_of().await, Some(0));
    assert_eq!(usage_cents(&activation.invoice.line_items), 0);

    let renewed = env
        .services()
        .bill_subscription_period_at(
            tenant.id,
            subscription_id,
            noon(renewal),
            InvoiceDelivery::Suppressed,
        )
        .await
        .unwrap()
        .expect("the renewal is invoiced");
    assert_eq!(cycle_of().await, Some(1));
    assert_eq!(
        usage_cents(&renewed.invoice.line_items),
        2_000,
        "1,000 AI actions at 0.02, billed in arrears on the renewal"
    );
}
