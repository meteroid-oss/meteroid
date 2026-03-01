//! PayTheFly Crypto Payment Connector
//!
//! Integrates PayTheFly as a payment service provider for Meteroid.
//! Supports BSC (chainId=56) and TRON (chainId=728126428) blockchain payments
//! using EIP-712 typed structured data signing.
//!
//! ## PayTheFly API Specification
//!
//! - EIP-712 domain: name='PayTheFlyPro', version='1'
//! - Payment URL: https://pro.paythefly.com/pay?chainId=56&projectId=xxx&amount=0.01&...
//! - Amount is human-readable ("0.01"), NOT raw token units
//! - Webhook signature: HMAC-SHA256(data + "." + timestamp, projectKey)
//! - Webhook payload uses: `value` (not amount), `confirmed` (not status)
//! - tx_type: 1=payment, 2=withdrawal
//! - BSC chainId=56 (18 decimals), TRON chainId=728126428 (6 decimals)
//!
//! ## Security
//!
//! - MUST use Keccak-256, NEVER SHA3-256 (different padding)
//! - Webhook signatures use timing-safe comparison
//! - All secrets loaded from encrypted storage

use crate::domain::connectors::{Connector, PayTheFlyPublicData, PayTheFlySensitiveData, ProviderSensitiveData};
use crate::errors::StoreError;
use crate::services::ServicesEdge;
use error_stack::Report;

impl ServicesEdge {
    /// Validate PayTheFly connector configuration.
    ///
    /// Checks that the project ID is set and the chain ID is supported.
    /// Supported chains: BSC (56), TRON (728126428).
    pub async fn validate_paythefly_config(
        &self,
        data: &PayTheFlyPublicData,
    ) -> Result<(), Report<StoreError>> {
        // Validate chain ID
        let supported_chains = [56u64, 728126428u64];
        if !supported_chains.contains(&data.chain_id) {
            return Err(Report::new(StoreError::PaymentProviderError).attach(format!(
                "Unsupported PayTheFly chain ID: {}. Supported: BSC (56), TRON (728126428)",
                data.chain_id
            )));
        }

        // Validate project ID is not empty
        if data.project_id.is_empty() {
            return Err(Report::new(StoreError::PaymentProviderError)
                .attach("PayTheFly project_id is required"));
        }

        Ok(())
    }

    /// Verify a PayTheFly webhook signature.
    ///
    /// Signature: HMAC-SHA256(data + "." + timestamp, projectKey)
    /// Uses constant-time comparison to prevent timing attacks.
    pub fn verify_paythefly_webhook(
        sensitive: &PayTheFlySensitiveData,
        data: &str,
        timestamp: &str,
        signature: &str,
    ) -> Result<bool, Report<StoreError>> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        use subtle::ConstantTimeEq;

        type HmacSha256 = Hmac<Sha256>;

        let mut mac = HmacSha256::new_from_slice(sensitive.project_key.as_bytes())
            .map_err(|_| Report::new(StoreError::PaymentProviderError)
                .attach("Invalid PayTheFly project key for HMAC"))?;

        // PayTheFly signature format: HMAC-SHA256(data + "." + timestamp, projectKey)
        let message = format!("{}.{}", data, timestamp);
        mac.update(message.as_bytes());

        let expected = hex::encode(mac.finalize().into_bytes());

        // Timing-safe comparison to prevent timing attacks
        let expected_bytes = expected.as_bytes();
        let signature_bytes = signature.as_bytes();

        if expected_bytes.len() != signature_bytes.len() {
            return Ok(false);
        }

        Ok(expected_bytes.ct_eq(signature_bytes).into())
    }
}
