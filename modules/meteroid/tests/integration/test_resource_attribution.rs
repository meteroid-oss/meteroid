//! Usage attribution on resources against the real stack (ADR 0011 §3.5, §3.6): a subscription
//! for an `api_key` resource bills its key's events (one per resource at a time), a customer-level one
//! the rest; what no subscription bills is listed as unattributed usage.
use crate::data::ids::{
    COMP_LEETCODE_RATE_ID, CUST_COMODO_ID, CUST_SPOTIFY_ID, CUST_UBER_ID,
    CUST_UBER_PAYMENT_METHOD_ID, INVOICING_ENTITY_ID, METRIC_BANDWIDTH, ORGANIZATION_ID,
    PLAN_VERSION_1_LEETCODE_ID, PRODUCT_FAMILY_ID, TENANT_ID, USER_ID,
};
use crate::harness::{TestEnv, test_env};
use crate::helpers::credit::fund;
use crate::helpers::credit_recompute::clear_of_midnight;
use crate::helpers::credit_recompute::metric_component;
use crate::helpers::resources::{API_KEY, API_KEY_ALIAS, api_key_resource, attribute, keyed_type};
use crate::metering_it::clients::AllClients as MeteringClients;
use crate::metering_it::container::MeteringSetup;
use crate::{helpers, metering_it, meteroid_it};
use backon::Retryable;
use chrono::{DateTime, Datelike, Days, Months, NaiveDate, NaiveTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    AliasOr, BaseId, BillableMetricId, CustomerId, InvoiceId, QuoteId, ResourceId, SubscriptionId,
};
use diesel_models::credit_notes::CreditNoteRow;
use diesel_models::customers::CustomerRowNew;
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::subscriptions::SubscriptionRow;
use metering_grpc::meteroid::metering::v1::{Event, IngestRequest, event};
use meteroid::clients::usage::MeteringUsageClient;
use meteroid_mailer::config::MailerConfig;
use meteroid_store::clients::usage::UsageClient;
use meteroid_store::domain::billable_metrics::{Dimension, SegmentationMatrix};
use meteroid_store::domain::enums::BillingMetricAggregateEnum;
use meteroid_store::domain::enums::{InvoiceStatusEnum, InvoiceType};
use meteroid_store::domain::invoice_annex_templates::{
    AnnexColumn, AnnexDefinition, AnnexTemplateNew, ColumnAggregation, ColumnSource, EventSource,
    EventTableDefinition,
};
use meteroid_store::domain::invoices::InvoiceAnnexCell;
use meteroid_store::domain::price_components::{DimensionRate, DimensionRates, UsagePricingModel};
use meteroid_store::domain::rate_cards::RateCardUnmatched;
use meteroid_store::domain::subscription_amendment::{
    AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::domain::subscription_components::CreateSubscriptionComponents;
use meteroid_store::domain::{
    ApiTokenNew, BillableMetric, BillableMetricNew, CreateSubscription, CreatedSubscription,
    Invoice, InvoicingEntityPatch, MetricAttribution, PaginationRequest, ResourceMatch,
    ScopeSegment, SubscriptionActivationCondition, SubscriptionNew, SubscriptionPatch,
    TenantUpdate, UsagePeriod, UsageScope,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::ResourcesInterface;
use meteroid_store::repositories::api_tokens::ApiTokensInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::subscriptions::{
    CancellationEffectiveAt, SubscriptionInterface, SubscriptionInterfaceAuto,
};
use meteroid_store::repositories::tenants::TenantInterface;
use meteroid_store::repositories::{InvoiceInterface, QuotesInterface};
use meteroid_testkit::annex_templates::{create_annex_template, set_plan_version_annex_templates};
use rstest::rstest;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::sync::Arc;
use std::time::Duration;
use testcontainers::{ContainerAsync, GenericImage};
use tonic::Request;
use uuid::Uuid;

const CODE: &str = "halyard_tokens";

struct Stack {
    meteroid: meteroid_it::container::MeteroidSetup,
    usage: Arc<MeteringUsageClient>,
    metric: BillableMetric,
    ingest: MeteringClients,
    _clickhouse: ContainerAsync<GenericImage>,
    _metering: MeteringSetup,
    _kafka: ContainerAsync<GenericImage>,
    kafka_port: u16,
}

/// One of the customer's events: `tokens` of a `model`, with an `api_key` when it has one.
struct Tokens {
    customer: CustomerId,
    at: DateTime<Utc>,
    api_key: Option<&'static str>,
    model: &'static str,
    tokens: u64,
}

impl Stack {
    async fn start() -> Self {
        helpers::init::logging();
        let postgres = meteroid_it::container::create_seeded_test_database(
            meteroid_it::container::SeedLevel::PLANS,
        )
        .await;
        let ((kafka, kafka_port), (clickhouse, ch_http_port, ch_tcp_port)) =
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
            metering_it::container::start_metering(metering_config.clone(), metering_listener)
                .await;
        let ingest = MeteringClients::from_channel(
            metering.channel.clone(),
            "pv_sand_9XzHg0EYO2Usy9ITU6bbhBnkYYbx/2vO7XtUUeQ7Wq9EZCAbBG",
            &metering_config.internal_auth,
        );
        let usage = Arc::new(MeteringUsageClient::from_channel(
            metering.channel.clone(),
            &metering_config.internal_auth,
        ));
        let meteroid = meteroid_it::container::start_meteroid_with_grpc_listener(
            meteroid_listener,
            metering_port,
            postgres,
            meteroid_it::container::SeedLevel::PLANS,
            usage.clone(),
            meteroid_mailer::service::mailer_service(MailerConfig::dummy()),
        )
        .await;
        usage.bind_tenant_lookup(meteroid.store.keep_last_events_lookup());

        // Halyard's model usage: tokens per model, priced per 1M tokens.
        let metric = meteroid
            .store
            .insert_billable_metric(
                Actor::System,
                BillableMetricNew {
                    name: "Model tokens".to_owned(),
                    description: None,
                    code: CODE.to_owned(),
                    catalog_code: None,
                    aggregation_type: BillingMetricAggregateEnum::Sum,
                    aggregation_key: Some("tokens".to_owned()),
                    unit_conversion_factor: Some(1_000_000),
                    unit_conversion_rounding: None,
                    segmentation_matrix: Some(SegmentationMatrix::Single(Dimension {
                        key: "model".to_owned(),
                        values: vec!["lite".to_owned(), "standard".to_owned(), "max".to_owned()],
                    })),
                    filters: None,
                    usage_group_key: None,
                    usage_breakdown_key: None,
                    attributed_to: None,
                    unit_label: None,
                    tenant_id: TENANT_ID,
                    product_family_id: PRODUCT_FAMILY_ID,
                    product_id: None,
                },
            )
            .await
            .expect("the token metric");

        Self {
            meteroid,
            usage,
            metric,
            ingest,
            _clickhouse: clickhouse,
            _metering: metering,
            _kafka: kafka,
            kafka_port,
        }
    }

    /// Pay as you go, model usage only, for the customer's `api_key` resource `key` when set.
    async fn subscribe(
        &self,
        customer_id: CustomerId,
        start: NaiveDate,
        key: Option<&str>,
    ) -> Result<SubscriptionId, error_stack::Report<StoreError>> {
        self.subscribe_until(customer_id, start, None, key).await
    }

    async fn subscribe_until(
        &self,
        customer_id: CustomerId,
        start: NaiveDate,
        end: Option<NaiveDate>,
        key: Option<&str>,
    ) -> Result<SubscriptionId, error_stack::Report<StoreError>> {
        self.subscribe_with(customer_id, start, end, None, false, key)
            .await
    }

    /// `advance_fee` keeps the plan's €35 monthly rate, billed in advance.
    async fn subscribe_with(
        &self,
        customer_id: CustomerId,
        start: NaiveDate,
        end: Option<NaiveDate>,
        billing_day_anchor: Option<u16>,
        advance_fee: bool,
        key: Option<&str>,
    ) -> Result<SubscriptionId, error_stack::Report<StoreError>> {
        let resource_id = match key {
            Some(key) => Some(self.resource(customer_id, key).await),
            None => None,
        };
        let mut request = self.request(customer_id, start, resource_id);
        request.subscription.end_date = end;
        request.subscription.billing_day_anchor = billing_day_anchor;
        if advance_fee && let Some(components) = request.price_components.as_mut() {
            components.remove_components.clear();
        }
        self.create(request).await.map(|created| created.id)
    }

    async fn resource(&self, customer_id: CustomerId, key: &str) -> ResourceId {
        api_key_resource(&self.meteroid.store, TENANT_ID, customer_id, key).await
    }

    /// Lite €2, standard €12, max €60 per 1M tokens, from `start`.
    fn request(
        &self,
        customer_id: CustomerId,
        start: NaiveDate,
        resource_id: Option<ResourceId>,
    ) -> CreateSubscription {
        let rate = |model: &str, euros: i64| DimensionRate {
            key: vec![model.to_owned()],
            rate: Decimal::from(euros),
            label: None,
        };
        CreateSubscription {
            option: None,
            subscription: SubscriptionNew {
                customer_id,
                plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                net_terms: None,
                invoice_memo: None,
                invoice_threshold: None,
                start_date: start,
                end_date: None,
                billing_start_date: Some(start),
                activation_condition: SubscriptionActivationCondition::OnStart,
                offer: Default::default(),
                trial: Default::default(),
                billing_day_anchor: None,
                payment_methods_config: None,
                auto_advance_invoices: true,
                charge_automatically: false,
                purchase_order: None,
                backdate_invoices: false,
                skip_checkout_session: false,
                skip_past_invoices: false,
                resource_id,
                custom_properties: serde_json::Value::Null,
                external_id: None,
            },
            price_components: Some(CreateSubscriptionComponents {
                parameterized_components: vec![],
                overridden_components: vec![],
                extra_components: vec![metric_component(
                    "Model usage",
                    self.metric.id,
                    UsagePricingModel::DimensionRates(DimensionRates {
                        dimensions: vec!["model".to_owned()],
                        rates: vec![rate("lite", 2), rate("standard", 12), rate("max", 60)],
                        unit_size: Decimal::ONE,
                        on_unmatched: RateCardUnmatched::Ignore,
                    }),
                )],
                remove_components: vec![COMP_LEETCODE_RATE_ID],
            }),
            add_ons: None,
            coupons: None,
            entitlements: vec![],
            minimum_override: None,
            drop_minimum: false,
        }
    }

    async fn create(
        &self,
        request: CreateSubscription,
    ) -> Result<CreatedSubscription, error_stack::Report<StoreError>> {
        self.meteroid
            .services
            .insert_subscription(Actor::System, request, TENANT_ID)
            .await
    }

    /// Ingests the events, then waits until metering counts every customer's tokens in `range`.
    async fn ingest(&self, events: Vec<Tokens>, range: UsagePeriod) {
        self.ingest_keyed(events.into_iter().map(|e| (e, API_KEY)).collect(), range)
            .await
    }

    /// `ingest`, each event carrying its key under the property paired with it.
    async fn ingest_keyed(&self, events: Vec<(Tokens, &'static str)>, range: UsagePeriod) {
        self.ingest_over(events, range, Decimal::ZERO).await
    }

    /// `ingest_keyed` into a range already holding `stored` tokens of each customer ingested.
    async fn ingest_over(
        &self,
        events: Vec<(Tokens, &'static str)>,
        range: UsagePeriod,
        stored: Decimal,
    ) {
        let mut expected: Vec<(CustomerId, Decimal)> = Vec::new();
        for (e, _) in &events {
            match expected
                .iter_mut()
                .find(|(customer, _)| *customer == e.customer)
            {
                Some((_, total)) => *total += Decimal::from(e.tokens),
                None => expected.push((e.customer, stored + Decimal::from(e.tokens))),
            }
        }
        let events = events
            .into_iter()
            .map(|(e, property)| {
                let mut properties = std::collections::HashMap::from([
                    ("model".to_owned(), e.model.to_owned()),
                    ("tokens".to_owned(), e.tokens.to_string()),
                ]);
                if let Some(key) = e.api_key {
                    properties.insert(property.to_owned(), key.to_owned());
                }
                Event {
                    id: Uuid::now_v7().to_string(),
                    code: CODE.to_owned(),
                    customer_id: Some(event::CustomerId::MeteroidCustomerId(
                        e.customer.to_string(),
                    )),
                    timestamp: e.at.to_rfc3339(),
                    properties,
                }
            })
            .collect::<Vec<_>>();
        let failures = self
            .ingest
            .events
            .clone()
            .ingest(Request::new(IngestRequest {
                events,
                allow_backfilling: true,
            }))
            .await
            .expect("Could not ingest events")
            .into_inner()
            .failures;
        assert!(failures.is_empty(), "{failures:?}");

        for (customer, total) in expected {
            (|| async {
                let stored = self
                    .usage
                    .fetch_total_usage(&TENANT_ID, &customer, &self.metric, range.clone())
                    .await
                    .unwrap_or(Decimal::ZERO);
                if stored == total {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("{stored} of {total} tokens stored"))
                }
            })
            .retry(
                backon::ConstantBuilder::default()
                    .with_delay(Duration::from_millis(300))
                    .with_max_times(300),
            )
            .await
            .expect("the events reach the store");
        }
    }

    /// 0 issues a period's invoice at its close; a negative grace settles a termination inline.
    async fn grace_hours(&self, hours: i32) {
        self.meteroid
            .store
            .patch_invoicing_entity(
                Actor::User { id: USER_ID },
                InvoicingEntityPatch {
                    id: INVOICING_ENTITY_ID,
                    grace_period_hours: Some(hours),
                    ..Default::default()
                },
                TENANT_ID,
            )
            .await
            .expect("the entity's grace");
    }

    async fn grace_elapsed(&self) {
        use diesel_async::RunQueryDsl;
        let mut conn = self.meteroid.store.get_conn().await.unwrap();
        diesel::sql_query(
            "UPDATE invoice SET finalize_after = now() - interval '1 minute' \
             WHERE tenant_id = $1 AND finalize_after IS NOT NULL",
        )
        .bind::<diesel::sql_types::Uuid, _>(TENANT_ID)
        .execute(&mut conn)
        .await
        .expect("drafts due");
    }

    async fn consolidate(&self) {
        self.meteroid
            .store
            .patch_invoicing_entity(
                Actor::User { id: USER_ID },
                InvoicingEntityPatch {
                    id: INVOICING_ENTITY_ID,
                    consolidate_recurring_invoices: Some(true),
                    ..Default::default()
                },
                TENANT_ID,
            )
            .await
            .expect("consolidated recurring invoices");
    }

    /// An amendment of `subscription_id` dated `on` adding `component`.
    async fn amend_adding(
        &self,
        subscription_id: SubscriptionId,
        on: NaiveDate,
        component: meteroid_store::domain::subscription_components::ExtraComponent,
    ) {
        self.meteroid
            .services
            .apply_amendment_immediate_at(
                Actor::System,
                subscription_id,
                TENANT_ID,
                SubscriptionAmendment {
                    apply_mode: PlanChangeMode::Immediate,
                    component_changes: ComponentChanges {
                        edited: vec![],
                        added: vec![AddExtraComponent {
                            name: component.name,
                            product: component.product_ref,
                            price_entry: component.price_entry,
                            fee_structure: component.fee_structure,
                        }],
                        removed: vec![],
                    },
                    add_on_changes: AddOnChanges::default(),
                },
                on,
            )
            .await
            .expect("the amendment");
    }

    /// The key's subscription for `customer` from `start`, pricing nothing: its fees come later.
    async fn subscribe_unpriced(
        &self,
        customer: CustomerId,
        start: NaiveDate,
    ) -> (
        SubscriptionId,
        meteroid_store::domain::subscription_components::ExtraComponent,
    ) {
        let key_a = self.resource(customer, "key-A").await;
        let mut request = self.request(customer, start, Some(key_a));
        let tokens = request
            .price_components
            .as_mut()
            .map(|components| components.extra_components.remove(0))
            .expect("the token component");
        (self.create(request).await.unwrap().id, tokens)
    }

    async fn process_cycles(&self) {
        for _ in 0..3 {
            self.meteroid
                .services
                .run_subscription_steps()
                .await
                .expect("the controller's steps");
            self.meteroid
                .services
                .run_invoice_deadlines()
                .await
                .expect("the invoices' deadlines");
        }
    }

    /// What the subscription's finalized invoices billed for the token metric on usage dated
    /// from `from`.
    async fn invoiced(&self, subscription_id: SubscriptionId, from: NaiveDate) -> i64 {
        self.meteroid
            .store
            .list_invoices(
                TENANT_ID,
                None,
                Some(subscription_id),
                None,
                None,
                None,
                false,
                None,
                vec![],
                Some("created_at.asc".to_owned()),
                PaginationRequest {
                    page: 0,
                    per_page: None,
                },
            )
            .await
            .expect("invoices")
            .items
            .into_iter()
            .map(|invoice| invoice.invoice)
            .filter(|invoice| {
                invoice.status == meteroid_store::domain::enums::InvoiceStatusEnum::Finalized
            })
            .flat_map(|invoice| invoice.line_items)
            .filter(|line| line.metric_id == Some(self.metric.id) && line.start_date >= from)
            .map(|line| line.amount_subtotal)
            .sum()
    }

    /// The token usage the subscription's next invoice bills.
    async fn previewed(&self, subscription_id: SubscriptionId) -> i64 {
        self.previewed_metric(subscription_id, self.metric.id).await
    }

    async fn previewed_metric(
        &self,
        subscription_id: SubscriptionId,
        metric: BillableMetricId,
    ) -> i64 {
        let details = self
            .meteroid
            .store
            .get_subscription_details(TENANT_ID, subscription_id)
            .await
            .expect("subscription details");
        self.meteroid
            .services
            .compute_upcoming_invoice(&details)
            .await
            .expect("the upcoming invoice")
            .invoice_lines
            .into_iter()
            .filter(|line| line.metric_id == Some(metric))
            .map(|line| line.amount_subtotal)
            .sum()
    }

    async fn unattributed(
        &self,
        customer_id: CustomerId,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Vec<(BillableMetricId, Decimal)> {
        self.meteroid
            .services
            .unattributed_usage(
                TENANT_ID,
                customer_id,
                from.and_time(NaiveTime::MIN),
                to.and_time(NaiveTime::MIN),
            )
            .await
            .expect("unattributed usage")
            .into_iter()
            .map(|usage| (usage.metric.id, usage.quantity))
            .collect()
    }
}

impl Stack {
    /// A new customer of the seeded invoicing entity, untaxed (no billing address).
    async fn customer(&self, name: &str) -> CustomerId {
        let id = CustomerId::new();
        let mut conn = self.meteroid.store.get_conn().await.unwrap();
        CustomerRowNew {
            id,
            name: name.to_owned(),
            created_at: None,
            tenant_id: TENANT_ID,
            alias: None,
            currency: "EUR".to_owned(),
            invoicing_entity_id: INVOICING_ENTITY_ID,
            billing_address: None,
            shipping_address: None,
            billing_email: None,
            current_payment_method_id: None,
            vat_number: None,
            legal_number: None,
            buyer_reference: None,
            custom_taxes: serde_json::json!([]),
            invoicing_emails: vec![],
            phone: None,
            tax_status: diesel_models::enums::CustomerTaxStatusEnum::Taxable,
            exemption_reason: None,
            vat_number_format_valid: false,
            connected_account_id: None,
            vat_number_validation_status: None,
            custom_properties: serde_json::Value::Null,
            preferred_locales: vec![],
            customer_type: diesel_models::enums::CustomerTypeEnum::Company,
            first_name: None,
            last_name: None,
        }
        .insert(&mut conn)
        .await
        .expect("a customer");
        id
    }

    async fn set_resource(
        &self,
        subscription_id: SubscriptionId,
        resource_id: Option<ResourceId>,
    ) -> Result<meteroid_store::domain::Subscription, error_stack::Report<StoreError>> {
        self.meteroid
            .store
            .patch_subscription(
                TENANT_ID,
                SubscriptionPatch {
                    id: subscription_id,
                    charge_automatically: None,
                    auto_advance_invoices: None,
                    net_terms: None,
                    invoice_memo: None,
                    purchase_order: None,
                    payment_methods_config: None,
                    custom_properties: None,
                    resource_id: Some(resource_id),
                },
                Actor::System,
            )
            .await
    }

    async fn subscription_row(&self, subscription_id: SubscriptionId) -> SubscriptionRow {
        let mut conn = self.meteroid.store.get_conn().await.unwrap();
        SubscriptionRow::get_subscription_by_id(&mut conn, &TENANT_ID, subscription_id)
            .await
            .expect("the subscription")
            .subscription
    }

    async fn finalized(&self, subscription_id: SubscriptionId) -> Vec<Invoice> {
        self.invoices_in(subscription_id, InvoiceStatusEnum::Finalized)
            .await
    }

    async fn invoices_in(
        &self,
        subscription_id: SubscriptionId,
        status: InvoiceStatusEnum,
    ) -> Vec<Invoice> {
        self.invoices_of(subscription_id)
            .await
            .into_iter()
            .filter(|invoice| invoice.status == status)
            .collect()
    }

    async fn invoices_of(&self, subscription_id: SubscriptionId) -> Vec<Invoice> {
        self.meteroid
            .store
            .list_invoices(
                TENANT_ID,
                None,
                Some(subscription_id),
                None,
                None,
                None,
                false,
                None,
                vec![],
                Some("created_at.asc".to_owned()),
                PaginationRequest {
                    page: 0,
                    per_page: None,
                },
            )
            .await
            .expect("invoices")
            .items
            .into_iter()
            .map(|invoice| invoice.invoice)
            .collect()
    }

    /// What finalized credit notes gave back on the invoice, tax included.
    async fn credited(&self, invoice_id: InvoiceId) -> i64 {
        let mut conn = self.meteroid.store.get_conn().await.unwrap();
        CreditNoteRow::list_by_invoice_id(&mut conn, TENANT_ID, invoice_id)
            .await
            .expect("credit notes")
            .iter()
            .filter(|note| note.finalized_at.is_some() && note.voided_at.is_none())
            .map(|note| note.total.abs())
            .sum()
    }
}

/// Tokens per model in the annexes of the subscription's finalized invoices.
fn annexed_tokens(invoices: &[Invoice]) -> Vec<(String, Decimal)> {
    let mut tokens = Vec::new();
    for invoice in invoices {
        for annex in &invoice.annexes {
            for group in &annex.table.groups {
                for row in group.rows.iter().filter(|row| !row.emphasis) {
                    if let [InvoiceAnnexCell::Text(model), InvoiceAnnexCell::Number(sum)] =
                        &row.cells[..]
                    {
                        tokens.push((model.clone(), *sum));
                    }
                }
            }
        }
    }
    tokens.sort();
    tokens
}

fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap()
}

fn at(date: NaiveDate, hour: u32) -> DateTime<Utc> {
    date.and_hms_opt(hour, 0, 0).unwrap().and_utc()
}

/// "Today" for tests that read period boundaries: yesterday, so every boundary lies a day behind the
/// clock and a run crossing midnight UTC moves none of them.
fn scenario_day() -> NaiveDate {
    Utc::now().date_naive() - Days::new(1)
}

fn range(from: NaiveDate, to: NaiveDate) -> UsagePeriod {
    UsagePeriod {
        start: from.and_time(NaiveTime::MIN),
        end: to.and_time(NaiveTime::MIN),
    }
}

/// S19 on Spotify (A for `key-A`, B for the customer) and its all-resources variant on Uber (B for
/// `key-B`), both billed over last month; S10's attribution rows on Comodo.
/// Rules: R-145
#[tokio::test]
async fn test_each_event_is_billed_by_one_subscription_or_listed_as_unattributed() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let today = Utc::now().date_naive();
    let this_month = month_start(today);
    let last_month = this_month - Months::new(1);
    let next_month = this_month + Months::new(1);
    let s10_b_start = today - Days::new(5);

    let mut events = Vec::new();
    for customer in [CUST_SPOTIFY_ID, CUST_UBER_ID] {
        events.extend([
            // E1: standard 5M = €60.00
            Tokens {
                customer,
                at: at(last_month + Days::new(2), 10),
                api_key: Some("key-A"),
                model: "standard",
                tokens: 5_000_000,
            },
            // E2: a key no subscription has, standard 2M = €24.00
            Tokens {
                customer,
                at: at(last_month + Days::new(3), 10),
                api_key: Some("key-Z"),
                model: "standard",
                tokens: 2_000_000,
            },
            // E3: no api_key at all, lite 10M = €20.00
            Tokens {
                customer,
                at: at(last_month + Days::new(4), 10),
                api_key: None,
                model: "lite",
                tokens: 10_000_000,
            },
        ]);
    }
    events.extend([
        // S10: A's key, lite 350M = €700.00
        Tokens {
            customer: CUST_COMODO_ID,
            at: at(last_month + Days::new(5), 10),
            api_key: Some("key-A"),
            model: "lite",
            tokens: 350_000_000,
        },
        // S10: a key no subscription has, lite 10M (€20.00 at the lite price)
        Tokens {
            customer: CUST_COMODO_ID,
            at: at(last_month + Days::new(6), 10),
            api_key: Some("key-Z"),
            model: "lite",
            tokens: 10_000_000,
        },
        // S10: B's key, standard 12.5M = €150.00, in B's open period
        Tokens {
            customer: CUST_COMODO_ID,
            at: at(s10_b_start, 1),
            api_key: Some("key-B"),
            model: "standard",
            tokens: 12_500_000,
        },
    ]);
    stack.ingest(events, range(last_month, next_month)).await;

    let s19_a = stack
        .subscribe(CUST_SPOTIFY_ID, last_month, Some("key-A"))
        .await
        .expect("A");
    let s19_b = stack
        .subscribe(CUST_SPOTIFY_ID, last_month, None)
        .await
        .expect("B, for the customer");
    let all_a = stack
        .subscribe(CUST_UBER_ID, last_month, Some("key-A"))
        .await
        .expect("A");
    let all_b = stack
        .subscribe(CUST_UBER_ID, last_month, Some("key-B"))
        .await
        .expect("B");
    let s10_a = stack
        .subscribe(CUST_COMODO_ID, last_month, Some("key-A"))
        .await
        .expect("A");
    let s10_b = stack
        .subscribe(CUST_COMODO_ID, s10_b_start, Some("key-B"))
        .await
        .expect("B");

    // A second subscription for key-A is refused: a plan change upgrades A instead.
    let duplicate = stack
        .create(stack.request(
            CUST_SPOTIFY_ID,
            today,
            Some(stack.resource(CUST_SPOTIFY_ID, "key-A").await),
        ))
        .await
        .expect_err("a second subscription for key-A");
    assert!(
        matches!(duplicate.current_context(), StoreError::InvalidArgument(m) if m.contains("already sold")),
        "{duplicate:?}"
    );
    // The plan is sold for the customer, so one for another key warns.
    let elsewhere = stack
        .create(stack.request(
            CUST_SPOTIFY_ID,
            today,
            Some(stack.resource(CUST_SPOTIFY_ID, "key-D").await),
        ))
        .await
        .expect("a subscription for key-D");
    assert_eq!(
        elsewhere.warnings,
        vec!["plan_sold_for_mismatch".to_owned()]
    );

    // Last month's invoices are issued when it closes: on the 1st they would still be in grace.
    stack.grace_hours(0).await;
    stack.process_cycles().await;

    // S19: E1 billed once, by A; B is the fallback for E2 + E3.
    assert_eq!(stack.invoiced(s19_a, last_month).await, 6_000);
    assert_eq!(stack.invoiced(s19_b, last_month).await, 4_400);
    assert!(
        stack
            .unattributed(CUST_SPOTIFY_ID, last_month, this_month)
            .await
            .is_empty()
    );

    // All for a resource: E2 + E3 (12M tokens, €44.00 at the plan's prices) are billed by nobody.
    assert_eq!(stack.invoiced(all_a, last_month).await, 6_000);
    assert_eq!(stack.invoiced(all_b, last_month).await, 0);
    assert_eq!(
        stack
            .unattributed(CUST_UBER_ID, last_month, this_month)
            .await,
        vec![(stack.metric.id, dec!(12_000_000))]
    );

    // S10: A bills key-A; B bills key-B on its own period; key-Z is kept and billed to nobody.
    assert_eq!(stack.invoiced(s10_a, last_month).await, 70_000);
    assert_eq!(stack.previewed(s10_b).await, 15_000);
    assert_eq!(
        stack
            .unattributed(CUST_COMODO_ID, last_month, next_month)
            .await,
        vec![(stack.metric.id, dec!(10_000_000))]
    );
}

/// The next-invoice preview and the invoice agree for a resource, whose subscription keeps it once
/// active.
#[tokio::test]
async fn test_preview_and_invoice_agree_and_an_active_subscription_keeps_its_resource() {
    preview_and_invoice_agree(Opened::Earlier).await;
}

/// The same, cancelled on the day the period renewed, once its renewal invoice was issued: the
/// usage of that day up to the cancellation is billed once, on a final invoice of its own.
#[tokio::test]
async fn test_a_cancellation_on_the_renewal_day_bills_that_day_s_usage() {
    preview_and_invoice_agree(Opened::RenewedToday { issued: true }).await;
}

/// The same, while the renewal invoice billing a fee in advance is still a draft in its grace
/// period: it keeps billing its own period only, its fee credited back whole once issued, and the
/// final invoice bills that day's usage.
#[tokio::test]
async fn test_a_cancellation_on_the_renewal_day_leaves_the_renewal_draft_its_own_period() {
    preview_and_invoice_agree(Opened::RenewedToday { issued: false }).await;
}

/// The same, on subscriptions started today whose first invoice billed a fee in advance: the fee
/// is credited back whole, the usage billed on a final invoice.
#[tokio::test]
async fn test_a_cancellation_on_the_first_day_bills_that_day_s_usage() {
    preview_and_invoice_agree(Opened::StartedToday).await;
}

/// When the period open at the cancellation began.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Opened {
    /// Periods from 40 days ago: today is not a period's first day.
    Earlier,
    /// Renewed today; the renewal invoice issued, or still a draft in its grace period, billing
    /// the €35 rate in advance.
    RenewedToday { issued: bool },
    /// Started today, its first invoice billing the €35 rate in advance.
    StartedToday,
}

async fn preview_and_invoice_agree(opened: Opened) {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let clock = Utc::now();
    let today = clock.date_naive();
    let (start, anchor) = match opened {
        Opened::Earlier => (today - Days::new(40), None),
        Opened::RenewedToday { .. } => (today - Days::new(40), Some(today.day() as u16)),
        Opened::StartedToday => (today, None),
    };
    // The period opened today bills a fee in advance, which the cancellation gives back whole.
    let advance_fee = matches!(
        opened,
        Opened::StartedToday | Opened::RenewedToday { issued: false }
    );
    let opens_today = opened != Opened::Earlier;
    let now = (clock - chrono::Duration::minutes(1)).max(at(today, 0));

    stack
        .ingest(
            vec![
                // standard 1M = €12.00
                Tokens {
                    customer: CUST_SPOTIFY_ID,
                    at: now,
                    api_key: Some("key-A"),
                    model: "standard",
                    tokens: 1_000_000,
                },
                // standard 2M = €24.00
                Tokens {
                    customer: CUST_SPOTIFY_ID,
                    at: now,
                    api_key: Some("key-B"),
                    model: "standard",
                    tokens: 2_000_000,
                },
                // lite 1M = €2.00
                Tokens {
                    customer: CUST_SPOTIFY_ID,
                    at: now,
                    api_key: None,
                    model: "lite",
                    tokens: 1_000_000,
                },
            ],
            range(start, today + Days::new(1)),
        )
        .await;

    // Before subscribing: a first invoice is issued as the subscription is created.
    match opened {
        Opened::RenewedToday { issued: true } | Opened::StartedToday => stack.grace_hours(0).await,
        Opened::RenewedToday { issued: false } => stack.grace_hours(48).await,
        Opened::Earlier => {}
    }
    let a = stack
        .subscribe_with(
            CUST_SPOTIFY_ID,
            start,
            None,
            anchor,
            advance_fee,
            Some("key-A"),
        )
        .await
        .expect("A");
    let b = stack
        .subscribe_with(CUST_SPOTIFY_ID, start, None, anchor, advance_fee, None)
        .await
        .expect("B");
    stack.process_cycles().await;
    for subscription in [a, b] {
        let period = stack
            .subscription_row(subscription)
            .await
            .current_period_start;
        assert_eq!(
            period == today,
            opens_today,
            "the open period starts {period}"
        );
        let today_s = stack
            .invoices_of(subscription)
            .await
            .into_iter()
            .filter(|invoice| invoice.billing_period_start == today)
            .map(|invoice| invoice.status)
            .collect::<Vec<_>>();
        match opened {
            Opened::Earlier => assert!(today_s.is_empty(), "{today_s:?}"),
            Opened::RenewedToday { issued: false } => {
                assert_eq!(today_s, vec![InvoiceStatusEnum::Draft])
            }
            Opened::RenewedToday { issued: true } | Opened::StartedToday => assert!(
                matches!(
                    today_s[..],
                    [InvoiceStatusEnum::Finalized | InvoiceStatusEnum::Closed]
                ),
                "{today_s:?}"
            ),
        }
    }
    assert_eq!(stack.invoiced(a, start).await, 0);
    assert_eq!(stack.invoiced(b, start).await, 0);
    fund(
        &stack.meteroid.store,
        TENANT_ID,
        CUST_SPOTIFY_ID,
        None,
        100_000,
    )
    .await;

    assert_eq!(stack.previewed(a).await, 1_200);
    assert_eq!(stack.previewed(b).await, 2_600);

    let key_b = stack.resource(CUST_SPOTIFY_ID, "key-B").await;
    let refused = stack
        .set_resource(a, Some(key_b))
        .await
        .expect_err("the resource of an active subscription is fixed");
    assert!(matches!(
        refused.current_context(),
        StoreError::InvalidArgument(_)
    ));

    // Ending both now bills the period so far: each invoice is its preview. A negative grace
    // settles the termination inline, so the closing invoices finalize within the test.
    // One at a time: B's attribution depends on A's end, so B's cancellation priced while A's is
    // still uncommitted reads metering again and is retried later, past this test's passes.
    stack.grace_hours(-1).await;
    for subscription in [a, b] {
        stack
            .meteroid
            .services
            .cancel_subscription(
                Actor::System,
                subscription,
                TENANT_ID,
                None,
                CancellationEffectiveAt::Immediate,
            )
            .await
            .expect("terminate");
        stack.process_cycles().await;
        assert_eq!(
            stack.subscription_row(subscription).await.status,
            SubscriptionStatusEnum::Cancelled
        );
    }
    assert_eq!(stack.invoiced(a, start).await, 1_200);
    assert_eq!(stack.invoiced(b, start).await, 2_600);

    // A renewal draft outlives the termination in its grace: once that elapses it is issued, and
    // the endings sweep credits it, as its worker does; run twice, it credits nothing more.
    if opened == (Opened::RenewedToday { issued: false }) {
        stack.grace_elapsed().await;
        stack.process_cycles().await;
    }
    for _ in 0..2 {
        stack
            .meteroid
            .services
            .settle_missed_plan_change_endings(100)
            .await
            .expect("endings sweep");
    }

    // Today's usage is on one invoice of each: the final one when the period opened today, whose
    // lines read it from the period's start to the cancellation. Nothing is left to bill it again.
    for (subscription, usage) in [(a, 1_200), (b, 2_600)] {
        let invoices = stack.invoices_of(subscription).await;
        let finals = invoices
            .iter()
            .filter(|invoice| invoice.invoice_type == InvoiceType::Final)
            .collect::<Vec<_>>();
        if !opens_today {
            assert!(finals.is_empty());
            continue;
        }
        assert!(
            invoices.iter().all(|invoice| matches!(
                invoice.status,
                InvoiceStatusEnum::Finalized | InvoiceStatusEnum::Closed
            )),
            "{:?}",
            invoices
                .iter()
                .map(|invoice| (&invoice.invoice_type, &invoice.status))
                .collect::<Vec<_>>()
        );
        let [final_invoice] = finals[..] else {
            panic!("one final invoice, not {}", finals.len());
        };
        assert_eq!(final_invoice.status, InvoiceStatusEnum::Finalized);
        assert_eq!(final_invoice.billing_period_start, today);
        let lines = final_invoice
            .line_items
            .iter()
            .filter(|line| line.metric_id == Some(stack.metric.id))
            .collect::<Vec<_>>();
        assert!(
            lines.iter().all(|line| line.start_date == today
                && line.end_date == today
                && line.usage_ends_at.is_some_and(|end| end > now)),
            "{lines:?}"
        );
        assert_eq!(
            lines.iter().map(|line| line.amount_subtotal).sum::<i64>(),
            usage
        );
        // The period's own invoice bills none of it.
        assert!(
            invoices
                .iter()
                .filter(|invoice| invoice.invoice_type != InvoiceType::Final)
                .flat_map(|invoice| &invoice.line_items)
                .all(|line| line.metric_id != Some(stack.metric.id) || line.amount_subtotal == 0),
        );
        // The fee the period's own invoice billed in advance came back whole, once.
        if advance_fee {
            let opening = invoices
                .iter()
                .find(|invoice| {
                    invoice.invoice_type == InvoiceType::Recurring
                        && invoice.billing_period_start == today
                })
                .expect("the period's own invoice");
            assert_eq!(opening.status, InvoiceStatusEnum::Finalized);
            assert!(opening.total > 0);
            assert_eq!(stack.credited(opening.id).await, opening.total);
        }
    }
}

/// A resource subscription's claim ends with it: cancelled at a date, or cancelled at once before
/// its fixed term. Its key's later events fall back to the customer's subscription.
#[tokio::test]
async fn test_a_resource_claims_nothing_once_its_subscription_ended() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let today = Utc::now().date_naive();
    let this_month = month_start(today);
    let last_month = this_month - Months::new(1);
    let next_month = this_month + Months::new(1);

    let cancelled = stack.customer("Cancelled at a date").await;
    let fixed_term = stack.customer("Fixed term cancelled at once").await;
    let mut pairs = Vec::new();
    for (customer, end) in [
        (cancelled, None),
        (fixed_term, Some(next_month + Months::new(1))),
    ] {
        let a = stack
            .subscribe_until(customer, last_month, end, Some("key-A"))
            .await
            .expect("A");
        let b = stack
            .subscribe(customer, last_month, None)
            .await
            .expect("B");
        pairs.push((customer, a, b));
    }
    stack
        .meteroid
        .services
        .cancel_subscription(
            Actor::System,
            pairs[0].1,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(this_month),
        )
        .await
        .expect("cancel A at the start of this month");
    stack.process_cycles().await;
    stack
        .meteroid
        .services
        .cancel_subscription(
            Actor::System,
            pairs[1].1,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await
        .expect("end the fixed-term A now");
    stack.process_cycles().await;
    for (_, a, _) in &pairs {
        assert_eq!(
            stack.subscription_row(*a).await.status,
            SubscriptionStatusEnum::Cancelled
        );
    }
    tokio::time::sleep(Duration::from_millis(1_100)).await;

    // key-A, standard 1M = €12.00, after each A ended.
    let now = Utc::now();
    stack
        .ingest(
            pairs
                .iter()
                .map(|(customer, _, _)| Tokens {
                    customer: *customer,
                    at: now,
                    api_key: Some("key-A"),
                    model: "standard",
                    tokens: 1_000_000,
                })
                .collect(),
            range(last_month, next_month),
        )
        .await;

    for (_, a, b) in &pairs {
        assert_eq!(stack.previewed(*b).await, 1_200, "B falls back on key-A");
        assert_eq!(stack.invoiced(*a, this_month).await, 0);
    }
}

/// A resource subscription never claims usage invoiced already: A (key-A, periods from 25 days
/// ago) next to B (the customer, from 40 days ago, invoiced once) bills E1 while B billed E2; a
/// subscription for key-A starting before B's open period is refused.
#[tokio::test]
async fn test_a_resource_subscription_never_claims_invoiced_usage() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let today = Utc::now().date_naive();
    let a_start = today - Days::new(25);
    let b_start = today - Days::new(40);
    let early = at(a_start, 1);
    let now = Utc::now() - chrono::Duration::minutes(1);

    // Annexes list each subscription's events per model, summing their tokens.
    let template = create_annex_template(
        &stack.meteroid.store,
        AnnexTemplateNew {
            tenant_id: TENANT_ID,
            name: "Tokens".to_owned(),
            definition: AnnexDefinition::EventTable(EventTableDefinition {
                title: "Tokens".to_owned(),
                description: None,
                source: EventSource {
                    event_codes: vec![CODE.to_owned()],
                },
                columns: vec![
                    AnnexColumn {
                        label: "Model".to_owned(),
                        source: ColumnSource::Property("model".to_owned()),
                        transform: None,
                        aggregation: Some(ColumnAggregation::First),
                    },
                    AnnexColumn {
                        label: "Tokens".to_owned(),
                        source: ColumnSource::Property("tokens".to_owned()),
                        transform: None,
                        aggregation: Some(ColumnAggregation::Sum),
                    },
                ],
                aggregate_by: vec!["model".to_owned()],
                group_by: vec![],
                sort_by: vec![],
                show_group_subtotals: false,
                show_total: false,
            }),
        },
    )
    .await;
    set_plan_version_annex_templates(
        &stack.meteroid.store,
        TENANT_ID,
        PLAN_VERSION_1_LEETCODE_ID,
        &[template.id],
    )
    .await;

    let changed = stack.customer("Two subscriptions").await;
    let backdated = stack.customer("Backdated resource").await;
    let mut events = Vec::new();
    {
        let customer = changed;
        events.extend([
            // E1 key-A standard 1M = €12.00; E2 key-B standard 2M = €24.00, in B's first period.
            Tokens {
                customer,
                at: early,
                api_key: Some("key-A"),
                model: "standard",
                tokens: 1_000_000,
            },
            Tokens {
                customer,
                at: early,
                api_key: Some("key-B"),
                model: "standard",
                tokens: 2_000_000,
            },
            // E3 key-A standard 3M = €36.00; E4 key-B standard 4M = €48.00, after the change.
            Tokens {
                customer,
                at: now,
                api_key: Some("key-A"),
                model: "standard",
                tokens: 3_000_000,
            },
            Tokens {
                customer,
                at: now,
                api_key: Some("key-B"),
                model: "standard",
                tokens: 4_000_000,
            },
        ]);
    }
    // B of the backdated customer invoices key-B's usage over its first period.
    events.push(Tokens {
        customer: backdated,
        at: early,
        api_key: Some("key-B"),
        model: "standard",
        tokens: 2_000_000,
    });
    stack
        .ingest(events, range(b_start, today + Days::new(1)))
        .await;

    let mut pairs = Vec::new();
    for customer in [changed] {
        let a = stack
            .subscribe(customer, a_start, Some("key-A"))
            .await
            .expect("A");
        let b = stack.subscribe(customer, b_start, None).await.expect("B");
        pairs.push((a, b));
    }
    stack.process_cycles().await;

    for (a, b) in &pairs {
        let b_period = stack.subscription_row(*b).await.current_period_start;
        assert!(
            b_period > a_start,
            "B invoiced its first period after A started"
        );
        assert_eq!(stack.invoiced(*b, b_start).await, 2_400, "E2; E1 is A's");
        assert_eq!(
            stack.invoiced(*a, a_start).await,
            0,
            "A's first period is open"
        );
        assert_eq!(
            annexed_tokens(&stack.finalized(*b).await),
            vec![("standard".to_owned(), dec!(2_000_000))],
            "B's annex lists E2 only"
        );
    }

    // A: E1 and E3 (key-A); B: E2 then E4 (no resource claims key-B).
    let (a, b) = pairs[0];
    assert_eq!(stack.previewed(a).await, 4_800);
    assert_eq!(stack.previewed(b).await, 4_800);

    // A resource subscription cannot reach back into usage B already invoiced.
    let b = stack.subscribe(backdated, b_start, None).await.expect("B");
    stack.process_cycles().await;
    let refused = stack
        .subscribe(backdated, a_start, Some("key-A"))
        .await
        .expect_err("a subscription for key-A starting before B's invoiced usage");
    assert!(matches!(
        refused.current_context(),
        StoreError::InvalidArgument(_)
    ));
    // One waiting for its activation claims from then, past anything invoiced: it may move.
    let mut pending = stack.request(backdated, a_start, None);
    pending.subscription.activation_condition = SubscriptionActivationCondition::Manual;
    let pending = stack
        .create(pending)
        .await
        .expect("a pending subscription")
        .id;
    let key_c = stack.resource(backdated, "key-C").await;
    stack
        .set_resource(pending, Some(key_c))
        .await
        .expect("a pending subscription moves onto key-C");
    assert_eq!(
        stack.subscription_row(pending).await.resource_id,
        Some(key_c)
    );

    let b_period = stack.subscription_row(b).await.current_period_start;
    stack
        .subscribe(backdated, b_period, Some("key-A"))
        .await
        .expect("a subscription for key-A starting where B's open period starts");
}

