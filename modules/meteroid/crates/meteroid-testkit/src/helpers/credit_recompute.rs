//! A metered fixture: usage events read by the services and by any pending-usage reader (the live
//! credit engine of the usage controls), metered subscriptions, entitlements and the alerts outbox.
use crate::data::ids::{CUST_UBER_ID, METRIC_BANDWIDTH, PLAN_VERSION_1_LEETCODE_ID, TENANT_ID};
use crate::helpers::credit::fund;
use crate::mailer::MockMailerService;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use chrono::{DateTime, Days, NaiveDateTime, NaiveTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    BillableMetricId, CustomerId, FeatureId, PlanVersionId, SubscriptionId, TenantId,
};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use error_stack::bail;
use meteroid::module::Modules;
use meteroid_store::StoreResult;
use meteroid_store::clients::pending_usage::{
    CustomerUsage, PendingUsageRequest, PendingUsageSource,
};
use meteroid_store::clients::usage::{
    EventSearchOptions, EventSearchResult, GroupedUsageData, IngestEventsRequest,
    IngestEventsResult, MutableMockUsageClient, UsageClient, UsageData, WindowedUsageData,
};
use meteroid_store::domain::UsagePeriod;
use meteroid_store::domain::entitlements::{
    EntitlementValue, FeatureNew, FeatureType, OverageBehavior, ResetPeriod,
};
use meteroid_store::domain::enums::{BillingMetricAggregateEnum, BillingPeriodEnum, FeeTypeEnum};
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::domain::price_components::{
    PriceEntry, PriceInput, ProductRef, UsagePricingModel,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::subscription_components::{
    CreateSubscriptionComponents, ExtraComponent,
};
use meteroid_store::domain::{
    BillableMetric, CreateSubscription, SegmentationMatrix, SubscriptionActivationCondition,
    SubscriptionNew, UsageFilter,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::EntitlementsInterface;
use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const FUNDED: i64 = 1_000_000;

pub struct Call {
    pub metric_id: BillableMetricId,
    pub group_by: Option<String>,
    pub since: NaiveDateTime,
    pub until: NaiveDateTime,
    pub customers: usize,
}

pub struct Event {
    metric_id: BillableMetricId,
    pub at: NaiveDateTime,
    groups: Vec<GroupedUsageData>,
}

/// Every customer consumed the same events; the metric's filters and aggregation are honoured
/// and reads are recorded.
#[derive(Clone)]
pub struct Events {
    pub events: Arc<Mutex<Vec<Event>>>,
    pub calls: Arc<Mutex<Vec<Call>>>,
    pub unavailable: Arc<AtomicBool>,
}

impl Events {
    fn readable(&self) -> StoreResult<()> {
        if AtomicBool::load(&self.unavailable, Ordering::SeqCst) {
            bail!(StoreError::MeteringServiceError);
        }
        Ok(())
    }

    pub fn add(
        &self,
        metric_id: BillableMetricId,
        at: NaiveDateTime,
        groups: Vec<GroupedUsageData>,
    ) {
        self.events.lock().unwrap().push(Event {
            metric_id,
            at,
            groups,
        });
    }
}

fn passes_filters(metric: &BillableMetric, group: &GroupedUsageData) -> bool {
    metric.filters.as_ref().is_none_or(|filters| {
        filters.0.iter().all(|filter| {
            let value = group.dimensions.get(&filter.property);
            let listed = value.is_some_and(|value| filter.values.contains(value));
            if filter.op.is_negation() {
                value.is_some() && !listed
            } else {
                listed
            }
        })
    })
}

/// The dimensions a read is broken down by: the metric's own, and the requested one.
fn projected(
    metric: &BillableMetric,
    group_by: Option<&str>,
    group: &GroupedUsageData,
) -> GroupedUsageData {
    let Some(extra) = group_by else {
        return group.clone();
    };
    let mut kept = vec![extra];
    kept.extend(metric.usage_group_key.as_deref());
    match metric.segmentation_matrix.as_ref() {
        None => {}
        Some(SegmentationMatrix::Single(dimension)) => kept.push(&dimension.key),
        Some(SegmentationMatrix::Double {
            dimension1,
            dimension2,
        }) => kept.extend([dimension1.key.as_str(), dimension2.key.as_str()]),
        Some(SegmentationMatrix::Linked {
            dimension1_key,
            dimension2_key,
            ..
        }) => kept.extend([dimension1_key.as_str(), dimension2_key.as_str()]),
    }
    GroupedUsageData {
        value: group.value,
        dimensions: group
            .dimensions
            .iter()
            .filter(|(key, _)| kept.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    }
}

impl Events {
    fn aggregate(
        &self,
        metric: &BillableMetric,
        group_by: Option<&str>,
        since: NaiveDateTime,
        until: NaiveDateTime,
    ) -> Vec<GroupedUsageData> {
        let additive = matches!(
            metric.aggregation_type,
            BillingMetricAggregateEnum::Sum | BillingMetricAggregateEnum::Count
        );
        let mut totals: HashMap<String, GroupedUsageData> = HashMap::new();
        for event in self.events.lock().unwrap().iter() {
            if event.metric_id != metric.id || event.at < since || event.at >= until {
                continue;
            }
            for group in event
                .groups
                .iter()
                .filter(|group| passes_filters(metric, group))
                .map(|group| projected(metric, group_by, group))
            {
                let key = format!("{:?}", group.dimensions.iter().collect::<BTreeMap<_, _>>());
                match totals.get_mut(&key) {
                    Some(total) if additive => total.value += group.value,
                    Some(total) => total.value = total.value.max(group.value),
                    None => {
                        totals.insert(key, group);
                    }
                }
            }
        }
        totals.into_values().collect()
    }
}

#[async_trait::async_trait]
impl PendingUsageSource for Events {
    async fn usage_since(
        &self,
        request: PendingUsageRequest<'_>,
    ) -> StoreResult<Vec<CustomerUsage>> {
        self.readable()?;
        self.calls.lock().unwrap().push(Call {
            metric_id: request.metric.id,
            group_by: request.group_by.map(str::to_owned),
            since: request.since,
            until: request.until,
            customers: request.customer_ids.len(),
        });
        let groups = self.aggregate(
            request.metric,
            request.group_by,
            request.since,
            request.until,
        );
        if groups.is_empty() {
            return Ok(Vec::new());
        }
        Ok(request
            .customer_ids
            .iter()
            .map(|customer_id| CustomerUsage {
                customer_id: *customer_id,
                groups: groups.clone(),
            })
            .collect())
    }
}

/// The services' usage client reading the very events a pending-usage reader reads, so both paths
/// of an entitlement are fed one event set.
struct EventsUsage(Events);

impl EventsUsage {
    fn read(
        &self,
        metric: &BillableMetric,
        group_by: Option<&str>,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.0.readable()?;
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_owned()));
        }
        Ok(UsageData {
            data: self.0.aggregate(metric, group_by, period.start, period.end),
            period,
        })
    }
}

