pub mod billing_events;
pub mod period_transitions;

pub use period_transitions::CycleTransitionResult;

/// A read the metering service cannot serve yet (one predating usage filters, during a deploy) is
/// no failure of the work: it is retried later without counting against its retries.
pub(super) fn waits_on_metering(error: &error_stack::Report<crate::errors::StoreError>) -> bool {
    error.frames().any(|frame| {
        frame
            .downcast_ref::<crate::errors::StoreError>()
            .is_some_and(|error| matches!(error, crate::errors::StoreError::MeteringNotReady))
    })
}

/// When work waiting on the metering service is tried again.
pub(super) fn metering_retry_time() -> chrono::NaiveDateTime {
    chrono::Utc::now().naive_utc() + chrono::Duration::minutes(5)
}