/// Two versions of one event, same id and timestamp, one per key: the first ingested is kept,
/// the re-send is dropped, and the event is billed once.
#[tokio::test]
async fn test_the_first_version_of_an_event_is_billed_once() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let last_month = month_start(Utc::now().date_naive()) - Months::new(1);
    let customer = stack.customer("Versions").await;
    stack
        .send_versions(&stack.ingest, customer, last_month)
        .await;

    assert_eq!(
        stack.read_key(customer, last_month, "key-A").await,
        dec!(1_000_000)
    );
    assert_eq!(
        stack.read_key(customer, last_month, "key-B").await,
        Decimal::ZERO,
        "the re-sent copy is dropped"
    );
    stack.bill_versions(customer, last_month, 1_200, 0).await;
}

/// The same versions for a keep-last tenant: the last copy ingested wins, billed once.
#[tokio::test]
async fn test_a_keep_last_tenant_bills_the_last_version_once() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let last_month = month_start(Utc::now().date_naive()) - Months::new(1);
    let customer = stack.customer("Versions").await;
    stack
        .meteroid
        .store
        .update_tenant(
            Actor::System,
            TenantUpdate {
                keep_last_events: Some(true),
                ..Default::default()
            },
            ORGANIZATION_ID,
            TENANT_ID,
        )
        .await
        .expect("keep-last tenant");
    // A new key: metering caches the seeded one with the flag off.
    let (api_key, _) = stack
        .meteroid
        .store
        .insert_api_token(
            Actor::System,
            ApiTokenNew {
                name: "keep-last".to_owned(),
                tenant_id: TENANT_ID,
            },
        )
        .await
        .expect("api key");
    let ingest = MeteringClients::from_channel(
        stack._metering.channel.clone(),
        &api_key,
        &stack._metering._config.internal_auth,
    );
    stack.send_versions(&ingest, customer, last_month).await;

    assert_eq!(
        stack.read_key(customer, last_month, "key-B").await,
        dec!(1_000_000)
    );
    assert_eq!(
        stack.read_key(customer, last_month, "key-A").await,
        Decimal::ZERO,
        "the earlier copy is superseded"
    );
    stack.bill_versions(customer, last_month, 0, 1_200).await;
}

