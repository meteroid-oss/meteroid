//! Deterministic synthetic usage-event generation for seeded sandbox tenants.
//!
//! Pure and side-effect free: given a metric stream and a set of customers with
//! their active windows, it produces [`GeneratedEvent`]s with realistic diurnal
//! and weekly seasonality plus a growth trend. Callers map these to the metering
//! ingest format and send them best-effort. Generation is fully determined by the
//! per-customer `seed`, so re-runs are stable and unit-testable.

use chrono::{Datelike, NaiveDate, NaiveDateTime, Weekday};
use common_domain::ids::{BaseId, CustomerId, TenantId};
use metering_grpc::meteroid::metering::v1::Event;
use metering_grpc::meteroid::metering::v1::event::CustomerId as EventCustomerId;
use meteroid_store::clients::usage::{IngestEventsRequest, UsageClient};
use std::sync::Arc;

/// Safety bound so a mis-specified stream can never flood ClickHouse.
const MAX_EVENTS_PER_CUSTOMER: usize = 8_000;

#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedEvent {
    pub id: String,
    pub code: String,
    pub customer_id: CustomerId,
    pub timestamp: NaiveDateTime,
    pub properties: Vec<(String, String)>,
}

/// Shape of a metric's synthetic usage.
#[derive(Clone)]
pub enum UsageShape {
    /// Many discrete events per day, each carrying a value under `value_key`.
    /// Suits Sum/Count metrics (e.g. `api_calls`).
    Counter {
        value_key: String,
        per_event_min: f64,
        per_event_max: f64,
    },
    /// One reading per day carrying a slowly drifting level under `value_key`.
    /// Suits Max/Latest/Mean metrics (e.g. `storage_gb`).
    Gauge {
        value_key: String,
        floor: f64,
        ceil: f64,
    },
}

/// An optional categorical dimension attached to each event for breakdowns.
#[derive(Clone)]
pub struct Dimension {
    pub key: String,
    pub values: Vec<String>,
}

/// One metric's generation parameters, shared across customers.
#[derive(Clone)]
pub struct UsageStream {
    pub metric_code: String,
    pub shape: UsageShape,
    /// Baseline events/day for a customer with `scale == 1.0` (Counter only).
    pub base_daily_volume: u32,
    /// Multiplicative growth applied per 30-day step across the window.
    pub monthly_growth: f64,
    pub dimensions: Vec<Dimension>,
}

impl UsageStream {
    pub fn value_key(&self) -> &str {
        match &self.shape {
            UsageShape::Counter { value_key, .. } | UsageShape::Gauge { value_key, .. } => {
                value_key
            }
        }
    }
}

/// A customer's active window and relative size for a stream.
#[derive(Clone)]
pub struct UsageCustomer {
    pub customer_id: CustomerId,
    pub scale: f64,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub seed: u64,
}

/// Generate all events for one metric stream across every customer.
pub fn generate_stream(stream: &UsageStream, customers: &[UsageCustomer]) -> Vec<GeneratedEvent> {
    let mut out = Vec::new();
    for c in customers {
        generate_for_customer(stream, c, &mut out);
    }
    out
}

