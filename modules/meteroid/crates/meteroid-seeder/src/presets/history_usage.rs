//! Turns a credit customer's usage targets into token events that hit every target exactly at
//! list price. Usage older than 48 hours is one event per day, model and API key; the last 48 hours
//! keep one event per request so the live view looks like live traffic.

use chrono::{Datelike, Duration, NaiveDateTime, Timelike, Weekday};
use common_domain::ids::{BaseId, CustomerId};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::presets::credit_traffic::{
    API_KEY_PROPERTY, MODEL_PROPERTY, MODELS, TOKENS_METRIC, TOKENS_PROPERTY,
};
use crate::presets::scenarios::timeline::{Clock, UsageAmount, UsagePlan, UsageTarget};
use crate::usage::{GeneratedEvent, Rng, fnv1a};

/// Events newer than this before `as_of` are generated per request.
pub const REQUEST_GRAIN_WINDOW: Duration = Duration::hours(48);
const MILLION: u64 = 1_000_000;
/// Tokens of every model but the one taking the remainder are a multiple of this, when the price
/// allows it, so per-model totals read as round numbers.
const ROUND_TOKENS: u64 = 10_000;
const MAX_RUNS_PER_SLOT: u64 = 240;

/// List prices of the models, per million tokens, in a denomination with `scale` decimals.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelPrices {
    pub per_million: [Decimal; 3],
    pub scale: u32,
}

impl ModelPrices {
    pub fn price(&self, tokens: [u64; 3]) -> Decimal {
        tokens
            .iter()
            .zip(&self.per_million)
            .map(|(t, p)| Decimal::from(*t) * p / Decimal::from(MILLION))
            .sum()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowTokens {
    pub from: NaiveDateTime,
    pub to: NaiveDateTime,
    pub tokens: [u64; 3],
}

#[derive(Clone, Debug)]
pub struct CustomerUsage {
    pub events: Vec<GeneratedEvent>,
    /// One per usage target, in the history's order.
    pub windows: Vec<WindowTokens>,
}

impl CustomerUsage {
    pub fn total_tokens(&self) -> u64 {
        self.windows.iter().flat_map(|w| w.tokens).sum()
    }

    pub fn first_event_at(&self) -> Option<NaiveDateTime> {
        self.events.iter().map(|e| e.timestamp).min()
    }
}

#[derive(Clone, Copy)]
enum Keys<'a> {
    Weighted(&'a [(&'static str, u32)]),
    Fixed(&'static str),
}

struct Stream<'a> {
    keys: Keys<'a>,
    tokens: [u64; 3],
}

struct Piece<'a> {
    from: NaiveDateTime,
    to: NaiveDateTime,
    streams: Vec<Stream<'a>>,
}

pub fn generate(
    customer_id: CustomerId,
    plan: &UsagePlan,
    clock: &Clock,
    prices: Option<&ModelPrices>,
) -> Result<CustomerUsage, String> {
    let customer = customer_id.as_base62();
    let seed = fnv1a(&format!("halyard-history:{customer}"));
    let fine_from = clock.as_of - REQUEST_GRAIN_WINDOW;
    let mut events = Vec::new();
    let mut windows = Vec::new();

    for (t_idx, target) in plan.targets.iter().enumerate() {
        let from = clock.resolve(target.from);
        let to = clock.resolve(target.to);
        if from >= to || to > clock.as_of {
            return Err(format!(
                "usage window {from} - {to} is empty or ends after {}",
                clock.as_of
            ));
        }
        let pieces = pieces(target, plan, clock, prices, from, to)?;
        let mut window_tokens = [0u64; 3];
        for (p_idx, piece) in pieces.iter().enumerate() {
            let slots = slots(piece.from, piece.to, fine_from);
            let weights: Vec<f64> = slots
                .iter()
                .map(|s| slot_weight(s, from, plan.monthly_growth))
                .collect();
            for (s_idx, stream) in piece.streams.iter().enumerate() {
                for (model, tokens) in stream.tokens.iter().enumerate() {
                    window_tokens[model] += tokens;
                    for (slot_idx, (slot, slot_tokens)) in
                        slots.iter().zip(split(*tokens, &weights)).enumerate()
                    {
                        if slot_tokens == 0 {
                            continue;
                        }
                        let ctx = EmitContext {
                            customer_id,
                            customer: &customer,
                            id_prefix: format!("{t_idx}-{p_idx}-{s_idx}-{slot_idx}-{model}"),
                            model,
                            keys: stream.keys,
                        };
                        if slot.fine {
                            let mut rng = Rng::new(
                                seed ^ fnv1a(&format!(
                                    "{t_idx}:{p_idx}:{s_idx}:{slot_idx}:{model}"
                                )),
                            );
                            let share = f64::from(plan.mix[model])
                                / f64::from(plan.mix.iter().sum::<u32>().max(1));
                            let expected = plan.runs_per_hour
                                * (slot.to - slot.from).num_seconds() as f64
                                / 3600.0
                                * activity(slot.from)
                                * share;
                            let runs = (expected.round() as u64).clamp(1, MAX_RUNS_PER_SLOT);
                            emit_requests(&ctx, slot, slot_tokens, runs, &mut rng, &mut events);
                        } else {
                            emit_aggregate(&ctx, slot, slot_tokens, &mut events);
                        }
                    }
                }
            }
        }
        if let (UsageAmount::Priced(amount), Some(prices)) = (&target.amount, prices)
            && prices.price(window_tokens) != *amount
        {
            return Err(format!(
                "usage {from} - {to} prices at {}, not {amount}",
                prices.price(window_tokens)
            ));
        }
        windows.push(WindowTokens {
            from,
            to,
            tokens: window_tokens,
        });
    }
    Ok(CustomerUsage { events, windows })
}

