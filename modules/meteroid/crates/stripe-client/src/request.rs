use reqwest::StatusCode;
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum RetryStrategy {
    /// No retries
    NoRetry,
    /// Run it with given params.
    Retry(RetryParams),
}

impl RetryStrategy {
    pub fn default() -> RetryStrategy {
        RetryStrategy::Retry(RetryParams {
            count: 5,
            backoff: Backoff::Exponential(Duration::from_millis(100)),
        })
    }

    pub fn test(
        &self,
        status: Option<StatusCode>,
        stripe_should_retry: Option<bool>,
        retry_count: u32,
    ) -> Outcome {
        // if stripe explicitly says not to retry then don't
        if !stripe_should_retry.unwrap_or(true) {
            return Outcome::Stop;
        }

        use RetryStrategy::{NoRetry, Retry};

        match (self, status, retry_count) {
            (NoRetry, _, _) => Outcome::Stop,
            // client errors usually cannot be solved with retries
            // see: https://stripe.com/docs/error-handling#content-errors
            (_, Some(s), _) if s.is_client_error() => Outcome::Stop,

            (Retry(params), _, c) if c < params.count => match params.backoff {
                Backoff::Fixed(duration) => Outcome::Continue(duration),
                Backoff::Exponential(duration) => Outcome::Continue(calculate_backoff(duration, c)),
            },

            // stop unknown cases should to prevent infinite loops
            _ => Outcome::Stop,
        }
    }
}

fn calculate_backoff(initial_delay: Duration, retry_count: u32) -> Duration {
    // initial delay is not expected to be a big number so downcasting it to u64 should be fine
    Duration::from_millis(2_u64.pow(retry_count) * initial_delay.as_millis() as u64)
}

#[derive(PartialEq, Eq, Debug)]
pub enum Outcome {
    Stop,
    Continue(Duration),
}

#[derive(Clone, Debug)]
pub struct RetryParams {
    /// max number of retries.
    pub count: u32,
    /// back-off Strategy
    pub backoff: Backoff,
}

#[derive(Clone, Debug)]
pub enum Backoff {
    /// fixed delays between retries
    Fixed(Duration),
    /// exponential delays between retries with initial duration
    Exponential(Duration),
}

#[cfg(test)]
mod tests {
    use crate::request::{Backoff, RetryParams};
    use reqwest::StatusCode;
    use std::time::Duration;

    use super::{Outcome, RetryStrategy};

    #[test]
    fn test_no_retry_strategy() {
        let strategy = RetryStrategy::NoRetry;
        assert_eq!(strategy.test(None, None, 1), Outcome::Stop);
        assert_eq!(
            strategy.test(Some(StatusCode::INTERNAL_SERVER_ERROR), None, 1),
            Outcome::Stop
        );
    }

    #[test]
    fn test_fixed_retry_strategy() {
        let strategy = RetryStrategy::Retry(RetryParams {
            count: 3,
            backoff: Backoff::Fixed(Duration::from_secs(1)),
        });

        assert_eq!(strategy.test(None, None, 3), Outcome::Stop);
        assert_eq!(
            strategy.test(Some(StatusCode::INTERNAL_SERVER_ERROR), Some(false), 1),
            Outcome::Stop
        );

        assert_eq!(
            strategy.test(Some(StatusCode::BAD_REQUEST), None, 1),
            Outcome::Stop
        );

        assert_eq!(
            strategy.test(Some(StatusCode::INTERNAL_SERVER_ERROR), None, 2),
            Outcome::Continue(Duration::from_secs(1))
        );
    }

    #[test]
    fn test_exponential_retry_strategy() {
        let strategy = RetryStrategy::Retry(RetryParams {
            count: 3,
            backoff: Backoff::Exponential(Duration::from_secs(1)),
        });

        assert_eq!(strategy.test(None, None, 3), Outcome::Stop);
        assert_eq!(
            strategy.test(Some(StatusCode::INTERNAL_SERVER_ERROR), Some(false), 1),
            Outcome::Stop
        );

        assert_eq!(
            strategy.test(Some(StatusCode::BAD_REQUEST), None, 1),
            Outcome::Stop
        );

        assert_eq!(
            strategy.test(Some(StatusCode::INTERNAL_SERVER_ERROR), None, 2),
            Outcome::Continue(Duration::from_secs(4))
        );
    }
}
