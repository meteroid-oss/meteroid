//! Deterministic LLM-API traffic of the "Halyard AI" sandbox, which the live sandbox worker keeps
//! appending after the seeded history, so wallets, limits and budgets keep moving.
//!
//! Events are cut into fixed buckets and their ids derive from (customer, bucket, index), so
//! ingesting a bucket twice is harmless. Seeded history uses its own `hal-` ids and stops at the
//! seed's `as_of`, the tenant's creation bucket, where live traffic starts.

use chrono::{DateTime, Datelike, NaiveDateTime, Timelike, Weekday};
use common_domain::ids::{BaseId, CustomerId};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::usage::{GeneratedEvent, Rng, fnv1a};

pub const TOKENS_METRIC: &str = "halyard_tokens";
pub const TOKENS_PROPERTY: &str = "tokens";
pub const MODEL_PROPERTY: &str = "model";
pub const API_KEY_PROPERTY: &str = "api_key";
/// Usage is metered in tokens and priced per million.
pub const TOKENS_PER_PRICE_UNIT: i32 = 1_000_000;

pub const BUCKET_SECS: i64 = 30;
/// How many completed buckets a live tick (re)ingests; overlap is absorbed by the event ids.
pub const LIVE_LOOKBACK_BUCKETS: i64 = 4;

pub const MODELS: [&str; 3] = ["halyard-lite", "halyard-standard", "halyard-max"];

/// List price per million tokens, in USD.
pub fn price_per_million_tokens(model: &str) -> Option<Decimal> {
    match model {
        "halyard-lite" => Some(dec!(2)),
        "halyard-standard" => Some(dec!(12)),
        "halyard-max" => Some(dec!(60)),
        _ => None,
    }
}

fn token_range(model_idx: usize) -> (u32, u32) {
    match model_idx {
        0 => (2_000, 12_000),
        1 => (4_000, 30_000),
        _ => (6_000, 40_000),
    }
}

pub struct Traffic {
    pub alias: &'static str,
    pub runs_per_hour: f64,
    /// Share of tokens on each of [`MODELS`].
    pub model_mix: [u32; 3],
    pub api_keys: &'static [(&'static str, u32)],
    /// Whether the live worker keeps sending this account's traffic after the seed. An account
    /// whose usage is stopped (credit exhausted, limit reached) stays quiet.
    pub live: bool,
}

pub const TRAFFIC: &[Traffic] = &[
    Traffic {
        alias: "tessera-health",
        runs_per_hour: 4.6,
        model_mix: [17, 69, 14],
        api_keys: &[("clinical-notes", 70), ("triage", 30)],
        live: true,
    },
    Traffic {
        alias: "brightloom-labs",
        runs_per_hour: 27.0,
        model_mix: [55, 35, 10],
        api_keys: &[("prod-web", 60), ("prod-batch", 30), ("staging", 10)],
        live: true,
    },
    Traffic {
        alias: "quillbase",
        runs_per_hour: 3.2,
        model_mix: [40, 45, 15],
        api_keys: &[("prod-chat", 55), ("prod-batch", 35), ("staging", 10)],
        live: true,
    },
    Traffic {
        alias: "nightjar-robotics",
        runs_per_hour: 6.0,
        model_mix: [70, 25, 5],
        api_keys: &[("fleet-ops", 80), ("simulation", 20)],
        live: false,
    },
    Traffic {
        alias: "ferrous-logistics",
        runs_per_hour: 1.7,
        model_mix: [50, 40, 10],
        api_keys: &[("route-planner", 100)],
        live: true,
    },
    Traffic {
        alias: "lumen-notes",
        runs_per_hour: 1.6,
        model_mix: [58, 39, 3],
        api_keys: &[("lumen-prod", 80), ("lumen-staging", 20)],
        live: true,
    },
    Traffic {
        alias: "wren-labs",
        runs_per_hour: 2.0,
        model_mix: [76, 24, 0],
        api_keys: &[("wren-dev", 100)],
        live: false,
    },
    Traffic {
        alias: "meridian-freight",
        runs_per_hour: 125.0,
        model_mix: [50, 40, 10],
        api_keys: &[("dispatch", 70), ("route-optimizer", 30)],
        live: true,
    },
];

pub fn traffic_for(alias: &str) -> Option<&'static Traffic> {
    TRAFFIC.iter().find(|t| t.alias == alias)
}