fn pieces<'a>(
    target: &UsageTarget,
    plan: &'a UsagePlan,
    clock: &Clock,
    prices: Option<&ModelPrices>,
    from: NaiveDateTime,
    to: NaiveDateTime,
) -> Result<Vec<Piece<'a>>, String> {
    let total = match &target.amount {
        UsageAmount::Priced(total) => *total,
        UsageAmount::Tokens(tokens) => {
            if !target.checkpoints.is_empty() || !target.spikes.is_empty() {
                return Err("checkpoints and spikes need a priced target".to_string());
            }
            return Ok(vec![Piece {
                from,
                to,
                streams: vec![Stream {
                    keys: Keys::Weighted(plan.api_keys),
                    tokens: *tokens,
                }],
            }]);
        }
        UsageAmount::TokensByKey(by_key) => {
            if !target.checkpoints.is_empty() || !target.spikes.is_empty() {
                return Err("checkpoints and spikes need a priced target".to_string());
            }
            return Ok(vec![Piece {
                from,
                to,
                streams: by_key
                    .iter()
                    .map(|(key, tokens)| Stream {
                        keys: Keys::Fixed(key),
                        tokens: *tokens,
                    })
                    .collect(),
            }]);
        }
    };
    let prices = prices.ok_or("a priced target needs the plan's model prices")?;

    let mut out = Vec::new();
    let mut spike_total = Decimal::ZERO;
    let mut spikes: Vec<(NaiveDateTime, NaiveDateTime, Decimal, &'static str)> = target
        .spikes
        .iter()
        .map(|s| {
            (
                clock.resolve(s.from),
                clock.resolve(s.to),
                s.amount,
                s.api_key,
            )
        })
        .collect();
    spikes.sort_by_key(|s| s.0);
    for (i, (s_from, s_to, amount, key)) in spikes.iter().enumerate() {
        if *s_from < from || s_to > &to || s_from >= s_to {
            return Err(format!("spike {s_from} - {s_to} is outside {from} - {to}"));
        }
        if i > 0 && spikes[i - 1].1 > *s_from {
            return Err("spikes overlap".to_string());
        }
        spike_total += amount;
        out.push(Piece {
            from: *s_from,
            to: *s_to,
            streams: vec![Stream {
                keys: Keys::Fixed(key),
                tokens: solve(*amount, &prices.per_million, prices.scale, plan.mix)?,
            }],
        });
    }
    let baseline = total - spike_total;
    if baseline.is_sign_negative() {
        return Err(format!("spikes exceed the target {total}"));
    }

    let mut cuts = vec![(from, Decimal::ZERO)];
    let mut checkpoints: Vec<(NaiveDateTime, Decimal)> = target
        .checkpoints
        .iter()
        .map(|(at, cum)| (clock.resolve(*at), *cum))
        .collect();
    checkpoints.sort_by_key(|c| c.0);
    for (at, cumulative) in checkpoints {
        if at <= from || at >= to {
            return Err(format!("checkpoint {at} is outside {from} - {to}"));
        }
        if spikes
            .iter()
            .any(|(s_from, s_to, ..)| *s_from < at && at < *s_to)
        {
            return Err(format!("checkpoint {at} falls inside a spike"));
        }
        let spiked: Decimal = spikes
            .iter()
            .filter(|(_, s_to, ..)| *s_to <= at)
            .map(|s| s.2)
            .sum();
        let base = cumulative - spiked;
        if base < cuts.last().expect("a first cut").1 || base > baseline {
            return Err(format!("checkpoint {at} at {cumulative} is out of order"));
        }
        cuts.push((at, base));
    }
    cuts.push((to, baseline));
    for pair in cuts.windows(2) {
        let amount = pair[1].1 - pair[0].1;
        if amount.is_zero() {
            continue;
        }
        out.push(Piece {
            from: pair[0].0,
            to: pair[1].0,
            streams: vec![Stream {
                keys: Keys::Weighted(plan.api_keys),
                tokens: solve(amount, &prices.per_million, prices.scale, plan.mix)?,
            }],
        });
    }
    Ok(out)
}