fn generate_for_customer(stream: &UsageStream, c: &UsageCustomer, out: &mut Vec<GeneratedEvent>) {
    if c.end < c.start {
        return;
    }
    let total_days = (c.end - c.start).num_days().max(0);
    let mut produced = 0usize;
    let mut day = c.start;
    let mut day_idx: i64 = 0;

    while day <= c.end && produced < MAX_EVENTS_PER_CUSTOMER {
        let months = day_idx as f64 / 30.0;
        // Per-customer growth variance so accounts ramp at different rates; the
        // aggregate usage curve is then bumpy rather than one smooth exponential.
        let growth = effective_monthly_growth(stream.monthly_growth, c.seed).powf(months);
        // Calendar seasonality (summer/December lulls, an autumn peak), shared across
        // customers, so month-over-month usage revenue isn't monotonically smooth.
        let season = seasonal_factor(day);
        let weekday_factor = match day.weekday() {
            Weekday::Sat | Weekday::Sun => 0.35,
            _ => 1.0,
        };
        let epoch_day = epoch_day(day);
        // Seeded on the absolute date so a re-seed re-sends the same values for an id.
        let mut rng = Rng::new(c.seed ^ (epoch_day as u64).wrapping_mul(0x0000_0100_0000_01B3));

        match &stream.shape {
            UsageShape::Counter {
                value_key,
                per_event_min,
                per_event_max,
            } => {
                let expected =
                    stream.base_daily_volume as f64 * c.scale * growth * weekday_factor * season;
                // Occasional usage spikes (a launch, a batch job) punctuate the trend.
                let spike = if rng.unit() < 0.04 {
                    rng.range(2.0, 3.5)
                } else {
                    1.0
                };
                let count = (expected * spike * rng.range(0.7, 1.3)).round().max(0.0) as i64;
                for i in 0..count {
                    if produced >= MAX_EVENTS_PER_CUSTOMER {
                        break;
                    }
                    let ts = at_time(day, &mut rng);
                    let value = rng.range(*per_event_min, *per_event_max);
                    out.push(GeneratedEvent {
                        id: event_id(&stream.metric_code, c.seed, epoch_day, i),
                        code: stream.metric_code.clone(),
                        customer_id: c.customer_id,
                        timestamp: ts,
                        properties: props(value_key, value, &stream.dimensions, &mut rng),
                    });
                    produced += 1;
                }
            }
            UsageShape::Gauge {
                value_key,
                floor,
                ceil,
            } => {
                let progress = if total_days > 0 {
                    day_idx as f64 / total_days as f64
                } else {
                    1.0
                };
                let base = floor + (ceil - floor) * progress;
                let value = (base * c.scale * growth * season * rng.range(0.9, 1.1)).max(0.0);
                let ts = day.and_hms_opt(12, 0, 0).expect("valid midday");
                out.push(GeneratedEvent {
                    id: event_id(&stream.metric_code, c.seed, epoch_day, 0),
                    code: stream.metric_code.clone(),
                    customer_id: c.customer_id,
                    timestamp: ts,
                    properties: props(value_key, value, &stream.dimensions, &mut rng),
                });
                produced += 1;
            }
        }

        day = match day.succ_opt() {
            Some(d) => d,
            None => break,
        };
        day_idx += 1;
    }
}

fn props(
    value_key: &str,
    value: f64,
    dimensions: &[Dimension],
    rng: &mut Rng,
) -> Vec<(String, String)> {
    let mut p = Vec::with_capacity(1 + dimensions.len());
    p.push((value_key.to_string(), format!("{value:.4}")));
    for dim in dimensions {
        if !dim.values.is_empty() {
            let idx = (rng.unit() * dim.values.len() as f64) as usize;
            let idx = idx.min(dim.values.len() - 1);
            p.push((dim.key.clone(), dim.values[idx].clone()));
        }
    }
    p
}

/// Business-hours-biased timestamp within `day` (peaks early afternoon UTC).
fn at_time(day: NaiveDate, rng: &mut Rng) -> NaiveDateTime {
    let hour = (7.0 + rng.range(0.0, 11.0) * rng.unit().powf(0.5)).min(23.0) as u32;
    let minute = (rng.unit() * 60.0) as u32;
    let second = (rng.unit() * 60.0) as u32;
    day.and_hms_opt(hour.min(23), minute.min(59), second.min(59))
        .expect("valid time")
}

fn epoch_day(day: NaiveDate) -> i64 {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid epoch");
    day.signed_duration_since(epoch).num_days()
}

/// Keyed on the absolute date so a re-seed on another day re-sends an id only for the same date.
fn event_id(code: &str, seed: u64, epoch_day: i64, i: i64) -> String {
    format!("sbx-{code}-{seed:016x}-{epoch_day}-{i}")
}

/// Default breakdown dimension so usage charts have a non-trivial split.
fn default_dimensions() -> Vec<Dimension> {
    vec![Dimension {
        key: "region".to_string(),
        values: vec![
            "us-east".to_string(),
            "eu-west".to_string(),
            "ap-south".to_string(),
        ],
    }]
}

