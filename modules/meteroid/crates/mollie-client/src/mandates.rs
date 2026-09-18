use crate::client::MollieClient;
use crate::error::MollieError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;

/// `mandate-status` schema.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MandateStatus {
    Valid,
    Pending,
    Invalid,
    #[serde(other)]
    Unknown,
}

/// `cardExpiryDate` is `YYYY-MM-DD` here (a payment's card details use `MM/YY`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MandateDetails {
    pub card_holder: Option<String>,
    /// Last four digits of the PAN.
    pub card_number: Option<String>,
    /// Brand label, e.g. `Visa`, `Mastercard`, `American Express`.
    pub card_label: Option<String>,
    /// Stable per card number; absent for non-card mandates.
    pub card_fingerprint: Option<String>,
    pub card_expiry_date: Option<String>,
    pub consumer_name: Option<String>,
    /// IBAN (SEPA) or e-mail (PayPal).
    pub consumer_account: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MollieMandate {
    pub id: String,
    pub mode: Option<String>,
    pub status: MandateStatus,
    /// `creditcard` | `directdebit` | `paypal` (| `bacs`).
    pub method: Option<String>,
    pub customer_id: Option<String>,
    #[serde(default)]
    pub details: Option<MandateDetails>,
    pub mandate_reference: Option<String>,
    pub signature_date: Option<String>,
    pub created_at: Option<String>,
}

impl MollieClient {
    pub async fn get_mandate(
        &self,
        customer_id: &str,
        mandate_id: &str,
        api_key: &SecretString,
    ) -> Result<MollieMandate, MollieError> {
        self.get(
            &format!("/customers/{customer_id}/mandates/{mandate_id}"),
            api_key,
            RetryStrategy::default(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_mandate_deserializes() {
        let mandate: MollieMandate = serde_json::from_str(
            r#"{"resource":"mandate","id":"mdt_h3gAaD5zP","mode":"test","status":"valid",
                "method":"creditcard","customerId":"cst_4qqhO89gsT",
                "details":{"cardHolder":"John Doe","cardNumber":"1234","cardLabel":"Mastercard",
                           "cardFingerprint":"fHB3CCKx9REkz8fPplT8N4nq","cardExpiryDate":"2030-12-31"},
                "mandateReference":null,"signatureDate":"2023-05-07","createdAt":"2023-05-07T10:49:08+00:00"}"#,
        )
        .expect("mandate must deserialize");
        assert_eq!(mandate.status, MandateStatus::Valid);
        let details = mandate.details.expect("details");
        assert_eq!(details.card_number.as_deref(), Some("1234"));
        assert_eq!(details.card_label.as_deref(), Some("Mastercard"));
        assert_eq!(details.card_expiry_date.as_deref(), Some("2030-12-31"));
        assert_eq!(
            details.card_fingerprint.as_deref(),
            Some("fHB3CCKx9REkz8fPplT8N4nq")
        );
    }

    #[test]
    fn unknown_status_does_not_fail_parsing() {
        let mandate: MollieMandate =
            serde_json::from_str(r#"{"id":"mdt_x","status":"brand_new"}"#).unwrap();
        assert_eq!(mandate.status, MandateStatus::Unknown);
    }
}
