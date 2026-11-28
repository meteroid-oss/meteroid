//! Which workflow runs caused an event (ADR-0016 §6). An action runs inside [`scope`]; every
//! event the store publishes meanwhile carries the chain in its pgmq headers, so the workflow
//! matcher can refuse a recursion.

use common_domain::ids::{WorkflowId, WorkflowRunId};
use serde::{Deserialize, Serialize};
use std::future::Future;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginLink {
    pub workflow_id: WorkflowId,
    pub run_id: WorkflowRunId,
}

tokio::task_local! {
    static ORIGIN: Vec<OriginLink>;
}

/// Runs `fut` with `chain` (oldest run first) as the origin of the events it publishes.
pub async fn scope<F: Future>(chain: Vec<OriginLink>, fut: F) -> F::Output {
    ORIGIN.scope(chain, fut).await
}

pub fn current() -> Vec<OriginLink> {
    ORIGIN.try_with(Clone::clone).unwrap_or_default()
}
