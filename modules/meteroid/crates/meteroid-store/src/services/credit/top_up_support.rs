//! Shared by self-service and automatic top-ups: the anchor operation's metadata, the smallest
//! charge, and the provider errors as stored and as a customer may read them.
use super::{invalid, precondition};
use crate::StoreResult;
use chrono::{DateTime, Utc};
use common_domain::ids::CustomerPaymentMethodId;
use diesel_models::customer_credit::CustomerCreditOperationRow;
use serde_json::Value;

pub(crate) const MAX_ERROR_BYTES: usize = 500;

/// Card networks decline dust: the smallest charge, in minor units, of a currency with decimals.
const MIN_CHARGE_MINOR_UNITS: i64 = 100;

/// The smallest amount a top-up may charge in `currency`.
pub fn minimum_charge_minor(currency: &str) -> StoreResult<i64> {
    let precision = crate::constants::Currencies::resolve_currency_precision(currency)
        .ok_or_else(|| precondition(format!("unsupported top-up currency {currency}")))?;
    Ok(if precision == 0 {
        1
    } else {
        MIN_CHARGE_MINOR_UNITS
    })
}

pub(crate) fn metadata_payment_method_id(metadata: &Value) -> StoreResult<CustomerPaymentMethodId> {
    metadata
        .get("payment_method_id")
        .and_then(Value::as_str)
        .ok_or_else(|| precondition("top-up is missing its payment method"))?
        .parse::<CustomerPaymentMethodId>()
        .map_err(|error| invalid(format!("top-up payment method is invalid: {error}")))
}

/// The expiry a manual top-up's anchor carries, if the request set one; absent for every
/// automatic top-up and for a manual one that never expires.
pub(crate) fn metadata_expires_at(metadata: &Value) -> StoreResult<Option<DateTime<Utc>>> {
    match metadata.get("expires_at") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
            .map(|at| at.with_timezone(&Utc))
            .map(Some)
            .ok_or_else(|| invalid("top-up expiry is invalid")),
    }
}

pub fn metadata_object(value: &mut Value) -> StoreResult<&mut serde_json::Map<String, Value>> {
    value
        .as_object_mut()
        .ok_or_else(|| precondition("top-up metadata is not an object"))
}

pub(crate) fn increment_attempts(mut metadata: Value) -> StoreResult<Value> {
    let object = metadata_object(&mut metadata)?;
    let attempts = object
        .get(CustomerCreditOperationRow::ATTEMPTS_KEY)
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| invalid("top-up attempt count overflowed"))?;
    object.insert(
        CustomerCreditOperationRow::ATTEMPTS_KEY.to_owned(),
        Value::from(attempts),
    );
    Ok(metadata)
}

pub fn truncate_error(mut value: String) -> String {
    if value.len() > MAX_ERROR_BYTES {
        // Error text may be a Debug report with multi-byte box-drawing characters.
        let mut end = MAX_ERROR_BYTES;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
    }
    value
}

/// Customer-safe rendering of a provider failure: the stored error can carry account and request
/// ids and must never reach a customer. Only a recognised decline class survives.
pub fn customer_safe_decline_reason(message: &str) -> String {
    let lowered = message.to_ascii_lowercase();
    CUSTOMER_SAFE_DECLINES
        .iter()
        .find(|(needle, _)| lowered.contains(needle))
        .map_or(GENERIC_DECLINE, |(_, safe)| *safe)
        .to_owned()
}

pub const GENERIC_DECLINE: &str = "Your payment was declined.";

/// Most specific first: `insufficient_funds` and `expired_card` are also reported as
/// `card_declined`.
const CUSTOMER_SAFE_DECLINES: &[(&str, &str)] = &[
    (
        "insufficient_funds",
        "Your card was declined for insufficient funds.",
    ),
    ("expired_card", "Your card has expired."),
    ("incorrect_cvc", "Your card's security code was incorrect."),
    (
        "incorrect_number",
        "Your card number could not be verified.",
    ),
    (
        "processing_error",
        "Your card issuer could not process the payment.",
    ),
    (
        "authentication_required",
        "Your bank asked you to confirm this payment.",
    ),
    (
        "requires_action",
        "Your bank asked you to confirm this payment.",
    ),
    ("card_declined", "Your card was declined."),
    ("card was declined", "Your card was declined."),
    ("card_error", "Your card was declined."),
    ("no payment method", "No usable payment method is on file."),
    (
        "top-up stopped:",
        "These credits can't be bought right now, so automatic top-ups are paused.",
    ),
    (
        "price changed: review",
        "The price of these credits changed: review your automatic top-up to resume it.",
    ),
    (
        "payment was reversed",
        "A recent top-up payment was reversed.",
    ),
    (
        "terms changed by the merchant",
        "Your automatic top-up was changed: confirm the new terms to resume it.",
    ),
    (
        "more than the self-service limit",
        "One top-up would charge more than the limit for automatic top-ups: review your terms.",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_error_splits_on_char_boundary() {
        let value = "├╴".repeat(300);
        assert!(value.len() > MAX_ERROR_BYTES);
        let truncated = truncate_error(value);
        assert!(truncated.len() <= MAX_ERROR_BYTES);
        assert!(truncated.len() > MAX_ERROR_BYTES - 4);
        assert!(truncated.chars().all(|c| c == '├' || c == '╴'));
    }

    #[test]
    fn truncate_error_keeps_short_values() {
        let value = "card_declined ├╴ retry".to_owned();
        assert_eq!(truncate_error(value.clone()), value);
        let exact = "a".repeat(MAX_ERROR_BYTES);
        assert_eq!(truncate_error(exact.clone()), exact);
    }
}