impl Stack {
    /// One event sent twice, key-A then key-B, and consumed by metering.
    async fn send_versions(
        &self,
        ingest: &MeteringClients,
        customer: CustomerId,
        month: NaiveDate,
    ) {
        let at = at(month + Days::new(3), 10);
        let id = Uuid::now_v7().to_string();
        for key in ["key-A", "key-B"] {
            ingest
                .events
                .clone()
                .ingest(Request::new(IngestRequest {
                    events: vec![Event {
                        id: id.clone(),
                        code: CODE.to_owned(),
                        customer_id: Some(event::CustomerId::MeteroidCustomerId(
                            customer.to_string(),
                        )),
                        timestamp: at.to_rfc3339(),
                        properties: std::collections::HashMap::from([
                            ("api_key".to_owned(), key.to_owned()),
                            ("model".to_owned(), "standard".to_owned()),
                            ("tokens".to_owned(), "1000000".to_owned()),
                        ]),
                    }],
                    allow_backfilling: true,
                }))
                .await
                .expect("ingest");
            metering_it::kafka::wait_until_consumed(self.kafka_port, "meteroid-events-raw")
                .await
                .expect("consumed");
        }
    }

    /// The month's usage a subscription for `key` reads, once the stored copy is readable.
    async fn read_key(&self, customer: CustomerId, month: NaiveDate, key: &str) -> Decimal {
        let scoped = |key: &str| BillableMetric {
            usage_scope: UsageScope {
                segments: vec![ScopeSegment {
                    from: None,
                    until: None,
                    matching: Some(ResourceMatch {
                        properties: vec![API_KEY.to_owned()],
                        value: key.to_owned(),
                    }),
                    excluded: vec![],
                }],
            },
            ..self.metric.clone()
        };
        let period = range(month, month + Months::new(1));
        let whole = self.metric.clone();
        (|| async {
            let read = self
                .usage
                .fetch_total_usage(&TENANT_ID, &customer, &whole, period.clone())
                .await
                .unwrap_or(Decimal::ZERO);
            if read == dec!(1_000_000) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("the customer reads {read}"))
            }
        })
        .retry(
            backon::ConstantBuilder::default()
                .with_delay(Duration::from_millis(300))
                .with_max_times(300),
        )
        .await
        .expect("one copy readable");
        self.usage
            .fetch_total_usage(&TENANT_ID, &customer, &scoped(key), period)
            .await
            .expect("scoped usage")
    }

    async fn bill_versions(
        &self,
        customer: CustomerId,
        month: NaiveDate,
        a_cents: i64,
        b_cents: i64,
    ) {
        let a = self
            .subscribe(customer, month, Some("key-A"))
            .await
            .expect("A");
        let b = self
            .subscribe(customer, month, Some("key-B"))
            .await
            .expect("B");
        // Last month's invoices are issued when it closes: on the 1st they would still be in grace.
        self.grace_hours(0).await;
        self.process_cycles().await;
        assert_eq!(self.invoiced(a, month).await, a_cents);
        assert_eq!(self.invoiced(b, month).await, b_cents);
    }
}

/// Routing (§3.6): an event's key, by its canonical property or its alias, routes it to the
/// subscription for that resource; an unknown key or none goes to the customer's subscription.
#[tokio::test]
async fn test_an_event_routes_by_its_key_or_alias_else_to_the_customer() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let last_month = month_start(Utc::now().date_naive()) - Months::new(1);
    let customer = stack.customer("Routing").await;
    let a = stack
        .subscribe(customer, last_month, Some("key-A"))
        .await
        .expect("A");
    let b = stack
        .subscribe(customer, last_month, None)
        .await
        .expect("B");
    let tokens = |api_key, model, tokens, day| Tokens {
        customer,
        at: at(last_month + Days::new(day), 10),
        api_key,
        model,
        tokens,
    };
    stack
        .ingest_keyed(
            vec![
                // standard 1M = €12.00 by the canonical key, lite 1M = €2.00 by the alias
                (tokens(Some("key-A"), "standard", 1_000_000, 2), API_KEY),
                (tokens(Some("key-A"), "lite", 1_000_000, 3), "key"),
                // an unknown key, standard 2M = €24.00, and no key, lite 10M = €20.00
                (tokens(Some("key-Z"), "standard", 2_000_000, 4), API_KEY),
                (tokens(None, "lite", 10_000_000, 5), API_KEY),
            ],
            range(last_month, last_month + Months::new(1)),
        )
        .await;
    stack.grace_hours(0).await;
    stack.process_cycles().await;
    assert_eq!(stack.invoiced(a, last_month).await, 1_400);
    assert_eq!(stack.invoiced(b, last_month).await, 4_400);
}

/// §3.5: a draft's resource can change, an active one's can't; another customer's resource is
/// refused; a quote for a resource converts into a subscription for it.
#[tokio::test]
async fn test_a_subscription_for_a_resource_is_scoped_to_its_customer() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let today = Utc::now().date_naive();
    let s1 = stack.resource(CUST_SPOTIFY_ID, "s1").await;
    let s2 = stack.resource(CUST_SPOTIFY_ID, "s2").await;
    let s3 = stack.resource(CUST_SPOTIFY_ID, "s3").await;
    let elsewhere = stack.resource(CUST_UBER_ID, "s1").await;

    let refused = stack
        .create(stack.request(CUST_SPOTIFY_ID, today, Some(elsewhere)))
        .await
        .expect_err("another customer's resource");
    assert!(matches!(
        refused.current_context(),
        StoreError::InvalidArgument(_)
    ));

    let mut pending = stack.request(CUST_SPOTIFY_ID, today, Some(s1));
    pending.subscription.activation_condition = SubscriptionActivationCondition::Manual;
    let draft = stack.create(pending).await.expect("a draft for s1");
    assert_eq!(draft.resource_id, Some(s1));
    let moved = stack
        .set_resource(draft.id, Some(s2))
        .await
        .expect("a draft's resource changes");
    assert_eq!(moved.resource_id, Some(s2));

    let active = stack
        .create(stack.request(CUST_SPOTIFY_ID, today, Some(s1)))
        .await
        .expect("an active subscription for s1");
    assert!(stack.set_resource(active.id, None).await.is_err());

    let quote_id = QuoteId::new();
    stack
        .meteroid
        .store
        .insert_quote_with_details(
            meteroid_store::domain::quotes::QuoteNew {
                id: quote_id,
                status: meteroid_store::domain::enums::QuoteStatusEnum::Draft,
                tenant_id: TENANT_ID,
                customer_id: CUST_SPOTIFY_ID,
                plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                currency: "EUR".to_owned(),
                quote_number: "Q-RESOURCE".to_owned(),
                trial_duration_days: None,
                trial_free: None,
                trial_plan_id: None,
                trial_end_action: Default::default(),
                billing_start_date: Some(today),
                billing_end_date: None,
                billing_day_anchor: None,
                activation_condition: SubscriptionActivationCondition::OnStart,
                valid_until: None,
                expires_at: None,
                internal_notes: None,
                cover_image: None,
                overview: None,
                terms_and_services: None,
                net_terms: 30,
                attachments: vec![],
                pdf_document_id: None,
                sharing_key: Some(Uuid::new_v4().to_string()),
                recipients: vec![],
                auto_advance_invoices: true,
                charge_automatically: false,
                invoice_memo: None,
                invoice_threshold: None,
                create_subscription_on_acceptance: false,
                payment_methods_config: None,
                custom_properties: serde_json::Value::Null,
                resource_id: Some(s3),
                plan_option: None,
                entitlements: vec![],
                owner_id: None,
                template_id: None,
                display_options: Default::default(),
                sections: vec![],
                signature_level: Default::default(),
                consent_text: None,
                countersigners: vec![],
                hubspot_deal_id: None,
                max_discount_percent: None,
                allow_questions: true,
                allow_change_requests: true,
                allow_decline: true,
            },
            vec![],
            vec![],
            vec![],
            vec![],
            Actor::System,
        )
        .await
        .expect("a quote for s3");
    stack
        .meteroid
        .store
        .accept_quote(Actor::System, quote_id, TENANT_ID)
        .await
        .expect("accepted");
    let converted = stack
        .meteroid
        .services
        .convert_quote_to_subscription(Actor::System, TENANT_ID, quote_id)
        .await
        .expect("converted");
    assert_eq!(converted.subscription.resource_id, Some(s3));
}

/// While a subscription for a resource prices a metric, its attribution and its type's keys are
/// fixed: a change would move usage over finalized periods. What each subscription bills stays to
/// the cent, and the preview reads where usage would go.
/// Rules: R-144
#[tokio::test]
async fn test_an_attribution_change_is_refused_while_a_subscription_for_a_resource_prices_it() {
    use meteroid_store::repositories::ResourcesInterface;
    use meteroid_store::repositories::resources::ResourceTypePatch;
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let today = Utc::now().date_naive();
    let last_month = month_start(today) - Months::new(1);
    let customer = stack.customer("Attribution change").await;
    let a = stack
        .subscribe(customer, last_month, Some("key-A"))
        .await
        .expect("A");
    let b = stack
        .subscribe(customer, last_month, None)
        .await
        .expect("B");
    let now = Utc::now() - chrono::Duration::minutes(1);
    stack
        .ingest(
            vec![
                // last month, standard 1M = €12.00; now, standard 2M = €24.00
                Tokens {
                    customer,
                    at: at(last_month + Days::new(2), 10),
                    api_key: Some("key-A"),
                    model: "standard",
                    tokens: 1_000_000,
                },
                Tokens {
                    customer,
                    at: now,
                    api_key: Some("key-A"),
                    model: "standard",
                    tokens: 2_000_000,
                },
            ],
            range(last_month, today + Days::new(1)),
        )
        .await;
    let type_id = keyed_type(
        &stack.meteroid.store,
        TENANT_ID,
        API_KEY,
        &[API_KEY, API_KEY_ALIAS],
    )
    .await
    .id;
    stack.grace_hours(0).await;
    stack.process_cycles().await;
    let billed = || async {
        (
            stack.invoiced(a, last_month).await,
            stack.invoiced(b, last_month).await,
            stack.previewed(a).await,
            stack.previewed(b).await,
        )
    };
    assert_eq!(billed().await, (1_200, 0, 2_400, 0));
    let refused_while_sold = |refused: error_stack::Report<StoreError>| {
        assert!(
            matches!(refused.current_context(), StoreError::InvalidArgument(m) if m.contains("prices this metric")),
            "{refused:?}"
        );
    };

    let to_customer = stack
        .meteroid
        .services
        .update_billable_metric(Actor::System, stack.metric.id, TENANT_ID, attribute(None))
        .await
        .expect_err("to the customer");
    refused_while_sold(to_customer);
    assert_eq!(billed().await, (1_200, 0, 2_400, 0), "nothing moved");

    let keys = stack
        .meteroid
        .store
        .update_resource_type(
            TENANT_ID,
            AliasOr::Id(type_id),
            ResourceTypePatch {
                name: None,
                plural_name: None,
                parent_type_id: None,
                keys: Some(vec![API_KEY_ALIAS.to_owned(), API_KEY.to_owned()]),
                attributes: None,
                archived: None,
            },
        )
        .await
        .expect_err("its keys reordered");
    assert!(
        matches!(keys.current_context(), StoreError::InvalidArgument(m) if m.contains("keys can't change")),
        "{keys:?}"
    );
    assert_eq!(billed().await, (1_200, 0, 2_400, 0), "nothing moved");

    // The other way: a metric attributed to the customer, priced by a subscription for key-B.
    let calls = stack
        .meteroid
        .store
        .insert_billable_metric(
            Actor::System,
            BillableMetricNew {
                name: "Calls".to_owned(),
                description: None,
                code: "halyard_calls".to_owned(),
                catalog_code: None,
                aggregation_type: BillingMetricAggregateEnum::Count,
                aggregation_key: None,
                unit_conversion_factor: None,
                unit_conversion_rounding: None,
                segmentation_matrix: None,
                filters: None,
                usage_group_key: None,
                usage_breakdown_key: None,
                attributed_to: None,
                unit_label: None,
                tenant_id: TENANT_ID,
                product_family_id: PRODUCT_FAMILY_ID,
                product_id: None,
            },
        )
        .await
        .expect("the calls metric");
    let key_b = stack.resource(customer, "key-B").await;
    let mut request = stack.request(customer, today, Some(key_b));
    if let Some(components) = request.price_components.as_mut() {
        components.extra_components = vec![metric_component(
            "Calls",
            calls.id,
            UsagePricingModel::PerUnit { rate: dec!(1) },
        )];
    }
    stack.create(request).await.expect("for key-B");
    let to_type = stack
        .meteroid
        .services
        .update_billable_metric(
            Actor::System,
            calls.id,
            TENANT_ID,
            attribute(Some(MetricAttribution {
                type_id,
                property: None,
            })),
        )
        .await
        .expect_err("to the type");
    refused_while_sold(to_type);
    assert_eq!(billed().await, (1_200, 0, 2_400, 0), "nothing moved");

    let current = stack
        .meteroid
        .services
        .preview_metric_attribution(
            TENANT_ID,
            stack.metric.id,
            Some(MetricAttribution {
                type_id,
                property: None,
            }),
        )
        .await
        .expect("preview");
    assert_eq!(current.resource_subscriptions, dec!(2_000_000));
    assert_eq!(current.not_billed, Decimal::ZERO);
    let customer_only = stack
        .meteroid
        .services
        .preview_metric_attribution(TENANT_ID, stack.metric.id, None)
        .await
        .expect("preview");
    assert_eq!(customer_only.resource_subscriptions, Decimal::ZERO);
    assert_eq!(customer_only.customer_subscriptions, dec!(2_000_000));
}

/// The attribution preview of a max metric: each subscription for a key bills the largest of its
/// key's events, the customer-level one the largest of the rest; never the whole's max less the
/// rest's. Its figures don't add up to the usage, and it says so; a sum's do.
#[tokio::test]
async fn test_the_attribution_preview_of_a_max_metric_reads_each_subscription_s_own_events() {
    let stack = Stack::start().await;
    let today = Utc::now().date_naive();
    let customer = stack.customer("Peaks").await;
    let key_a = stack.resource(customer, "key-A").await;
    let key_b = stack.resource(customer, "key-B").await;
    let type_id = keyed_type(
        &stack.meteroid.store,
        TENANT_ID,
        API_KEY,
        &[API_KEY, API_KEY_ALIAS],
    )
    .await
    .id;
    let attributed = Some(MetricAttribution {
        type_id,
        property: None,
    });
    let peak = stack
        .meteroid
        .store
        .insert_billable_metric(
            Actor::System,
            BillableMetricNew {
                name: "Peak tokens".to_owned(),
                description: None,
                code: CODE.to_owned(),
                catalog_code: Some("peak_tokens".to_owned()),
                aggregation_type: BillingMetricAggregateEnum::Max,
                aggregation_key: Some("tokens".to_owned()),
                unit_conversion_factor: None,
                unit_conversion_rounding: None,
                segmentation_matrix: None,
                filters: None,
                usage_group_key: None,
                usage_breakdown_key: None,
                attributed_to: attributed.clone(),
                unit_label: None,
                tenant_id: TENANT_ID,
                product_family_id: PRODUCT_FAMILY_ID,
                product_id: None,
            },
        )
        .await
        .expect("the peak metric");
    let start = today - Days::new(3);
    for resource_id in [Some(key_a), Some(key_b), None] {
        let mut request = stack.request(customer, start, resource_id);
        if let Some(components) = request.price_components.as_mut() {
            components.extra_components = vec![metric_component(
                "Peak",
                peak.id,
                UsagePricingModel::PerUnit { rate: dec!(1) },
            )];
        }
        stack
            .create(request)
            .await
            .expect("a subscription pricing the peak");
    }
    let now = Utc::now() - chrono::Duration::minutes(5);
    let tokens = |api_key, hours: i64, tokens| Tokens {
        customer,
        at: now - chrono::Duration::hours(hours),
        api_key,
        model: "standard",
        tokens,
    };
    stack
        .ingest(
            vec![
                tokens(Some("key-A"), 3, 5),
                tokens(Some("key-A"), 2, 3),
                tokens(Some("key-B"), 1, 7),
                tokens(None, 1, 4),
            ],
            range(start, today + Days::new(1)),
        )
        .await;

    let preview = |attributed_to| {
        stack
            .meteroid
            .services
            .preview_metric_attribution(TENANT_ID, peak.id, attributed_to)
    };
    let to_keys = preview(attributed.clone()).await.expect("the preview");
    assert!(!to_keys.additive);
    assert_eq!(
        (
            to_keys.resource_subscriptions,
            to_keys.customer_subscriptions,
            to_keys.not_billed
        ),
        (dec!(12), dec!(4), Decimal::ZERO),
        "key-A's 5 and key-B's 7 for the keys, the rest's 4 for the customer"
    );
    let to_customer = preview(None).await.expect("the preview");
    assert_eq!(
        (
            to_customer.resource_subscriptions,
            to_customer.customer_subscriptions,
            to_customer.not_billed
        ),
        (Decimal::ZERO, dec!(7), Decimal::ZERO)
    );
    let sum = stack
        .meteroid
        .services
        .preview_metric_attribution(TENANT_ID, stack.metric.id, attributed)
        .await
        .expect("the token sum's preview");
    assert!(sum.additive);
}

/// §4: a leftover usage filter (`api_key = key-A` on A) becomes a type, a resource and A's
/// `resource_id`, its metric attributed to the type; S19's totals are unchanged.
#[tokio::test]
async fn test_a_leftover_usage_filter_converts_with_billing_unchanged() {
    use diesel_async::SimpleAsyncConnection;
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let last_month = month_start(Utc::now().date_naive()) - Months::new(1);
    let customer = stack.customer("Leftover filter").await;
    stack
        .ingest(
            vec![
                // E1 key-A standard 5M = €60.00; E2 key-Z standard 2M = €24.00; E3 lite 10M = €20.00
                Tokens {
                    customer,
                    at: at(last_month + Days::new(2), 10),
                    api_key: Some("key-A"),
                    model: "standard",
                    tokens: 5_000_000,
                },
                Tokens {
                    customer,
                    at: at(last_month + Days::new(3), 10),
                    api_key: Some("key-Z"),
                    model: "standard",
                    tokens: 2_000_000,
                },
                Tokens {
                    customer,
                    at: at(last_month + Days::new(4), 10),
                    api_key: None,
                    model: "lite",
                    tokens: 10_000_000,
                },
            ],
            range(last_month, last_month + Months::new(1)),
        )
        .await;
    let a = stack
        .subscribe(customer, last_month, None)
        .await
        .expect("A");
    let b = stack
        .subscribe(customer, last_month, None)
        .await
        .expect("B");

    let migration =
        include_str!("../../migrations/diesel/2026-11-26-011852_entitlements_v2_billing/up.sql");
    let conversion = &migration[migration
        .find("-- Leftover usage filters become resources:")
        .expect("the conversion")
        ..migration
            .find("-- From when a row a late change wrote bills its metric")
            .expect("the floors")];
    {
        let mut conn = stack.meteroid.store.get_conn().await.unwrap();
        conn.batch_execute(&format!(
            "{FILTER_CHECK_DROPPED};
             UPDATE subscription SET usage_filter_property = 'api_key', \
             usage_filter_value = 'key-A' WHERE id = '{a}';
             INSERT INTO subscription_usage_filter
                 (id, tenant_id, subscription_id, property, value, effective_from)
             SELECT gen_random_uuid(), tenant_id, id, 'api_key', 'key-A', start_date::timestamp
             FROM subscription WHERE id = '{a}'",
            a = a.as_uuid()
        ))
        .await
        .expect("a leftover usage filter");
        conn.batch_execute(conversion)
            .await
            .expect("the conversion");
    }
    let resource_id = stack
        .subscription_row(a)
        .await
        .resource_id
        .expect("A's resource");
    let resource_type = keyed_type(
        &stack.meteroid.store,
        TENANT_ID,
        API_KEY,
        &[API_KEY, API_KEY_ALIAS],
    )
    .await;
    let metric = stack
        .meteroid
        .store
        .find_billable_metric_by_id(stack.metric.id, TENANT_ID)
        .await
        .expect("the metric");
    assert_eq!(metric.attributed_to_type_id, Some(resource_type.id));
    assert_eq!(stack.resource(customer, "key-A").await, resource_id);

    stack.grace_hours(0).await;
    stack.process_cycles().await;
    assert_eq!(stack.invoiced(a, last_month).await, 6_000);
    assert_eq!(stack.invoiced(b, last_month).await, 4_400);
}

/// A subscription priced on the token-free seeded metric, for `resource_id` when set.
fn bandwidth_request(
    customer_id: CustomerId,
    start: NaiveDate,
    activation: SubscriptionActivationCondition,
    resource_id: Option<ResourceId>,
) -> CreateSubscription {
    let mut request = crate::harness::subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start)
        .activation(activation)
        .no_trial()
        .request();
    request.subscription.resource_id = resource_id;
    request.price_components = Some(CreateSubscriptionComponents {
        parameterized_components: vec![],
        overridden_components: vec![],
        extra_components: vec![metric_component(
            "Bandwidth",
            METRIC_BANDWIDTH,
            UsagePricingModel::PerUnit { rate: dec!(1) },
        )],
        remove_components: vec![],
    });
    request
}

