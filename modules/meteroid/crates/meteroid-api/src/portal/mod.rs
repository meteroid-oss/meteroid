pub use meteroid_app::portal::branding;
pub mod checkout;
pub mod customer;
pub mod invoice;
pub mod pricing;
pub mod purchase;
pub mod quotes;
pub mod shared;
pub mod subscription;
pub mod upcoming;
pub mod wallets;

/// What a customer reads when the portal fails on our side; the detail stays in the error source,
/// which is logged server-side.
pub(crate) const INTERNAL_ERROR: &str =
    "Something went wrong on our side. Please try again in a moment.";

/// A payment refusal as the paying customer may read it: a recognised decline class, never the
/// provider's text, and what to do next.
pub(crate) fn customer_payment_message(message: &str) -> String {
    if message.contains("already being processed") {
        return message.to_string();
    }
    format!(
        "{} Please try again or use a different payment method.",
        meteroid_store::services::customer_safe_decline_reason(message)
    )
}

/// A transaction shown to the paying customer carries only the decline class.
pub(crate) fn customer_facing_transaction(
    mut transaction: meteroid_grpc::meteroid::api::invoices::v1::Transaction,
) -> meteroid_grpc::meteroid::api::invoices::v1::Transaction {
    transaction.error = transaction
        .error
        .as_deref()
        .map(meteroid_store::services::customer_safe_decline_reason);
    transaction
}

#[cfg(test)]
mod tests {
    use super::customer_payment_message;

    #[test]
    fn a_decline_reads_as_its_class_never_the_provider_text() {
        assert_eq!(
            customer_payment_message("Test gateway: payment declined (card_declined)"),
            "Your card was declined. Please try again or use a different payment method."
        );
        assert_eq!(
            customer_payment_message("acct_123: something odd"),
            "Your payment was declined. Please try again or use a different payment method."
        );
    }
}
