//! The Halyard AI catalog of the credit billing scenarios, seeded by the sandbox preset in a
//! sandbox tenant of its own, driven in business time (2025, so every scenario day is past), with
//! usage kept as dated events and read like the metering backend reads them.
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, CreditGrantId, CustomerId, InvoiceId, PlanVersionId, SubscriptionId, TenantId,
};
use diesel::{ExpressionMethods, JoinOnDsl, QueryDsl};
use diesel_async::RunQueryDsl;
use diesel_models::customer_credit::{
    CustomerCreditAccountRow, CustomerCreditEntryRow, CustomerCreditGrantRow,
};
use error_stack::bail;
use meteroid_seeder::presets;
use meteroid_store::StoreResult;
use meteroid_store::clients::pending_usage::{
    CustomerUsage, PendingUsageRequest, PendingUsageSource,
};
use meteroid_store::clients::usage::{
    EventSearchOptions, EventSearchResult, GroupedUsageData, IngestEventsRequest,
    IngestEventsResult, UsageClient, UsageData, WindowedUsageData,
};
use meteroid_store::domain::enums::CustomerType;
use meteroid_store::domain::subscriptions::PaymentMethodsConfig;
use meteroid_store::domain::{
    Address, BillableMetric, CreateSubscription, CustomerNew, CustomerTaxRate, CustomerTaxStatus,
    Invoice, InvoicingEntityPatch, PaginationRequest, SegmentationMatrix,
    SubscriptionActivationCondition, SubscriptionNew, UsagePeriod,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::customers::CustomersInterface;
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use meteroid_store::services::InvoiceDelivery;
use meteroid_store::services::sandbox_replay::SandboxReplayInterface;
use meteroid_store::{Services, Store};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::ORGANIZATION_ID;
use crate::harness::{TestEnv, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;
use crate::revenue::{Revenue, RevenueRow};

pub const LITE: &str = "halyard-lite";
pub const STANDARD: &str = "halyard-standard";
pub const MAX: &str = "halyard-max";
pub const M: u64 = 1_000_000;

pub fn day(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, month, day).unwrap()
}

/// 00:00 UTC of a 2025 day.
pub fn at(month: u32, d: u32) -> DateTime<Utc> {
    day(month, d).and_time(NaiveTime::MIN).and_utc()
}

pub fn at_time(month: u32, d: u32, hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
    day(month, d)
        .and_hms_opt(hour, minute, second)
        .unwrap()
        .and_utc()
}

#[derive(Clone, Copy, Debug)]
pub enum Tax {
    Untaxed,
    NewYork,
    Ohio,
}

struct Event {
    customer: CustomerId,
    at: NaiveDateTime,
    properties: HashMap<String, String>,
}

/// Dated token events per customer, summed like the metering backend over any window, by model,
/// counting only the events a subscription's usage scope claims.
#[derive(Default)]
pub struct ScenarioUsage {
    events: Mutex<Vec<Event>>,
}

impl ScenarioUsage {
    fn aggregate(
        &self,
        customer: &CustomerId,
        metric: &BillableMetric,
        group_by: Option<&str>,
        since: NaiveDateTime,
        until: NaiveDateTime,
    ) -> Vec<GroupedUsageData> {
        let value_key = metric.aggregation_key.as_deref().unwrap_or_default();
        let mut kept: Vec<&str> = group_by.into_iter().collect();
        kept.extend(metric.usage_group_key.as_deref());
        if let Some(SegmentationMatrix::Single(dimension)) = &metric.segmentation_matrix {
            kept.push(&dimension.key);
        }
        let mut totals: BTreeMap<Vec<(String, String)>, Decimal> = BTreeMap::new();
        for event in self.events.lock().unwrap().iter() {
            if event.customer != *customer
                || event.at < since
                || event.at >= until
                || !metric.usage_scope.contains(&event.properties, event.at)
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

    /// An event with arbitrary properties, for metrics other than the token metric.
    pub fn record_with(
        &self,
        customer: CustomerId,
        at: DateTime<Utc>,
        properties: &[(&str, &str)],
    ) {
        self.events.lock().unwrap().push(Event {
            customer,
            at: at.naive_utc(),
            properties: properties
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        });
    }

    pub fn record(
        &self,
        customer: CustomerId,
        at: DateTime<Utc>,
        model: &str,
        tokens: u64,
        api_key: Option<&str>,
    ) {
        let mut properties = HashMap::from([
            ("model".to_owned(), model.to_owned()),
            ("tokens".to_owned(), tokens.to_string()),
        ]);
        if let Some(key) = api_key {
            properties.insert("api_key".to_owned(), key.to_owned());
        }
        self.events.lock().unwrap().push(Event {
            customer,
            at: at.naive_utc(),
            properties,
        });
    }
}

#[async_trait::async_trait]
impl UsageClient for ScenarioUsage {
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
                "scenario usage is read per customer".to_owned()
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
        _request: IngestEventsRequest,
    ) -> StoreResult<IngestEventsResult> {
        bail!(StoreError::InvalidArgument(
            "scenario usage is recorded by the test".to_owned()
        ));
    }
}

#[async_trait::async_trait]
impl PendingUsageSource for ScenarioUsage {
    async fn usage_since(
        &self,
        request: PendingUsageRequest<'_>,
    ) -> StoreResult<Vec<CustomerUsage>> {
        Ok(request
            .customer_ids
            .iter()
            .filter_map(|customer_id| {
                let groups = self.aggregate(
                    customer_id,
                    request.metric,
                    request.group_by,
                    request.since,
                    request.until,
                );
                (!groups.is_empty()).then_some(CustomerUsage {
                    customer_id: *customer_id,
                    groups,
                })
            })
            .collect())
    }
}

async fn plan_version(env: &TestEnv, tenant: TenantId, name: &str) -> PlanVersionId {
    use diesel_models::schema::plan::dsl as p;
    use diesel_models::schema::plan_version::dsl as v;
    p::plan
        .inner_join(v::plan_version.on(v::plan_id.eq(p::id)))
        .filter(p::tenant_id.eq(tenant))
        .filter(p::name.eq(name))
        .filter(v::is_draft_version.eq(false))
        .select(v::id)
        .order(v::version.desc())
        .first::<PlanVersionId>(&mut env.conn().await)
        .await
        .unwrap_or_else(|e| panic!("plan {name}: {e:?}"))
}

pub struct Halyard {
    pub env: TestEnv,
    pub usage: Arc<ScenarioUsage>,
    pub tenant: TenantId,
    pub payg: PlanVersionId,
    pub scale: PlanVersionId,
    pub studio: PlanVersionId,
    pub enterprise: PlanVersionId,
    skip_corpus: std::sync::atomic::AtomicBool,
}

/// Every scenario's documents are compared with its corpus file when it ends (`crate::corpus`).
impl Drop for Halyard {
    fn drop(&mut self) {
        if std::thread::panicking()
            || std::sync::atomic::AtomicBool::load(
                &self.skip_corpus,
                std::sync::atomic::Ordering::Relaxed,
            )
        {
            return;
        }
        if let Some(name) = crate::corpus::current_test_name() {
            crate::corpus::assert_matches(
                self.env.setup.database_url(),
                self.tenant.as_uuid(),
                &crate::corpus::claim(&name),
            );
        }
    }
}

/// One invoice's figures as the timeline's INV rows give them: subtotal, discount, promotional
/// credit, tax, total, paid credit applied, amount due.
pub fn money(invoice: &Invoice) -> (i64, i64, i64, i64, i64, i64, i64) {
    (
        invoice.subtotal,
        invoice.discount,
        invoice.promotional_credits,
        invoice.tax_amount,
        invoice.total,
        invoice.applied_credits,
        invoice.amount_due,
    )
}

/// An invoice's lines as (name, subtotal), sorted.
pub fn lines(invoice: &Invoice) -> Vec<(String, i64)> {
    let mut lines: Vec<_> = invoice
        .line_items
        .iter()
        .map(|line| (line.name.clone(), line.amount_subtotal))
        .collect();
    lines.sort();
    lines
}

impl Halyard {
    pub async fn new() -> Self {
        let usage = Arc::new(ScenarioUsage::default());
        let env = test_env_with_seed_and_usage(SeedLevel::MINIMAL, usage.clone()).await;
        let mut scenario = presets::ai_credits_scenario();
        scenario.customers.clear();
        scenario.quotes.clear();
        let tenant = presets::run_preset_at(
            env.store(),
            env.services(),
            scenario,
            ORGANIZATION_ID,
            Some("Halyard scenarios".to_owned()),
            common_domain::clock::now().naive_utc(),
        )
        .await
        .expect("the Halyard catalog")
        .id;
        // C5: the invoicing entity bills with no grace, so the settlement grace is 30 minutes.
        let entity = env
            .store()
            .get_invoicing_entity(tenant, None)
            .await
            .expect("invoicing entity");
        env.store()
            .patch_invoicing_entity(
                Actor::System,
                InvoicingEntityPatch {
                    id: entity.id,
                    grace_period_hours: Some(0),
                    ..Default::default()
                },
                tenant,
            )
            .await
            .expect("no invoicing grace");
        let plan = |name: &'static str| plan_version(&env, tenant, name);
        let (payg, scale, studio, enterprise) = (
            plan("Pay as you go").await,
            plan("Scale").await,
            plan("Studio").await,
            plan("Enterprise").await,
        );
        Self {
            env,
            usage,
            tenant,
            payg,
            scale,
            studio,
            enterprise,
            skip_corpus: Default::default(),
        }
    }

    pub fn store(&self) -> &Store {
        self.env.store()
    }

    /// Leaves this test out of the document corpus, e.g. when its documents depend on the clock.
    pub fn skip_corpus(&self, _reason: &'static str) {
        self.skip_corpus
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn services(&self) -> &Services {
        self.env.services()
    }

    pub async fn customer(&self, name: &str, tax: Tax) -> CustomerId {
        let (state, custom_taxes) = match tax {
            Tax::Untaxed => ("CA", vec![]),
            Tax::NewYork => (
                "NY",
                vec![CustomerTaxRate {
                    tax_code: "SALES_TAX".to_owned(),
                    name: "New York sales tax".to_owned(),
                    rate: dec!(0.08875),
                }],
            ),
            Tax::Ohio => (
                "OH",
                vec![CustomerTaxRate {
                    tax_code: "SALES_TAX".to_owned(),
                    name: "Ohio sales tax".to_owned(),
                    rate: dec!(0.08),
                }],
            ),
        };
        self.store()
            .insert_customer(
                Actor::System,
                CustomerNew {
                    name: name.to_owned(),
                    alias: None,
                    billing_email: Some(format!("billing@{}.test", name.to_lowercase())),
                    invoicing_emails: vec![],
                    phone: None,
                    currency: "USD".to_owned(),
                    billing_address: Some(Address {
                        line1: Some("1 Market St".to_owned()),
                        line2: None,
                        city: Some("Springfield".to_owned()),
                        country: Some(common_domain::country::CountryCode::from_str("US").unwrap()),
                        state: Some(state.to_owned()),
                        zip_code: Some("10001".to_owned()),
                    }),
                    shipping_address: None,
                    invoicing_entity_id: None,
                    force_created_date: Some(at(1, 1).naive_utc()),
                    vat_number: None,
                    legal_number: None,
                    buyer_reference: None,
                    custom_taxes,
                    tax_status: CustomerTaxStatus::Taxable,
                    exemption_reason: None,
                    connected_account_id: None,
                    custom_properties: serde_json::Value::Null,
                    preferred_locales: vec![],
                    customer_type: CustomerType::Company,
                    first_name: None,
                    last_name: None,
                },
                self.tenant,
            )
            .await
            .expect("customer")
            .id
    }

    /// A subscription starting `start`, anchored on its day, billed with backdated documents.
    pub fn request(
        &self,
        customer: CustomerId,
        plan: PlanVersionId,
        start: NaiveDate,
    ) -> CreateSubscription {
        CreateSubscription {
            subscription: SubscriptionNew {
                customer_id: customer,
                plan_version_id: plan,
                net_terms: None,
                invoice_memo: None,
                invoice_threshold: None,
                start_date: start,
                end_date: None,
                billing_start_date: None,
                activation_condition: SubscriptionActivationCondition::OnStart,
                trial_duration: Some(0),
                billing_day_anchor: None,
                payment_methods_config: None::<PaymentMethodsConfig>,
                auto_advance_invoices: true,
                charge_automatically: false,
                purchase_order: None,
                backdate_invoices: true,
                skip_checkout_session: false,
                skip_past_invoices: false,
                usage_filter: None,
                custom_properties: serde_json::Value::Null,
            },
            price_components: None,
            add_ons: None,
            coupons: None,
            entitlements: vec![],
            minimum_override: None,
            drop_minimum: false,
        }
    }

    /// Starts `request` at its start date's midnight; returns it and its first invoice.
    pub async fn start(&self, request: CreateSubscription) -> (SubscriptionId, Option<Invoice>) {
        let started = request
            .subscription
            .start_date
            .and_time(NaiveTime::MIN)
            .and_utc();
        let (created, invoice) = self
            .services()
            .insert_subscription_started_at(
                Actor::System,
                request,
                self.tenant,
                started,
                InvoiceDelivery::Suppressed,
            )
            .await
            .expect("subscription started");
        (created.id, invoice.map(|detailed| detailed.invoice))
    }

    pub async fn subscribe(
        &self,
        customer: CustomerId,
        plan: PlanVersionId,
        start: NaiveDate,
    ) -> (SubscriptionId, Option<Invoice>) {
        self.start(self.request(customer, plan, start)).await
    }

    /// Bills the period ending at `at`, issued then.
    pub async fn renew(&self, subscription: SubscriptionId, at: DateTime<Utc>) -> Option<Invoice> {
        self.services()
            .bill_subscription_period_at(self.tenant, subscription, at, InvoiceDelivery::Suppressed)
            .await
            .unwrap_or_else(|e| panic!("renewal at {at}: {e:?}"))
            .map(|detailed| detailed.invoice)
    }

    /// The amount due collected at `paid_at`, as a card charge succeeding then.
    pub async fn pay(&self, invoice: InvoiceId, paid_at: DateTime<Utc>) -> Invoice {
        self.services()
            .settle_invoice_payment_at(
                Actor::System,
                self.tenant,
                invoice,
                paid_at,
                Some(format!("payment-{invoice}")),
                InvoiceDelivery::Suppressed,
            )
            .await
            .unwrap_or_else(|e| panic!("payment at {paid_at}: {e:?}"))
            .invoice
    }

    pub async fn sweep(&self, customer: CustomerId, at: DateTime<Utc>) {
        self.store()
            .expire_customer_credit_for(self.tenant, customer, at)
            .await
            .expect("expiry sweep");
    }

    pub fn tokens(&self, customer: CustomerId, at: DateTime<Utc>, model: &str, tokens: u64) {
        self.usage.record(customer, at, model, tokens, None);
    }

    pub fn keyed_tokens(
        &self,
        customer: CustomerId,
        at: DateTime<Utc>,
        model: &str,
        tokens: u64,
        key: &str,
    ) {
        self.usage.record(customer, at, model, tokens, Some(key));
    }

    /// The subscription's invoices, oldest first.
    pub async fn invoices(&self, subscription: SubscriptionId) -> Vec<Invoice> {
        self.store()
            .list_invoices(
                self.tenant,
                None,
                Some(subscription),
                None,
                None,
                None,
                false,
                None,
                vec![],
                Some("created_at.asc".to_string()),
                PaginationRequest {
                    page: 0,
                    per_page: None,
                },
            )
            .await
            .expect("invoices")
            .items
            .into_iter()
            .map(|i| i.invoice)
            .collect()
    }

    pub async fn invoice(&self, id: InvoiceId) -> Invoice {
        self.store()
            .get_invoice_by_id(self.tenant, id)
            .await
            .expect("invoice")
    }

    /// Every grant of the customer, as stored.
    pub async fn grants(&self, customer: CustomerId) -> Vec<CustomerCreditGrantRow> {
        let mut conn = self.env.conn().await;
        let mut rows = CustomerCreditGrantRow::list_for_customer(&mut conn, self.tenant, customer)
            .await
            .expect("grants");
        rows.sort_by_key(|g| (g.created_at, g.id.as_uuid()));
        rows
    }

    /// The customer's wallet denomination of a grant (an ISO code or a credit unit code).
    pub async fn denomination(&self, grant: &CustomerCreditGrantRow) -> String {
        let mut conn = self.env.conn().await;
        CustomerCreditAccountRow::list_for_customer(&mut conn, self.tenant, grant.customer_id)
            .await
            .expect("accounts")
            .into_iter()
            .find(|account| account.id == grant.account_id)
            .map(|account| account.currency)
            .expect("the grant's account")
    }

    /// A grant's movements as "KIND amount", in recording order.
    pub async fn movements(&self, grant: CreditGrantId) -> Vec<String> {
        let mut conn = self.env.conn().await;
        CustomerCreditEntryRow::list_by_grant(&mut conn, self.tenant, grant)
            .await
            .expect("movements")
            .into_iter()
            .map(|entry| format!("{} {}", entry.kind, entry.amount))
            .collect()
    }

    /// Movements with their reason, "KIND amount reason".
    pub async fn movements_with_reasons(&self, grant: CreditGrantId) -> Vec<String> {
        let mut conn = self.env.conn().await;
        CustomerCreditEntryRow::list_by_grant(&mut conn, self.tenant, grant)
            .await
            .expect("movements")
            .into_iter()
            .map(|entry| match entry.reason {
                Some(reason) => format!("{} {} {reason}", entry.kind, entry.amount),
                None => format!("{} {}", entry.kind, entry.amount),
            })
            .collect()
    }

    pub async fn check(&self) {
        crate::helpers::invariants::assert_credit_invariants(self.store(), self.tenant).await;
    }

    /// The customer's revenue as of `as_of`; None when this test binary derives no revenue.
    pub async fn revenue(&self, customer: CustomerId, as_of: NaiveDate) -> Option<Revenue> {
        crate::revenue::revenue(self.store(), customer, as_of).await
    }

    /// The customer's revenue rows, each as last written under its id; None when this test
    /// binary derives no revenue.
    pub async fn revenue_rows(&self, customer: CustomerId) -> Option<Vec<RevenueRow>> {
        crate::revenue::rows(self.store(), customer).await
    }

    pub fn hours(hours: i64) -> Duration {
        Duration::hours(hours)
    }
}

/// The one grant of `source_kind`, `classification` and face `amount`, the latest when several.
pub fn grant<'a>(
    grants: &'a [CustomerCreditGrantRow],
    source_kind: &str,
    classification: &str,
    amount: i64,
) -> &'a CustomerCreditGrantRow {
    grants
        .iter()
        .rev()
        .find(|g| {
            g.source_kind == source_kind && g.classification == classification && g.amount == amount
        })
        .unwrap_or_else(|| {
            panic!(
                "no {source_kind} {classification} grant of {amount} among {:?}",
                summary(grants)
            )
        })
}

/// Grants as "SOURCE CLASSIFICATION face remaining", in creation order.
pub fn summary(grants: &[CustomerCreditGrantRow]) -> Vec<String> {
    grants
        .iter()
        .map(|g| {
            format!(
                "{} {} {} {}",
                g.source_kind, g.classification, g.amount, g.remaining
            )
        })
        .collect()
}

/// The grant of `source_kind` expiring at `expires_at`.
pub fn expiring<'a>(
    grants: &'a [CustomerCreditGrantRow],
    source_kind: &str,
    expires_at: DateTime<Utc>,
) -> &'a CustomerCreditGrantRow {
    grants
        .iter()
        .find(|g| g.source_kind == source_kind && g.expires_at == Some(expires_at))
        .unwrap_or_else(|| {
            panic!(
                "no {source_kind} grant expiring at {expires_at} among {:?}",
                summary(grants)
            )
        })
}