/// A resource subscription activated on checkout claims nothing while it waits: the
/// customer-level subscription invoices its key's usage meanwhile. Once paid, it bills its key
/// from the day of its activation, though its first period started earlier: once each.
#[tokio::test]
async fn test_a_checkout_subscription_claims_its_resource_once_activated() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    crate::data::payment::run_mock_payment_provider_seed(&stack.meteroid.store.pool, false).await;
    crate::data::payment::run_customer_payment_methods_seed(&stack.meteroid.store.pool).await;
    let customer = CUST_UBER_ID;
    let today = Utc::now().date_naive();
    let customer_start = today - Days::new(40);
    let start = today - Days::new(25);
    // key-A standard: 1M = €12.00 while it waits, 3M = €36.00 once activated.
    let tokens = |at, tokens| Tokens {
        customer,
        at,
        api_key: Some("key-A"),
        model: "standard",
        tokens,
    };
    stack
        .ingest(
            vec![tokens(at(start, 1), 1_000_000)],
            range(customer_start, today),
        )
        .await;

    let key_a = stack.resource(customer, "key-A").await;
    let mut request = stack.request(customer, start, Some(key_a));
    request.subscription.activation_condition = SubscriptionActivationCondition::OnCheckout;
    let pending = stack
        .create(request)
        .await
        .expect("the subscription for key-A")
        .id;
    let customer_level = stack
        .subscribe(customer, customer_start, None)
        .await
        .expect("the customer-level subscription");
    stack.grace_hours(0).await;
    stack.process_cycles().await;
    assert_eq!(
        stack.subscription_row(pending).await.status,
        SubscriptionStatusEnum::PendingActivation
    );
    assert!(
        stack
            .subscription_row(customer_level)
            .await
            .current_period_start
            > start,
        "the customer-level subscription invoiced a period while it waited"
    );
    assert_eq!(stack.invoiced(customer_level, customer_start).await, 1_200);

    stack
        .meteroid
        .services
        .complete_subscription_checkout_tx(
            &mut stack.meteroid.store.get_conn().await.unwrap(),
            TENANT_ID,
            pending,
            CUST_UBER_PAYMENT_METHOD_ID,
            0,
            "EUR".to_owned(),
            None,
        )
        .await
        .expect("the checkout, usage only");
    stack
        .ingest(
            vec![tokens(Utc::now(), 3_000_000)],
            range(today, today + Days::new(1)),
        )
        .await;
    let activated = stack.subscription_row(pending).await;
    assert_eq!(activated.status, SubscriptionStatusEnum::Active);
    assert_eq!(
        activated.current_period_start, start,
        "its first period starts at its start date"
    );
    assert_eq!(
        stack.previewed(pending).await,
        3_600,
        "the usage invoiced while it waited is not billed again"
    );
    assert_eq!(stack.previewed(customer_level).await, 0);
    assert_eq!(stack.invoiced(customer_level, customer_start).await, 1_200);
}

fn standard(customer: CustomerId, at: DateTime<Utc>, key: &'static str, tokens: u64) -> Tokens {
    Tokens {
        customer,
        at,
        api_key: Some(key),
        model: "standard",
        tokens,
    }
}

/// A subscription for a resource activated by hand days after its start claims its usage from the
/// day of its activation: before it, the customer's other subscriptions bill it, and its first
/// period still starts earlier.
#[tokio::test]
async fn test_a_late_activation_counts_usage_from_then() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let customer = stack.customer("Activated late").await;
    let today = Utc::now().date_naive();
    let start = today - Days::new(10);
    // Standard tokens: 1M = €12.00 before the activation, 3M = €36.00 after it.
    stack
        .ingest(
            vec![standard(customer, at(start, 1), "key-A", 1_000_000)],
            range(start, today),
        )
        .await;
    let key_a = stack.resource(customer, "key-A").await;
    let mut request = stack.request(customer, start, Some(key_a));
    request.subscription.activation_condition = SubscriptionActivationCondition::Manual;
    let late = stack
        .create(request)
        .await
        .expect("a manual subscription")
        .id;
    stack
        .meteroid
        .services
        .activate_subscription_manual(TENANT_ID, late)
        .await
        .expect("the activation");
    stack
        .ingest(
            vec![standard(customer, Utc::now(), "key-A", 3_000_000)],
            range(today, today + Days::new(1)),
        )
        .await;
    assert_eq!(
        stack.subscription_row(late).await.current_period_start,
        start
    );
    assert_eq!(stack.previewed(late).await, 3_600);
}

/// An event at a claim's very start is the claiming subscription's, one at its very end the
/// customer-level subscription's: each on one invoice.
#[tokio::test]
async fn test_an_event_at_a_claim_boundary_is_billed_once() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let customer = stack.customer("Claim boundaries").await;
    let today = Utc::now().date_naive();
    let customer_start = today - Days::new(20);
    let start = today - Days::new(10);
    // key-A standard: 1M = €12.00 at the claim's start, 3M = €36.00 at its end.
    stack
        .ingest(
            vec![
                standard(customer, at(start, 0), "key-A", 1_000_000),
                standard(customer, at(today, 0), "key-A", 3_000_000),
            ],
            range(customer_start, today + Days::new(1)),
        )
        .await;
    let customer_level = stack
        .subscribe(customer, customer_start, None)
        .await
        .expect("the customer-level subscription");
    let for_key = stack
        .subscribe(customer, start, Some("key-A"))
        .await
        .expect("the subscription for key-A");
    stack.grace_hours(0).await;
    stack
        .meteroid
        .services
        .cancel_subscription(
            Actor::System,
            for_key,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(today),
        )
        .await
        .expect("ending today");
    stack.process_cycles().await;
    assert_eq!(
        stack.subscription_row(for_key).await.status,
        SubscriptionStatusEnum::Cancelled
    );
    assert_eq!(stack.invoiced(for_key, start).await, 1_200);
    assert_eq!(stack.previewed(customer_level).await, 3_600);
}

/// An import completed before today claims its key up to its end date, not to the start of its
/// last period: the customer-level subscription bills the key from that end only.
#[tokio::test]
async fn test_a_completed_import_claims_up_to_its_end_date() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let customer = stack.customer("Completed import").await;
    let today = Utc::now().date_naive();
    let start = today - Days::new(50);
    let end = today - Days::new(5);
    let customer_start = today - Days::new(15);
    // key-A standard: 1M = €12.00 in the import's last period, 3M = €36.00 after its end.
    stack
        .ingest(
            vec![
                standard(customer, at(today - Days::new(10), 1), "key-A", 1_000_000),
                standard(customer, at(today - Days::new(3), 1), "key-A", 3_000_000),
            ],
            range(start, today + Days::new(1)),
        )
        .await;
    let key_a = stack.resource(customer, "key-A").await;
    let mut request = stack.request(customer, start, Some(key_a));
    request.subscription.end_date = Some(end);
    request.subscription.skip_past_invoices = true;
    let imported = stack.create(request).await.expect("the import").id;
    let row = stack.subscription_row(imported).await;
    assert_eq!(row.status, SubscriptionStatusEnum::Completed);
    assert!(row.current_period_start < today - Days::new(10));
    let customer_level = stack
        .subscribe(customer, customer_start, None)
        .await
        .expect("the customer-level subscription");
    assert_eq!(stack.previewed(customer_level).await, 3_600);
}

/// A customer-level subscription that has not started has invoiced nothing: a subscription for a
/// resource may start before it.
#[rstest]
#[tokio::test]
async fn test_a_future_subscription_bounds_no_start(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let today = Utc::now().date_naive();
    let key_a = api_key_resource(env.store(), TENANT_ID, CUST_UBER_ID, "key-A").await;
    env.services()
        .insert_subscription(
            Actor::System,
            bandwidth_request(
                CUST_UBER_ID,
                today + Days::new(10),
                SubscriptionActivationCondition::OnStart,
                None,
            ),
            TENANT_ID,
        )
        .await
        .expect("a customer-level subscription starting later");
    env.services()
        .insert_subscription(
            Actor::System,
            bandwidth_request(
                CUST_UBER_ID,
                today,
                SubscriptionActivationCondition::OnStart,
                Some(key_a),
            ),
            TENANT_ID,
        )
        .await
        .expect("a subscription for key-A starting today");
}

impl Stack {
    async fn cancel(
        &self,
        subscription_id: SubscriptionId,
        actor: Actor,
        at: CancellationEffectiveAt,
    ) {
        self.meteroid
            .services
            .cancel_subscription(actor, subscription_id, TENANT_ID, None, at)
            .await
            .expect("the cancellation");
    }

    async fn usage_floor(&self, row_id: Uuid) -> Option<DateTime<Utc>> {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::subscription_component::dsl as c;
        c::subscription_component
            .filter(c::id.eq(row_id))
            .select(c::usage_from)
            .first(&mut self.meteroid.store.get_conn().await.unwrap())
            .await
            .unwrap()
    }
}

/// An amendment adding the key's metric to its subscription, dated before what the customer-level
/// subscription has since finalized of the key's usage: its row keeps the change's date for its
/// fees, and bills the metric only past that invoiced ground, so each event is billed once.
/// Rules: R-143
#[tokio::test]
async fn test_a_late_change_adding_an_attributed_metric_bills_once() {
    let stack = Stack::start().await;
    let customer = stack.customer("Late amendment").await;
    let today = scenario_day();
    let customer_start = today - Months::new(1);
    let start = today - Days::new(20);
    // key-A standard: 1M = €12.00 after the amendment's date, 3M = €36.00 today.
    stack
        .ingest(
            vec![
                standard(customer, at(start + Days::new(3), 1), "key-A", 1_000_000),
                standard(
                    customer,
                    Utc::now() - chrono::Duration::minutes(1),
                    "key-A",
                    3_000_000,
                ),
            ],
            range(customer_start, today + Days::new(2)),
        )
        .await;
    stack.grace_hours(0).await;
    let customer_level = stack
        .subscribe(customer, customer_start, None)
        .await
        .unwrap();
    let key_a = stack.resource(customer, "key-A").await;
    let mut request = stack.request(customer, start, Some(key_a));
    let tokens = request
        .price_components
        .as_mut()
        .map(|components| components.extra_components.remove(0))
        .expect("the token component");
    let for_key = stack.create(request).await.unwrap().id;
    stack.process_cycles().await;
    assert_eq!(
        stack.invoiced(customer_level, customer_start).await,
        1_200,
        "nothing claimed key-A"
    );

    stack
        .meteroid
        .services
        .apply_amendment_immediate_at(
            Actor::System,
            for_key,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![AddExtraComponent {
                        name: tokens.name,
                        product: tokens.product_ref,
                        price_entry: tokens.price_entry,
                        fee_structure: tokens.fee_structure,
                    }],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            start + Days::new(2),
        )
        .await
        .expect("the amendment, dated after key-A's first event");
    stack.process_cycles().await;
    let added = stack
        .meteroid
        .store
        .get_subscription_details(TENANT_ID, for_key)
        .await
        .unwrap()
        .price_components
        .into_iter()
        .find(|component| component.fee.metric_id() == Some(stack.metric.id))
        .expect("the added component");
    assert_eq!(
        added.effective_from,
        start + Days::new(2),
        "its fees keep the change's date"
    );
    assert_eq!(
        stack.previewed(for_key).await,
        3_600,
        "the key's event the customer-level subscription invoiced is not billed again"
    );
    assert_eq!(stack.previewed(customer_level).await, 0);
    assert_eq!(stack.invoiced(customer_level, customer_start).await, 1_200);
    assert_eq!(
        stack.usage_floor(added.id.as_uuid()).await,
        Some(at(today, 0)),
        "its usage counts from the invoiced ground"
    );
}

/// The key's subscription and the customer-level one, its cancellation recorded for `ends_on`,
/// the customer-level subscription's past period finalized with usage.
async fn cancelled_after_invoiced_ground(
    stack: &Stack,
    name: &str,
    ends_on: NaiveDate,
) -> SubscriptionId {
    let customer = stack.customer(name).await;
    let today = scenario_day();
    let customer_start = today - Months::new(1);
    stack
        .ingest(
            vec![Tokens {
                customer,
                at: at(today - Days::new(10), 1),
                api_key: None,
                model: "standard",
                tokens: 1_000_000,
            }],
            range(customer_start, today + Days::new(2)),
        )
        .await;
    stack.grace_hours(0).await;
    stack
        .subscribe(customer, customer_start, None)
        .await
        .expect("the customer-level subscription");
    let for_key = stack
        .subscribe(customer, today - Days::new(20), Some("key-A"))
        .await
        .expect("the key's subscription");
    stack
        .cancel(
            for_key,
            Actor::System,
            CancellationEffectiveAt::Date(ends_on),
        )
        .await;
    for_key
}

/// Withdrawing a cancellation once the customer-level subscription finalized the key's usage
/// from the cancellation on: the key's subscription would bill it again, so it is refused (a
/// cancellation still ahead of that ground is withdrawn as before).
/// Rules: R-143
#[tokio::test]
async fn test_withdrawing_a_cancellation_over_invoiced_ground_is_refused() {
    let stack = Stack::start().await;
    let today = scenario_day();
    let for_key = cancelled_after_invoiced_ground(&stack, "Withdrawn", today + Days::new(5)).await;
    stack.process_cycles().await;
    {
        use diesel_async::RunQueryDsl;
        // The controller has not applied it yet: its instant passed, its status is still live.
        diesel::sql_query(
            "UPDATE subscription_ending SET at = $2 WHERE subscription_id = $1 AND kind = 'CANCEL'",
        )
        .bind::<diesel::sql_types::Uuid, _>(for_key.as_uuid())
        .bind::<diesel::sql_types::Timestamp, _>((today - Days::new(15)).and_time(NaiveTime::MIN))
        .execute(&mut stack.meteroid.store.get_conn().await.unwrap())
        .await
        .unwrap();
    }
    let withdraw = || async {
        let event = stack
            .meteroid
            .store
            .get_subscription_details(TENANT_ID, for_key)
            .await
            .unwrap()
            .pending_events
            .into_iter()
            .next()
            .expect("the pending cancellation");
        stack
            .meteroid
            .services
            .cancel_scheduled_event(Actor::System, event.id, for_key, TENANT_ID)
            .await
    };
    let refused = withdraw()
        .await
        .expect_err("withdrawn over invoiced ground");
    assert!(
        matches!(refused.current_context(), StoreError::InvalidArgument(m) if m.contains("invoiced")),
        "{refused:?}"
    );
    assert_eq!(
        cancelled_events(&stack.meteroid.store, for_key).await,
        1,
        "a refused withdrawal keeps the cancellation's event"
    );

    let ahead =
        cancelled_after_invoiced_ground(&stack, "Withdrawn ahead", today + Days::new(5)).await;
    stack.process_cycles().await;
    let event = stack
        .meteroid
        .store
        .get_subscription_details(TENANT_ID, ahead)
        .await
        .unwrap()
        .pending_events
        .into_iter()
        .next()
        .expect("the pending cancellation");
    stack
        .meteroid
        .services
        .cancel_scheduled_event(Actor::System, event.id, ahead, TENANT_ID)
        .await
        .expect("a cancellation still ahead is withdrawn");
    assert_eq!(
        cancelled_events(&stack.meteroid.store, ahead).await,
        0,
        "its `Cancelled` event goes with it (R-141)"
    );
}

/// The subscription's `Cancelled` events: one is recorded with its cancellation, before it applies.
async fn cancelled_events(store: &meteroid_store::Store, subscription_id: SubscriptionId) -> i64 {
    use diesel_async::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Count {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        n: i64,
    }
    diesel::sql_query(
        "SELECT count(*) AS n FROM subscription_event
         WHERE subscription_id = $1 AND event_type = 'CANCELLED'",
    )
    .bind::<diesel::sql_types::Uuid, _>(subscription_id.as_uuid())
    .get_result::<Count>(&mut store.get_conn().await.unwrap())
    .await
    .unwrap()
    .n
}

/// A cancellation of the key's subscription dated before what the customer-level subscription
/// finalized of the key's usage would hand that usage back to it, billed nowhere: refused.
/// Rules: R-143
#[tokio::test]
async fn test_a_cancellation_dated_over_invoiced_ground_is_refused() {
    let stack = Stack::start().await;
    let today = scenario_day();
    let for_key = cancelled_after_invoiced_ground(&stack, "Backdated", today + Days::new(5)).await;
    stack.process_cycles().await;
    let refused = stack
        .meteroid
        .services
        .cancel_subscription(
            Actor::System,
            for_key,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(today - Days::new(15)),
        )
        .await
        .expect_err("dated over invoiced ground");
    assert!(
        matches!(refused.current_context(), StoreError::InvalidArgument(m) if m.contains("invoiced")),
        "{refused:?}"
    );
}

async fn sql(env: &TestEnv, query: &str, subscription_id: SubscriptionId) {
    use diesel_async::RunQueryDsl;
    diesel::sql_query(query)
        .bind::<diesel::sql_types::Uuid, _>(subscription_id.as_uuid())
        .execute(&mut env.conn().await)
        .await
        .unwrap();
}

fn quote_for(
    customer_id: CustomerId,
    resource_id: ResourceId,
    start: NaiveDate,
) -> meteroid_store::domain::quotes::QuoteNew {
    meteroid_store::domain::quotes::QuoteNew {
        id: QuoteId::new(),
        status: meteroid_store::domain::enums::QuoteStatusEnum::Draft,
        tenant_id: TENANT_ID,
        customer_id,
        plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
        currency: "EUR".to_owned(),
        quote_number: format!("Q-{}", Uuid::new_v4()),
        trial_duration_days: None,
        trial_free: None,
        trial_plan_id: None,
        trial_end_action: Default::default(),
        billing_start_date: Some(start),
        billing_end_date: None,
        billing_day_anchor: None,
        activation_condition: SubscriptionActivationCondition::OnStart,
        valid_until: None,
        expires_at: None,
        internal_notes: None,
        cover_image: None,
        overview: None,
        terms_and_services: None,
        net_terms: 30,
        attachments: vec![],
        pdf_document_id: None,
        sharing_key: Some(Uuid::new_v4().to_string()),
        recipients: vec![],
        auto_advance_invoices: true,
        charge_automatically: false,
        invoice_memo: None,
        invoice_threshold: None,
        create_subscription_on_acceptance: false,
        payment_methods_config: None,
        custom_properties: serde_json::Value::Null,
        resource_id: Some(resource_id),
        plan_option: None,
        entitlements: vec![],
        owner_id: None,
        template_id: None,
        display_options: Default::default(),
        sections: vec![],
        signature_level: Default::default(),
        consent_text: None,
        countersigners: vec![],
        hubspot_deal_id: None,
        max_discount_percent: None,
        allow_questions: true,
        allow_change_requests: true,
        allow_decline: true,
    }
}

/// A self-serve checkout of the leetcode plan with bandwidth for `resource_id`, from `start`.
fn self_serve_checkout(
    resource_id: ResourceId,
    start: NaiveDate,
) -> meteroid_store::domain::CreateCheckoutSession {
    meteroid_store::domain::CreateCheckoutSession {
        tenant_id: TENANT_ID,
        customer_id: CUST_UBER_ID,
        plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
        option: None,
        billing_start_date: Some(start),
        billing_day_anchor: Some(start.day() as i16),
        net_terms: None,
        offer: Default::default(),
        trial: Default::default(),
        end_date: None,
        auto_advance_invoices: true,
        charge_automatically: true,
        invoice_memo: None,
        invoice_threshold: None,
        purchase_order: None,
        payment_methods_config: None,
        components: bandwidth_request(
            CUST_UBER_ID,
            start,
            SubscriptionActivationCondition::OnCheckout,
            None,
        )
        .price_components,
        add_ons: None,
        coupon_code: None,
        coupon_ids: vec![],
        expires_in_hours: Some(24),
        metadata: None,
        checkout_type: meteroid_store::domain::checkout_sessions::CheckoutType::SelfServe,
        subscription_id: None,
        change_date: None,
        success_url: None,
        cancel_url: None,
        resource_id: Some(resource_id),
    }
}

fn already_sold(error: &error_stack::Report<StoreError>) -> bool {
    matches!(error.current_context(), StoreError::InvalidArgument(m) if m.contains("already sold"))
}

/// One resource is sold by one subscription at a time: a second one over any of its period is
/// refused, whether created, accepted or converted from a quote, or checked out; after its end it
/// is not.
/// Rules: R-145
#[rstest]
#[tokio::test]
async fn test_a_resource_is_sold_by_one_subscription_at_a_time(#[future] test_env: TestEnv) {
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
    let env = test_env.await;
    let today = Utc::now().date_naive();
    let key_a = api_key_resource(env.store(), TENANT_ID, CUST_UBER_ID, "key-A").await;
    let until = |start, end: Option<NaiveDate>| {
        let mut request = bandwidth_request(
            CUST_UBER_ID,
            start,
            SubscriptionActivationCondition::OnStart,
            Some(key_a),
        );
        request.subscription.end_date = end;
        request
    };
    let insert = |request| {
        env.services()
            .insert_subscription(Actor::System, request, TENANT_ID)
    };

    insert(until(today - Days::new(10), Some(today + Days::new(5))))
        .await
        .expect("the first subscription for key-A");
    let refused = insert(until(today, None))
        .await
        .expect_err("sold over its period");
    assert!(already_sold(&refused), "{refused:?}");
    let refused = insert(until(today - Days::new(20), Some(today - Days::new(9))))
        .await
        .expect_err("ending inside its period");
    assert!(already_sold(&refused), "{refused:?}");

    let quote = quote_for(CUST_UBER_ID, key_a, today);
    let quote_id = quote.id;
    env.store()
        .insert_quote_with_details(quote, vec![], vec![], vec![], vec![], Actor::System)
        .await
        .expect("a quote for key-A");
    let refused = env
        .store()
        .accept_quote(Actor::System, quote_id, TENANT_ID)
        .await
        .expect_err("accepted over its period");
    assert!(already_sold(&refused), "{refused:?}");
    {
        use diesel_async::SimpleAsyncConnection;
        env.conn()
            .await
            .batch_execute(&format!(
                "UPDATE quote SET status = 'ACCEPTED' WHERE id = '{}'",
                quote_id.as_uuid()
            ))
            .await
            .unwrap();
    }
    let refused = env
        .services()
        .convert_quote_to_subscription(Actor::System, TENANT_ID, quote_id)
        .await
        .expect_err("converted over its period");
    assert!(already_sold(&refused), "{refused:?}");

    let refused = env
        .store()
        .create_checkout_session(self_serve_checkout(key_a, today))
        .await
        .expect_err("checked out over its period");
    assert!(already_sold(&refused), "{refused:?}");

    insert(until(today + Days::new(5), None))
        .await
        .expect("from its end on");
}

