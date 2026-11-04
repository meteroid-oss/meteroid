//! Invoice-time customer credit: purchases and top-ups, automatic top-ups, the credit an invoice
//! draws, and the errors the ledger reports.
use crate::errors::StoreError;
use error_stack::Report;

pub(crate) mod billing_windows;
pub(crate) mod collected_payment;
pub(crate) mod coupon_grant;
pub(crate) mod expiry_settlement;
pub(crate) mod forecast;
pub mod metrics;
pub(crate) mod mint;
pub(crate) mod payment_reversal;
pub(crate) mod prepaid_purchase_tax;
pub(crate) mod promotional;
pub(crate) mod public_api;
pub(crate) mod purchase;
mod replay_top_up;
pub(crate) mod reversal_sweep;
pub(crate) mod scope;
pub(crate) mod subscription_end;
pub(crate) mod top_up_checkout;
pub(crate) mod top_up_support;
pub(crate) mod unit_wallets;
pub(crate) mod value_application;
mod wallet_payment;
pub(crate) mod wallet_purchase;
mod wallet_settings;

pub(crate) use payment_reversal::{
    follow_credit_given_back_tx, follow_grants_given_back_tx, reconcile_purchased_credit_tx,
    suspend_top_up_of_refunded_purchase_tx,
};
pub(crate) use wallet_purchase::{ReissuedPurchase, complete_purchase_tx};

/// Caller-caused and fixable by changing the request.
pub fn invalid(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.into()))
}

/// The current state does not allow the request; retrying it unchanged cannot succeed.
pub fn precondition(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::FailedPrecondition(message.into()))
}
