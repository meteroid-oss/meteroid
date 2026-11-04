//! Usage filters against the real stack (Kafka, ClickHouse, metering): a customer's events are
//! billed once across its subscriptions. A subscription with a usage filter bills the events
//! that match it; one without bills the events no filter claims; what no subscription bills is
//! kept and listed as unattributed usage. Scenarios S19 and S10 of the credit scenario timelines.
use crate::data::ids::{
    COMP_LEETCODE_RATE_ID, CUST_COMODO_ID, CUST_SPOTIFY_ID, CUST_UBER_ID, INVOICING_ENTITY_ID,
    ORGANIZATION_ID, PLAN_VERSION_1_LEETCODE_ID, PRODUCT_FAMILY_ID, TENANT_ID, USER_ID,
};
use crate::helpers::credit::fund;
use crate::helpers::credit_recompute::clear_of_midnight;
use crate::helpers::credit_recompute::metric_component;
use crate::metering_it::clients::AllClients as MeteringClients;
use crate::metering_it::container::MeteringSetup;
use crate::{helpers, metering_it, meteroid_it};
use backon::Retryable;
use chrono::{DateTime, Datelike, Days, Months, NaiveDate, NaiveTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, BillableMetricId, CustomerId, SubscriptionId};
use diesel_models::customers::CustomerRowNew;
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::subscriptions::SubscriptionRow;
use metering_grpc::meteroid::metering::v1::{Event, IngestRequest, event};
use meteroid::clients::usage::MeteringUsageClient;
use meteroid_mailer::config::MailerConfig;
use meteroid_store::clients::usage::UsageClient;
use meteroid_store::domain::billable_metrics::{Dimension, SegmentationMatrix};
use meteroid_store::domain::enums::BillingMetricAggregateEnum;
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::domain::invoice_annex_templates::{
    AnnexColumn, AnnexDefinition, AnnexTemplateNew, ColumnAggregation, ColumnSource, EventSource,
    EventTableDefinition,
};
use meteroid_store::domain::invoices::InvoiceAnnexCell;
use meteroid_store::domain::price_components::{MatrixDimension, MatrixRow, UsagePricingModel};
use meteroid_store::domain::scheduled_events::{ScheduledEventData, ScheduledEventNew};
use meteroid_store::domain::subscription_components::CreateSubscriptionComponents;
use meteroid_store::domain::{
    ApiTokenNew, BillableMetric, BillableMetricNew, CreateSubscription, Invoice,
    InvoicingEntityPatch, PaginationRequest, ScopeSegment, SubscriptionActivationCondition,
    SubscriptionNew, SubscriptionPatch, TenantUpdate, UsageFilter, UsagePeriod, UsageScope,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::api_tokens::ApiTokensInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::subscriptions::{
    CancellationEffectiveAt, SubscriptionInterface, SubscriptionInterfaceAuto,
};
use meteroid_store::repositories::tenants::TenantInterface;
use meteroid_testkit::annex_templates::{create_annex_template, set_plan_version_annex_templates};
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

    /// Pay as you go, model usage only: lite €2, standard €12, max €60 per 1M tokens.
    async fn subscribe(
        &self,
        customer_id: CustomerId,
        start: NaiveDate,
        usage_filter: Option<(&str, &str)>,
    ) -> Result<SubscriptionId, error_stack::Report<StoreError>> {
        self.subscribe_until(customer_id, start, None, usage_filter)
            .await
    }

    async fn subscribe_until(
        &self,
        customer_id: CustomerId,
        start: NaiveDate,
        end: Option<NaiveDate>,
        usage_filter: Option<(&str, &str)>,
    ) -> Result<SubscriptionId, error_stack::Report<StoreError>> {
        self.subscribe_with(customer_id, start, end, None, usage_filter)
            .await
    }

    async fn subscribe_with(
        &self,
        customer_id: CustomerId,
        start: NaiveDate,
        end: Option<NaiveDate>,
        billing_day_anchor: Option<u16>,
        usage_filter: Option<(&str, &str)>,
    ) -> Result<SubscriptionId, error_stack::Report<StoreError>> {
        let rate = |model: &str, euros: i64| MatrixRow {
            dimension1: MatrixDimension {
                key: "model".to_owned(),
                value: model.to_owned(),
            },
            dimension2: None,
            per_unit_price: Decimal::from(euros),
        };
        self.meteroid
            .services
            .insert_subscription(
                Actor::System,
                CreateSubscription {
                    subscription: SubscriptionNew {
                        customer_id,
                        plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                        net_terms: None,
                        invoice_memo: None,
                        invoice_threshold: None,
                        start_date: start,
                        end_date: end,
                        billing_start_date: Some(start),
                        activation_condition: SubscriptionActivationCondition::OnStart,
                        trial_duration: None,
                        billing_day_anchor,
                        payment_methods_config: None,
                        auto_advance_invoices: true,
                        charge_automatically: false,
                        purchase_order: None,
                        backdate_invoices: false,
                        skip_checkout_session: false,
                        skip_past_invoices: false,
                        usage_filter: usage_filter.map(|(property, value)| {
                            UsageFilter::new(property.to_owned(), value.to_owned()).unwrap()
                        }),
                        custom_properties: serde_json::Value::Null,
                    },
                    price_components: Some(CreateSubscriptionComponents {
                        parameterized_components: vec![],
                        overridden_components: vec![],
                        extra_components: vec![metric_component(
                            "Model usage",
                            self.metric.id,
                            UsagePricingModel::Matrix {
                                rates: vec![rate("lite", 2), rate("standard", 12), rate("max", 60)],
                            },
                        )],
                        remove_components: vec![COMP_LEETCODE_RATE_ID],
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
            .map(|created| created.id)
    }

    /// Ingests the events, then waits until metering counts every customer's tokens in `range`.
    async fn ingest(&self, events: Vec<Tokens>, range: UsagePeriod) {
        let mut expected: Vec<(CustomerId, Decimal)> = Vec::new();
        for e in &events {
            match expected
                .iter_mut()
                .find(|(customer, _)| *customer == e.customer)
            {
                Some((_, total)) => *total += Decimal::from(e.tokens),
                None => expected.push((e.customer, Decimal::from(e.tokens))),
            }
        }
        let events = events
            .into_iter()
            .map(|e| {
                let mut properties = std::collections::HashMap::from([
                    ("model".to_owned(), e.model.to_owned()),
                    ("tokens".to_owned(), e.tokens.to_string()),
                ]);
                if let Some(key) = e.api_key {
                    properties.insert("api_key".to_owned(), key.to_owned());
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

    async fn process_cycles(&self) {
        for _ in 0..3 {
            self.meteroid
                .services
                .get_and_process_cycle_transitions()
                .await
                .expect("cycle transitions");
            self.meteroid
                .services
                .get_and_process_due_events()
                .await
                .expect("due events");
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
            .filter(|line| line.metric_id == Some(self.metric.id))
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

    async fn set_filter(
        &self,
        subscription_id: SubscriptionId,
        filter: Option<(&str, &str)>,
    ) -> meteroid_store::domain::Subscription {
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
                    usage_filter: Some(filter.map(|(property, value)| {
                        UsageFilter::new(property.to_owned(), value.to_owned()).unwrap()
                    })),
                },
                Actor::System,
            )
            .await
            .expect("change the usage filter")
    }

    async fn subscription_row(&self, subscription_id: SubscriptionId) -> SubscriptionRow {
        let mut conn = self.meteroid.store.get_conn().await.unwrap();
        SubscriptionRow::get_subscription_by_id(&mut conn, &TENANT_ID, subscription_id)
            .await
            .expect("the subscription")
            .subscription
    }

    async fn finalized(&self, subscription_id: SubscriptionId) -> Vec<Invoice> {
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
            .filter(|invoice| invoice.status == InvoiceStatusEnum::Finalized)
            .collect()
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

fn range(from: NaiveDate, to: NaiveDate) -> UsagePeriod {
    UsagePeriod {
        start: from.and_time(NaiveTime::MIN),
        end: to.and_time(NaiveTime::MIN),
    }
}

/// S19 on Spotify (A filtered `api_key = key-A`, B without a filter) and its all-filtered variant
/// on Uber (B filters `key-B`), both billed over last month; S10's attribution rows on Comodo.
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
        .subscribe(CUST_SPOTIFY_ID, last_month, Some(("api_key", "key-A")))
        .await
        .expect("A");
    let s19_b = stack
        .subscribe(CUST_SPOTIFY_ID, last_month, None)
        .await
        .expect("B, accepted without a filter");
    let all_a = stack
        .subscribe(CUST_UBER_ID, last_month, Some(("api_key", "key-A")))
        .await
        .expect("A");
    let all_b = stack
        .subscribe(CUST_UBER_ID, last_month, Some(("api_key", "key-B")))
        .await
        .expect("B");
    let s10_a = stack
        .subscribe(CUST_COMODO_ID, last_month, Some(("api_key", "key-A")))
        .await
        .expect("A");
    let s10_b = stack
        .subscribe(CUST_COMODO_ID, s10_b_start, Some(("api_key", "key-B")))
        .await
        .expect("B");

    let duplicate = stack
        .subscribe(CUST_SPOTIFY_ID, today, Some(("api_key", "key-A")))
        .await
        .expect_err("a filter another live subscription of the customer uses");
    assert!(matches!(
        duplicate.current_context(),
        StoreError::InvalidArgument(_)
    ));

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

    // All filtered: E2 + E3 (12M tokens, €44.00 at the plan's prices) are billed by nobody.
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

/// The next-invoice preview and the invoice agree under usage filters, and changing a filter
/// mid-period re-attributes all of the period's usage not yet invoiced.
#[tokio::test]
async fn test_preview_and_invoice_agree_and_a_filter_change_applies_to_unbilled_usage() {
    preview_and_invoice_agree(false).await;
}

/// Periods from 40 days ago, so today is never a period's first day unless anchored on it.
async fn preview_and_invoice_agree(renewed_today: bool) {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let clock = Utc::now();
    let today = clock.date_naive();
    let start = today - Days::new(40);
    let anchor = renewed_today.then(|| today.day() as u16);
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

    let a = stack
        .subscribe_with(
            CUST_SPOTIFY_ID,
            start,
            None,
            anchor,
            Some(("api_key", "key-A")),
        )
        .await
        .expect("A");
    let b = stack
        .subscribe_with(CUST_SPOTIFY_ID, start, None, anchor, None)
        .await
        .expect("B");
    if renewed_today {
        stack.grace_hours(0).await;
    }
    stack.process_cycles().await;
    for subscription in [a, b] {
        let period = stack
            .subscription_row(subscription)
            .await
            .current_period_start;
        assert_eq!(
            period == today,
            renewed_today,
            "the open period starts {period}"
        );
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

    // A now bills key-B: the whole open period follows the new filter, key-A falls back to B.
    stack.set_filter(a, Some(("api_key", "key-B"))).await;
    assert_eq!(stack.previewed(a).await, 2_400);
    assert_eq!(stack.previewed(b).await, 1_400);

    // Ending both now bills the period so far: each invoice is its preview. A negative grace
    // settles the termination inline, so the closing invoices finalize within the test.
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
    }
    stack.process_cycles().await;
    assert_eq!(stack.invoiced(a, start).await, 2_400);
    assert_eq!(stack.invoiced(b, start).await, 1_400);
}

/// A filtered subscription's claim ends with it: cancelled at a date, paused, or cancelled at once
/// before its fixed term. Its key's later events fall back to the unfiltered subscription.
#[tokio::test]
async fn test_a_usage_filter_claims_nothing_once_its_subscription_ended() {
    clear_of_midnight(600).await;
    let stack = Stack::start().await;
    let today = Utc::now().date_naive();
    let this_month = month_start(today);
    let last_month = this_month - Months::new(1);
    let next_month = this_month + Months::new(1);

    let cancelled = stack.customer("Cancelled at a date").await;
    let paused = stack.customer("Paused").await;
    let fixed_term = stack.customer("Fixed term cancelled at once").await;
    let mut pairs = Vec::new();
    for (customer, end) in [
        (cancelled, None),
        (paused, None),
        (fixed_term, Some(next_month + Months::new(1))),
    ] {
        let a = stack
            .subscribe_until(customer, last_month, end, Some(("api_key", "key-A")))
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
    {
        let mut conn = stack.meteroid.store.get_conn().await.unwrap();
        stack
            .meteroid
            .store
            .schedule_events(
                &mut conn,
                vec![ScheduledEventNew {
                    subscription_id: pairs[1].1,
                    tenant_id: TENANT_ID,
                    scheduled_time: this_month.and_time(NaiveTime::MIN),
                    event_data: ScheduledEventData::PauseSubscription,
                    source: "test".to_owned(),
                    created_by: Actor::System,
                }],
            )
            .await
            .expect("pause A at the start of this month");
    }
    stack.process_cycles().await;
    stack
        .meteroid
        .services
        .cancel_subscription(
            Actor::System,
            pairs[2].1,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await
        .expect("end the fixed-term A now");
    stack.process_cycles().await;
    for (_, a, _) in &pairs {
        assert!(matches!(
            stack.subscription_row(*a).await.status,
            SubscriptionStatusEnum::Cancelled | SubscriptionStatusEnum::Paused
        ));
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

/// A filter change applies from what neither subscription has invoiced: A (periods from 25 days
/// ago) changes key-A to key-B after B (periods from 40 days ago, already invoiced once) billed
/// its first period. Nothing invoiced moves: each event is billed once.
#[tokio::test]
async fn test_a_usage_filter_change_never_moves_invoiced_usage() {
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

    let changed = stack.customer("Filter changed").await;
    let removed = stack.customer("Filter removed").await;
    let backdated = stack.customer("Backdated filter").await;
    let mut events = Vec::new();
    for customer in [changed, removed] {
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
    stack
        .ingest(events, range(b_start, today + Days::new(1)))
        .await;

    let mut pairs = Vec::new();
    for customer in [changed, removed] {
        let a = stack
            .subscribe(customer, a_start, Some(("api_key", "key-A")))
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

    let (a, b) = pairs[0];
    let b_period = stack.subscription_row(b).await.current_period_start;
    let updated = stack.set_filter(a, Some(("api_key", "key-B"))).await;
    assert_eq!(
        updated.usage_filter_since,
        Some(b_period.and_time(NaiveTime::MIN)),
        "the change applies from B's open period, not A's"
    );
    // A: E1 under key-A before the change, E4 under key-B after; B: E3, no longer claimed.
    assert_eq!(stack.previewed(a).await, 6_000);
    assert_eq!(stack.previewed(b).await, 3_600);

    let (a, b) = pairs[1];
    let updated = stack.set_filter(a, None).await;
    assert_eq!(
        updated.usage_filter_since,
        Some(
            stack
                .subscription_row(b)
                .await
                .current_period_start
                .and_time(NaiveTime::MIN)
        )
    );
    // A: E1, then without a filter E3 + E4; B, unfiltered, also bills E3 + E4.
    assert_eq!(stack.previewed(a).await, 9_600);
    assert_eq!(stack.previewed(b).await, 8_400);

    // A filter cannot reach back into usage B already invoiced.
    let b = stack.subscribe(backdated, b_start, None).await.expect("B");
    stack.process_cycles().await;
    let refused = stack
        .subscribe(backdated, a_start, Some(("api_key", "key-A")))
        .await
        .expect_err("a filtered subscription starting before B's invoiced usage");
    assert!(matches!(
        refused.current_context(),
        StoreError::InvalidArgument(_)
    ));
    let b_period = stack.subscription_row(b).await.current_period_start;
    stack
        .subscribe(backdated, b_period, Some(("api_key", "key-A")))
        .await
        .expect("a filtered subscription starting where B's open period starts");
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

    /// The month's usage a subscription filtered on `key` reads, once the stored copy is readable.
    async fn read_key(&self, customer: CustomerId, month: NaiveDate, key: &str) -> Decimal {
        let scoped = |key: &str| BillableMetric {
            usage_scope: UsageScope {
                segments: vec![ScopeSegment {
                    from: None,
                    until: None,
                    matching: Some(UsageFilter::new("api_key".to_owned(), key.to_owned()).unwrap()),
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
            .subscribe(customer, month, Some(("api_key", "key-A")))
            .await
            .expect("A");
        let b = self
            .subscribe(customer, month, Some(("api_key", "key-B")))
            .await
            .expect("B");
        // Last month's invoices are issued when it closes: on the 1st they would still be in grace.
        self.grace_hours(0).await;
        self.process_cycles().await;
        assert_eq!(self.invoiced(a, month).await, a_cents);
        assert_eq!(self.invoiced(b, month).await, b_cents);
    }
}