/// Smallest token count whose price has at most `scale` decimals.
fn token_grain(price_per_million: Decimal, scale: u32) -> u64 {
    let mantissa = price_per_million.normalize().mantissa().unsigned_abs();
    let price_scale = price_per_million.normalize().scale();
    let modulus = 10u128.pow(price_scale + 6);
    let scaled = mantissa * 10u128.pow(scale);
    (modulus / gcd(scaled, modulus)) as u64
}

fn gcd(a: u128, b: u128) -> u128 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn lcm(a: u64, b: u64) -> u64 {
    a / gcd(u128::from(a), u128::from(b)) as u64 * b
}

/// Tokens per model, close to `mix`, priced at exactly `amount`. The cheapest model with a
/// one-minor-unit grain takes the remainder; the others are rounded to tidy multiples.
pub fn solve(
    amount: Decimal,
    per_million: &[Decimal; 3],
    scale: u32,
    mix: [u32; 3],
) -> Result<[u64; 3], String> {
    if amount.is_sign_negative() || amount.normalize().scale() > scale {
        return Err(format!(
            "{amount} is not a whole amount at {scale} decimals"
        ));
    }
    let minor = Decimal::new(1, scale);
    let grains: Vec<u64> = per_million.iter().map(|p| token_grain(*p, scale)).collect();
    let remainder_model = (0..3)
        .filter(|m| mix[*m] > 0)
        .find(|m| Decimal::from(grains[*m]) * per_million[*m] / Decimal::from(MILLION) == minor)
        .ok_or("no model in the mix can take a remainder of one minor unit")?;

    let mix_total = Decimal::from(mix.iter().sum::<u32>());
    let average: Decimal = (0..3)
        .map(|m| Decimal::from(mix[m]) / mix_total * per_million[m])
        .sum();
    let ideal_tokens = amount / average * Decimal::from(MILLION);

    let mut tokens = [0u64; 3];
    for m in (0..3).filter(|m| *m != remainder_model && mix[*m] > 0) {
        let step = lcm(grains[m], ROUND_TOKENS);
        let share = ideal_tokens * Decimal::from(mix[m]) / mix_total / Decimal::from(step);
        tokens[m] = share.round().to_u64().unwrap_or(0) * step;
    }
    let price = |tokens: &[u64; 3]| -> Decimal {
        (0..3)
            .filter(|m| *m != remainder_model)
            .map(|m| Decimal::from(tokens[m]) * per_million[m] / Decimal::from(MILLION))
            .sum()
    };
    // Rounding up may leave nothing (or less) for the remainder: take steps back off the
    // dearest model until it fits.
    while price(&tokens) > amount {
        let m = (0..3)
            .filter(|m| *m != remainder_model && tokens[*m] > 0)
            .max_by_key(|m| per_million[*m])
            .ok_or("the remainder cannot be negative")?;
        tokens[m] -= lcm(grains[m], ROUND_TOKENS);
    }
    let rest = amount - price(&tokens);
    let units = rest / minor;
    if !units.fract().is_zero() {
        return Err(format!("{rest} is not a whole number of minor units"));
    }
    tokens[remainder_model] =
        units.to_u64().ok_or("remainder out of range")? * grains[remainder_model];
    Ok(tokens)
}

struct Slot {
    from: NaiveDateTime,
    to: NaiveDateTime,
    fine: bool,
}

