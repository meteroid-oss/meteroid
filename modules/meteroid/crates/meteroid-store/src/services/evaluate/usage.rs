//! Usage for `evaluate`, in two passes: the first lists the windows it needs, they are read through
//! the store's usage client, the second prices them. Run inside `transaction_reading_usage_first`
//! the reads are planned before its locks: a pass not served them is redone.

use billing_core::evaluate::{Evaluation, UsageNeed, evaluate};
use billing_core::pricer::{Usage, UsageRead, UsageReads};
use chrono::{DateTime, Utc};
use error_stack::Report;

use super::load::LoadedFacts;
use super::pricer::StorePricer;
use crate::StoreResult;
use crate::domain::UsagePeriod;
use crate::errors::StoreError;
use crate::services::clients::usage::UsageClient;

/// The usage windows evaluating `loaded` at `now` reads, before any is read.
pub fn plan_usage(loaded: &LoadedFacts, now: DateTime<Utc>) -> StoreResult<Vec<UsageNeed>> {
    Ok(evaluate(&loaded.facts, now, None, &StorePricer)
        .map_err(crate::services::invoice_lines::fees::pricing_error)?
        .usage_needed)
}

/// One window's usage per group as metering returns it, before the metric's conversion, which
/// the pricer applies; an empty window reads nothing, as `fetch_usage` does.
pub async fn read_one(
    client: &dyn UsageClient,
    loaded: &LoadedFacts,
    read: &UsageRead,
) -> StoreResult<Usage> {
    let metric = loaded
        .metrics
        .iter()
        .find(|metric| metric.id == read.metric)
        .ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(format!(
                "metric with id {}",
                read.metric
            )))
        })?;
    let period = UsagePeriod {
        start: read.window.from.naive_utc(),
        end: read.window.until.naive_utc(),
    };
    if period.end <= period.start {
        return Ok(Vec::new());
    }
    let usage = client
        .fetch_usage(&loaded.tenant_id, &loaded.customer_id, metric, period)
        .await?;
    Ok(usage
        .data
        .into_iter()
        .map(|group| (group.dimensions.into_iter().collect(), group.value))
        .collect())
}

/// Reads `needs` through `client`, the same client and filters `fetch_usage` reads with.
pub async fn read_usage(
    client: &dyn UsageClient,
    loaded: &LoadedFacts,
    needs: &[UsageNeed],
) -> StoreResult<UsageReads> {
    let mut reads = UsageReads::new();
    for need in needs {
        if reads.contains(&need.read) {
            continue;
        }
        let usage = read_one(client, loaded, &need.read).await?;
        reads.insert(need.read, usage);
    }
    let needed: Vec<UsageRead> = needs.iter().map(|need| need.read).collect();
    crate::services::volume_agreements::serve_bases(client, loaded, &mut reads, &needed).await?;
    Ok(reads)
}

/// `evaluate` at `now` with the usage of every window it needs, read through `client`.
pub async fn evaluate_reading_usage(
    client: &dyn UsageClient,
    loaded: &LoadedFacts,
    now: DateTime<Utc>,
) -> StoreResult<Evaluation> {
    let needs = plan_usage(loaded, now)?;
    let reads = read_usage(client, loaded, &needs).await?;
    evaluate(&loaded.facts, now, Some(&reads), &StorePricer)
        .map_err(crate::services::invoice_lines::fees::pricing_error)
}

/// What a usage line was priced over, to read its usage again grouped by its metric's
/// breakdown key: its metric, the window its charges read, and its rates by dimension.
pub(super) struct BreakdownRead {
    metric: common_domain::ids::BillableMetricId,
    window: billing_core::pricer::UsageWindow,
    rated: Option<billing_core::pricing::CardRates>,
}

impl BreakdownRead {
    /// None for a line of no usage fee, or of usage read over no window.
    pub(super) fn of(
        charges: &[&billing_core::evaluate::Charge],
        fee: &super::pricer::StoreFee,
    ) -> Option<Self> {
        let crate::domain::SubscriptionFee::Usage { metric_id, .. } = &fee.fee else {
            return None;
        };
        let windows = charges.iter().filter_map(|charge| charge.usage);
        let from = windows.clone().map(|window| window.from).min()?;
        let until = windows.map(|window| window.until).max()?;
        Some(BreakdownRead {
            metric: *metric_id,
            window: billing_core::pricer::UsageWindow { from, until },
            rated: fee
                .rate_card
                .clone()
                .map(|card| billing_core::pricing::CardRates {
                    on: from.date_naive(),
                    ..card
                }),
        })
    }
}

/// Breaks each usage line down by its metric's breakdown key, the breakdown
/// read apart from the usage it was priced on. A breakdown bills nothing, so a line it cannot
/// be read for goes out without it.
pub(super) async fn attach_breakdowns(
    client: &dyn UsageClient,
    loaded: &LoadedFacts,
    lines: &mut [(crate::domain::LineItem, Option<BreakdownRead>)],
) {
    for (line, read) in lines.iter_mut() {
        let Some(read) = read else {
            continue;
        };
        let Some(metric) = loaded
            .metrics
            .iter()
            .find(|metric| metric.id == read.metric)
        else {
            continue;
        };
        let Some(key) = metric.usage_breakdown_key.as_deref() else {
            continue;
        };
        let period = UsagePeriod {
            start: read.window.from.naive_utc(),
            end: read.window.until.naive_utc(),
        };
        if period.end <= period.start {
            continue;
        }
        match client
            .fetch_usage_grouped_by(&loaded.tenant_id, &loaded.customer_id, metric, key, period)
            .await
        {
            Ok(usage) => {
                line.usage_breakdown = crate::services::invoice_lines::component::line_breakdown(
                    line.group_by_dimensions.as_ref(),
                    metric,
                    key,
                    read.rated.as_ref(),
                    &usage.data,
                );
            }
            Err(error) => log::warn!(
                "Billing metric {} without its usage breakdown: {error:?}",
                metric.id
            ),
        }
    }
}
