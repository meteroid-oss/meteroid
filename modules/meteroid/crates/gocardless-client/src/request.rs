use std::time::Duration;

/// Inner retries the HTTP client performs on transient network errors before
/// surfacing failure to the adapter (which wraps each call once).
#[derive(Clone, Debug)]
pub enum RetryStrategy {
    NoRetry,
    Retry(RetryParams),
}

#[derive(Clone, Debug)]
pub struct RetryParams {
    pub count: u8,
    pub backoff: Backoff,
}

#[derive(Clone, Debug)]
pub enum Backoff {
    Exponential(Duration),
}

impl RetryStrategy {
    pub fn default() -> Self {
        RetryStrategy::Retry(RetryParams {
            count: 3,
            backoff: Backoff::Exponential(Duration::from_millis(200)),
        })
    }
}