/// Request weights that give each model its share of tokens, given its typical request size.
fn run_weights(traffic: &Traffic) -> [u32; 3] {
    let mut weights = [0u32; 3];
    for (model, share) in traffic.model_mix.iter().enumerate() {
        let (lo, hi) = token_range(model);
        weights[model] = share * 1_000_000 / ((lo + hi) / 2);
    }
    weights
}

/// Business-hours-peaked, quieter on weekends, never silent.
fn activity(at: NaiveDateTime) -> f64 {
    let hour = f64::from(at.hour()) + f64::from(at.minute()) / 60.0;
    let daily = 0.7 + 0.3 * (std::f64::consts::TAU * (hour - 14.0) / 24.0).cos();
    match at.weekday() {
        Weekday::Sat | Weekday::Sun => daily * 0.6,
        _ => daily,
    }
}

fn weighted<T>(rng: &mut Rng, items: impl Iterator<Item = (T, u32)> + Clone) -> T {
    let total: u32 = items.clone().map(|(_, w)| w).sum();
    let mut pick = (rng.unit() * f64::from(total)) as u32;
    let mut last = None;
    for (item, weight) in items {
        if pick < weight {
            return item;
        }
        pick -= weight;
        last = Some(item);
    }
    last.expect("a non-empty weighted set")
}

fn bucket_start(index: i64) -> NaiveDateTime {
    DateTime::from_timestamp(index * BUCKET_SECS, 0)
        .expect("a bucket within the supported range")
        .naive_utc()
}

fn bucket_index(at: NaiveDateTime) -> i64 {
    at.and_utc().timestamp().div_euclid(BUCKET_SECS)
}

/// Every run of `customer` that started in `[from, to)`, bucket by bucket.
pub fn run_events(
    customer: CustomerId,
    traffic: &Traffic,
    from: NaiveDateTime,
    to: NaiveDateTime,
) -> Vec<GeneratedEvent> {
    let mut events = Vec::new();
    let first = bucket_index(from);
    let last = bucket_index(to - chrono::Duration::seconds(1));
    let customer_key = customer.as_base62();
    for index in first..=last {
        let start = bucket_start(index);
        let expected = traffic.runs_per_hour * BUCKET_SECS as f64 / 3600.0 * activity(start);
        let mut rng = Rng::new(fnv1a(&format!("{customer_key}:{index}")));
        let runs = expected.floor() as i64 + i64::from(rng.unit() < expected.fract());
        for i in 0..runs {
            let at = start + chrono::Duration::seconds((rng.unit() * BUCKET_SECS as f64) as i64);
            let model_idx = weighted(&mut rng, run_weights(traffic).into_iter().enumerate());
            let (lo, hi) = token_range(model_idx);
            let tokens = rng.range(f64::from(lo), f64::from(hi)).round() as u64;
            let api_key = weighted(&mut rng, traffic.api_keys.iter().map(|(k, w)| (*k, *w)));
            if at < from || at >= to {
                continue;
            }
            events.push(GeneratedEvent {
                id: format!("halyard-{customer_key}-{index}-{i}"),
                code: TOKENS_METRIC.to_string(),
                customer_id: customer,
                timestamp: at,
                properties: vec![
                    (TOKENS_PROPERTY.to_string(), tokens.to_string()),
                    (MODEL_PROPERTY.to_string(), MODELS[model_idx].to_string()),
                    (API_KEY_PROPERTY.to_string(), api_key.to_string()),
                ],
            });
        }
    }
    events
}

/// The newest completed buckets before `now`, for every customer with live traffic. Nothing
/// before `not_before` (the end of the seeded history) is sent, so the two never overlap.
pub fn live_events(
    customers: &[(CustomerId, &str)],
    now: NaiveDateTime,
    not_before: NaiveDateTime,
) -> Vec<GeneratedEvent> {
    let end = bucket_start(bucket_index(now));
    let start = (end - chrono::Duration::seconds(BUCKET_SECS * LIVE_LOOKBACK_BUCKETS))
        .max(seed_cutoff(not_before));
    if start >= end {
        return vec![];
    }
    customers
        .iter()
        .filter_map(|(id, alias)| {
            traffic_for(alias)
                .filter(|t| t.live)
                .map(|t| run_events(*id, t, start, end))
        })
        .flatten()
        .collect()
}

