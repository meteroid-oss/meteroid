//! Totals of a segmented metric over months of backfilled events, against the real metering
//! stack: the total adds up every segment, as the per-segment rows billing prices do.
use crate::data::ids::{CUST_SPOTIFY_ID, CUST_UBER_ID, PRODUCT_FAMILY_ID, TENANT_ID};
use crate::{helpers, metering_it, meteroid_it};
use backon::Retryable;
use chrono::{Datelike, Duration, Months, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::CustomerId;
use metering_grpc::meteroid::metering::v1::{Event, IngestRequest, event};
use meteroid::clients::usage::MeteringUsageClient;
use meteroid_mailer::config::MailerConfig;
use meteroid_store::clients::usage::{TenantUsageGrain, UsageClient};
use meteroid_store::domain::billable_metrics::{Dimension, SegmentationMatrix};
use meteroid_store::domain::enums::BillingMetricAggregateEnum;
use meteroid_store::domain::{BillableMetric, BillableMetricNew, UsagePeriod};
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::sync::Arc;
use tonic::Request;
use uuid::Uuid;

const CODE: &str = "llm_tokens";
const MODELS: [&str; 3] = ["lite", "standard", "max"];
const HISTORY_DAYS: i64 = 200;
const OTHER_CUSTOMER_DAYS: i64 = 30;

struct Rollup {
    customer: CustomerId,
    at: NaiveDateTime,
    model: &'static str,
    tokens: i64,
}

fn tokens_of(model: &str, days_ago: i64) -> i64 {
    match model {
        "lite" => 10_000 + days_ago * 7,
        "standard" => 3_000 + days_ago * 11,
        _ => 500 + days_ago * 13,
    }
}

fn history(today: NaiveDateTime) -> Vec<Rollup> {
    let noon = |days_ago: i64| today - Duration::days(days_ago) + Duration::hours(12);
    let mut rollups: Vec<Rollup> = (1..=HISTORY_DAYS)
        .flat_map(|days_ago| {
            MODELS.into_iter().map(move |model| Rollup {
                customer: CUST_SPOTIFY_ID,
                at: noon(days_ago),
                model,
                tokens: tokens_of(model, days_ago),
            })
        })
        .collect();
    rollups.extend((1..=OTHER_CUSTOMER_DAYS).map(|days_ago| Rollup {
        customer: CUST_UBER_ID,
        at: noon(days_ago),
        model: "lite",
        tokens: 77,
    }));
    // A model outside the metric's segments is not its usage.
    rollups.push(Rollup {
        customer: CUST_SPOTIFY_ID,
        at: noon(3),
        model: "legacy",
        tokens: 999_999,
    });
    rollups
}

fn to_event(n: usize, rollup: &Rollup) -> Event {
    Event {
        id: Uuid::from_u128(n as u128 + 1).to_string(),
        code: CODE.to_owned(),
        customer_id: Some(event::CustomerId::MeteroidCustomerId(
            rollup.customer.to_string(),
        )),
        timestamp: rollup.at.and_utc().to_rfc3339(),
        properties: [
            ("model".to_owned(), rollup.model.to_owned()),
            ("tokens".to_owned(), rollup.tokens.to_string()),
        ]
        .into(),
    }
}

fn metered(rollup: &&Rollup) -> bool {
    MODELS.contains(&rollup.model)
}

fn segmented_metric(name: &str, aggregation_type: BillingMetricAggregateEnum) -> BillableMetricNew {
    BillableMetricNew {
        name: name.to_owned(),
        description: None,
        code: CODE.to_owned(),
        aggregation_type,
        aggregation_key: Some("tokens".to_owned()),
        unit_conversion_factor: None,
        unit_conversion_rounding: None,
        segmentation_matrix: Some(SegmentationMatrix::Single(Dimension {
            key: "model".to_owned(),
            values: MODELS.iter().map(|m| (*m).to_owned()).collect(),
        })),
        filters: None,
        usage_group_key: None,
        unit_label: None,
        tenant_id: TENANT_ID,
        product_family_id: PRODUCT_FAMILY_ID,
        product_id: None,
    }
}

async fn await_ingested(
    client: &MeteringUsageClient,
    metric: &BillableMetric,
    customer: CustomerId,
    period: &UsagePeriod,
    expected: u64,
) {
    (|| async {
        let count = client
            .count_events(&TENANT_ID, &customer, metric, period.clone())
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        if count == expected {
            Ok(())
        } else {
            Err(anyhow::anyhow!("{count} of {expected} events"))
        }
    })
    .retry(
        backon::ConstantBuilder::default()
            .with_delay(std::time::Duration::from_millis(200))
            .with_max_times(150),
    )
    .await
    .expect("the events reach the store");
}

fn sum_of(rollups: &[&Rollup], period: &UsagePeriod) -> Decimal {
    rollups
        .iter()
        .filter(|r| r.at >= period.start && r.at < period.end)
        .map(|r| Decimal::from(r.tokens))
        .sum()
}

fn month_start(date: NaiveDate) -> NaiveDateTime {
    date.with_day(1).unwrap().and_time(NaiveTime::MIN)
}

#[tokio::test]
async fn test_segmented_totals_over_backfilled_months() {
    helpers::init::logging();
    let postgres = meteroid_it::container::create_seeded_test_database(
        meteroid_it::container::SeedLevel::PRODUCT,
    )
    .await;
    let ((_kafka, kafka_port), (_clickhouse, ch_http_port, ch_tcp_port)) =
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
        metering_it::container::start_metering(metering_config.clone(), metering_listener).await;
    let mut ingest_clients = metering_it::clients::AllClients::from_channel(
        metering.channel.clone(),
        "pv_sand_9XzHg0EYO2Usy9ITU6bbhBnkYYbx/2vO7XtUUeQ7Wq9EZCAbBG",
        &metering_config.internal_auth,
    );
    let usage_client = Arc::new(MeteringUsageClient::from_channel(
        metering.channel.clone(),
        &metering_config.internal_auth,
    ));
    let meteroid = meteroid_it::container::start_meteroid_with_grpc_listener(
        meteroid_listener,
        metering_port,
        postgres,
        meteroid_it::container::SeedLevel::PRODUCT,
        usage_client.clone(),
        meteroid_mailer::service::mailer_service(MailerConfig::dummy()),
    )
    .await;

    let tokens = meteroid
        .store
        .insert_billable_metric(
            Actor::System,
            segmented_metric("Tokens", BillingMetricAggregateEnum::Sum),
        )
        .await
        .unwrap();
    let peak = meteroid
        .store
        .insert_billable_metric(
            Actor::System,
            segmented_metric("Peak tokens", BillingMetricAggregateEnum::Max),
        )
        .await
        .unwrap();

    let now = Utc::now().naive_utc();
    let today = now.date().and_time(NaiveTime::MIN);
    let rollups = history(today);
    let events: Vec<Event> = rollups
        .iter()
        .enumerate()
        .map(|(n, rollup)| to_event(n, rollup))
        .collect();
    for chunk in events.chunks(500) {
        let failures = ingest_clients
            .events
            .ingest(Request::new(IngestRequest {
                events: chunk.to_vec(),
                allow_backfilling: true,
            }))
            .await
            .expect("Could not ingest events")
            .into_inner()
            .failures;
        assert!(failures.is_empty(), "{failures:?}");
    }

    let first = today - Duration::days(HISTORY_DAYS);
    let span = UsagePeriod {
        start: first,
        end: now,
    };
    await_ingested(
        &usage_client,
        &tokens,
        CUST_SPOTIFY_ID,
        &span,
        (HISTORY_DAYS as u64) * MODELS.len() as u64,
    )
    .await;
    await_ingested(
        &usage_client,
        &tokens,
        CUST_UBER_ID,
        &span,
        OTHER_CUSTOMER_DAYS as u64,
    )
    .await;

    let spotify: Vec<&Rollup> = rollups
        .iter()
        .filter(|r| r.customer == CUST_SPOTIFY_ID)
        .filter(metered)
        .collect();
    let total = usage_client
        .fetch_total_usage(&TENANT_ID, &CUST_SPOTIFY_ID, &tokens, span.clone())
        .await
        .unwrap();
    assert_eq!(
        total,
        sum_of(&spotify, &span),
        "every segment of the backfilled history"
    );

    let peak_total = usage_client
        .fetch_total_usage(&TENANT_ID, &CUST_SPOTIFY_ID, &peak, span.clone())
        .await
        .unwrap();
    let expected_peak = spotify.iter().map(|r| r.tokens).max().unwrap();
    assert_eq!(
        peak_total,
        Decimal::from(expected_peak),
        "the peak across segments"
    );

    let mut month = month_start(first.date());
    let mut months = 0;
    while month < now {
        let period = UsagePeriod {
            start: month.max(first),
            end: (month + Months::new(1)).min(now),
        };
        let expected_total = sum_of(&spotify, &period);
        let total = usage_client
            .fetch_total_usage(&TENANT_ID, &CUST_SPOTIFY_ID, &tokens, period.clone())
            .await
            .unwrap();
        assert_eq!(total, expected_total, "month of {}", period.start);

        let billed: BTreeMap<String, Decimal> = usage_client
            .fetch_usage(&TENANT_ID, &CUST_SPOTIFY_ID, &tokens, period.clone())
            .await
            .unwrap()
            .data
            .into_iter()
            .map(|row| (row.dimensions["model"].clone(), row.value))
            .collect();
        let expected_billed: BTreeMap<String, Decimal> = MODELS
            .iter()
            .map(|model| {
                let of_model: Vec<&Rollup> = spotify
                    .iter()
                    .copied()
                    .filter(|r| r.model == *model)
                    .collect();
                ((*model).to_owned(), sum_of(&of_model, &period))
            })
            .filter(|(_, value)| !value.is_zero())
            .collect();
        assert_eq!(
            billed, expected_billed,
            "billed segments, month of {}",
            period.start
        );
        assert_eq!(billed.values().copied().sum::<Decimal>(), total);
        month = month + Months::new(1);
        months += 1;
    }
    assert!(months >= 6, "the history spans {months} months");

    let recent = UsagePeriod {
        start: today - Duration::days(40),
        end: today,
    };
    let points = usage_client
        .fetch_tenant_usage(&TENANT_ID, &tokens, recent.clone(), TenantUsageGrain::Day)
        .await
        .unwrap();
    let mut daily: BTreeMap<NaiveDateTime, Decimal> = BTreeMap::new();
    for point in points {
        assert!(
            daily.insert(point.window_start, point.value).is_none(),
            "one point per day, not one per segment"
        );
    }
    let all_metered: Vec<&Rollup> = rollups.iter().filter(metered).collect();
    let expected_daily: BTreeMap<NaiveDateTime, Decimal> = (1..=40)
        .map(|days_ago| {
            let day = today - Duration::days(days_ago);
            let period = UsagePeriod {
                start: day,
                end: day + Duration::days(1),
            };
            (day, sum_of(&all_metered, &period))
        })
        .collect();
    assert_eq!(daily, expected_daily);
}
