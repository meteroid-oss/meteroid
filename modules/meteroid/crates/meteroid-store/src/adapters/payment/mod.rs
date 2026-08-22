//! Provider-agnostic payment connector layer.
//!
//! The core interacts with payment providers exclusively through the
//! [`PaymentConnector`] trait; adapters translate at the boundary so no
//! provider-specific type leaks. The trait splits into ops sub-traits;
//! unsupported methods return [`ConnectorError::Unsupported`] rather than panic.

pub mod bridge;
pub mod connector;
#[cfg(any(test, feature = "test-utils"))]
pub mod contract;
pub mod error;
pub mod events;
pub mod factory;
pub mod gocardless;
pub mod mock;
pub mod model;
pub mod stancer;
pub mod stripe;

pub use factory::{initialize_payment_connector, provider_capabilities};
pub use gocardless::GoCardlessConnector;
pub use mock::MockConnector;
pub use stancer::StancerConnector;
pub use stripe::StripeConnector;

pub use connector::{
    ConnectorCapabilities, ConnectorIdentity, CustomerOps, HostedSetupCompletion, MandateOps,
    MandateSetupMode, PaymentConnector, PaymentOps, ReconcileOps, RefundOps, WebhookOps,
};
pub use error::{ConnectorError, HostedSetupPending};
pub use events::{
    DisputeEvent, NormalizedEventKind, NormalizedEventSubscription, NormalizedWebhookEvent,
    PaymentFailedEvent, PaymentMethodAttachedEvent, PaymentMethodDetachedEvent,
    PaymentMethodExpiringEvent, PaymentMethodUpdatedEvent, PaymentPendingEvent,
    PaymentRefundedEvent, PaymentRequiresActionEvent, PaymentSucceededEvent,
};
pub use model::{
    ChargeAcknowledged, ChargeFailure, ChargeOutcome, ChargeReceipt, ChargeRequest,
    CreateCustomerRequest, DeclineKind, ExternalCustomerRef, IdempotencyKey,
    MandateSetupInstruction, MandateSetupRequest, PaymentMethodSnapshot, RefundAcknowledged,
    RefundFailure, RefundOutcome, RefundReason, RefundReceipt, RefundRequest, RegisteredWebhook,
    RemoteTransactionStatus, RequiresActionInstruction,
};
