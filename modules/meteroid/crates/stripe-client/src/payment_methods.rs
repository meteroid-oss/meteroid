use crate::client::StripeClient;
use crate::error::StripeError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;


#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StripePaymentMethodType {
    BacsDebit,
    Card,
    SepaDebit,
    UsBankAccount, // ACH
}

#[derive(Clone, Debug, Deserialize)]
pub struct PaymentMethod {
    pub id: String,

    #[serde(rename = "type")]
    pub _type: StripePaymentMethodType,

    pub sepa_debit: Option<PaymentMethodSepaDebit>,
    pub us_bank_account: Option<PaymentMethodUsBankDebit>,
    pub bacs_debit: Option<PaymentMethodBacsDebit>,
    pub card: Option<PaymentMethodCard>,



}

#[derive(Clone, Debug, Deserialize)]
pub struct PaymentMethodCard {
    pub country: Option<String>,
    pub fingerprint: Option<String>,
    pub last4: Option<String>,
    pub exp_month: i32,
    pub exp_year: i32,
    pub brand: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PaymentMethodSepaDebit {
    pub bank_code: Option<String>,
    pub branch_code: Option<String>,
    pub country: Option<String>,
    pub fingerprint: Option<String>,
    pub last4: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PaymentMethodUsBankDebit {
    pub routing_number: Option<String>,
    pub fingerprint: Option<String>,
    pub last4: Option<String>,
    pub bank_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PaymentMethodBacsDebit {
    pub sort_code: Option<String>,
    pub fingerprint: Option<String>,
    pub last4: Option<String>,
}

#[async_trait::async_trait]
pub trait PaymentMethodsApi {
    async fn get_payment_method(&self, id: &str, customer_id: &str, secret_key: &SecretString) -> Result<PaymentMethod, StripeError>;
}


// TODO we could support all payment methods, with a generic json fallback
#[async_trait::async_trait]
impl PaymentMethodsApi for StripeClient {
    async fn get_payment_method(&self, id: &str, customer_id: &str, secret_key: &SecretString) -> Result<PaymentMethod, StripeError> {
        self.get(&format!("/customers/{}/payment_methods/{}", customer_id, id), secret_key, RetryStrategy::default())
            .await
    }
}