/// A subscription for a resource ends on a date in its second period, and another sells the
/// resource from then: lengthening the first one's term, or clearing its end date, would sell the
/// resource twice, so both are refused and the end date stays. Moved earlier, the first one's
/// claims move: its customer is marked for recompute.
/// Rules: R-145, R-140
#[rstest]
#[tokio::test]
async fn test_an_end_date_is_not_moved_over_a_later_sale_of_its_resource(
    #[future] test_env: TestEnv,
) {
    use diesel_async::RunQueryDsl;
    let env = test_env.await;
    let today = Utc::now().date_naive();
    let key_a = api_key_resource(env.store(), TENANT_ID, CUST_UBER_ID, "key-A").await;
    let ends = today + Days::new(45);
    let until = |start, end: Option<NaiveDate>| {
        let mut request = bandwidth_request(
            CUST_UBER_ID,
            start,
            SubscriptionActivationCondition::OnStart,
            Some(key_a),
        );
        request.subscription.end_date = end;
        request
    };
    let first = env
        .services()
        .insert_subscription(
            Actor::System,
            until(today - Days::new(10), Some(ends)),
            TENANT_ID,
        )
        .await
        .expect("the first subscription for key-A")
        .id;
    env.services()
        .insert_subscription(Actor::System, until(ends, None), TENANT_ID)
        .await
        .expect("key-A sold again from its end");

    for (asked, why) in [
        (Some(ends + Days::new(10)), "lengthened"),
        (None, "cleared"),
    ] {
        let refused = env
            .services()
            .change_subscription_end_date(Actor::System, TENANT_ID, first, asked)
            .await
            .expect_err(why);
        assert!(already_sold(&refused), "{why}: {refused:?}");
    }
    let end_date = || {
        let env = &env;
        async move {
            SubscriptionRow::get_subscription_by_id(&mut env.conn().await, &TENANT_ID, first)
                .await
                .unwrap()
                .subscription
                .end_date
        }
    };
    assert_eq!(end_date().await, Some(ends));

    sql(
        &env,
        "DELETE FROM credit_recompute_mark WHERE $1 = $1",
        first,
    )
    .await;
    env.services()
        .change_subscription_end_date(Actor::System, TENANT_ID, first, Some(ends - Days::new(5)))
        .await
        .expect("moved earlier, in a period not billed yet");
    assert_eq!(end_date().await, Some(ends - Days::new(5)));
    #[derive(diesel::QueryableByName)]
    struct Count {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        n: i64,
    }
    let marked: Count = diesel::sql_query(
        "SELECT count(*) AS n FROM credit_recompute_mark WHERE tenant_id = $1 AND customer_id = $2",
    )
    .bind::<diesel::sql_types::Uuid, _>(TENANT_ID.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(CUST_UBER_ID.as_uuid())
    .get_result(&mut env.conn().await)
    .await
    .unwrap();
    assert_eq!(marked.n, 1, "the moved claims mark their customer");
}

/// A subscription for a resource that ended (cancelled) sold it up to its end: another one for the
/// resource backdated over that is refused as the resource's fault, naming when it may start;
/// from the end on, it is accepted.
/// Rules: R-145
#[rstest]
#[tokio::test]
async fn test_a_resource_is_not_sold_again_over_an_ended_subscription(#[future] test_env: TestEnv) {
    use meteroid_store::repositories::resources::ResourceAtFault;
    let env = test_env.await;
    let today = Utc::now().date_naive();
    let key_a = api_key_resource(env.store(), TENANT_ID, CUST_UBER_ID, "key-A").await;
    let request = |start| {
        bandwidth_request(
            CUST_UBER_ID,
            start,
            SubscriptionActivationCondition::OnStart,
            Some(key_a),
        )
    };
    let first = env
        .services()
        .insert_subscription(Actor::System, request(today - Days::new(20)), TENANT_ID)
        .await
        .expect("the first subscription for key-A")
        .id;
    env.services()
        .cancel_subscription(
            Actor::System,
            first,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(today),
        )
        .await
        .expect("cancelled from today");
    env.process_cycles().await;
    assert_eq!(
        env.get_subscription(first).await.status,
        SubscriptionStatusEnum::Cancelled
    );

    for start in [today - Days::new(10), today - Days::new(30)] {
        let refused = env
            .services()
            .insert_subscription(Actor::System, request(start), TENANT_ID)
            .await
            .expect_err("over what the ended one sold");
        assert!(ResourceAtFault::of(&refused), "{refused:?}");
        assert!(
            matches!(refused.current_context(), StoreError::InvalidArgument(m) if m.contains("was sold by subscription") && m.contains("can start from then")),
            "{refused:?}"
        );
    }
    env.services()
        .insert_subscription(Actor::System, request(today), TENANT_ID)
        .await
        .expect("from its end on");
}

/// Returns once another transaction of the test database waits on a lock, a minute at most: the
/// caller holds what it waits for until this returns.
async fn until_another_waits(env: &TestEnv) {
    use diesel_async::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Waiting {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        n: i64,
    }
    let mut conn = env.conn().await;
    for _ in 0..3_000 {
        let waiting: Waiting = diesel::sql_query(
            "SELECT count(*) AS n FROM pg_stat_activity \
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .get_result(&mut conn)
        .await
        .unwrap();
        if waiting.n > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("nothing waited on the locks held");
}

/// A subscription for the key, backdated over the customer-level renewal draft, is being created
/// when that draft is finalized: the finalization waits for the creation to commit (both take the
/// customer's lock before reading what the other writes), then settles the period.
/// Rules: R-143
#[rstest]
#[tokio::test]
async fn test_a_finalization_waits_for_a_resource_subscription_in_flight(
    #[future] test_env: TestEnv,
) {
    use scoped_futures::ScopedFutureExt;
    let env = test_env.await;
    let today = Utc::now().date_naive();
    let key_a = api_key_resource(env.store(), TENANT_ID, CUST_UBER_ID, "key-A").await;
    env.set_consolidate_and_grace(false, 48).await;
    let customer_level = env
        .services()
        .insert_subscription(
            Actor::System,
            bandwidth_request(
                CUST_UBER_ID,
                today - Months::new(1),
                SubscriptionActivationCondition::OnStart,
                None,
            ),
            TENANT_ID,
        )
        .await
        .unwrap()
        .id;
    env.process_cycles().await;
    let draft = env
        .get_invoices(customer_level)
        .await
        .into_iter()
        .find(|invoice| {
            invoice.status == InvoiceStatusEnum::Draft
                && invoice.invoice_type == InvoiceType::Recurring
        })
        .expect("the renewal draft, in its grace period");

    let locked = tokio::sync::Notify::new();
    let creating = env.store().transaction(|conn| {
        let (env, locked) = (&env, &locked);
        async move {
            let created = env
                .services()
                .insert_subscription_tx(
                    Actor::System,
                    conn,
                    bandwidth_request(
                        CUST_UBER_ID,
                        today - Days::new(10),
                        SubscriptionActivationCondition::OnStart,
                        Some(key_a),
                    ),
                    TENANT_ID,
                )
                .await?;
            locked.notify_one();
            until_another_waits(env).await;
            Ok(created.id)
        }
        .scope_boxed()
    });
    let finalizing = async {
        locked.notified().await;
        env.services()
            .finalize_invoice(Actor::System, draft.id, TENANT_ID)
            .await
    };
    let (created, finalized) = tokio::time::timeout(Duration::from_secs(120), async {
        tokio::join!(creating, finalizing)
    })
    .await
    .expect("neither stalls");
    let for_key = created.expect("the key's subscription");
    let finalized = finalized.expect("finalized once the creation committed");
    assert_eq!(finalized.invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(env.get_subscription(for_key).await.resource_id, Some(key_a));
}

/// The api_key type's keys change while a subscription for one of its resources is being created:
/// the change waits for the creation to commit, then sees it live and is refused.
/// Rules: R-144
#[rstest]
#[tokio::test]
async fn test_a_key_change_waits_for_a_sale_in_flight(#[future] test_env: TestEnv) {
    use meteroid_store::repositories::resources::ResourceTypePatch;
    use scoped_futures::ScopedFutureExt;
    let env = test_env.await;
    let today = Utc::now().date_naive();
    let key_a = api_key_resource(env.store(), TENANT_ID, CUST_UBER_ID, "key-A").await;

    let sold = tokio::sync::Notify::new();
    let selling = env.store().transaction(|conn| {
        let (env, sold) = (&env, &sold);
        async move {
            let created = env
                .services()
                .insert_subscription_tx(
                    Actor::System,
                    conn,
                    bandwidth_request(
                        CUST_UBER_ID,
                        today,
                        SubscriptionActivationCondition::OnStart,
                        Some(key_a),
                    ),
                    TENANT_ID,
                )
                .await?;
            sold.notify_one();
            until_another_waits(env).await;
            Ok(created.id)
        }
        .scope_boxed()
    });
    let changing = async {
        sold.notified().await;
        env.store()
            .update_resource_type(
                TENANT_ID,
                AliasOr::Alias(API_KEY.to_owned()),
                ResourceTypePatch {
                    name: None,
                    plural_name: None,
                    parent_type_id: None,
                    keys: Some(vec![API_KEY.to_owned()]),
                    attributes: None,
                    archived: None,
                },
            )
            .await
    };
    let (sold_by, changed) = tokio::time::timeout(Duration::from_secs(120), async {
        tokio::join!(selling, changing)
    })
    .await
    .expect("neither stalls");
    sold_by.expect("the sale");
    let refused = changed.expect_err("a subscription for one of its resources is live");
    assert!(
        matches!(refused.current_context(), StoreError::InvalidArgument(m) if m.contains("keys can't change")),
        "{refused:?}"
    );
}

/// The attribution preview counts every subscription draft of the customers whose usage a change
/// moves, as the change marks them for refresh: a recurring draft and an adjustment alike.
#[rstest]
#[tokio::test]
async fn test_the_attribution_preview_counts_every_draft_the_change_refreshes(
    #[future] test_env: TestEnv,
) {
    use diesel_async::RunQueryDsl;
    let env = test_env.await;
    let today = Utc::now().date_naive();
    let key_a = api_key_resource(env.store(), TENANT_ID, CUST_UBER_ID, "key-A").await;
    let type_id = keyed_type(env.store(), TENANT_ID, API_KEY, &[API_KEY, API_KEY_ALIAS])
        .await
        .id;
    let for_key = env
        .services()
        .insert_subscription(
            Actor::System,
            bandwidth_request(
                CUST_UBER_ID,
                today,
                SubscriptionActivationCondition::OnStart,
                Some(key_a),
            ),
            TENANT_ID,
        )
        .await
        .unwrap()
        .id;
    env.insert_draft_invoice(for_key, today + Days::new(40))
        .await;
    let adjustment = env
        .insert_draft_invoice(for_key, today + Days::new(80))
        .await;
    diesel::sql_query("UPDATE invoice SET invoice_type = 'ADJUSTMENT' WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(adjustment.id.as_uuid())
        .execute(&mut env.conn().await)
        .await
        .unwrap();
    #[derive(diesel::QueryableByName)]
    struct Drafts {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        total: i64,
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        recurring: i64,
    }
    let drafts: Drafts = diesel::sql_query(
        "SELECT count(*) AS total, count(*) FILTER (WHERE invoice_type = 'RECURRING') AS recurring \
         FROM invoice WHERE tenant_id = $1 AND customer_id = $2 AND status = 'DRAFT' \
           AND subscription_id IS NOT NULL AND consolidated_into_invoice_id IS NULL",
    )
    .bind::<diesel::sql_types::Uuid, _>(TENANT_ID.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(CUST_UBER_ID.as_uuid())
    .get_result(&mut env.conn().await)
    .await
    .unwrap();
    assert!(
        drafts.recurring < drafts.total,
        "an adjustment draft among them"
    );

    let preview = env
        .services()
        .preview_metric_attribution(
            TENANT_ID,
            METRIC_BANDWIDTH,
            Some(MetricAttribution {
                type_id,
                property: None,
            }),
        )
        .await
        .expect("the preview");
    assert_eq!(i64::from(preview.draft_invoices), drafts.total);
}

/// A self-serve checkout for a resource, its debit pending; returns the session and its charge.
async fn pending_checkout(
    env: &TestEnv,
    resource_id: ResourceId,
    start: NaiveDate,
) -> (
    common_domain::ids::CheckoutSessionId,
    meteroid_store::domain::PaymentTransaction,
) {
    use meteroid_store::domain::checkout_sessions::CheckoutCompletionResult;
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
    let session = env
        .store()
        .create_checkout_session(self_serve_checkout(resource_id, start))
        .await
        .expect("the checkout");
    let CheckoutCompletionResult::AwaitingPayment { transaction, .. } = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3500,
            "EUR".to_owned(),
            None,
        )
        .await
        .expect("the debit")
    else {
        panic!("a debit settling later");
    };
    (session.id, transaction)
}

async fn settle(env: &TestEnv, mut transaction: meteroid_store::domain::PaymentTransaction) {
    use diesel_async::RunQueryDsl;
    diesel::sql_query(
        "UPDATE payment_transaction SET status = 'SETTLED', processed_at = now() WHERE id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(transaction.id.as_uuid())
    .execute(&mut env.conn().await)
    .await
    .unwrap();
    transaction.status = meteroid_store::domain::PaymentStatusEnum::Settled;
    transaction.processed_at = Some(Utc::now().naive_utc());
    env.services()
        .on_payment_transaction_settled(transaction.into())
        .await
        .expect("the settlement");
}

/// A debit settling after the customer-level subscription invoiced past the checkout's start:
/// the subscription is created all the same, claiming its resource from its activation.
#[rstest]
#[tokio::test]
async fn test_a_checkout_settled_after_a_renewal_creates_its_subscription(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
    let env = test_env.await;
    env.seed_payments().await;
    env.set_mock_charge_behavior(meteroid_store::domain::connectors::MockChargeBehavior::Pending)
        .await;
    let today = Utc::now().date_naive();
    let customer_level = env
        .services()
        .insert_subscription(
            Actor::System,
            bandwidth_request(
                CUST_UBER_ID,
                today - Months::new(1),
                SubscriptionActivationCondition::OnStart,
                None,
            ),
            TENANT_ID,
        )
        .await
        .unwrap()
        .id;
    let key_a = api_key_resource(env.store(), TENANT_ID, CUST_UBER_ID, "key-A").await;
    let (session_id, transaction) = pending_checkout(&env, key_a, today - Days::new(3)).await;
    env.process_cycles().await;
    assert_eq!(
        env.get_subscription(customer_level)
            .await
            .current_period_start,
        today,
        "invoiced past the checkout's start"
    );

    settle(&env, transaction).await;
    let created = env
        .store()
        .get_checkout_session(TENANT_ID, session_id)
        .await
        .unwrap()
        .subscription_id
        .expect("its subscription");
    let row = env.get_subscription(created).await;
    assert_eq!(row.status, SubscriptionStatusEnum::Active);
    assert_eq!(row.resource_id, Some(key_a));
    let claimed_from =
        meteroid_store::usage_controls_support::usage_attribution::attributed_subscriptions(
            &mut env.conn().await,
            TENANT_ID,
            CUST_UBER_ID,
            false,
        )
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.id == created)
        .and_then(|s| s.activated_from)
        .expect("claimed from its activation");
    assert!(claimed_from >= today.and_time(NaiveTime::MIN));
}

/// A resource archived before the debit settles: no subscription can be created, so the money is
/// kept as the customer's credit, once however often the settlement is delivered, reported as its
/// own event (a webhook) and on the customer's timeline.
#[rstest]
#[tokio::test]
async fn test_a_checkout_whose_resource_was_archived_keeps_its_payment_as_credit(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::repositories::ResourcesInterface;
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
    let env = test_env.await;
    env.seed_payments().await;
    env.set_mock_charge_behavior(meteroid_store::domain::connectors::MockChargeBehavior::Pending)
        .await;
    let today = Utc::now().date_naive();
    let key_a = api_key_resource(env.store(), TENANT_ID, CUST_UBER_ID, "key-A").await;
    let (session_id, transaction) = pending_checkout(&env, key_a, today).await;
    let amount = transaction.amount;
    env.store()
        .set_resource_archived(TENANT_ID, key_a, true)
        .await
        .expect("archived");

    settle(&env, transaction.clone()).await;
    settle(&env, transaction.clone()).await;
    let session = env
        .store()
        .get_checkout_session(TENANT_ID, session_id)
        .await
        .unwrap();
    assert_eq!(session.subscription_id, None);
    assert_eq!(
        session.status,
        meteroid_store::domain::checkout_sessions::CheckoutSessionStatus::Cancelled
    );
    #[derive(diesel::QueryableByName)]
    struct Kept {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        grants: i64,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::BigInt>)]
        amount: Option<i64>,
    }
    let kept: Kept = {
        use diesel_async::RunQueryDsl;
        diesel::sql_query(
            "SELECT count(*) AS grants, sum(amount)::bigint AS amount FROM customer_credit_grant \
             WHERE source_kind = 'OVERPAYMENT' AND source_id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(transaction.id.as_uuid())
        .get_result(&mut env.conn().await)
        .await
        .unwrap()
    };
    assert_eq!((kept.grants, kept.amount), (1, Some(amount)));
    let for_key: i64 = {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::subscription::dsl as s;
        s::subscription
            .filter(s::resource_id.eq(key_a))
            .count()
            .get_result(&mut env.conn().await)
            .await
            .unwrap()
    };
    assert_eq!(for_key, 0);
    // Reported once, as its own event.
    let reported = kept_as_credit_events(&env).await;
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert_eq!(reported[0].checkout_session_id, session_id);
    assert_eq!(reported[0].payment_transaction_id, transaction.id);
    assert_eq!(reported[0].resource_id, Some(key_a));
    assert_eq!(reported[0].amount, amount);
    // And on the customer's timeline, once.
    #[derive(diesel::QueryableByName)]
    struct Logged {
        #[diesel(sql_type = diesel::sql_types::Text)]
        entity_type: String,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
        agg_customer_id: Option<Uuid>,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::BigInt>)]
        amount: Option<i64>,
    }
    let logged: Vec<Logged> = {
        use diesel_async::RunQueryDsl;
        diesel::sql_query(
            "SELECT entity_type, agg_customer_id, (metadata->>'amount')::bigint AS amount \
             FROM entity_activity \
             WHERE activity_type = 'checkout.payment_kept_as_credit' AND entity_id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(transaction.id.as_uuid())
        .load(&mut env.conn().await)
        .await
        .unwrap()
    };
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].entity_type, "payment_transaction");
    assert_eq!(logged[0].agg_customer_id, Some(CUST_UBER_ID.as_uuid()));
    assert_eq!(logged[0].amount, Some(amount));
}

async fn kept_as_credit_events(
    env: &TestEnv,
) -> Vec<meteroid_store::domain::outbox_event::CheckoutPaymentKeptAsCreditEvent> {
    use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
    use meteroid_store::domain::outbox_event::OutboxEvent;
    use meteroid_store::domain::pgmq::PgmqQueue;
    use meteroid_store::repositories::pgmq::PgmqInterface;
    env.store()
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(200),
            MessageReadVtSec(0),
        )
        .await
        .expect("the webhook_out queue")
        .into_iter()
        .filter_map(|m| m.message)
        .filter_map(|m| serde_json::from_value::<OutboxEvent>(m.0).ok())
        .filter_map(|event| match event {
            OutboxEvent::CheckoutPaymentKeptAsCredit(event) => Some(*event),
            _ => None,
        })
        .collect()
}