/// The bucket boundary at or before `at`: seeded history ends there, live traffic starts there.
pub fn seed_cutoff(at: NaiveDateTime) -> NaiveDateTime {
    bucket_start(bucket_index(at))
}

/// What an event costs at list price, before rounding to the currency's minor unit.
pub fn event_cost(event: &GeneratedEvent) -> Decimal {
    let property = |key: &str| {
        event
            .properties
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    let tokens: Decimal = property(TOKENS_PROPERTY)
        .and_then(|v| v.parse().ok())
        .unwrap_or_default();
    let rate = property(MODEL_PROPERTY)
        .and_then(price_per_million_tokens)
        .unwrap_or_default();
    tokens / Decimal::from(TOKENS_PER_PRICE_UNIT) * rate
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn day(d: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 3, d)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    fn customer() -> CustomerId {
        CustomerId::new()
    }

    #[test]
    fn replaying_a_window_is_identical_and_split_windows_agree() {
        let c = customer();
        let t = traffic_for("quillbase").unwrap();
        let whole = run_events(c, t, day(2), day(4));
        assert!(!whole.is_empty());
        assert_eq!(whole, run_events(c, t, day(2), day(4)));

        let mut split = run_events(c, t, day(2), day(3));
        split.extend(run_events(c, t, day(3), day(4)));
        assert_eq!(whole, split);
    }

    #[test]
    fn events_stay_in_their_window_and_carry_the_billed_properties() {
        let c = customer();
        let events = run_events(c, traffic_for("brightloom-labs").unwrap(), day(2), day(3));
        for event in &events {
            assert!(event.timestamp >= day(2) && event.timestamp < day(3));
            assert_eq!(event.code, TOKENS_METRIC);
            assert!(event_cost(event) > Decimal::ZERO);
            for key in [TOKENS_PROPERTY, MODEL_PROPERTY, API_KEY_PROPERTY] {
                assert!(event.properties.iter().any(|(k, _)| k == key), "{key}");
            }
        }
        let ids: std::collections::HashSet<_> = events.iter().map(|e| &e.id).collect();
        assert_eq!(ids.len(), events.len());
    }

    #[test]
    fn live_ticks_one_bucket_apart_overlap_by_their_shared_buckets() {
        let c = customer();
        let now = day(3).and_utc().timestamp() + 14 * 3600;
        let at = |secs: i64| DateTime::from_timestamp(now + secs, 0).unwrap().naive_utc();
        let customers = [(c, "brightloom-labs")];
        let first = live_events(&customers, at(0), day(1));
        let next = live_events(&customers, at(BUCKET_SECS), day(1));
        let first_ids: std::collections::HashSet<_> = first.iter().map(|e| &e.id).collect();
        let next_ids: std::collections::HashSet<_> = next.iter().map(|e| &e.id).collect();
        let newest = bucket_start(bucket_index(at(0)));
        let next_from = bucket_start(bucket_index(at(BUCKET_SECS)))
            - chrono::Duration::seconds(BUCKET_SECS * LIVE_LOOKBACK_BUCKETS);
        // Only the newest bucket is new; the buckets both ticks cover carry the same events.
        assert!(
            next.iter()
                .filter(|e| !first_ids.contains(&e.id))
                .all(|e| e.timestamp >= newest)
        );
        assert!(
            first
                .iter()
                .filter(|e| e.timestamp >= next_from)
                .all(|e| next_ids.contains(&e.id))
        );
        assert!(first.iter().all(|e| e.timestamp < at(0)));
    }

    #[test]
    fn unknown_or_stopped_customers_have_no_live_traffic() {
        assert!(live_events(&[(customer(), "someone-else")], day(3), day(1)).is_empty());
        assert!(live_events(&[(customer(), "nightjar-robotics")], day(3), day(1)).is_empty());
    }

    #[test]
    fn live_traffic_never_reaches_back_before_the_seeded_history_ends() {
        let c = customer();
        let customers = [(c, "brightloom-labs")];
        let seeded_until = day(3) + chrono::Duration::seconds(75);
        let now = seeded_until + chrono::Duration::seconds(BUCKET_SECS);
        let events = live_events(&customers, now, seeded_until);
        assert!(
            events
                .iter()
                .all(|e| e.timestamp >= seed_cutoff(seeded_until))
        );
        assert!(live_events(&customers, seeded_until, seeded_until).is_empty());
        assert!(events.iter().all(|e| e.id.starts_with("halyard-")));
    }
}
