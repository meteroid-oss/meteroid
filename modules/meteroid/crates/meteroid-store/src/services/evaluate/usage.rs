//! Usage for `evaluate`, in two passes: the first lists the windows it needs, they are read through
//! the store's usage client, the second prices them. Run inside `transaction_reading_usage_first`
//! the reads are planned before its locks: a pass not served them is redone (Phase 4.6).

use billing_core::evaluate::{Evaluation, UsageNeed, evaluate};
use billing_core::pricer::{Usage, UsageRead, UsageReads};
use chrono::{DateTime, Utc};
use common_domain::ids::BaseId;
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
        .find(|metric| metric.id.as_uuid() == read.metric)
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
