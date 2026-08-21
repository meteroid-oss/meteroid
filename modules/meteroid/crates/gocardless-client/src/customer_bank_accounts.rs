//! Customer bank accounts. We only read one (after a mandate is created) to
//! recover a display hint for the account — GoCardless returns the last couple
//! of digits in `account_number_ending`, never the full number.

use crate::client::GoCardlessClient;
use crate::error::GoCardlessError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct CustomerBankAccount {
    pub id: String,
    /// Last digits of the account number (e.g. "11"); the full number is never
    /// returned. May be absent for some schemes.
    pub account_number_ending: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CustomerBankAccountEnvelope {
    customer_bank_accounts: CustomerBankAccount,
}

#[async_trait::async_trait]
pub trait CustomerBankAccountApi {
    async fn get_customer_bank_account(
        &self,
        id: &str,
        access_token: &SecretString,
    ) -> Result<CustomerBankAccount, GoCardlessError>;
}

#[async_trait::async_trait]
impl CustomerBankAccountApi for GoCardlessClient {
    async fn get_customer_bank_account(
        &self,
        id: &str,
        access_token: &SecretString,
    ) -> Result<CustomerBankAccount, GoCardlessError> {
        let resp: CustomerBankAccountEnvelope = self
            .get(
                &format!("/customer_bank_accounts/{id}"),
                access_token,
                RetryStrategy::default(),
            )
            .await?;
        Ok(resp.customer_bank_accounts)
    }
}