/// Per-customer monthly growth: the stream's base rate nudged by a deterministic
/// per-seed offset in ~[-0.01, +0.05] (floored at flat), so accounts expand at
/// different rates and the aggregate curve isn't a single smooth exponential.
fn effective_monthly_growth(base: f64, seed: u64) -> f64 {
    let unit = (seed % 1000) as f64 / 1000.0; // deterministic [0, 1)
    (base + (unit * 0.06 - 0.01)).max(1.0)
}

/// Calendar-seasonality multiplier (~0.82..=1.18) shared across customers: a yearly
/// sine (autumn peak, summer trough) plus a December holiday dip, so month-over-month
/// usage has realistic waves instead of a monotonic ramp.
fn seasonal_factor(day: NaiveDate) -> f64 {
    let phase = (day.ordinal() as f64 / 365.0) * std::f64::consts::TAU;
    let yearly = 1.0 + 0.15 * (phase - 1.4).sin();
    let december_dip = if day.month() == 12 { 0.92 } else { 1.0 };
    (yearly * december_dip).clamp(0.8, 1.2)
}

/// Build a stream for a metric. Gauge metrics (Max/Min/Mean/Latest) emit one
/// drifting reading per day; everything else emits many small counter events.
pub fn stream_for(
    metric_code: &str,
    value_key: &str,
    is_gauge: bool,
    base_daily_volume: u32,
) -> UsageStream {
    let shape = if is_gauge {
        UsageShape::Gauge {
            value_key: value_key.to_string(),
            floor: 5.0,
            ceil: 500.0,
        }
    } else {
        UsageShape::Counter {
            value_key: value_key.to_string(),
            per_event_min: 1.0,
            per_event_max: 8.0,
        }
    };
    UsageStream {
        metric_code: metric_code.to_string(),
        shape,
        base_daily_volume,
        monthly_growth: 1.03,
        dimensions: default_dimensions(),
    }
}

/// Deterministic per-customer generation seed for a metric.
pub fn seed_for(id: CustomerId, code: &str) -> u64 {
    fnv1a(&format!("{}:{}", id.as_base62(), code))
}

/// Deterministic per-customer usage multiplier in ~[0.5, 3.0].
pub fn scale_for(id: CustomerId) -> f64 {
    0.5 + 2.5 * ((fnv1a(&format!("scale:{}", id.as_base62())) % 1000) as f64 / 1000.0)
}

/// Map a generated event to the metering ingest wire type.
pub fn to_ingest_event(event: GeneratedEvent) -> Event {
    Event {
        id: event.id,
        code: event.code,
        customer_id: Some(EventCustomerId::MeteroidCustomerId(
            event.customer_id.as_base62(),
        )),
        timestamp: event.timestamp.and_utc().to_rfc3339(),
        properties: event.properties.into_iter().collect(),
    }
}

/// Best-effort chunked ingest (max 500/req) with backfilling enabled. Logs and
/// continues on failure; never returns an error.
pub async fn ingest_all(
    client: Arc<dyn UsageClient>,
    tenant_id: TenantId,
    keep_last_events: bool,
    events: Vec<Event>,
) -> usize {
    let total = events.len();
    let mut sent = 0usize;
    for chunk in events.chunks(500) {
        let request = IngestEventsRequest {
            events: chunk.to_vec(),
            allow_backfilling: true,
            fail_on_error: false,
            keep_last_events,
        };
        match client.ingest_events(&tenant_id, request).await {
            Ok(result) => sent += chunk.len().saturating_sub(result.failures.len()),
            Err(e) => log::warn!("usage ingest failed for tenant {tenant_id}: {e:?}"),
        }
    }
    log::debug!("usage ingest: {sent}/{total} events for tenant {tenant_id}");
    sent
}

