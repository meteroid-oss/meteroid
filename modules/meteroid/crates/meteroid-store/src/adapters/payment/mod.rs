//! Provider-agnostic payment connector layer.
//!
//! The core interacts with payment providers exclusively through the
//! [`PaymentConnector`] trait; adapters translate at the boundary so no
//! provider-specific type leaks. The trait splits into ops sub-traits;
//! unsupported methods return [`ConnectorError::Unsupported`] rather than panic.

pub mod account;
pub mod bridge;
pub mod connector;
#[doc(hidden)]
pub mod contract;
pub mod error;
pub mod events;
pub mod factory;
pub mod gocardless;
pub mod mock;
pub mod model;
pub mod mollie;
pub mod stancer;
pub mod stripe;

pub use account::{ValidatedCredentials, is_direct_debit, mask_secret, requires_webhook_secret};
pub use factory::{initialize_payment_connector, payment_capabilities, provider_capabilities};
pub use gocardless::GoCardlessConnector;
pub use mock::{MockConnector, MockProviderInvocation, MockProviderOperation};
pub use mock::{mock_provider_invocations, reset_mock_provider_invocations};
pub use mollie::MollieConnector;
pub use stancer::StancerConnector;
pub use stripe::StripeConnector;

pub use connector::{
    ConnectorIdentity, ConnectorRef, CredentialOps, CustomerOps, HOSTED_RETURN_PATH,
    HostedSetupCompletion, MandateOps, MandateSetupMode, PaymentCapabilities, PaymentConnector,
    PaymentOps, ReconcileOps, RefundOps, WalletOps, WebhookOps,
};
pub use error::{ConnectorError, CredentialsRejected, HostedSetupPending, credentials_rejected};
pub use events::{
    DisputeEvent, NormalizedEventKind, NormalizedEventSubscription, NormalizedWebhookEvent,
    ObservedRefundOnWire, PaymentFailedEvent, PaymentMethodAttachedEvent,
    PaymentMethodDetachedEvent, PaymentMethodExpiringEvent, PaymentMethodUpdatedEvent,
    PaymentPendingEvent, PaymentRefundedEvent, PaymentRequiresActionEvent, PaymentReversedEvent,
    PaymentSucceededEvent, RefundStatusChangedEvent,
};
pub use model::{
    ChargeAcknowledged, ChargeFailure, ChargeOutcome, ChargeReceipt, ChargeRequest,
    CreateCustomerRequest, DeclineKind, ExternalCustomerRef, IdempotencyKey,
    MandateSetupInstruction, MandateSetupRequest, PaymentMethodSnapshot,
    PrepareWalletChargeRequest, RefundAcknowledged, RefundFailure, RefundOutcome, RefundReason,
    RefundReceipt, RefundRequest, RefundSnapshot, RegisteredWebhook, RemoteRefundStatus,
    RemoteTransactionStatus, RequiresActionInstruction, WebhookDeliveryUnit,
};
