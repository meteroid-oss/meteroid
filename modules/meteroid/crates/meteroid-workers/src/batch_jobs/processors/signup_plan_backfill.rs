//! Starts the customers a sign-up plan backfill previewed on their product line's free plan.
use std::str::FromStr;

use async_trait::async_trait;
use bytes::Bytes;
use common_domain::actor::Actor;
use common_domain::ids::{CustomerId, UserId};
use meteroid_app::workers::batch_jobs::{SIGNUP_PLAN_BACKFILL_HEADER, SignupPlanBackfillParams};
use meteroid_store::Services;
use meteroid_store::domain::batch_jobs::{BatchJob, BatchJobChunk};
use meteroid_store::errors::StoreError;
use meteroid_store::services::signup_backfill::BackfillOutcome;

use crate::batch_jobs::engine::{BatchJobProcessor, ChunkDefinition, ChunkResult, ItemFailure};

const CHUNK_SIZE: i32 = 50;

pub struct SignupPlanBackfillProcessor {
    services: Services,
}

impl SignupPlanBackfillProcessor {
    pub fn new(services: Services) -> Self {
        Self { services }
    }
}

fn customer_ids(input_data: Option<&Bytes>) -> Result<Vec<&str>, String> {
    let data = input_data.ok_or("A sign-up plan backfill has no customers to start")?;
    let text = std::str::from_utf8(data).map_err(|e| format!("Unreadable customer ids: {e}"))?;
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    if lines.next() != Some(SIGNUP_PLAN_BACKFILL_HEADER) {
        return Err(format!(
            "The customer ids lack their '{SIGNUP_PLAN_BACKFILL_HEADER}' header"
        ));
    }
    Ok(lines.collect())
}

fn params(job: &BatchJob) -> Result<SignupPlanBackfillParams, String> {
    let value = job
        .input_params
        .clone()
        .ok_or("A sign-up plan backfill names no product line")?;
    serde_json::from_value(value).map_err(|e| format!("Invalid backfill params: {e}"))
}

/// Worth retrying the chunk for: the next attempt may reach the database or metering. Any other
/// error is the customer's alone.
fn transient(error: &StoreError) -> bool {
    matches!(
        error,
        StoreError::DatabaseConnectionError
            | StoreError::MeteringServiceError
            | StoreError::MeteringNotReady
            | StoreError::UsageUnavailable
    )
}

#[async_trait]
impl BatchJobProcessor for SignupPlanBackfillProcessor {
    async fn prepare_chunks(
        &self,
        job: &BatchJob,
        input_data: Option<Bytes>,
    ) -> Result<Vec<ChunkDefinition>, String> {
        params(job)?;
        let count = customer_ids(input_data.as_ref())?.len() as i32;
        Ok((0..count)
            .step_by(CHUNK_SIZE as usize)
            .map(|offset| ChunkDefinition {
                item_offset: offset,
                item_count: CHUNK_SIZE.min(count - offset),
            })
            .collect())
    }

    /// Each customer in its own transaction: one already on a plan is skipped, so a chunk
    /// retried after a transient error starts no one twice. Any other error is the customer's.
    async fn process_chunk(
        &self,
        job: &BatchJob,
        chunk: &BatchJobChunk,
        input_data: Option<Bytes>,
    ) -> Result<ChunkResult, String> {
        let params = params(job)?;
        let ids = customer_ids(input_data.as_ref())?;
        let actor = Actor::User {
            id: UserId::from(job.created_by),
        };
        let mut result = ChunkResult {
            processed: 0,
            failures: Vec::new(),
            created_entities: Vec::new(),
        };
        let slice = ids
            .iter()
            .enumerate()
            .skip(chunk.item_offset as usize)
            .take(chunk.item_count as usize);
        for (index, raw) in slice {
            let fail = |reason: String| ItemFailure {
                item_index: index as i32,
                item_identifier: Some((*raw).to_owned()),
                reason,
            };
            let Ok(customer_id) = CustomerId::from_str(raw) else {
                result.failures.push(fail("Not a customer id".to_owned()));
                continue;
            };
            let outcome = self
                .services
                .backfill_signup_plan(
                    actor.clone(),
                    job.tenant_id,
                    params.product_family_id,
                    params.plan_version_id,
                    Some(job.id),
                    customer_id,
                )
                .await;
            match outcome {
                Ok(BackfillOutcome::Subscribed(_) | BackfillOutcome::Skipped) => {
                    result.processed += 1
                }
                Ok(BackfillOutcome::NotEligible) => result
                    .failures
                    .push(fail("Left off by the plan's eligibility rule".to_owned())),
                Err(report) if transient(report.current_context()) => {
                    return Err(format!("Starting customer {raw}: {report:?}"));
                }
                Err(report) => result.failures.push(fail(match report.current_context() {
                    StoreError::InvalidArgument(reason) | StoreError::ValueNotFound(reason) => {
                        reason.clone()
                    }
                    other => other.to_string(),
                })),
            }
        }
        Ok(result)
    }

    fn chunk_size(&self) -> i32 {
        CHUNK_SIZE
    }
}