/// Days before `fine_from`, hours after it, clipped to `[from, to)`.
fn slots(from: NaiveDateTime, to: NaiveDateTime, fine_from: NaiveDateTime) -> Vec<Slot> {
    let mut out = Vec::new();
    let mut cursor = from;
    while cursor < to {
        let fine = cursor >= fine_from;
        let boundary = if fine {
            cursor
                .with_minute(0)
                .and_then(|t| t.with_second(0))
                .and_then(|t| t.with_nanosecond(0))
                .expect("an hour")
                + Duration::hours(1)
        } else {
            (cursor.date() + Duration::days(1))
                .and_hms_opt(0, 0, 0)
                .expect("midnight")
                .min(fine_from)
        };
        let end = boundary.min(to);
        out.push(Slot {
            from: cursor,
            to: end,
            fine,
        });
        cursor = end;
    }
    out
}

fn slot_weight(slot: &Slot, window_from: NaiveDateTime, monthly_growth: f64) -> f64 {
    let secs = (slot.to - slot.from).num_seconds() as f64;
    let weekday = match slot.from.weekday() {
        Weekday::Sat | Weekday::Sun => 0.6,
        _ => 1.0,
    };
    let diurnal = if slot.fine { activity(slot.from) } else { 1.0 };
    let months = (slot.from - window_from).num_seconds() as f64 / (30.0 * 86_400.0);
    secs * weekday * diurnal * monthly_growth.powf(months)
}

/// Business-hours-peaked, never silent.
fn activity(at: NaiveDateTime) -> f64 {
    let hour = f64::from(at.hour()) + f64::from(at.minute()) / 60.0;
    0.7 + 0.3 * (std::f64::consts::TAU * (hour - 14.0) / 24.0).cos()
}

/// `total` split in proportion to `weights`, by largest remainder, so the parts sum to it exactly.
fn split(total: u64, weights: &[f64]) -> Vec<u64> {
    let sum: f64 = weights.iter().sum();
    if weights.is_empty() {
        return vec![];
    }
    if sum <= 0.0 {
        let mut parts = vec![0; weights.len()];
        *parts.last_mut().expect("a part") = total;
        return parts;
    }
    let exact: Vec<f64> = weights.iter().map(|w| total as f64 * w / sum).collect();
    let mut parts: Vec<u64> = exact.iter().map(|e| e.floor() as u64).collect();
    let assigned: u64 = parts.iter().sum();
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by(|a, b| {
        (exact[*b] - exact[*b].floor())
            .total_cmp(&(exact[*a] - exact[*a].floor()))
            .then(a.cmp(b))
    });
    // Float error can leave the floors a few tokens off either way; settle it exactly here.
    if assigned <= total {
        for i in order.iter().cycle().take((total - assigned) as usize) {
            parts[*i] += 1;
        }
    } else {
        let mut excess = assigned - total;
        for i in order.iter().rev().cycle() {
            if excess == 0 {
                break;
            }
            if parts[*i] > 0 {
                parts[*i] -= 1;
                excess -= 1;
            }
        }
    }
    parts
}

struct EmitContext<'a> {
    customer_id: CustomerId,
    customer: &'a str,
    id_prefix: String,
    model: usize,
    keys: Keys<'a>,
}

fn event(
    ctx: &EmitContext,
    id: String,
    at: NaiveDateTime,
    key: &str,
    tokens: u64,
) -> GeneratedEvent {
    GeneratedEvent {
        id,
        code: TOKENS_METRIC.to_string(),
        customer_id: ctx.customer_id,
        timestamp: at,
        properties: vec![
            (TOKENS_PROPERTY.to_string(), tokens.to_string()),
            (MODEL_PROPERTY.to_string(), MODELS[ctx.model].to_string()),
            (API_KEY_PROPERTY.to_string(), key.to_string()),
        ],
    }
}

fn emit_aggregate(ctx: &EmitContext, slot: &Slot, tokens: u64, out: &mut Vec<GeneratedEvent>) {
    let at = slot.from + (slot.to - slot.from) / 2;
    let at = at.with_nanosecond(0).expect("whole seconds").max(slot.from);
    match ctx.keys {
        Keys::Fixed(key) => out.push(event(
            ctx,
            format!("hal-{}-d{}", ctx.customer, ctx.id_prefix),
            at,
            key,
            tokens,
        )),
        Keys::Weighted(keys) => {
            let weights: Vec<f64> = keys.iter().map(|(_, w)| f64::from(*w)).collect();
            for (k_idx, ((key, _), part)) in keys.iter().zip(split(tokens, &weights)).enumerate() {
                if part > 0 {
                    out.push(event(
                        ctx,
                        format!("hal-{}-d{}-{k_idx}", ctx.customer, ctx.id_prefix),
                        at,
                        key,
                        part,
                    ));
                }
            }
        }
    }
}

