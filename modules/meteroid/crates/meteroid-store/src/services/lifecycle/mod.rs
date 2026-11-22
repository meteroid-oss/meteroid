pub mod controller;
pub(crate) mod derived;

/// A read the metering service cannot serve yet (one predating usage filters, during a deploy) is
/// no failure of the work: it is retried later without counting against its retries.
pub(super) fn waits_on_metering(error: &error_stack::Report<crate::errors::StoreError>) -> bool {
    error.frames().any(|frame| {
        frame
            .downcast_ref::<crate::errors::StoreError>()
            .is_some_and(|error| matches!(error, crate::errors::StoreError::MeteringNotReady))
    })
}

/// A document whose credit shares do not add up to its draws (`settle`'s `Unbalanced`): the same
/// inputs fail the same way, so the work stops and is logged instead of being retried.
pub(super) fn violates_invariant(error: &error_stack::Report<crate::errors::StoreError>) -> bool {
    error.frames().any(|frame| {
        frame
            .downcast_ref::<crate::errors::StoreError>()
            .is_some_and(|error| {
                matches!(
                    error,
                    crate::errors::StoreError::BillingInvariantViolated(_)
                )
            })
    })
}

/// When work waiting on the metering service is tried again.
pub(super) fn metering_retry_time() -> chrono::NaiveDateTime {
    common_clock::now().naive_utc() + chrono::Duration::minutes(5)
}

#[cfg(test)]
mod tests {
    use crate::errors::StoreError;
    use error_stack::Report;

    /// `settle`'s `Unbalanced`, wrapped by the invoice computation, still stops the work; any
    /// other failure is retried.
    /// Rules: R-095
    #[test]
    fn an_unbalanced_document_is_never_retried() {
        let unbalanced = crate::services::credit::document::settle_error(
            billing_core::credit::SettleError::Unbalanced,
        )
        .change_context(StoreError::InvoiceComputationError);
        assert!(super::violates_invariant(&unbalanced));
        assert!(!super::violates_invariant(&Report::new(
            StoreError::InvoiceComputationError
        )));
    }
}