#[async_trait::async_trait]
impl UsageClient for EventsUsage {
    async fn fetch_usage(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.read(metric, None, period)
    }

    async fn fetch_usage_grouped_by(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: &str,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.read(metric, Some(group_by), period)
    }

    async fn fetch_total_usage(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<Decimal> {
        let values = self
            .read(metric, None, period)?
            .data
            .into_iter()
            .map(|g| g.value);
        Ok(match metric.aggregation_type {
            BillingMetricAggregateEnum::Sum | BillingMetricAggregateEnum::Count => values.sum(),
            _ => values.max().unwrap_or(Decimal::ZERO),
        })
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
        _tenant_id: &TenantId,
        _customer_id: Option<&CustomerId>,
        _metric: &BillableMetric,
        _period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        bail!(StoreError::InvalidArgument("not read here".to_owned()))
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

pub fn groups() -> Vec<GroupedUsageData> {
    [("eu", 1200), ("us", 350)]
        .into_iter()
        .map(|(region, value)| GroupedUsageData {
            value: Decimal::from(value),
            dimensions: HashMap::from([("region".to_owned(), region.to_owned())]),
        })
        .collect()
}

pub fn usage_component(name: &str, model: UsagePricingModel) -> ExtraComponent {
    metric_component(name, METRIC_BANDWIDTH, model)
}

pub fn metric_component(
    name: &str,
    metric_id: BillableMetricId,
    model: UsagePricingModel,
) -> ExtraComponent {
    ExtraComponent {
        name: name.to_owned(),
        product_ref: ProductRef::New {
            name: format!("{name} Product"),
            fee_type: FeeTypeEnum::Usage,
            fee_structure: FeeStructure::Usage {
                metric_id,
                model: UsageModel::from(&model),
            },
        },
        price_entry: PriceEntry::New(PriceInput {
            cadence: BillingPeriodEnum::Monthly,
            currency: "EUR".to_owned(),
            pricing: Pricing::Usage(model),
            credit_unit: None,
        }),
    }
}

pub fn started_on() -> chrono::NaiveDate {
    common_domain::clock::now().date_naive() - Days::new(3)
}

pub async fn subscribe(
    setup: &MeteroidSetup,
    customer_id: CustomerId,
    components: Vec<ExtraComponent>,
) -> SubscriptionId {
    subscribe_from(setup, customer_id, components, started_on()).await
}

pub async fn subscribe_from(
    setup: &MeteroidSetup,
    customer_id: CustomerId,
    components: Vec<ExtraComponent>,
    start: chrono::NaiveDate,
) -> SubscriptionId {
    subscribe_on(
        setup,
        customer_id,
        PLAN_VERSION_1_LEETCODE_ID,
        components,
        start,
    )
    .await
}

pub async fn subscribe_on(
    setup: &MeteroidSetup,
    customer_id: CustomerId,
    plan_version_id: PlanVersionId,
    components: Vec<ExtraComponent>,
    start: chrono::NaiveDate,
) -> SubscriptionId {
    subscribe_with(setup, customer_id, plan_version_id, components, start, None).await
}

pub async fn subscribe_filtered(
    setup: &MeteroidSetup,
    customer_id: CustomerId,
    components: Vec<ExtraComponent>,
    start: chrono::NaiveDate,
    usage_filter: Option<UsageFilter>,
) -> SubscriptionId {
    subscribe_with(
        setup,
        customer_id,
        PLAN_VERSION_1_LEETCODE_ID,
        components,
        start,
        usage_filter,
    )
    .await
}

async fn subscribe_with(
    setup: &MeteroidSetup,
    customer_id: CustomerId,
    plan_version_id: PlanVersionId,
    components: Vec<ExtraComponent>,
    start: chrono::NaiveDate,
    usage_filter: Option<UsageFilter>,
) -> SubscriptionId {
    setup
        .services
        .insert_subscription(
            Actor::System,
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id,
                    plan_version_id,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: start,
                    end_date: None,
                    billing_start_date: Some(start),
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: None,
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                    usage_filter,
                    custom_properties: serde_json::Value::Null,
                },
                price_components: Some(CreateSubscriptionComponents {
                    parameterized_components: vec![],
                    overridden_components: vec![],
                    extra_components: components,
                    remove_components: vec![],
                }),
                add_ons: None,
                coupons: None,
                entitlements: vec![],
                minimum_override: None,
                drop_minimum: false,
            },
            TENANT_ID,
        )
        .await
        .expect("a metered subscription")
        .id
}

pub struct Fixture {
    pub setup: MeteroidSetup,
    pub events: Events,
    pub usage: MutableMockUsageClient,
}

pub async fn fixture() -> Fixture {
    build_fixture(false, Modules::core()).await
}

/// Like [`fixture`], serving the edition `modules` too.
pub async fn fixture_with(modules: Modules) -> Fixture {
    build_fixture(false, modules).await
}

/// The services' usage client reads the events too, so both paths of an entitlement are fed one
/// event set.
pub async fn fixture_over_events() -> Fixture {
    build_fixture(true, Modules::core()).await
}

/// Like [`fixture_over_events`], serving the edition `modules` too.
pub async fn fixture_over_events_with(modules: Modules) -> Fixture {
    build_fixture(true, modules).await
}

async fn build_fixture(over_events: bool, modules: Modules) -> Fixture {
    crate::helpers::init::logging();
    let database = crate::meteroid_it::container::create_test_database().await;
    let usage = MutableMockUsageClient::new();
    let events = Events {
        events: Arc::default(),
        calls: Arc::default(),
        unavailable: Arc::default(),
    };
    let services_usage: Arc<dyn UsageClient> = if over_events {
        Arc::new(EventsUsage(events.clone()))
    } else {
        Arc::new(usage.clone())
    };
    let setup = crate::meteroid_it::container::start_meteroid_with_clients_and_modules(
        database,
        SeedLevel::PLANS,
        services_usage,
        Arc::new(MockMailerService::new()),
        modules,
    )
    .await;
    let at = started_on().and_time(NaiveTime::from_hms_opt(1, 0, 0).unwrap());
    usage.set_usage_for_any_nonempty_window(
        METRIC_BANDWIDTH,
        UsageData {
            data: groups(),
            period: UsagePeriod {
                start: at,
                end: at + chrono::Duration::days(1),
            },
        },
    );
    events.add(METRIC_BANDWIDTH, at, groups());
    Fixture {
        setup,
        events,
        usage,
    }
}

/// Shortly after a fold-grid boundary, so recomputes a few seconds apart share one fold instant.
/// Up to eleven minutes ahead of the clock: a calendar window may already be the next one.
pub fn steady_now() -> DateTime<Utc> {
    let grid = 600;
    let floored = common_domain::clock::now().timestamp().div_euclid(grid) * grid;
    DateTime::from_timestamp(floored + grid + 60, 0).unwrap()
}

pub fn usage_of(at: NaiveDateTime, groups: Vec<GroupedUsageData>) -> UsageData {
    UsageData {
        data: groups,
        period: UsagePeriod {
            start: at,
            end: at + chrono::Duration::days(1),
        },
    }
}

pub async fn linear_subscription(f: &Fixture) -> SubscriptionId {
    linear_subscription_from(f, started_on()).await
}

pub async fn linear_subscription_from(f: &Fixture, start: chrono::NaiveDate) -> SubscriptionId {
    fund(&f.setup.store, TENANT_ID, CUST_UBER_ID, None, FUNDED).await;
    subscribe_from(
        &f.setup,
        CUST_UBER_ID,
        vec![usage_component(
            "Linear",
            UsagePricingModel::PerUnit { rate: dec!(0.07) },
        )],
        start,
    )
    .await
}

pub async fn execute(f: &Fixture, sql: String) {
    use diesel_async::SimpleAsyncConnection;
    f.setup
        .store
        .get_conn()
        .await
        .unwrap()
        .batch_execute(&sql)
        .await
        .expect("test statement");
}

pub fn plain(value: i64) -> Vec<GroupedUsageData> {
    vec![GroupedUsageData {
        value: Decimal::from(value),
        dimensions: HashMap::new(),
    }]
}

pub fn by_key(api_key: &str, value: i64) -> GroupedUsageData {
    GroupedUsageData {
        value: Decimal::from(value),
        dimensions: HashMap::from([("api_key".to_owned(), api_key.to_owned())]),
    }
}

pub async fn activated_at(f: &Fixture, subscription_id: SubscriptionId) -> NaiveDateTime {
    let mut conn = f.setup.store.get_conn().await.unwrap();
    f.setup
        .store
        .get_subscription_details_with_conn(&mut conn, TENANT_ID, subscription_id)
        .await
        .unwrap()
        .subscription
        .activated_at
        .expect("an activated subscription")
}

pub async fn add_entitlement(
    f: &Fixture,
    code: &str,
    limit: i64,
    overage_behavior: OverageBehavior,
) -> FeatureId {
    f.setup
        .store
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: None,
                name: code.to_owned(),
                code: code.to_owned(),
                description: None,
                feature_type: FeatureType::metered(METRIC_BANDWIDTH),
                entitlement: Some(EntitlementValue::Metered {
                    limit: Some(Decimal::from(limit)),
                    reset_period: ResetPeriod::BillingCycle,
                    overage_behavior,
                    thresholds: vec![],
                    enabled: true,
                    group_by: None,
                }),
            },
        )
        .await
        .expect("a metered feature")
        .id
}