/// A refusal that is not its resource's (here its customer archived meanwhile) may lift: the
/// settlement fails and is retried, nothing is kept as credit, and once it lifts the subscription
/// is created.
#[rstest]
#[tokio::test]
async fn test_a_checkout_refused_for_another_reason_than_its_resource_is_retried(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
    let env = test_env.await;
    env.seed_payments().await;
    env.set_mock_charge_behavior(meteroid_store::domain::connectors::MockChargeBehavior::Pending)
        .await;
    let today = Utc::now().date_naive();
    let key_a = api_key_resource(env.store(), TENANT_ID, CUST_UBER_ID, "key-A").await;
    let (session_id, mut transaction) = pending_checkout(&env, key_a, today).await;
    let archive = async |archived: bool| {
        use diesel_async::RunQueryDsl;
        diesel::sql_query(
            "UPDATE customer SET archived_at = CASE WHEN $2 THEN now() END WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(CUST_UBER_ID.as_uuid())
        .bind::<diesel::sql_types::Bool, _>(archived)
        .execute(&mut env.conn().await)
        .await
        .unwrap();
    };
    archive(true).await;
    {
        use diesel_async::RunQueryDsl;
        diesel::sql_query(
            "UPDATE payment_transaction SET status = 'SETTLED', processed_at = now() \
             WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(transaction.id.as_uuid())
        .execute(&mut env.conn().await)
        .await
        .unwrap();
    }
    transaction.status = meteroid_store::domain::PaymentStatusEnum::Settled;
    transaction.processed_at = Some(Utc::now().naive_utc());
    let refused = env
        .services()
        .on_payment_transaction_settled(transaction.clone().into())
        .await
        .expect_err("an archived customer's subscription is refused");
    assert!(
        matches!(refused.current_context(), StoreError::InvalidArgument(_)),
        "{refused:?}"
    );
    let session = env
        .store()
        .get_checkout_session(TENANT_ID, session_id)
        .await
        .unwrap();
    assert_ne!(
        session.status,
        meteroid_store::domain::checkout_sessions::CheckoutSessionStatus::Cancelled
    );
    let grants: i64 = {
        #[derive(diesel::QueryableByName)]
        struct Grants {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            grants: i64,
        }
        use diesel_async::RunQueryDsl;
        diesel::sql_query(
            "SELECT count(*) AS grants FROM customer_credit_grant \
             WHERE source_kind = 'OVERPAYMENT' AND source_id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(transaction.id.as_uuid())
        .get_result::<Grants>(&mut env.conn().await)
        .await
        .unwrap()
        .grants
    };
    assert_eq!(grants, 0, "nothing kept as credit");
    assert!(kept_as_credit_events(&env).await.is_empty());

    archive(false).await;
    env.services()
        .on_payment_transaction_settled(transaction.into())
        .await
        .expect("the retried settlement");
    let created = env
        .store()
        .get_checkout_session(TENANT_ID, session_id)
        .await
        .unwrap()
        .subscription_id
        .expect("its subscription");
    assert_eq!(env.get_subscription(created).await.resource_id, Some(key_a));
}

/// §3.6: a subscription for a resource bills none of a metric attributed to the customer only,
/// and says so when created; nobody bills that usage, listed as unattributed.
#[tokio::test]
async fn test_a_subscription_for_a_resource_bills_nothing_of_a_metric_not_attributed_to_its_type() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let last_month = month_start(Utc::now().date_naive()) - Months::new(1);
    let customer = stack.customer("Unattributed").await;
    let key_a = stack.resource(customer, "key-A").await;
    // Making the type attributed every metric to it; the token metric is the customer's again.
    stack
        .meteroid
        .store
        .update_billable_metric(Actor::System, stack.metric.id, TENANT_ID, attribute(None))
        .await
        .expect("the metric attributed to the customer only");

    let created = stack
        .create(stack.request(customer, last_month, Some(key_a)))
        .await
        .expect("accepted, with warnings");
    assert_eq!(
        created.warnings,
        vec![
            "plan_sold_for_mismatch".to_owned(),
            "metric_not_attributed".to_owned()
        ]
    );
    assert_eq!(
        created
            .metrics_not_attributed
            .iter()
            .map(|m| m.code.clone())
            .collect::<Vec<_>>(),
        vec![CODE.to_owned()]
    );

    // The subscription's own key, standard 1M (€12.00 at its price).
    stack
        .ingest(
            vec![Tokens {
                customer,
                at: at(last_month + Days::new(2), 10),
                api_key: Some("key-A"),
                model: "standard",
                tokens: 1_000_000,
            }],
            range(last_month, last_month + Months::new(1)),
        )
        .await;
    stack.grace_hours(0).await;
    stack.process_cycles().await;
    let lines = stack
        .invoices_of(created.id)
        .await
        .into_iter()
        .flat_map(|invoice| invoice.line_items)
        .filter(|line| line.metric_id == Some(stack.metric.id))
        .collect::<Vec<_>>();
    assert!(
        lines
            .iter()
            .all(|line| line.amount_subtotal == 0
                && line.quantity.unwrap_or_default() == Decimal::ZERO),
        "{lines:?}"
    );
    assert_eq!(stack.invoiced(created.id, last_month).await, 0);
    let unattributed = stack
        .unattributed(customer, last_month, last_month + Months::new(1))
        .await;
    assert!(
        unattributed
            .iter()
            .any(|(metric, quantity)| *metric == stack.metric.id && *quantity > Decimal::ZERO),
        "{unattributed:?}"
    );
}

/// §3.6: a sale for a resource of a plan pricing a metric not attributed to the resource's type is
/// accepted with `metric_not_attributed`, naming the metric: a subscription (store, its preview,
/// v1) and a v1 checkout session. A customer-wide sale of the same plan bills the metric: no
/// warning. A subscription says so on every read of it too: the store's, v1, v2 and the dashboard's.
#[tokio::test]
async fn test_a_sale_for_a_resource_names_the_metrics_its_type_never_bills() {
    use crate::data::ids::{PLAN_USAGE_ALPHA_ID, PLAN_VERSION_USAGE_ALPHA_ID};
    use crate::harness::subscription;
    use meteroid_store::repositories::resources::ResourceTypeNew;
    helpers::init::logging();
    let postgres = meteroid_it::container::create_seeded_test_database(
        meteroid_it::container::SeedLevel::PLANS,
    )
    .await;
    let setup =
        meteroid_it::container::start_meteroid(postgres, meteroid_it::container::SeedLevel::PLANS)
            .await;
    let store = &setup.store;
    let site = store
        .create_resource_type(ResourceTypeNew {
            tenant_id: TENANT_ID,
            code: "site".to_owned(),
            name: "Site".to_owned(),
            plural_name: "Sites".to_owned(),
            parent_type_id: None,
            keys: vec!["site_id".to_owned()],
            attributes: Default::default(),
        })
        .await
        .expect("a type no metric is attributed to");
    let s1 = helpers::resources::resource(store, &site, CUST_UBER_ID, "s1").await;
    let s2 = helpers::resources::resource(store, &site, CUST_UBER_ID, "s2").await;
    let s3 = helpers::resources::resource(store, &site, CUST_UBER_ID, "s3").await;
    let bandwidth_metric = store
        .find_billable_metric_by_id(METRIC_BANDWIDTH, TENANT_ID)
        .await
        .expect("the plan's metric");
    let bandwidth = bandwidth_metric.code.clone();
    let warned = vec![
        "plan_sold_for_mismatch".to_owned(),
        "metric_not_attributed".to_owned(),
    ];
    let today = Utc::now().date_naive();
    let request = |resource_id: Option<ResourceId>| {
        let mut request = subscription()
            .customer(CUST_UBER_ID)
            .plan_version(PLAN_VERSION_USAGE_ALPHA_ID)
            .start_date(today)
            .on_start()
            .no_trial()
            .no_auto_charge()
            .request();
        request.subscription.resource_id = resource_id;
        request
    };

    let (_, details) = setup
        .services
        .preview_create_subscription(&request(Some(s1)), TENANT_ID)
        .await
        .expect("the preview");
    let previewed = setup
        .services
        .sale_warnings(TENANT_ID, &details)
        .await
        .expect("the preview's warnings");
    assert_eq!(previewed.codes, warned);
    assert_eq!(
        previewed
            .metrics_not_attributed
            .iter()
            .map(|m| m.code.clone())
            .collect::<Vec<_>>(),
        vec![bandwidth.clone()]
    );

    let created = setup
        .services
        .insert_subscription(Actor::System, request(Some(s1)), TENANT_ID)
        .await
        .expect("accepted, with warnings");
    assert_eq!(created.warnings, warned);
    assert_eq!(
        created
            .metrics_not_attributed
            .iter()
            .map(|m| m.code.clone())
            .collect::<Vec<_>>(),
        vec![bandwidth.clone()]
    );
    let customer_wide = setup
        .services
        .insert_subscription(Actor::System, request(None), TENANT_ID)
        .await
        .expect("a customer-wide subscription");
    assert!(
        customer_wide.warnings.is_empty(),
        "{:?}",
        customer_wide.warnings
    );
    assert!(customer_wide.metrics_not_attributed.is_empty());

    let read = store
        .get_subscription_warnings(TENANT_ID, created.id)
        .await
        .expect("its warnings, read");
    assert_eq!(read.codes, warned);
    assert_eq!(
        read.metrics_not_attributed
            .iter()
            .map(|m| m.code.clone())
            .collect::<Vec<_>>(),
        vec![bandwidth.clone()]
    );
    let read = store
        .get_subscription_warnings(TENANT_ID, customer_wide.id)
        .await
        .expect("no warning, read");
    assert!(read.codes.is_empty(), "{:?}", read.codes);

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let details = clients
        .subscriptions
        .clone()
        .get_subscription_details(tonic::Request::new(
            meteroid_grpc::meteroid::api::subscriptions::v1::GetSubscriptionDetailsRequest {
                subscription_id: created.id.as_proto(),
            },
        ))
        .await
        .expect("the dashboard's read")
        .into_inner();
    assert_eq!(details.warnings, warned);
    assert_eq!(details.metrics_not_attributed, vec![bandwidth.clone()]);

    let (api_key, _) = store
        .insert_api_token(
            Actor::System,
            ApiTokenNew {
                name: "unattributed".to_owned(),
                tenant_id: TENANT_ID,
            },
        )
        .await
        .expect("api key");
    let base = setup.config.rest_api_external_url.clone();
    let client = http_client_tracing::client();
    let answer = async |response: reqwest::Response| {
        let status = response.status();
        let body = response.text().await.unwrap();
        assert!(status.is_success(), "{status}: {body}");
        serde_json::from_str::<serde_json::Value>(&body).unwrap()
    };

    let v1_subscription = answer(
        client
            .post(format!("{base}/api/v1/subscriptions"))
            .bearer_auth(&api_key)
            .json(&serde_json::json!({
                "plan_id": PLAN_USAGE_ALPHA_ID.as_base62(),
                "customer_id_or_alias": CUST_UBER_ID.as_base62(),
                "start_date": today,
                "activation_condition": "ON_START",
                "auto_advance_invoices": false,
                "charge_automatically": false,
                "resource_id": s2.as_base62(),
            }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        v1_subscription["warnings"],
        serde_json::json!(["plan_sold_for_mismatch", "metric_not_attributed"])
    );
    assert_eq!(
        v1_subscription["metrics_not_attributed"],
        serde_json::json!([bandwidth])
    );
    let v1_read = answer(
        client
            .get(format!(
                "{base}/api/v1/subscriptions/{}",
                v1_subscription["id"].as_str().unwrap()
            ))
            .bearer_auth(&api_key)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        v1_read["warnings"],
        serde_json::json!(["plan_sold_for_mismatch", "metric_not_attributed"]),
        "read again: {v1_read}"
    );
    assert_eq!(
        v1_read["metrics_not_attributed"],
        serde_json::json!([bandwidth])
    );
    let v2_read = answer(
        client
            .get(format!(
                "{base}/api/v2/subscriptions/{}",
                created.id.as_base62()
            ))
            .bearer_auth(&api_key)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        v2_read["warnings"],
        serde_json::json!(["plan_sold_for_mismatch", "metric_not_attributed"]),
        "{v2_read}"
    );
    assert_eq!(
        v2_read["metrics_not_attributed"],
        serde_json::json!([bandwidth_metric.catalog_code])
    );
    let v2_customer_wide = answer(
        client
            .get(format!(
                "{base}/api/v2/subscriptions/{}",
                customer_wide.id.as_base62()
            ))
            .bearer_auth(&api_key)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert!(
        v2_customer_wide.get("warnings").is_none(),
        "{v2_customer_wide}"
    );

    let v1_checkout = answer(
        client
            .post(format!("{base}/api/v1/checkout-sessions"))
            .bearer_auth(&api_key)
            .json(&serde_json::json!({
                "customer_id": CUST_UBER_ID.as_base62(),
                "plan_version_id": PLAN_VERSION_USAGE_ALPHA_ID.as_base62(),
                "resource_id": s3.as_base62(),
            }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let session = &v1_checkout["session"];
    assert_eq!(
        session["warnings"],
        serde_json::json!(["plan_sold_for_mismatch", "metric_not_attributed"])
    );
    assert_eq!(
        session["metrics_not_attributed"],
        serde_json::json!([bandwidth])
    );
    let read = answer(
        client
            .get(format!(
                "{base}/api/v1/checkout-sessions/{}",
                session["id"].as_str().unwrap()
            ))
            .bearer_auth(&api_key)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert!(
        read["session"].get("warnings").is_none(),
        "told on creation only: {read}"
    );
}

/// Main's schema has no such check: filters are written as main wrote them, and the conversion
/// adds it back.
const FILTER_CHECK_DROPPED: &str =
    "ALTER TABLE subscription DROP CONSTRAINT IF EXISTS subscription_usage_filter_needs_resource";

/// Runs the migration's conversion of leftover usage filters.
async fn convert_usage_filters(env: &TestEnv) {
    try_convert_usage_filters(env)
        .await
        .expect("the conversion");
}

async fn try_convert_usage_filters(env: &TestEnv) -> Result<(), diesel::result::Error> {
    use diesel_async::SimpleAsyncConnection;
    let migration =
        include_str!("../../migrations/diesel/2026-11-26-011852_entitlements_v2_billing/up.sql");
    let conversion = &migration[migration
        .find("-- Leftover usage filters become resources:")
        .expect("the conversion")
        ..migration
            .find("-- From when a row a late change wrote bills its metric")
            .expect("the floors")];
    env.conn()
        .await
        .batch_execute(&format!("{FILTER_CHECK_DROPPED};\n{conversion}"))
        .await
}

/// The subscription's usage filter, one version from its start date, as main wrote it.
async fn filter_on(env: &TestEnv, subscription_id: SubscriptionId, property: &str, value: &str) {
    filter_from(
        env,
        subscription_id,
        property,
        value,
        "start_date::timestamp",
    )
    .await;
}

async fn filter_from(
    env: &TestEnv,
    subscription_id: SubscriptionId,
    property: &str,
    value: &str,
    from: &str,
) {
    use diesel_async::SimpleAsyncConnection;
    env.conn()
        .await
        .batch_execute(FILTER_CHECK_DROPPED)
        .await
        .unwrap();
    sql(
        env,
        &format!(
            "UPDATE subscription SET usage_filter_property = '{property}', \
             usage_filter_value = '{value}' WHERE id = $1"
        ),
        subscription_id,
    )
    .await;
    sql(
        env,
        &format!(
            "INSERT INTO subscription_usage_filter
                 (id, tenant_id, subscription_id, property, value, effective_from)
             SELECT gen_random_uuid(), tenant_id, id, '{property}', '{value}', {from}
             FROM subscription WHERE id = $1"
        ),
        subscription_id,
    )
    .await;
}

/// The conversion fails, naming `subscription_id` as one to convert by hand.
async fn refused_conversion(env: &TestEnv, subscription_id: SubscriptionId) {
    let failed = try_convert_usage_filters(env)
        .await
        .expect_err("converted by hand");
    assert!(
        failed.to_string().contains(&format!(
            "subscription {} has a filter history",
            subscription_id.as_uuid()
        )),
        "{failed}"
    );
}

/// A filter added after its subscription's start: before it, its usage was the customer's, which
/// a resource from the start would take from the customer-level subscriptions' open drafts.
#[rstest]
#[tokio::test]
async fn test_the_conversion_refuses_a_filter_added_after_the_start(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let filtered = insert_bandwidth(&env).await;
    filter_from(
        &env,
        filtered,
        "api_key",
        "k1",
        "start_date::timestamp + interval '2 days'",
    )
    .await;
    refused_conversion(&env, filtered).await;
}

/// A filter changed from k1 to k2: converted from its current one, k1's earlier usage would leave
/// it and k2's would be taken back from the customer-level subscriptions.
#[rstest]
#[tokio::test]
async fn test_the_conversion_refuses_a_filter_changed_over_its_life(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let filtered = insert_bandwidth(&env).await;
    filter_on(&env, filtered, "api_key", "k1").await;
    sql(
        &env,
        "UPDATE subscription_usage_filter SET effective_until = now() WHERE subscription_id = $1",
        filtered,
    )
    .await;
    filter_from(&env, filtered, "api_key", "k2", "now()").await;
    refused_conversion(&env, filtered).await;
}

/// A filter removed: the subscription no longer converts, though it claimed the key's usage up
/// to then.
#[rstest]
#[tokio::test]
async fn test_the_conversion_refuses_a_filter_removed_over_its_life(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let filtered = insert_bandwidth(&env).await;
    filter_on(&env, filtered, "api_key", "k1").await;
    sql(
        &env,
        "UPDATE subscription SET usage_filter_property = NULL, usage_filter_value = NULL
         WHERE id = $1",
        filtered,
    )
    .await;
    refused_conversion(&env, filtered).await;
}

/// Its start settled after its start date: main claimed from the start date, a resource claims
/// from the settled start, so the days between would change hands.
#[rstest]
#[tokio::test]
async fn test_the_conversion_refuses_a_start_settled_after_the_start_date(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let filtered = insert_bandwidth(&env).await;
    filter_on(&env, filtered, "api_key", "k1").await;
    sql(
        &env,
        "UPDATE subscription_start_settled SET at = at + interval '1 day' WHERE subscription_id = $1",
        filtered,
    )
    .await;
    refused_conversion(&env, filtered).await;
}

async fn bandwidth_attribution(env: &TestEnv) -> Option<common_domain::ids::ResourceTypeId> {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::billable_metric::dsl as m;
    m::billable_metric
        .filter(m::id.eq(METRIC_BANDWIDTH))
        .select(m::attributed_to_type_id)
        .first(&mut env.conn().await)
        .await
        .unwrap()
}

async fn insert_bandwidth(env: &TestEnv) -> SubscriptionId {
    env.services()
        .insert_subscription(
            Actor::System,
            bandwidth_request(
                CUST_UBER_ID,
                Utc::now().date_naive() - Days::new(5),
                SubscriptionActivationCondition::OnStart,
                None,
            ),
            TENANT_ID,
        )
        .await
        .unwrap()
        .id
}

/// A metric filtered on two properties could take neither type: left unattributed, its drafts
/// would be repriced customer-wide, so the conversion fails, naming it.
#[rstest]
#[tokio::test]
async fn test_the_conversion_refuses_a_metric_filtered_on_two_properties(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let by_key = insert_bandwidth(&env).await;
    let by_site = insert_bandwidth(&env).await;
    filter_on(&env, by_key, "api_key", "k1").await;
    filter_on(&env, by_site, "site", "s1").await;
    let failed = try_convert_usage_filters(&env)
        .await
        .expect_err("a metric filtered on two properties");
    assert!(
        failed.to_string().contains("filtered on 2 properties"),
        "{failed}"
    );
}

/// A filter on a property a type of that code already keys otherwise finds no resource: left
/// customer-wide, its usage would be billed by the customer, so the conversion fails, naming it.
#[rstest]
#[tokio::test]
async fn test_the_conversion_refuses_a_filter_it_leaves_without_a_resource(
    #[future] test_env: TestEnv,
) {
    use diesel_async::SimpleAsyncConnection;
    let env = test_env.await;
    let filtered = insert_bandwidth(&env).await;
    filter_on(&env, filtered, "site", "s1").await;
    env.conn()
        .await
        .batch_execute(&format!(
            "INSERT INTO resource_type (id, tenant_id, code, name, plural_name)
             VALUES ('00000000-0000-0000-0000-0000000005e1', '{tenant}', 'site', 'Site', 'Sites');
             INSERT INTO resource_type_key (tenant_id, resource_type_id, key, position)
             VALUES ('{tenant}', '00000000-0000-0000-0000-0000000005e1', 'site_id', 0);",
            tenant = TENANT_ID.as_uuid()
        ))
        .await
        .unwrap();
    let failed = try_convert_usage_filters(&env)
        .await
        .expect_err("a filter left without a resource");
    assert!(
        failed.to_string().contains(&format!(
            "subscription {} keeps its filter",
            filtered.as_uuid()
        )),
        "{failed}"
    );
}

/// A metric a converted subscription prices through an add-on only is attributed to its type.
#[rstest]
#[tokio::test]
async fn test_the_conversion_attributes_a_metric_priced_by_an_add_on(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;
    use meteroid_store::domain::prices::{FeeStructure, UsageModel};
    let env = test_env.await;
    let priced = insert_bandwidth(&env).await;
    let filtered = env
        .services()
        .insert_subscription(
            Actor::System,
            crate::harness::subscription()
                .customer(CUST_UBER_ID)
                .plan_version(PLAN_VERSION_1_LEETCODE_ID)
                .start_date(Utc::now().date_naive() - Days::new(5))
                .on_start()
                .no_trial()
                .request(),
            TENANT_ID,
        )
        .await
        .unwrap()
        .id;
    diesel::sql_query(
        "WITH bandwidth AS (SELECT s.tenant_id, c.product_id, c.price_id \
                            FROM subscription_component c JOIN subscription s ON s.id = c.subscription_id \
                            WHERE c.subscription_id = $1 AND c.name = 'Bandwidth'), \
              add_on AS (INSERT INTO add_on (id, name, tenant_id, created_at, updated_at, \
                                             product_id, price_id, self_serviceable, fee_structure) \
                         SELECT gen_random_uuid(), 'Bandwidth add-on', tenant_id, now(), now(), \
                                product_id, price_id, false, $3 FROM bandwidth \
                         RETURNING id, name, product_id, price_id) \
         INSERT INTO subscription_add_on (id, name, subscription_id, add_on_id, period, created_at, \
                                          product_id, price_id, quantity, effective_from) \
         SELECT gen_random_uuid(), name, $2, id, 'MONTHLY', now(), product_id, price_id, 1, \
                current_date FROM add_on",
    )
    .bind::<diesel::sql_types::Uuid, _>(priced.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(filtered.as_uuid())
    .bind::<diesel::sql_types::Jsonb, _>(
        serde_json::to_value(FeeStructure::Usage {
            metric_id: METRIC_BANDWIDTH,
            model: UsageModel::PerUnit,
        })
        .unwrap(),
    )
    .execute(&mut env.conn().await)
    .await
    .expect("the add-on");
    filter_on(&env, filtered, "api_key", "k1").await;
    convert_usage_filters(&env).await;

    let type_id = bandwidth_attribution(&env).await.expect("attributed");
    let resource = env
        .get_subscription(filtered)
        .await
        .resource_id
        .expect("its resource");
    let resource_type: common_domain::ids::ResourceTypeId = {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_models::schema::resource::dsl as r;
        r::resource
            .filter(r::id.eq(resource))
            .select(r::resource_type_id)
            .first(&mut env.conn().await)
            .await
            .unwrap()
    };
    assert_eq!(type_id, resource_type);
}

/// After the conversion a filter needs a resource: a previous-release pod writing one on a
/// customer-level subscription fails instead of billing the filtered usage customer-wide.
#[rstest]
#[tokio::test]
async fn test_after_the_conversion_a_filter_without_a_resource_is_refused(
    #[future] test_env: TestEnv,
) {
    use diesel_async::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Validated {
        #[diesel(sql_type = diesel::sql_types::Bool)]
        convalidated: bool,
    }
    let env = test_env.await;
    let filtered = insert_bandwidth(&env).await;
    let customer_level = insert_bandwidth(&env).await;
    filter_on(&env, filtered, "api_key", "k1").await;
    convert_usage_filters(&env).await;

    let check: Validated = diesel::sql_query(
        "SELECT convalidated FROM pg_constraint \
         WHERE conname = 'subscription_usage_filter_needs_resource'",
    )
    .get_result(&mut env.conn().await)
    .await
    .expect("the check");
    assert!(check.convalidated);
    let refused = diesel::sql_query(
        "UPDATE subscription SET usage_filter_property = 'api_key', usage_filter_value = 'k2' \
         WHERE id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(customer_level.as_uuid())
    .execute(&mut env.conn().await)
    .await
    .expect_err("a filter without a resource");
    assert!(
        refused
            .to_string()
            .contains("subscription_usage_filter_needs_resource"),
        "{refused}"
    );
}

/// A row billing a metric its charges don't name (its charge's structure doesn't fit its price,
/// so billing reads its stored fee): left unattributed, its usage would be billed by no one, so
/// the preflight lists it and the conversion fails, naming the subscription and the metric.
#[rstest]
#[tokio::test]
async fn test_the_conversion_refuses_a_billed_metric_its_charges_do_not_name(
    #[future] test_env: TestEnv,
) {
    use crate::data::ids::{COMP_USAGE_BETA_DB_STORAGE_ID, PRICE_LEETCODE_RATE_ID};
    let env = test_env.await;
    let filtered = insert_bandwidth(&env).await;
    sql(
        &env,
        &format!(
            "UPDATE subscription_component SET price_component_id = '{}', price_id = '{}'
             WHERE subscription_id = $1 AND name = 'Bandwidth'",
            COMP_USAGE_BETA_DB_STORAGE_ID.as_uuid(),
            PRICE_LEETCODE_RATE_ID.as_uuid()
        ),
        filtered,
    )
    .await;
    filter_on(&env, filtered, "api_key", "k1").await;

    let listed = preflight_checks(&env)
        .await
        .into_iter()
        .filter(|row| row.check_no == 17)
        .collect::<Vec<_>>();
    assert_eq!(listed.len(), 1, "one tenant");
    assert_eq!(listed[0].sample_ids, Some(vec![filtered.as_uuid()]));
    let detail = &listed[0].sample_details[0];
    assert!(
        detail.starts_with("component Bandwidth bills metric ")
            && detail.ends_with("its charges name it for: no filtered subscription"),
        "{detail}"
    );

    let failed = try_convert_usage_filters(&env)
        .await
        .expect_err("a billed metric left unattributed")
        .to_string();
    assert!(
        failed.contains(&format!("subscription {} bills metric", filtered.as_uuid()))
            && failed.contains(&METRIC_BANDWIDTH.as_uuid().to_string()),
        "{failed}"
    );
}

/// A metric attributed to another type before the conversion keeps it: the converted
/// subscription pricing it would bill none of its usage, so the conversion fails, naming both.
#[rstest]
#[tokio::test]
async fn test_the_conversion_refuses_a_billed_metric_attributed_to_another_type(
    #[future] test_env: TestEnv,
) {
    use diesel_async::SimpleAsyncConnection;
    let env = test_env.await;
    let filtered = insert_bandwidth(&env).await;
    env.conn()
        .await
        .batch_execute(&format!(
            "INSERT INTO resource_type (id, tenant_id, code, name, plural_name)
             VALUES ('00000000-0000-0000-0000-0000000005e2', '{tenant}', 'site', 'Site', 'Sites');
             INSERT INTO resource_type_key (tenant_id, resource_type_id, key, position)
             VALUES ('{tenant}', '00000000-0000-0000-0000-0000000005e2', 'site_id', 0);
             UPDATE billable_metric SET attributed_to_type_id = '00000000-0000-0000-0000-0000000005e2'
             WHERE id = '{metric}';",
            tenant = TENANT_ID.as_uuid(),
            metric = METRIC_BANDWIDTH.as_uuid(),
        ))
        .await
        .unwrap();
    filter_on(&env, filtered, "api_key", "k1").await;
    let failed = try_convert_usage_filters(&env)
        .await
        .expect_err("a billed metric attributed to another type")
        .to_string();
    assert!(
        failed.contains(&format!("subscription {} bills metric", filtered.as_uuid()))
            && failed.contains(&METRIC_BANDWIDTH.as_uuid().to_string()),
        "{failed}"
    );
}

#[derive(diesel::QueryableByName)]
struct PreflightRow {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    check_no: i32,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Array<diesel::sql_types::Uuid>>)]
    sample_ids: Option<Vec<Uuid>>,
    #[diesel(sql_type = diesel::sql_types::Array<diesel::sql_types::Text>)]
    sample_details: Vec<String>,
}

/// The checks of the entitlements v2 preflight, its second statement.
async fn preflight_checks(env: &TestEnv) -> Vec<PreflightRow> {
    use diesel_async::RunQueryDsl;
    let preflight = include_str!("../../migrations/preflight/entitlements_v2.sql");
    let checks = &preflight[preflight.find("-- The checks.").expect("the checks")..];
    diesel::sql_query(checks)
        .load(&mut env.conn().await)
        .await
        .expect("the preflight runs")
}

/// A property a type code cannot spell gets a slug code, as the preflight predicts; events keep
/// matching the property, the type's key.
#[rstest]
#[tokio::test]
async fn test_the_conversion_gives_a_dotted_property_a_slug_code(#[future] test_env: TestEnv) {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::resource_type::dsl as t;
    use diesel_models::schema::resource_type_key::dsl as k;
    let env = test_env.await;
    let filtered = insert_bandwidth(&env).await;
    filter_on(&env, filtered, "Metadata.Project", "p1").await;

    let predicted = preflight_checks(&env)
        .await
        .into_iter()
        .filter(|row| row.check_no == 7)
        .flat_map(|row| row.sample_details)
        .collect::<Vec<_>>();
    assert_eq!(
        predicted,
        vec!["property 'Metadata.Project': type code metadata_project, key 'Metadata.Project'"]
    );
    convert_usage_filters(&env).await;

    let type_id = bandwidth_attribution(&env).await.expect("attributed");
    let code: String = t::resource_type
        .filter(t::id.eq(type_id))
        .select(t::code)
        .first(&mut env.conn().await)
        .await
        .unwrap();
    let keys: Vec<String> = k::resource_type_key
        .filter(k::resource_type_id.eq(type_id))
        .select(k::key)
        .load(&mut env.conn().await)
        .await
        .unwrap();
    assert_eq!(code, "metadata_project");
    assert_eq!(keys, vec!["Metadata.Project".to_owned()]);
    assert!(env.get_subscription(filtered).await.resource_id.is_some());
}

/// v1 let a plan version's row hide an add-on and an add-on's replace the plan's; from deploy a
/// multi-instance add-on adds to the plan version's limit (times its instances), and an override
/// add-on's lower config number no longer wins. The preflight lists both, values side by side.
#[rstest]
#[tokio::test]
async fn test_the_preflight_lists_entitlement_values_an_add_on_changes(
    #[future] test_env: TestEnv,
) {
    use crate::data::ids::PLAN_LEETCODE_ID;
    use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
    use meteroid_store::domain::prices::{FeeStructure, UsageModel};
    let env = test_env.await;
    let subscription = insert_bandwidth(&env).await;
    let add_on = Uuid::now_v7();
    diesel::sql_query(
        "WITH bandwidth AS (SELECT s.tenant_id, c.product_id, c.price_id \
                            FROM subscription_component c JOIN subscription s ON s.id = c.subscription_id \
                            WHERE c.subscription_id = $1 AND c.name = 'Bandwidth'), \
              add_on AS (INSERT INTO add_on (id, name, tenant_id, created_at, updated_at, \
                                             product_id, price_id, self_serviceable, fee_structure) \
                         SELECT $2, 'Extra calls', tenant_id, now(), now(), product_id, price_id, \
                                false, $3 FROM bandwidth \
                         RETURNING id, name, product_id, price_id) \
         INSERT INTO subscription_add_on (id, name, subscription_id, add_on_id, period, created_at, \
                                          product_id, price_id, quantity, effective_from) \
         SELECT gen_random_uuid(), name, $1, id, 'MONTHLY', now(), product_id, price_id, 2, \
                current_date FROM add_on",
    )
    .bind::<diesel::sql_types::Uuid, _>(subscription.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(add_on)
    .bind::<diesel::sql_types::Jsonb, _>(
        serde_json::to_value(FeeStructure::Usage {
            metric_id: METRIC_BANDWIDTH,
            model: UsageModel::PerUnit,
        })
        .unwrap(),
    )
    .execute(&mut env.conn().await)
    .await
    .expect("two instances of a multi-instance add-on");
    let (calls, workers) = (Uuid::now_v7(), Uuid::now_v7());
    let row = |feature: Uuid,
               entity: Uuid,
               entity_type: &str,
               mode: &str,
               effect: &str,
               value: &str| {
        format!(
            "INSERT INTO entitlement (id, tenant_id, feature_id, entity_id, entity_type, mode, value,
                                      lineage_id, lineage_created_at, lineage_anchor, effect)
             SELECT id, '{tenant}', '{feature}', '{entity}', '{entity_type}', '{mode}', '{value}',
                    id, now(), now(), '{effect}'
             FROM (SELECT gen_random_uuid() AS id) i;",
            tenant = TENANT_ID.as_uuid(),
        )
    };
    env.conn()
        .await
        .batch_execute(&format!(
            "INSERT INTO feature (id, tenant_id, name, code, feature_type, metric_id, kind,
                                  limit_counted_by, policy)
             VALUES ('{calls}', '{tenant}', 'Preflight calls', 'preflight_calls', 'METERED', '{metric}',
                     'LIMIT', 'EVENTS',
                     '{{\"resets\": \"BILLING_CYCLE\", \"at_limit\": \"DENY\", \"grace_pct\": 0, \"thresholds\": []}}');
             INSERT INTO feature (id, tenant_id, name, code, feature_type, config_value_type, kind, value_type)
             VALUES ('{workers}', '{tenant}', 'Preflight workers', 'preflight_workers', 'CONFIG', 'NUMBER',
                     'VALUE', 'NUMBER');
             {}{}{}{}",
            row(
                calls,
                PLAN_VERSION_1_LEETCODE_ID.as_uuid(),
                "PLAN_VERSION",
                "OVERRIDE",
                "SET",
                r#"{"type": "metered", "limit": "100"}"#
            ),
            row(
                calls,
                add_on,
                "ADD_ON",
                "STACK",
                "ADD",
                r#"{"type": "metered", "limit": "1000"}"#
            ),
            row(
                workers,
                PLAN_LEETCODE_ID.as_uuid(),
                "PLAN",
                "OVERRIDE",
                "SET",
                r#"{"type": "config", "value": {"kind": "number", "value": "10"}}"#
            ),
            row(
                workers,
                add_on,
                "ADD_ON",
                "OVERRIDE",
                "RAISE_TO",
                r#"{"type": "config", "value": {"kind": "number", "value": "5"}}"#
            ),
            tenant = TENANT_ID.as_uuid(),
            metric = METRIC_BANDWIDTH.as_uuid(),
        ))
        .await
        .expect("a limit and a config number, with their plan's and the add-on's rows");

    let mut listed = preflight_checks(&env)
        .await
        .into_iter()
        .filter(|row| row.check_no == 16)
        .flat_map(|row| row.sample_details)
        .filter(|detail| detail.starts_with("feature preflight_"))
        .collect::<Vec<_>>();
    listed.sort();
    assert_eq!(
        listed,
        vec![
            "feature preflight_calls: 100 before, 2100 after",
            "feature preflight_workers: 5 before, 10 after",
        ]
    );
}

/// A subscription for a resource that priced the metric fixes its attribution once it has an
/// invoice, draft or finalized, though it billed none of the metric's usage: a change would move
/// what the customer-level subscription invoiced around it. Its invoice voided, a change marks for
/// the draft worker the drafts of the customers whose subscriptions for a resource priced it, and
/// only those.
/// Rules: R-144
#[rstest]
#[tokio::test]
async fn test_an_attribution_change_is_refused_once_an_ended_subscription_invoiced(
    #[future] test_env: TestEnv,
) {
    use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
    let owned = test_env.await;
    let env = &owned;
    let today = Utc::now().date_naive();
    let key_a = api_key_resource(env.store(), TENANT_ID, CUST_UBER_ID, "key-A").await;
    let type_id: common_domain::ids::ResourceTypeId = {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_models::schema::resource::dsl as r;
        r::resource
            .filter(r::id.eq(key_a))
            .select(r::resource_type_id)
            .first(&mut env.conn().await)
            .await
            .unwrap()
    };
    let update = |attributed_to: Option<MetricAttribution>| {
        env.services().update_billable_metric(
            Actor::System,
            METRIC_BANDWIDTH,
            TENANT_ID,
            attribute(attributed_to),
        )
    };
    let to_type = || {
        update(Some(MetricAttribution {
            type_id,
            property: None,
        }))
    };
    update(None).await.expect("nothing sold yet");
    let subscribe = |customer, resource_id| {
        env.services().insert_subscription(
            Actor::System,
            bandwidth_request(
                customer,
                today - Days::new(10),
                SubscriptionActivationCondition::OnStart,
                resource_id,
            ),
            TENANT_ID,
        )
    };
    let customer_level = subscribe(CUST_UBER_ID, None).await.unwrap().id;
    let elsewhere = subscribe(CUST_SPOTIFY_ID, None).await.unwrap().id;
    let for_key = subscribe(CUST_UBER_ID, Some(key_a)).await.unwrap().id;
    sql(
        env,
        "UPDATE subscription SET status = 'CANCELLED' WHERE id = $1",
        for_key,
    )
    .await;
    // Each one's invoice still open: the last one of the ended subscription, in its grace.
    for sub in [customer_level, elsewhere, for_key] {
        sql(
            env,
            "UPDATE invoice SET status = 'DRAFT' WHERE subscription_id = $1",
            sub,
        )
        .await;
    }
    let drafts = |sub| async move {
        #[derive(diesel::QueryableByName)]
        struct Drafts {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            n: i64,
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            unmarked: i64,
        }
        diesel::sql_query(
            "SELECT count(*) AS n, count(*) FILTER (WHERE data_updated_at IS NULL) AS unmarked \
             FROM invoice WHERE subscription_id = $1 AND status = 'DRAFT'",
        )
        .bind::<diesel::sql_types::Uuid, _>(SubscriptionId::as_uuid(&sub))
        .get_result::<Drafts>(&mut env.conn().await)
        .await
        .map(|d| (d.n, d.unmarked))
        .unwrap()
    };
    let fixed = |refused: error_stack::Report<StoreError>| {
        assert!(
            matches!(refused.current_context(), StoreError::InvalidArgument(m) if m.contains("can no longer change")),
            "{refused:?}"
        );
    };

    fixed(to_type().await.expect_err("to the type"));
    env.conn()
        .await
        .batch_execute(&format!(
            "UPDATE billable_metric SET attributed_to_type_id = '{}' WHERE id = '{}'",
            type_id.as_uuid(),
            METRIC_BANDWIDTH.as_uuid()
        ))
        .await
        .unwrap();
    fixed(update(None).await.expect_err("to the customer"));

    sql(
        env,
        "UPDATE invoice SET status = 'FINALIZED', line_items = (SELECT \
         COALESCE(jsonb_agg(l), '[]') FROM jsonb_array_elements(line_items) l \
         WHERE l ->> 'metric_id' IS NULL) WHERE subscription_id = $1",
        for_key,
    )
    .await;
    fixed(
        update(None)
            .await
            .expect_err("its draft finalized, billing none of it"),
    );

    sql(
        env,
        "UPDATE invoice SET status = 'VOID' WHERE subscription_id = $1",
        for_key,
    )
    .await;
    env.conn()
        .await
        .batch_execute("UPDATE invoice SET data_updated_at = now() WHERE status = 'DRAFT'")
        .await
        .unwrap();
    update(None).await.expect("its invoice voided");
    let (n, unmarked) = drafts(customer_level).await;
    assert!(n > 0);
    assert_eq!(
        unmarked, n,
        "its customer's drafts wait for the draft worker"
    );
    assert_eq!(
        drafts(elsewhere).await.1,
        0,
        "another customer's are untouched"
    );
}

/// Consolidated, the customer-level subscription's month is invoiced by its parent while its own
/// document stays a draft: its ground still counts, so a cancellation dated over it is refused.
/// Rules: R-143
#[tokio::test]
async fn test_a_cancellation_over_consolidated_ground_is_refused() {
    let stack = Stack::start().await;
    stack.consolidate().await;
    let today = scenario_day();
    let customer = stack.customer("Consolidated").await;
    let customer_start = today - Months::new(1);
    stack
        .ingest(
            vec![Tokens {
                customer,
                at: at(today - Days::new(10), 1),
                api_key: None,
                model: "standard",
                tokens: 1_000_000,
            }],
            range(customer_start, today + Days::new(2)),
        )
        .await;
    stack.grace_hours(0).await;
    let customer_level = stack
        .subscribe(customer, customer_start, None)
        .await
        .unwrap();
    stack
        .subscribe(customer, customer_start, None)
        .await
        .expect("a second, consolidated with it");
    let for_key = stack
        .subscribe(customer, today - Days::new(20), Some("key-A"))
        .await
        .unwrap();
    stack.process_cycles().await;
    {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::invoice::dsl as i;
        let consolidated: i64 = i::invoice
            .filter(i::subscription_id.eq(customer_level))
            .filter(i::consolidated_into_invoice_id.is_not_null())
            .count()
            .get_result(&mut stack.meteroid.store.get_conn().await.unwrap())
            .await
            .unwrap();
        assert!(consolidated > 0, "its month went on a consolidated invoice");
    }
    let refused = stack
        .meteroid
        .services
        .cancel_subscription(
            Actor::System,
            for_key,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(today - Days::new(15)),
        )
        .await
        .expect_err("dated over consolidated ground");
    assert!(
        matches!(refused.current_context(), StoreError::InvalidArgument(m) if m.contains("invoiced")),
        "{refused:?}"
    );
}

/// The customer-level subscription cancelled at once bills the key's usage up to that instant on
/// its final invoice; a fee added late to the key's subscription bills from that instant, not the
/// day's end: the morning once, the afternoon too.
/// Rules: R-143
#[tokio::test]
async fn test_a_late_fee_after_an_immediate_cancellation_bills_from_its_instant() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let customer = stack.customer("Cancelled at once").await;
    let today = Utc::now().date_naive();
    let customer_start = today - Months::new(1);
    stack.grace_hours(0).await;
    let customer_level = stack
        .subscribe(customer, customer_start, None)
        .await
        .unwrap();
    let (for_key, tokens) = stack
        .subscribe_unpriced(customer, today - Days::new(20))
        .await;
    stack.process_cycles().await;
    // key-A standard: 1M = €12.00 before the cancellation, billed by the customer-level one.
    stack
        .ingest(
            vec![standard(
                customer,
                Utc::now() - chrono::Duration::minutes(2),
                "key-A",
                1_000_000,
            )],
            range(today, today + Days::new(1)),
        )
        .await;
    stack
        .cancel(
            customer_level,
            Actor::System,
            CancellationEffectiveAt::Immediate,
        )
        .await;
    stack.process_cycles().await;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    // 3M = €36.00 after it.
    stack
        .ingest_over(
            vec![(standard(customer, Utc::now(), "key-A", 3_000_000), API_KEY)],
            range(today, today + Days::new(1)),
            dec!(1_000_000),
        )
        .await;
    stack.amend_adding(for_key, today, tokens).await;

    assert_eq!(stack.invoiced(customer_level, today).await, 1_200);
    assert_eq!(stack.previewed(for_key).await, 3_600);
}

/// A late fee adds a metric the customer-level subscription does not price: nothing it invoiced
/// holds that metric, so the fee bills it from its date, its other metric's ground aside.
/// Rules: R-143
#[tokio::test]
async fn test_a_late_fee_for_a_metric_no_other_prices_bills_from_its_date() {
    let stack = Stack::start().await;
    let requests = stack
        .meteroid
        .store
        .insert_billable_metric(
            Actor::System,
            BillableMetricNew {
                name: "Model requests".to_owned(),
                description: None,
                code: CODE.to_owned(),
                catalog_code: None,
                aggregation_type: BillingMetricAggregateEnum::Count,
                aggregation_key: None,
                unit_conversion_factor: None,
                unit_conversion_rounding: None,
                segmentation_matrix: None,
                filters: None,
                usage_group_key: None,
                usage_breakdown_key: None,
                attributed_to: None,
                unit_label: None,
                tenant_id: TENANT_ID,
                product_family_id: PRODUCT_FAMILY_ID,
                product_id: None,
            },
        )
        .await
        .expect("the request metric");
    let customer = stack.customer("Two metrics").await;
    let today = scenario_day();
    let customer_start = today - Months::new(1);
    // key-A, after the fee's date and before the customer-level subscription's renewal.
    stack
        .ingest(
            vec![standard(
                customer,
                at(today - Days::new(1), 1),
                "key-A",
                1_000_000,
            )],
            range(customer_start, today + Days::new(1)),
        )
        .await;
    stack.grace_hours(0).await;
    let customer_level = stack
        .subscribe(customer, customer_start, None)
        .await
        .unwrap();
    let (for_key, _) = stack
        .subscribe_unpriced(customer, today - Days::new(20))
        .await;
    stack.process_cycles().await;
    assert_eq!(stack.invoiced(customer_level, customer_start).await, 1_200);

    stack
        .amend_adding(
            for_key,
            today - Days::new(2),
            metric_component(
                "Requests",
                requests.id,
                UsagePricingModel::PerUnit { rate: dec!(1) },
            ),
        )
        .await;
    assert_eq!(stack.previewed_metric(for_key, requests.id).await, 100);
}

/// The key's subscription cancelled for a date, another sold the key from it: withdrawing that
/// cancellation, or moving it later, would sell the key twice, so both are refused.
/// Rules: R-145
#[tokio::test]
async fn test_a_cancellation_is_not_withdrawn_once_the_resource_is_sold_again() {
    let stack = Stack::start().await;
    let customer = stack.customer("Sold again").await;
    let today = scenario_day();
    let ends = today + Days::new(5);
    let first = stack
        .subscribe(customer, today - Days::new(20), Some("key-A"))
        .await
        .unwrap();
    stack
        .cancel(first, Actor::System, CancellationEffectiveAt::Date(ends))
        .await;
    stack
        .subscribe(customer, ends, Some("key-A"))
        .await
        .expect("the key sold again from the cancellation");

    let event = stack
        .meteroid
        .store
        .get_subscription_details(TENANT_ID, first)
        .await
        .unwrap()
        .pending_events
        .into_iter()
        .next()
        .expect("the pending cancellation");
    let refused = stack
        .meteroid
        .services
        .cancel_scheduled_event(Actor::System, event.id, first, TENANT_ID)
        .await
        .expect_err("withdrawn while the key is sold again");
    assert!(
        matches!(refused.current_context(), StoreError::InvalidArgument(m) if m.contains("already sold")),
        "{refused:?}"
    );
    assert_eq!(
        cancelled_events(&stack.meteroid.store, first).await,
        1,
        "a refused withdrawal keeps the cancellation's event"
    );
    let refused = stack
        .meteroid
        .services
        .cancel_subscription(
            Actor::System,
            first,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(ends + Days::new(5)),
        )
        .await
        .expect_err("moved later while the key is sold again");
    assert!(
        matches!(refused.current_context(), StoreError::InvalidArgument(m) if m.contains("already sold")),
        "{refused:?}"
    );
}

/// A cancelled subscription for a key invoiced usage its type's keys route: changing those keys
/// would move that usage, so it is refused though nothing is live.
/// Rules: R-144
#[tokio::test]
async fn test_a_type_key_change_is_refused_once_an_ended_subscription_invoiced_its_usage() {
    let stack = Stack::start().await;
    let customer = stack.customer("Keys").await;
    let today = scenario_day();
    let start = today - Months::new(1);
    stack
        .ingest(
            vec![standard(
                customer,
                at(start + Days::new(2), 1),
                "key-A",
                1_000_000,
            )],
            range(start, today + Days::new(1)),
        )
        .await;
    stack.grace_hours(0).await;
    let for_key = stack
        .subscribe(customer, start, Some("key-A"))
        .await
        .unwrap();
    stack.process_cycles().await;
    assert_eq!(stack.invoiced(for_key, start).await, 1_200);
    stack
        .cancel(for_key, Actor::System, CancellationEffectiveAt::Immediate)
        .await;
    stack.process_cycles().await;
    assert_eq!(
        stack.subscription_row(for_key).await.status,
        SubscriptionStatusEnum::Cancelled
    );

    let refused = stack
        .meteroid
        .store
        .update_resource_type(
            TENANT_ID,
            AliasOr::Alias(API_KEY.to_owned()),
            meteroid_store::repositories::resources::ResourceTypePatch {
                name: None,
                plural_name: None,
                parent_type_id: None,
                keys: Some(vec![API_KEY.to_owned()]),
                attributes: None,
                archived: None,
            },
        )
        .await
        .expect_err("its keys route invoiced usage");
    assert!(
        matches!(refused.current_context(), StoreError::InvalidArgument(m) if m.contains("keys")),
        "{refused:?}"
    );
}

/// A token pack (€10 a month, every 1M at €12) bought for the key by a debit priced before the
/// customer-level subscription renewed and settled after: its rows date from the pricing, so it
/// bills its tokens only past what that renewal invoiced. The key's event in between is billed
/// once, by the customer-level subscription.
/// Rules: R-143
#[tokio::test]
async fn test_an_add_on_settled_after_a_renewal_bills_its_metric_from_the_invoiced_ground() {
    use meteroid_store::domain::checkout_sessions::CheckoutCompletionResult;
    use meteroid_store::domain::enums::FeeTypeEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::repositories::add_ons::AddOnInterface;
    let stack = Stack::start().await;
    let pool = &stack.meteroid.store.pool;
    crate::data::payment::run_mock_payment_provider_seed(pool, false).await;
    crate::data::payment::run_customer_payment_methods_seed(pool).await;
    crate::data::payment::set_mock_charge_behavior(
        pool,
        meteroid_store::domain::connectors::MockChargeBehavior::Pending,
    )
    .await;
    let customer = CUST_UBER_ID;
    let today = scenario_day();
    let customer_start = today - Months::new(1);
    let priced_on = today - Days::new(5);
    // key-A standard: 1M = €12.00 after the pricing, 3M = €36.00 after the renewal.
    stack
        .ingest(
            vec![
                standard(
                    customer,
                    at(priced_on + Days::new(2), 1),
                    "key-A",
                    1_000_000,
                ),
                standard(
                    customer,
                    Utc::now() - chrono::Duration::minutes(1),
                    "key-A",
                    3_000_000,
                ),
            ],
            range(customer_start, today + Days::new(2)),
        )
        .await;
    stack.grace_hours(0).await;

    common_clock::test::set(at(priced_on, 12));
    let customer_level = stack
        .subscribe(customer, customer_start, None)
        .await
        .unwrap();
    let key_a = stack.resource(customer, "key-A").await;
    let mut request = stack.request(customer, today - Days::new(20), Some(key_a));
    request
        .price_components
        .as_mut()
        .expect("components")
        .extra_components
        .clear();
    let for_key = stack.create(request).await.unwrap().id;
    let pack = stack
        .meteroid
        .store
        .create_add_on_from_ref(
            "Token pack".to_string(),
            ProductRef::New {
                name: "Token pack".to_string(),
                fee_type: FeeTypeEnum::Capacity,
                fee_structure: FeeStructure::Capacity {
                    metric_id: stack.metric.id,
                },
            },
            PriceEntry::New(PriceInput {
                cadence: meteroid_store::domain::BillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: Pricing::Capacity {
                    rate: dec!(10),
                    included: 0,
                    overage_rate: dec!(12),
                },
                credit_unit: None,
            }),
            None,
            true,
            Some(10),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("the token pack")
        .id;
    let session = stack
        .meteroid
        .services
        .create_addon_purchase_checkout_session(
            TENANT_ID,
            for_key,
            pack,
            PLAN_VERSION_1_LEETCODE_ID,
            customer,
            None,
        )
        .await
        .expect("the checkout")
        .id;
    let complete = |amount| {
        stack.meteroid.services.complete_checkout(
            TENANT_ID,
            session,
            CUST_UBER_PAYMENT_METHOD_ID,
            amount,
            "EUR".to_owned(),
            None,
        )
    };
    let quoted: u64 = {
        let mismatch = format!("{:?}", complete(0).await.expect_err("its amount asked"));
        mismatch
            .split("server computed ")
            .nth(1)
            .and_then(|rest| rest.split(',').next())
            .and_then(|amount| amount.trim().parse().ok())
            .expect("the amount the server computed")
    };
    let CheckoutCompletionResult::AwaitingPayment { transaction, .. } =
        complete(quoted).await.expect("the debit")
    else {
        panic!("a debit settling later");
    };
    common_clock::test::set(Utc::now());
    stack.process_cycles().await;
    assert_eq!(
        stack.invoiced(customer_level, customer_start).await,
        1_200,
        "the renewal invoiced the key's event after the pricing"
    );

    let mut transaction = transaction;
    {
        use diesel_async::RunQueryDsl;
        diesel::sql_query(
            "UPDATE payment_transaction SET status = 'SETTLED', processed_at = now() WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(transaction.id.as_uuid())
        .execute(&mut stack.meteroid.store.get_conn().await.unwrap())
        .await
        .unwrap();
    }
    transaction.status = meteroid_store::domain::PaymentStatusEnum::Settled;
    transaction.processed_at = Some(Utc::now().naive_utc());
    stack
        .meteroid
        .services
        .on_payment_transaction_settled(transaction.into())
        .await
        .expect("the settlement");

    let bought = stack
        .meteroid
        .store
        .get_subscription_details(TENANT_ID, for_key)
        .await
        .unwrap()
        .add_ons
        .into_iter()
        .next()
        .expect("the pack");
    assert_eq!(bought.effective_from, priced_on, "dated from its pricing");
    let floor: Option<DateTime<Utc>> = {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::subscription_add_on::dsl as a;
        a::subscription_add_on
            .filter(a::id.eq(bought.id))
            .select(a::usage_from)
            .first(&mut stack.meteroid.store.get_conn().await.unwrap())
            .await
            .unwrap()
    };
    assert_eq!(
        floor,
        Some(at(today, 0)),
        "its tokens count from the renewal"
    );
    assert_eq!(
        stack.previewed(for_key).await,
        3_600,
        "the event the renewal invoiced is not billed again"
    );
    assert_eq!(stack.invoiced(customer_level, customer_start).await, 1_200);
}

/// A customer-level subscription activated on checkout days after its start bills its usage from
/// its start date, as before resources: the 1M before the payment too.
#[tokio::test]
async fn test_a_customer_level_checkout_subscription_bills_usage_from_its_start() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    crate::data::payment::run_mock_payment_provider_seed(&stack.meteroid.store.pool, false).await;
    crate::data::payment::run_customer_payment_methods_seed(&stack.meteroid.store.pool).await;
    let customer = CUST_UBER_ID;
    let today = Utc::now().date_naive();
    let start = today - Days::new(10);
    // Standard tokens: 1M = €12.00 before the payment, 3M = €36.00 after it.
    stack
        .ingest(
            vec![standard(customer, at(start, 1), "key-A", 1_000_000)],
            range(start, today),
        )
        .await;
    let mut request = stack.request(customer, start, None);
    request.subscription.activation_condition = SubscriptionActivationCondition::OnCheckout;
    let pending = stack.create(request).await.expect("the subscription").id;
    stack
        .meteroid
        .services
        .complete_subscription_checkout_tx(
            &mut stack.meteroid.store.get_conn().await.unwrap(),
            TENANT_ID,
            pending,
            CUST_UBER_PAYMENT_METHOD_ID,
            0,
            "EUR".to_owned(),
            None,
        )
        .await
        .expect("the checkout, usage only");
    stack
        .ingest(
            vec![standard(customer, Utc::now(), "key-A", 3_000_000)],
            range(today, today + Days::new(1)),
        )
        .await;
    assert_eq!(
        stack.subscription_row(pending).await.status,
        SubscriptionStatusEnum::Active
    );
    assert_eq!(stack.previewed(pending).await, 4_800);
}

/// Ended, a subscription's claim on usage is settled: cancelling it again, earlier or later, is
/// refused as a conflict, and its ending stays where it was.
/// Rules: R-141
#[tokio::test]
async fn test_an_ended_subscription_is_not_cancelled_again() {
    let stack = Stack::start().await;
    let today = scenario_day();
    let customer = stack.customer("Ended").await;
    let for_key = stack
        .subscribe(customer, today - Days::new(20), Some("key-A"))
        .await
        .unwrap();
    let ended_on = today - Days::new(2);
    stack
        .cancel(
            for_key,
            Actor::System,
            CancellationEffectiveAt::Date(ended_on),
        )
        .await;
    stack.process_cycles().await;
    assert_eq!(
        stack.subscription_row(for_key).await.status,
        SubscriptionStatusEnum::Cancelled
    );
    for again in [today - Days::new(10), today + Days::new(5)] {
        let refused = stack
            .meteroid
            .services
            .cancel_subscription(
                Actor::System,
                for_key,
                TENANT_ID,
                None,
                CancellationEffectiveAt::Date(again),
            )
            .await
            .expect_err("ended");
        assert!(
            matches!(refused.current_context(), StoreError::Conflict(m) if m.contains("has ended")),
            "{refused:?}"
        );
    }
    let ending: chrono::NaiveDateTime = {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::subscription_ending::dsl as e;
        e::subscription_ending
            .filter(e::subscription_id.eq(for_key))
            .filter(e::kind.eq("CANCEL"))
            .select(e::at)
            .first(&mut stack.meteroid.store.get_conn().await.unwrap())
            .await
            .unwrap()
    };
    assert_eq!(ending, ended_on.and_time(NaiveTime::MIN));
}

/// The customer-level subscription renewed over a month without the key's usage, or any: its
/// invoice bills no token line, yet it closed that month's usage. A cancellation of the key's
/// subscription dated inside it would hand it back to a month invoiced already: refused, whether
/// the two renew on the same day or not.
/// Rules: R-143
async fn a_cancellation_over_a_month_invoiced_without_usage_is_refused(same_anchor: bool) {
    let stack = Stack::start().await;
    let today = scenario_day();
    let customer = stack.customer("Quiet month").await;
    let customer_start = today - Months::new(1);
    stack.grace_hours(0).await;
    let customer_level = stack
        .subscribe(customer, customer_start, None)
        .await
        .unwrap();
    let anchor = same_anchor.then(|| customer_start.day() as u16);
    let for_key = stack
        .subscribe_with(
            customer,
            today - Days::new(20),
            None,
            anchor,
            false,
            Some("key-A"),
        )
        .await
        .unwrap();
    stack.process_cycles().await;
    assert_eq!(
        stack
            .subscription_row(customer_level)
            .await
            .current_period_start,
        today
    );
    assert_eq!(stack.invoiced(customer_level, customer_start).await, 0);
    let refused = stack
        .meteroid
        .services
        .cancel_subscription(
            Actor::System,
            for_key,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(today - Days::new(15)),
        )
        .await
        .expect_err("dated inside a month invoiced without usage");
    assert!(
        matches!(refused.current_context(), StoreError::InvalidArgument(m) if m.contains("invoiced")),
        "{refused:?}"
    );
}

#[tokio::test]
async fn test_a_cancellation_over_a_month_invoiced_without_usage_is_refused() {
    a_cancellation_over_a_month_invoiced_without_usage_is_refused(false).await;
}

#[tokio::test]
async fn test_a_cancellation_over_a_month_invoiced_without_usage_on_its_anchor_is_refused() {
    a_cancellation_over_a_month_invoiced_without_usage_is_refused(true).await;
}

/// The customer-level subscription's invoice put back in draft and edited by hand: nothing
/// recomputes it, so the usage it bills is invoiced, and a cancellation dated over it is refused.
/// Rules: R-143
#[tokio::test]
async fn test_a_cancellation_over_a_draft_edited_by_hand_is_refused() {
    use diesel_async::SimpleAsyncConnection;
    let stack = Stack::start().await;
    let today = scenario_day();
    let for_key = cancelled_after_invoiced_ground(&stack, "Edited", today + Days::new(5)).await;
    stack.process_cycles().await;
    stack
        .meteroid
        .store
        .get_conn()
        .await
        .unwrap()
        .batch_execute(&format!(
            "UPDATE invoice SET status = 'DRAFT', manually_edited_at = now()
             WHERE status = 'FINALIZED' AND customer_id =
                 (SELECT customer_id FROM subscription WHERE id = '{}')",
            for_key.as_uuid()
        ))
        .await
        .unwrap();
    let refused = stack
        .meteroid
        .services
        .cancel_subscription(
            Actor::System,
            for_key,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(today - Days::new(15)),
        )
        .await
        .expect_err("dated over a draft edited by hand");
    assert!(
        matches!(refused.current_context(), StoreError::InvalidArgument(m) if m.contains("invoiced")),
        "{refused:?}"
    );
}

/// A subscription for a resource ended before its ending was recorded as a fact (no CANCEL) ends
/// its claim at its termination: the key's usage after it is the customer-level subscription's.
#[tokio::test]
async fn test_an_ended_subscription_without_its_cancellation_claims_up_to_its_termination() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let customer = stack.customer("Terminated").await;
    let today = Utc::now().date_naive();
    let start = today - Days::new(20);
    // key-A standard: 1M = €12.00 after the key's subscription terminated.
    stack
        .ingest(
            vec![standard(
                customer,
                at(today - Days::new(3), 1),
                "key-A",
                1_000_000,
            )],
            range(start, today),
        )
        .await;
    let customer_level = stack.subscribe(customer, start, None).await.unwrap();
    let for_key = stack
        .subscribe(customer, start, Some("key-A"))
        .await
        .unwrap();
    {
        use diesel_async::RunQueryDsl;
        diesel::sql_query(
            "UPDATE subscription SET status = 'CANCELLED', terminated_at = $2 WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(for_key.as_uuid())
        .bind::<diesel::sql_types::Timestamp, _>((today - Days::new(5)).and_time(NaiveTime::MIN))
        .execute(&mut stack.meteroid.store.get_conn().await.unwrap())
        .await
        .unwrap();
        diesel::sql_query("DELETE FROM subscription_ending WHERE subscription_id = $1")
            .bind::<diesel::sql_types::Uuid, _>(for_key.as_uuid())
            .execute(&mut stack.meteroid.store.get_conn().await.unwrap())
            .await
            .unwrap();
    }
    assert_eq!(stack.previewed(customer_level).await, 1_200);
}

/// Ended with neither a cancellation, a termination nor an end date (an import), a subscription
/// for a resource claims up to its current period's start, as on main: not forever.
#[tokio::test]
async fn test_an_ended_subscription_without_any_ending_claims_up_to_its_period_start() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let customer = stack.customer("Ended without a date").await;
    let today = Utc::now().date_naive();
    let start = today - Days::new(20);
    // key-A standard: 1M = €12.00 after the key's subscription's last period start.
    stack
        .ingest(
            vec![standard(
                customer,
                at(today - Days::new(3), 1),
                "key-A",
                1_000_000,
            )],
            range(start, today),
        )
        .await;
    let customer_level = stack.subscribe(customer, start, None).await.unwrap();
    let for_key = stack
        .subscribe(customer, start, Some("key-A"))
        .await
        .unwrap();
    {
        use diesel_async::RunQueryDsl;
        diesel::sql_query(
            "UPDATE subscription SET status = 'CANCELLED', terminated_at = NULL, end_date = NULL,
                 current_period_start = $2
             WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(for_key.as_uuid())
        .bind::<diesel::sql_types::Date, _>(today - Days::new(5))
        .execute(&mut stack.meteroid.store.get_conn().await.unwrap())
        .await
        .unwrap();
        diesel::sql_query("DELETE FROM subscription_ending WHERE subscription_id = $1")
            .bind::<diesel::sql_types::Uuid, _>(for_key.as_uuid())
            .execute(&mut stack.meteroid.store.get_conn().await.unwrap())
            .await
            .unwrap();
    }
    assert_eq!(stack.previewed(customer_level).await, 1_200);
}

/// A rollback drops the usage floors: while a live subscription's row has one, it would bill the
/// usage before it again, so the billing down migration refuses; once that subscription ended, it
/// runs.
#[rstest]
#[tokio::test]
async fn test_a_rollback_is_refused_while_a_live_row_has_a_usage_floor(
    #[future] test_env: TestEnv,
) {
    use diesel_async::SimpleAsyncConnection;
    let env = test_env.await;
    let floored = insert_bandwidth(&env).await;
    sql(
        &env,
        "UPDATE subscription_component SET usage_from = now() WHERE subscription_id = $1",
        floored,
    )
    .await;
    let down =
        include_str!("../../migrations/diesel/2026-11-26-011852_entitlements_v2_billing/down.sql");
    let mut conn = env.conn().await;
    conn.batch_execute("BEGIN").await.unwrap();
    let refused = conn.batch_execute(down).await;
    conn.batch_execute("ROLLBACK").await.unwrap();
    let refused = refused.expect_err("a live floor").to_string();
    assert!(
        refused.contains(&format!("subscription {}", floored.as_uuid())),
        "{refused}"
    );

    sql(
        &env,
        "UPDATE subscription SET status = 'CANCELLED' WHERE id = $1",
        floored,
    )
    .await;
    conn.batch_execute("BEGIN").await.unwrap();
    let ran = conn.batch_execute(down).await;
    conn.batch_execute("ROLLBACK").await.unwrap();
    ran.expect("an ended subscription's floor no longer bills");
}

/// A pending On Start subscription for the key claims from its past start date: aborting it or
/// moving it off the key would hand back usage the customer-level one invoiced without it.
/// Rules: R-143
#[tokio::test]
async fn test_releasing_a_pending_claim_over_invoiced_ground_is_refused() {
    let stack = Stack::start().await;
    let today = scenario_day();
    let customer = stack.customer("Pending claim").await;
    let customer_start = today - Months::new(1);
    stack.grace_hours(0).await;
    stack
        .subscribe(customer, customer_start, None)
        .await
        .unwrap();
    let for_key = stack
        .subscribe(customer, today - Days::new(20), Some("key-A"))
        .await
        .unwrap();
    stack.process_cycles().await;
    {
        use diesel_async::RunQueryDsl;
        diesel::sql_query(
            "UPDATE subscription SET status = 'PENDING_ACTIVATION', activated_at = NULL
             WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(for_key.as_uuid())
        .execute(&mut stack.meteroid.store.get_conn().await.unwrap())
        .await
        .unwrap();
    }
    let invoiced = |refused: error_stack::Report<StoreError>| {
        assert!(
            matches!(refused.current_context(), StoreError::InvalidArgument(m) if m.contains("invoiced")),
            "{refused:?}"
        );
    };
    invoiced(
        stack
            .meteroid
            .services
            .cancel_subscription(
                Actor::System,
                for_key,
                TENANT_ID,
                None,
                CancellationEffectiveAt::Immediate,
            )
            .await
            .expect_err("aborted over invoiced ground"),
    );
    invoiced(
        stack
            .set_resource(for_key, None)
            .await
            .expect_err("moved off its key over invoiced ground"),
    );
}

/// An amendment at a period's end already past (the controller behind) is a late change: its row
/// bills the key's metric only past what the customer-level subscription invoiced since.
/// Rules: R-143
#[tokio::test]
async fn test_an_end_of_period_change_dated_in_the_past_is_a_late_change() {
    let stack = Stack::start().await;
    let customer = stack.customer("Controller behind").await;
    let today = scenario_day();
    let customer_start = today - Months::new(1);
    let start = today - Months::new(1) - Days::new(10);
    stack.grace_hours(0).await;
    stack
        .subscribe(customer, customer_start, None)
        .await
        .unwrap();
    let (for_key, tokens) = stack.subscribe_unpriced(customer, start).await;
    stack.process_cycles().await;
    // The controller has not renewed it past its first period, ended ten days ago.
    let period_end = start + Months::new(1);
    sql_on(
        &stack,
        "UPDATE subscription SET current_period_end = $2, current_period_start = $2 - interval '1 month'
         WHERE id = $1",
        for_key,
        period_end,
    )
    .await;
    stack
        .meteroid
        .services
        .schedule_amendment(
            Actor::System,
            for_key,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::EndOfPeriod,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![AddExtraComponent {
                        name: tokens.name,
                        product: tokens.product_ref,
                        price_entry: tokens.price_entry,
                        fee_structure: tokens.fee_structure,
                    }],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
        )
        .await
        .expect("the amendment at the period's end");
    let added: Uuid = {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::subscription_component::dsl as c;
        let rows: Vec<(Uuid, NaiveDate)> = c::subscription_component
            .filter(c::subscription_id.eq(for_key))
            .select((c::id, c::effective_from))
            .load(&mut stack.meteroid.store.get_conn().await.unwrap())
            .await
            .unwrap();
        rows.iter()
            .find(|(_, from)| *from == period_end)
            .unwrap_or_else(|| panic!("the row dated at the period's end: {rows:?}"))
            .0
    };
    assert_eq!(
        stack.usage_floor(added).await,
        Some(at(today, 0)),
        "what the customer-level subscription invoiced of the key is not billed again"
    );
}

async fn sql_on(stack: &Stack, query: &str, subscription_id: SubscriptionId, day: NaiveDate) {
    use diesel_async::RunQueryDsl;
    diesel::sql_query(query)
        .bind::<diesel::sql_types::Uuid, _>(subscription_id.as_uuid())
        .bind::<diesel::sql_types::Date, _>(day)
        .execute(&mut stack.meteroid.store.get_conn().await.unwrap())
        .await
        .unwrap();
}

/// The key's usage moves to a new subscription for it under the customer-level draft billing it:
/// the draft is refused a hand edit until refreshed, else it would bill that usage twice.
/// Rules: R-143
#[tokio::test]
async fn test_a_draft_whose_usage_moved_is_refreshed_before_a_hand_edit() {
    let stack = Stack::start().await;
    let customer = stack.customer("Moved under a draft").await;
    let today = scenario_day();
    let customer_start = today - Months::new(1);
    // key-A standard: 1M = €12.00, inside the new subscription's first period; lite 10M without
    // a key = €20.00, the customer-level subscription's.
    stack
        .ingest(
            vec![
                standard(customer, at(today - Days::new(3), 1), "key-A", 1_000_000),
                Tokens {
                    customer,
                    at: at(today - Days::new(4), 1),
                    api_key: None,
                    model: "lite",
                    tokens: 10_000_000,
                },
            ],
            range(customer_start, today + Days::new(2)),
        )
        .await;
    stack.grace_hours(48).await;
    let customer_level = stack
        .subscribe(customer, customer_start, None)
        .await
        .unwrap();
    stack.process_cycles().await;
    let draft = stack
        .invoices_in(customer_level, InvoiceStatusEnum::Draft)
        .await
        .into_iter()
        .find(|invoice| {
            invoice
                .line_items
                .iter()
                .any(|l| l.metric_id == Some(stack.metric.id))
        })
        .expect("the customer-level draft billing the key");
    let for_key = stack
        .subscribe(customer, today - Days::new(10), Some("key-A"))
        .await
        .expect("the key's subscription, over the draft's period");

    let discount = || meteroid_store::domain::UpdateInvoiceParams {
        memo: None,
        reference: None,
        purchase_order: None,
        due_date: None,
        line_items: None,
        discount: Some("0".to_owned()),
        customer_details: None,
        invoicing_entity_id: None,
        annexes: None,
        custom_properties: None,
    };
    let refused = stack
        .meteroid
        .services
        .update_draft_invoice(draft.id, TENANT_ID, discount())
        .await
        .expect_err("edited by hand over moved usage");
    assert!(
        matches!(refused.current_context(), StoreError::Conflict(_)),
        "{refused:?}"
    );
    stack
        .meteroid
        .services
        .refresh_invoice_data(draft.id, TENANT_ID)
        .await
        .expect("refreshed");
    stack
        .meteroid
        .services
        .update_draft_invoice(draft.id, TENANT_ID, discount())
        .await
        .expect("edited once refreshed");
    stack
        .meteroid
        .services
        .finalize_invoice(Actor::System, draft.id, TENANT_ID)
        .await
        .expect("finalized as edited");
    assert_eq!(
        stack.invoiced(customer_level, customer_start).await + stack.previewed(for_key).await,
        3_200,
        "the key's usage is billed once"
    );
}

/// The key's usage moves to a new subscription for it under the customer-level draft billing it:
/// the draft can't be waived (deleted) until refreshed, else it would forgive lines billing usage
/// it no longer claims. Waived, its period is invoiced ground: handing the key's usage back to it
/// is refused, though a draft would take it.
/// Rules: R-143, R-124
#[tokio::test]
async fn test_a_waived_draft_is_refreshed_first_and_is_invoiced_ground() {
    let stack = Stack::start().await;
    let today = scenario_day();
    let customer_start = today - Months::new(1);
    stack.grace_hours(48).await;
    let mut drafts = Vec::new();
    for name in ["Waived", "Kept as a draft"] {
        let customer = stack.customer(name).await;
        stack
            .ingest(
                vec![
                    standard(customer, at(today - Days::new(3), 1), "key-A", 1_000_000),
                    Tokens {
                        customer,
                        at: at(today - Days::new(4), 1),
                        api_key: None,
                        model: "lite",
                        tokens: 10_000_000,
                    },
                ],
                range(customer_start, today + Days::new(2)),
            )
            .await;
        let customer_level = stack
            .subscribe(customer, customer_start, None)
            .await
            .unwrap();
        stack.process_cycles().await;
        let draft = stack
            .invoices_in(customer_level, InvoiceStatusEnum::Draft)
            .await
            .into_iter()
            .find(|invoice| {
                invoice
                    .line_items
                    .iter()
                    .any(|l| l.metric_id == Some(stack.metric.id))
            })
            .expect("the customer-level draft billing the key");
        let for_key = stack
            .subscribe(customer, today - Days::new(10), Some("key-A"))
            .await
            .expect("the key's subscription, over the draft's period");
        drafts.push((draft, for_key));
    }

    let (waived, for_key) = &drafts[0];
    let delete = || {
        stack
            .meteroid
            .store
            .delete_invoice(Actor::User { id: USER_ID }, waived.id, TENANT_ID)
    };
    let refused = delete().await.expect_err("waived over moved usage");
    assert!(
        matches!(refused.current_context(), StoreError::Conflict(_)),
        "{refused:?}"
    );
    stack
        .meteroid
        .services
        .refresh_invoice_data(waived.id, TENANT_ID)
        .await
        .expect("refreshed");
    delete().await.expect("waived once refreshed");
    let cancel_back = |subscription_id: SubscriptionId| {
        stack.meteroid.services.cancel_subscription(
            Actor::System,
            subscription_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(today - Days::new(5)),
        )
    };
    let refused = cancel_back(*for_key)
        .await
        .expect_err("handed back over the waived period");
    assert!(
        matches!(refused.current_context(), StoreError::InvalidArgument(m) if m.contains("invoiced")),
        "{refused:?}"
    );

    let (_, for_key) = &drafts[1];
    cancel_back(*for_key)
        .await
        .expect("handed back to a draft, which recomputes");
}
