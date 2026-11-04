//! The wallet mappings the gRPC API and the modules share.
use chrono::{DateTime, Utc};
use meteroid_grpc::meteroid::api::wallets::v1 as server;
use meteroid_store::domain::credit_outlook::CreditGroupKind;

mod error;

pub use error::WalletApiError;

pub fn timestamp(at: DateTime<Utc>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: at.timestamp(),
        nanos: at.timestamp_subsec_nanos() as i32,
    }
}

pub fn group_kind(kind: CreditGroupKind) -> server::CreditGrantKind {
    match kind {
        CreditGroupKind::Included => server::CreditGrantKind::Included,
        CreditGroupKind::Prepaid => server::CreditGrantKind::Prepaid,
        CreditGroupKind::Bonus => server::CreditGrantKind::Bonus,
        CreditGroupKind::AccountBalance => server::CreditGrantKind::AccountBalance,
    }
}

/// The customer portal's wallet messages.
pub mod portal {
    use super::timestamp;
    use meteroid_grpc::meteroid::portal::wallets::v1 as server;
    use meteroid_store::adapters::payment::error::CustomerFacingMessage;
    use meteroid_store::domain::credit::{
        AutoTopUpStopReason, AutomaticTopUpConfig, AutomaticTopUpMethod, TopUpWalletQuote,
    };
    use meteroid_store::domain::credit_units::CreditUnit;
    use meteroid_store::errors::StoreError;
    use meteroid_store::services::customer_safe_decline_reason;
    use tonic::Status;

    use super::WalletApiError;

    /// A store error as the customer reads it: a provider's refusal is shown in its customer-safe
    /// words, which the dashboard never prefers over the operator's detail.
    pub fn portal_error(error: error_stack::Report<StoreError>) -> Status {
        if let Some(CustomerFacingMessage(message)) = error
            .frames()
            .find_map(|frame| frame.downcast_ref::<CustomerFacingMessage>())
        {
            return Status::failed_precondition(message.clone());
        }
        WalletApiError::from(error).into()
    }

    pub fn quote_to_server(quote: TopUpWalletQuote) -> server::TopUpQuote {
        server::TopUpQuote {
            charge: quote.charge_minor,
            currency: quote.currency,
            net: quote.net_minor,
            tax: quote.tax_minor,
        }
    }

    pub fn credit_unit_label(unit: &CreditUnit) -> server::CreditUnitLabel {
        server::CreditUnitLabel {
            code: unit.code.as_str().to_owned(),
            name: unit.name.clone(),
            plural_name: unit.plural_name.clone(),
            symbol: unit.symbol.clone(),
            precision: u32::from(unit.precision),
        }
    }

    fn auto_top_up_method(method: AutomaticTopUpMethod) -> server::AutoTopUpMethod {
        match method {
            AutomaticTopUpMethod::Fixed => server::AutoTopUpMethod::Fixed,
            AutomaticTopUpMethod::ToTarget => server::AutoTopUpMethod::ToTarget,
        }
    }

    pub fn auto_top_up(config: &AutomaticTopUpConfig, enabled: bool) -> server::AutoTopUp {
        server::AutoTopUp {
            method: auto_top_up_method(config.method).into(),
            threshold: config.threshold_minor,
            amount: config.amount_minor,
            target: config.target_minor,
            payment_method_id: config.payment_method_id.map(|id| id.as_proto()),
            minimum: config.minimum_minor,
            maximum: config.maximum_minor,
            daily_cap: config.daily_cap_minor,
            monthly_cap: config.monthly_cap_minor,
            enabled,
            suspended_at: config.suspended_at.map(timestamp),
            last_error: config
                .last_error
                .as_deref()
                .map(customer_safe_decline_reason),
            consecutive_failures: config.consecutive_failures,
            last_succeeded_at: config.last_succeeded_at.map(timestamp),
            stop_reason: match config.stop_reason() {
                None => server::AutoTopUpStopReason::Unspecified,
                Some(AutoTopUpStopReason::PriceChanged) => {
                    server::AutoTopUpStopReason::PriceChanged
                }
                Some(AutoTopUpStopReason::Unsellable) => server::AutoTopUpStopReason::Unsellable,
                Some(AutoTopUpStopReason::PaymentReversed) => {
                    server::AutoTopUpStopReason::PaymentReversed
                }
                Some(AutoTopUpStopReason::PaymentFailed) => {
                    server::AutoTopUpStopReason::PaymentFailed
                }
                Some(AutoTopUpStopReason::AwaitingCustomer) => {
                    server::AutoTopUpStopReason::AwaitingCustomer
                }
            }
            .into(),
            agreed_unit_price: config
                .agreed_price
                .as_ref()
                .map(|price| price.unit_price.normalize().to_string()),
            agreed_price_currency: config
                .agreed_price
                .as_ref()
                .map(|price| price.currency.clone()),
        }
    }
}