pub fn metered_value(
    reset_period: ResetPeriod,
    group_by: Option<&str>,
    limit: i64,
) -> EntitlementValue {
    EntitlementValue::Metered {
        limit: Some(Decimal::from(limit)),
        reset_period,
        overage_behavior: OverageBehavior::Block {
            grace_period_pct: None,
        },
        thresholds: vec![],
        enabled: true,
        group_by: group_by.map(str::to_owned),
    }
}

pub async fn add_metered_feature_limited(
    f: &Fixture,
    code: &str,
    reset_period: ResetPeriod,
    group_by: Option<&str>,
    limit: i64,
) -> StoreResult<FeatureId> {
    f.setup
        .store
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: None,
                name: code.to_owned(),
                code: code.to_owned(),
                description: None,
                feature_type: FeatureType::metered(METRIC_BANDWIDTH),
                entitlement: Some(metered_value(reset_period, group_by, limit)),
            },
        )
        .await
        .map(|feature| feature.id)
}

pub fn set_metering_total(f: &Fixture, total: i64) {
    set_metering_groups(f, plain(total));
}

pub fn set_metering_groups(f: &Fixture, groups: Vec<GroupedUsageData>) {
    let at = started_on().and_time(NaiveTime::MIN);
    f.usage
        .set_usage_for_any_nonempty_window(METRIC_BANDWIDTH, usage_of(at, groups));
}

pub async fn alert_outbox(f: &Fixture) -> Vec<OutboxEvent> {
    f.setup
        .store
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(1000),
            MessageReadVtSec(0),
        )
        .await
        .unwrap()
        .into_iter()
        .filter_map(|message| message.message)
        .filter_map(|message| serde_json::from_value::<OutboxEvent>(message.0).ok())
        .collect()
}

pub fn later(now: DateTime<Utc>, seconds: i64) -> DateTime<Utc> {
    now + chrono::Duration::seconds(seconds)
}

/// Waits past midnight UTC when fewer than `margin_secs` remain before it, so a test reading
/// daily windows never straddles two days.
pub async fn clear_of_midnight(margin_secs: i64) {
    let now = common_domain::clock::now();
    let midnight = (now.date_naive() + Days::new(1))
        .and_time(NaiveTime::MIN)
        .and_utc();
    let remaining = midnight - now;
    if remaining < chrono::Duration::seconds(margin_secs) {
        tokio::time::sleep((remaining + chrono::Duration::seconds(2)).to_std().unwrap()).await;
    }
}