pub(crate) fn fnv1a(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Minimal deterministic PRNG (splitmix64) — avoids a `rand` dependency and keeps
/// generation reproducible from the seed.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub(crate) fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub(crate) fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cust(seed: u64, start: (i32, u32, u32), end: (i32, u32, u32)) -> UsageCustomer {
        UsageCustomer {
            customer_id: CustomerId::from_const(uuid::Uuid::from_u128(seed as u128)),
            scale: 1.0,
            start: NaiveDate::from_ymd_opt(start.0, start.1, start.2).unwrap(),
            end: NaiveDate::from_ymd_opt(end.0, end.1, end.2).unwrap(),
            seed,
        }
    }

    fn counter_stream() -> UsageStream {
        UsageStream {
            metric_code: "api_calls".to_string(),
            shape: UsageShape::Counter {
                value_key: "value".to_string(),
                per_event_min: 1.0,
                per_event_max: 5.0,
            },
            base_daily_volume: 10,
            monthly_growth: 1.05,
            dimensions: vec![Dimension {
                key: "endpoint".to_string(),
                values: vec!["read".to_string(), "write".to_string()],
            }],
        }
    }

    #[test]
    fn deterministic_for_same_seed() {
        let s = counter_stream();
        let c = vec![cust(42, (2025, 1, 1), (2025, 1, 31))];
        assert_eq!(generate_stream(&s, &c), generate_stream(&s, &c));
    }

    #[test]
    fn different_seeds_diverge() {
        let s = counter_stream();
        let a = generate_stream(&s, &[cust(1, (2025, 1, 1), (2025, 1, 31))]);
        let b = generate_stream(&s, &[cust(2, (2025, 1, 1), (2025, 1, 31))]);
        assert_ne!(
            a.iter().map(|e| &e.id).collect::<Vec<_>>(),
            b.iter().map(|e| &e.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn events_fall_within_window_and_carry_value() {
        let s = counter_stream();
        let start = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2025, 3, 10).unwrap();
        let events = generate_stream(&s, &[cust(7, (2025, 3, 1), (2025, 3, 10))]);
        assert!(!events.is_empty());
        for e in &events {
            assert!(e.timestamp.date() >= start && e.timestamp.date() <= end);
            assert!(e.properties.iter().any(|(k, _)| k == "value"));
            assert!(e.id.starts_with("sbx-api_calls-"));
        }
    }

    #[test]
    fn ids_are_unique() {
        let s = counter_stream();
        let events = generate_stream(
            &s,
            &[
                cust(1, (2025, 1, 1), (2025, 2, 1)),
                cust(2, (2025, 1, 1), (2025, 2, 1)),
            ],
        );
        let mut ids: Vec<&String> = events.iter().map(|e| &e.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "event ids must be unique");
    }

    #[test]
    fn ids_carry_the_absolute_day() {
        let s = UsageStream {
            metric_code: "storage_gb".to_string(),
            shape: UsageShape::Gauge {
                value_key: "total".to_string(),
                floor: 10.0,
                ceil: 100.0,
            },
            base_daily_volume: 0,
            monthly_growth: 1.0,
            dimensions: vec![],
        };
        let id_on = |events: &[GeneratedEvent], d: NaiveDate| {
            events
                .iter()
                .find(|e| e.timestamp.date() == d)
                .map(|e| e.id.clone())
                .unwrap()
        };
        let jan5 = NaiveDate::from_ymd_opt(2025, 1, 5).unwrap();
        let early = generate_stream(&s, &[cust(5, (2025, 1, 1), (2025, 1, 31))]);
        let shifted = generate_stream(&s, &[cust(5, (2025, 1, 3), (2025, 2, 2))]);
        assert_eq!(
            id_on(&early, jan5),
            "sbx-storage_gb-0000000000000005-20093-0"
        );
        assert_eq!(id_on(&early, jan5), id_on(&shifted, jan5));
        assert_ne!(early[0].id, shifted[0].id);
    }

    #[test]
    fn gauge_emits_one_per_day_and_grows() {
        let s = UsageStream {
            metric_code: "storage_gb".to_string(),
            shape: UsageShape::Gauge {
                value_key: "total".to_string(),
                floor: 10.0,
                ceil: 100.0,
            },
            base_daily_volume: 0,
            monthly_growth: 1.0,
            dimensions: vec![],
        };
        let events = generate_stream(&s, &[cust(5, (2025, 1, 1), (2025, 1, 31))]);
        assert_eq!(events.len(), 31, "one gauge reading per day");
        let first: f64 = events.first().unwrap().properties[0].1.parse().unwrap();
        let last: f64 = events.last().unwrap().properties[0].1.parse().unwrap();
        assert!(
            last > first,
            "gauge level should drift upward over the window"
        );
    }
}