fn emit_requests(
    ctx: &EmitContext,
    slot: &Slot,
    tokens: u64,
    runs: u64,
    rng: &mut Rng,
    out: &mut Vec<GeneratedEvent>,
) {
    let runs = runs.min(tokens).max(1);
    let span = (slot.to - slot.from).num_seconds().max(1);
    let mut planned: Vec<(NaiveDateTime, &str, f64)> = (0..runs)
        .map(|_| {
            let at = slot.from + Duration::seconds((rng.unit() * span as f64) as i64);
            let key = match ctx.keys {
                Keys::Fixed(key) => key,
                Keys::Weighted(keys) => pick(rng, keys),
            };
            (
                at.min(slot.to - Duration::seconds(1)).max(slot.from),
                key,
                rng.range(0.4, 1.6),
            )
        })
        .collect();
    planned.sort_by_key(|p| p.0);
    let weights: Vec<f64> = planned.iter().map(|p| p.2).collect();
    for (i, ((at, key, _), part)) in planned.iter().zip(split(tokens, &weights)).enumerate() {
        if part > 0 {
            out.push(event(
                ctx,
                format!("hal-{}-r{}-{i}", ctx.customer, ctx.id_prefix),
                *at,
                key,
                part,
            ));
        }
    }
}

fn pick(rng: &mut Rng, keys: &[(&'static str, u32)]) -> &'static str {
    let total: u32 = keys.iter().map(|(_, w)| w).sum();
    let mut draw = (rng.unit() * f64::from(total)) as u32;
    for (key, weight) in keys {
        if draw < *weight {
            return key;
        }
        draw -= weight;
    }
    keys.last().expect("an API key").0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::scenarios::timeline::{Spike, When, cycle, cycle_at};
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;
    use std::collections::{HashMap, HashSet};

    fn usd() -> ModelPrices {
        ModelPrices {
            per_million: [dec!(2), dec!(12), dec!(60)],
            scale: 2,
        }
    }

    fn as_of() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 8)
            .unwrap()
            .and_hms_opt(19, 13, 0)
            .unwrap()
    }

    fn tokens_of(event: &GeneratedEvent) -> u64 {
        event.properties[0].1.parse().unwrap()
    }

    #[test]
    fn solved_tokens_price_at_exactly_the_target() {
        let enterprise = [dec!(1.6), dec!(9.6), dec!(48)];
        let credits = [dec!(200), dec!(1200), dec!(6000)];
        for amount in [
            dec!(620),
            dec!(1188),
            dec!(0.01),
            dec!(4.8),
            dec!(165),
            dec!(126),
        ] {
            for mix in [[55, 35, 10], [40, 45, 15], [100, 0, 0], [75, 25, 0]] {
                let t = solve(amount, &usd().per_million, 2, mix).unwrap();
                assert_eq!(usd().price(t), amount, "{amount} {mix:?}");
                let e = solve(amount, &enterprise, 2, mix).unwrap();
                let priced = ModelPrices {
                    per_million: enterprise,
                    scale: 2,
                }
                .price(e);
                assert_eq!(priced, amount);
            }
        }
        let c = solve(dec!(5480), &credits, 0, [58, 39, 3]).unwrap();
        assert_eq!(
            ModelPrices {
                per_million: credits,
                scale: 0
            }
            .price(c),
            dec!(5480)
        );
        assert!(solve(dec!(0.005), &usd().per_million, 2, [1, 1, 1]).is_err());
        let brightloom = solve(dec!(620), &usd().per_million, 2, [55, 35, 10]).unwrap();
        assert!(
            brightloom[1].is_multiple_of(ROUND_TOKENS)
                && brightloom[2].is_multiple_of(ROUND_TOKENS)
        );
    }

    #[test]
    fn aggregation_keeps_every_window_exact_and_ids_unique() {
        let clock = Clock::new(as_of(), 27);
        let plan = UsagePlan {
            mix: [55, 35, 10],
            api_keys: &[("prod-web", 60), ("prod-batch", 30), ("staging", 10)],
            monthly_growth: 1.25,
            runs_per_hour: 36.0,
            targets: vec![
                UsageTarget::new(cycle(2), cycle(1), UsageAmount::Priced(dec!(940)))
                    .with_checkpoint(cycle_at(2, 6, 10, 0), dec!(200))
                    .with_spike(Spike {
                        from: cycle_at(2, 20, 0, 0),
                        to: cycle_at(2, 23, 0, 0),
                        amount: dec!(90),
                        api_key: "prod-batch",
                    }),
                UsageTarget::new(
                    cycle(0),
                    When::Ago(Duration::zero()),
                    UsageAmount::Priced(dec!(1188)),
                )
                .with_checkpoint(cycle_at(0, 7, 10, 0), dec!(310)),
            ],
        };
        let customer = CustomerId::new();
        let usage = generate(customer, &plan, &clock, Some(&usd())).unwrap();
        assert_eq!(usage.windows.len(), 2);

        let priced_in = |from: NaiveDateTime, to: NaiveDateTime| -> Decimal {
            let mut tokens = [0u64; 3];
            for e in usage
                .events
                .iter()
                .filter(|e| e.timestamp >= from && e.timestamp < to)
            {
                let model = MODELS.iter().position(|m| *m == e.properties[1].1).unwrap();
                tokens[model] += tokens_of(e);
            }
            usd().price(tokens)
        };
        for (window, amount) in usage.windows.iter().zip([dec!(940), dec!(1188)]) {
            assert_eq!(usd().price(window.tokens), amount);
            assert_eq!(priced_in(window.from, window.to), amount);
        }
        assert_eq!(
            priced_in(
                clock.cycle_start(2).and_time(chrono::NaiveTime::MIN),
                clock.resolve(cycle_at(2, 6, 10, 0))
            ),
            dec!(200)
        );
        assert_eq!(
            priced_in(
                clock.cycle_start(0).and_time(chrono::NaiveTime::MIN),
                clock.resolve(cycle_at(0, 7, 10, 0))
            ),
            dec!(310)
        );
        let spike = priced_in(
            clock.resolve(cycle_at(2, 20, 0, 0)),
            clock.resolve(cycle_at(2, 23, 0, 0)),
        );
        assert!(spike > dec!(90));

        let ids: HashSet<_> = usage.events.iter().map(|e| &e.id).collect();
        assert_eq!(ids.len(), usage.events.len());
        assert!(
            usage
                .events
                .iter()
                .all(|e| e.timestamp < clock.as_of && tokens_of(e) > 0)
        );

        let fine_from = clock.as_of - REQUEST_GRAIN_WINDOW;
        let mut per_day: HashMap<(chrono::NaiveDate, String, String), usize> = HashMap::new();
        for e in usage.events.iter().filter(|e| e.timestamp < fine_from) {
            *per_day
                .entry((
                    e.timestamp.date(),
                    e.properties[1].1.clone(),
                    e.properties[2].1.clone(),
                ))
                .or_default() += 1;
        }
        // A day holds one event per model and key, two when a checkpoint or a spike cuts it.
        assert!(per_day.values().all(|n| *n <= 3));
        assert!(
            usage
                .events
                .iter()
                .filter(|e| e.timestamp >= fine_from)
                .count()
                > 48
        );
        assert!(usage.events.iter().all(|e| e.id.starts_with("hal-")));
    }

    #[test]
    fn explicit_tokens_by_key_are_kept() {
        let clock = Clock::new(as_of(), 23);
        let plan = UsagePlan {
            mix: [58, 39, 3],
            api_keys: &[("lumen-prod", 80), ("lumen-staging", 20)],
            monthly_growth: 1.0,
            runs_per_hour: 6.0,
            targets: vec![UsageTarget::new(
                cycle(0),
                When::Ago(Duration::zero()),
                UsageAmount::TokensByKey(vec![
                    ("lumen-prod", [2_400_000, 1_700_000, 100_000]),
                    ("lumen-staging", [600_000, 300_000, 50_000]),
                ]),
            )],
        };
        let usage = generate(CustomerId::new(), &plan, &clock, None).unwrap();
        let per_key = |key: &str| -> u64 {
            usage
                .events
                .iter()
                .filter(|e| e.properties[2].1 == key)
                .map(tokens_of)
                .sum()
        };
        assert_eq!(per_key("lumen-prod"), 4_200_000);
        assert_eq!(per_key("lumen-staging"), 950_000);
        assert_eq!(usage.windows[0].tokens, [3_000_000, 2_000_000, 150_000]);
    }
}
