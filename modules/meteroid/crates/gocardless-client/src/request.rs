use std::time::Duration;

/// Retry policy for outbound calls.
///
/// Like in `stripe-client` the connector adapter wraps each call once; this
/// strategy controls the *inner* retries the HTTP client does on transient
/// network errors before surfacing failure to the adapter.
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
