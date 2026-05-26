//! Provider-agnostic payment connector layer.
//!
//! The core code interacts with payment providers (Stripe, GoCardless, Adyen, …)
//! exclusively through the [`PaymentConnector`] trait. No provider-specific
//! type ever crosses into the rest of the codebase: adapters translate at the
//! boundary.
//!
//! The trait is split into ops sub-traits so a single provider impl reads as a
//! collection of focused responsibilities, and so a test double can implement
//! only the surface it needs. Methods a provider doesn't support return
//! [`ConnectorError::Unsupported`] rather than panicking.
//!
//! See [`connector`] for the trait family, [`model`] for request/outcome
//! types, [`events`] for the normalized webhook event vocabulary.

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
pub mod stripe;

pub use factory::initialize_payment_connector;
pub use gocardless::GoCardlessConnector;
pub use mock::MockConnector;
pub use stripe::StripeConnector;

pub use connector::{
    ConnectorCapabilities, ConnectorIdentity, CustomerOps, MandateOps, MandateSetupMode,
    PaymentConnector, PaymentOps, ReconcileOps, RefundOps, WebhookOps,
};
pub use error::ConnectorError;
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
