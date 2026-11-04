//! Outbox payloads of invoice-credit wallets. Amounts are minor units of the wallet currency. Ids
//! are derived from what they report, so a replayed producer never notifies twice.
use crate::domain::credit::{
    CreditApplication, CreditGrantSource, GrantClassification, GrantScope,
};
use chrono::{DateTime, Utc};
use common_domain::ids::{
    BaseId, CreditGrantId, CreditNoteId, CreditOperationId, CustomerId, EventId, InvoiceId,
    LedgerEntryId, PaymentTransactionId, ProductId, SubscriptionId, TenantId, WalletId,
};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use uuid::Uuid;

fn derived_id(routing_key: &str, parts: &[Uuid]) -> EventId {
    let mut identity = routing_key.to_owned();
    for part in parts {
        identity.push(':');
        identity.push_str(&part.to_string());
    }
    Uuid::new_v5(&Uuid::NAMESPACE_OID, identity.as_bytes()).into()
}

fn number_part(value: i64) -> Uuid {
    Uuid::from_u64_pair(0, value.cast_unsigned())
}

/// Credit entered a wallet: a grant was funded, or a void or an unbilled usage settlement
/// gave back what it drew.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletFundedEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub ledger_entry_id: LedgerEntryId,
    pub credit_grant_id: CreditGrantId,
    pub invoice_id: Option<InvoiceId>,
    pub credit_note_id: Option<CreditNoteId>,
    pub amount_minor: i64,
    pub currency: String,
    /// Why the grant exists; a returned draw reports the credit note or void that returned it.
    pub source: CreditGrantSource,
    pub classification: GrantClassification,
    pub application: CreditApplication,
    /// The subscription the grant belongs to; absent pays any of the customer's.
    #[serde(default)]
    pub subscription_id: Option<SubscriptionId>,
    /// The products the grant pays for; empty pays every product.
    #[serde(default)]
    pub product_ids: Vec<ProductId>,
    /// Set when `currency` is a credit unit code: the decimal places of its amounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit_precision: Option<u8>,
}

impl WalletFundedEvent {
    pub const ROUTING_KEY: &'static str = "wallet.funded";

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        scope: &GrantScope,
        ledger_entry_id: LedgerEntryId,
        credit_grant_id: CreditGrantId,
        invoice_id: Option<InvoiceId>,
        credit_note_id: Option<CreditNoteId>,
        amount_minor: i64,
        currency: String,
        source: CreditGrantSource,
        classification: GrantClassification,
    ) -> Self {
        Self {
            id: derived_id(Self::ROUTING_KEY, &[ledger_entry_id.as_uuid()]),
            tenant_id,
            customer_id,
            wallet_id,
            application: scope.application,
            subscription_id: scope.subscription_id,
            product_ids: scope.product_ids.clone(),
            ledger_entry_id,
            credit_grant_id,
            invoice_id,
            credit_note_id,
            amount_minor,
            currency,
            source,
            classification,
            credit_unit_precision: None,
        }
    }
}

/// Credit left a wallet other than by expiry: applied to an invoice, taken back by a credit note,
/// or removed by an operator.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletDebitedEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub ledger_entry_id: LedgerEntryId,
    pub credit_grant_id: CreditGrantId,
    pub invoice_id: Option<InvoiceId>,
    pub credit_note_id: Option<CreditNoteId>,
    pub amount_minor: i64,
    pub currency: String,
    pub reason: String,
    /// The user who removed the credit; absent for a document-driven movement.
    pub actor_id: Option<Uuid>,
    pub application: CreditApplication,
    /// The subscription the grant belongs to; absent pays any of the customer's.
    #[serde(default)]
    pub subscription_id: Option<SubscriptionId>,
    /// The products the grant pays for; empty pays every product.
    #[serde(default)]
    pub product_ids: Vec<ProductId>,
    /// Set when `currency` is a credit unit code: the decimal places of its amounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit_precision: Option<u8>,
}

impl WalletDebitedEvent {
    pub const ROUTING_KEY: &'static str = "wallet.debited";

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        scope: &GrantScope,
        ledger_entry_id: LedgerEntryId,
        credit_grant_id: CreditGrantId,
        invoice_id: Option<InvoiceId>,
        credit_note_id: Option<CreditNoteId>,
        amount_minor: i64,
        currency: String,
        reason: String,
        actor_id: Option<Uuid>,
    ) -> Self {
        Self {
            id: derived_id(Self::ROUTING_KEY, &[ledger_entry_id.as_uuid()]),
            tenant_id,
            customer_id,
            wallet_id,
            application: scope.application,
            subscription_id: scope.subscription_id,
            product_ids: scope.product_ids.clone(),
            ledger_entry_id,
            credit_grant_id,
            invoice_id,
            credit_note_id,
            amount_minor,
            currency,
            reason,
            actor_id,
            credit_unit_precision: None,
        }
    }
}

/// Why purchased credit that was already spent lost the money behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreditUnrecoveredCause {
    /// A failed collection clawed back: the reopened invoice collects it again.
    PaymentReversed,
    /// Lost to a dispute or chargeback: the invoice stays paid, the merchant bears the loss.
    PaymentDisputed,
    CreditNoteRefunded,
    /// Overpaid money refunded without a credit note after its credit was spent.
    PaymentRefunded,
}

impl CreditUnrecoveredCause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PaymentReversed => "PAYMENT_REVERSED",
            Self::PaymentDisputed => "PAYMENT_DISPUTED",
            Self::CreditNoteRefunded => "CREDIT_NOTE_REFUNDED",
            Self::PaymentRefunded => "PAYMENT_REFUNDED",
        }
    }
}

/// The money flow a loss is reported on: an invoice's payments, one refund note of it, or the
/// credit kept from an overpayment. Each keeps its own running total, so their notices never net
/// against each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreditUnrecoveredTrigger {
    Payments(CreditUnrecoveredCause),
    CreditNote(CreditNoteId),
    /// Credit kept from an overpayment (`Uuid` = the flow's key) taken back as the money went:
    /// reversed by the bank, or refunded.
    Overpayment(Uuid, CreditUnrecoveredCause),
}

impl CreditUnrecoveredTrigger {
    pub fn cause(self) -> CreditUnrecoveredCause {
        match self {
            Self::Payments(cause) | Self::Overpayment(_, cause) => cause,
            Self::CreditNote(_) => CreditUnrecoveredCause::CreditNoteRefunded,
        }
    }

    /// The flow's key; an invoice's payments are keyed by the invoice.
    pub fn source_id(self, invoice_id: Option<InvoiceId>) -> Option<Uuid> {
        match self {
            Self::Payments(_) => invoice_id.map(|invoice| invoice.as_uuid()),
            Self::CreditNote(note_id) => Some(note_id.as_uuid()),
            Self::Overpayment(key, _) => Some(key),
        }
    }

    fn credit_note_id(self) -> Option<CreditNoteId> {
        match self {
            Self::Payments(_) | Self::Overpayment(..) => None,
            Self::CreditNote(note_id) => Some(note_id),
        }
    }
}

/// One notice moving a wallet's reported loss on one money flow to `total_minor`.
#[derive(Debug, Clone)]
pub struct CreditLossNotice {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    /// The invoice whose money it was; absent for a payment no invoice took.
    pub invoice_id: Option<InvoiceId>,
    /// The payment whose reversal or reinstatement moved the loss, when one did.
    pub payment_id: Option<PaymentTransactionId>,
    pub trigger: CreditUnrecoveredTrigger,
    /// The flow's key (`CreditUnrecoveredTrigger::source_id`).
    pub source_id: Uuid,
    pub currency: String,
    pub credit_unit_precision: Option<u8>,
    /// 1-based position among the flow's notices; persisted with the total, so a replay
    /// derives the same id and a later notice to an equal total a new one.
    pub sequence: i32,
    pub total_minor: i64,
}

impl CreditLossNotice {
    fn id(&self, routing_key: &str) -> EventId {
        derived_id(
            routing_key,
            &[
                self.wallet_id.as_uuid(),
                self.invoice_id
                    .map_or(self.source_id, |invoice| invoice.as_uuid()),
                self.source_id,
                number_part(i64::from(self.sequence)),
                number_part(self.total_minor),
            ],
        )
    }
}

/// The money behind purchased credit came back (reversed payment, refund) after part of that
/// credit was spent, so the spent part could not be taken back from the wallet. Each notice
/// carries what it adds; with `wallet.credit_recovered` they net to the current loss.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletCreditUnrecoveredEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub invoice_id: Option<InvoiceId>,
    #[serde(default)]
    pub payment_id: Option<PaymentTransactionId>,
    pub credit_note_id: Option<CreditNoteId>,
    pub cause: CreditUnrecoveredCause,
    pub currency: String,
    pub taken_back_minor: i64,
    pub unrecovered_minor: i64,
    /// What this invoice's money flow (its payments, or the refund note) has lost in all.
    pub unrecovered_total_minor: i64,
    /// Set when `currency` is a credit unit code: the decimal places of its amounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit_precision: Option<u8>,
}

impl WalletCreditUnrecoveredEvent {
    pub const ROUTING_KEY: &'static str = "wallet.credit_unrecovered";

    pub fn new(notice: CreditLossNotice, taken_back_minor: i64, unrecovered_minor: i64) -> Self {
        Self {
            id: notice.id(Self::ROUTING_KEY),
            tenant_id: notice.tenant_id,
            customer_id: notice.customer_id,
            wallet_id: notice.wallet_id,
            invoice_id: notice.invoice_id,
            payment_id: notice.payment_id,
            credit_note_id: notice.trigger.credit_note_id(),
            cause: notice.trigger.cause(),
            currency: notice.currency,
            taken_back_minor,
            unrecovered_minor,
            unrecovered_total_minor: notice.total_minor,
            credit_unit_precision: notice.credit_unit_precision,
        }
    }
}

/// Spent credit an earlier `wallet.credit_unrecovered` reported is backed again: the money came
/// back (payment, won dispute) or the credit was given back and taken back.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletCreditRecoveredEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub invoice_id: Option<InvoiceId>,
    #[serde(default)]
    pub payment_id: Option<PaymentTransactionId>,
    pub credit_note_id: Option<CreditNoteId>,
    /// The cause of the loss this recovers.
    pub cause: CreditUnrecoveredCause,
    pub currency: String,
    pub recovered_minor: i64,
    pub unrecovered_total_minor: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit_precision: Option<u8>,
}

impl WalletCreditRecoveredEvent {
    pub const ROUTING_KEY: &'static str = "wallet.credit_recovered";

    pub fn new(notice: CreditLossNotice, recovered_minor: i64) -> Self {
        Self {
            id: notice.id(Self::ROUTING_KEY),
            tenant_id: notice.tenant_id,
            customer_id: notice.customer_id,
            wallet_id: notice.wallet_id,
            invoice_id: notice.invoice_id,
            payment_id: notice.payment_id,
            credit_note_id: notice.trigger.credit_note_id(),
            cause: notice.trigger.cause(),
            currency: notice.currency,
            recovered_minor,
            unrecovered_total_minor: notice.total_minor,
            credit_unit_precision: notice.credit_unit_precision,
        }
    }
}

/// Why a grant's value lapsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GrantLapseReason {
    #[default]
    Expired,
    /// Its subscription ended at or before its expiry.
    SubscriptionEnded,
}

/// A grant reached its expiry, or the end of its subscription, with value left; that value left
/// the wallet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditGrantExpiredEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub credit_grant_id: CreditGrantId,
    pub ledger_entry_id: LedgerEntryId,
    pub expired_minor: i64,
    pub currency: String,
    pub expired_at: DateTime<Utc>,
    #[serde(default)]
    pub reason: GrantLapseReason,
    pub application: CreditApplication,
    /// The subscription the grant belongs to; absent pays any of the customer's.
    #[serde(default)]
    pub subscription_id: Option<SubscriptionId>,
    /// The products the grant pays for; empty pays every product.
    #[serde(default)]
    pub product_ids: Vec<ProductId>,
    /// Set when `currency` is a credit unit code: the decimal places of its amounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit_precision: Option<u8>,
}

impl CreditGrantExpiredEvent {
    pub const ROUTING_KEY: &'static str = "credit_grant.expired";

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        scope: &GrantScope,
        credit_grant_id: CreditGrantId,
        ledger_entry_id: LedgerEntryId,
        expired_minor: i64,
        currency: String,
        expired_at: DateTime<Utc>,
        reason: GrantLapseReason,
    ) -> Self {
        Self {
            id: derived_id(Self::ROUTING_KEY, &[ledger_entry_id.as_uuid()]),
            tenant_id,
            customer_id,
            wallet_id,
            reason,
            application: scope.application,
            subscription_id: scope.subscription_id,
            product_ids: scope.product_ids.clone(),
            credit_grant_id,
            ledger_entry_id,
            expired_minor,
            currency,
            expired_at,
            credit_unit_precision: None,
        }
    }
}

/// A wallet's available balance fell below its low-balance threshold. Fired once per crossing:
/// the notice re-arms when the balance recovers to the threshold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletBalanceLowEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub currency: String,
    pub available_minor: i64,
    pub threshold_minor: i64,
    /// The part of `available_minor` that pays any charge of any subscription; absent on events
    /// recorded before it was reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_minor: Option<i64>,
    /// Set when `currency` is a credit unit code: the decimal places of its amounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit_precision: Option<u8>,
}

impl WalletBalanceLowEvent {
    pub const ROUTING_KEY: &'static str = "wallet.balance_low";

    pub fn new(
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        currency: String,
        available_minor: i64,
        threshold_minor: i64,
    ) -> Self {
        Self {
            id: EventId::new(),
            tenant_id,
            customer_id,
            wallet_id,
            currency,
            available_minor,
            threshold_minor,
            shared_minor: None,
            credit_unit_precision: None,
        }
    }

    pub fn with_shared(mut self, shared_minor: i64) -> Self {
        self.shared_minor = Some(shared_minor);
        self
    }
}

/// A wallet's live available balance reached zero (`wallet.depleted`), or came back above it
/// (`wallet.recovered`). Fired once per transition, durable across replicas and ticks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletDepletedEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub currency: String,
    /// Set when `currency` is a credit unit code: the decimal places of its amounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit_precision: Option<u8>,
}

impl WalletDepletedEvent {
    pub const ROUTING_KEY: &'static str = "wallet.depleted";

    pub fn new(
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        currency: String,
    ) -> Self {
        Self {
            id: EventId::new(),
            tenant_id,
            customer_id,
            wallet_id,
            currency,
            credit_unit_precision: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletRecoveredEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub currency: String,
    pub available_minor: i64,
    /// Set when `currency` is a credit unit code: the decimal places of its amounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit_precision: Option<u8>,
}

impl WalletRecoveredEvent {
    pub const ROUTING_KEY: &'static str = "wallet.recovered";

    pub fn new(
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        currency: String,
        available_minor: i64,
    ) -> Self {
        Self {
            id: EventId::new(),
            tenant_id,
            customer_id,
            wallet_id,
            currency,
            available_minor,
            credit_unit_precision: None,
        }
    }
}

/// Which spending cap of an automatic top-up suppressed a charge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TopUpCapPeriod {
    Daily,
    Monthly,
}

impl TopUpCapPeriod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "DAILY",
            Self::Monthly => "MONTHLY",
        }
    }
}

/// A spending cap suppressed an automatic top-up that would otherwise have fired. Fired once per
/// cap period: a fresh period is a new window key, so no extra state is kept for the next one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletAutoTopUpCapReachedEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub currency: String,
    pub period: TopUpCapPeriod,
    pub cap_minor: i64,
    /// What the suppressed top-up would have charged.
    pub attempted_amount_minor: i64,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
}

impl WalletAutoTopUpCapReachedEvent {
    pub const ROUTING_KEY: &'static str = "wallet.auto_top_up_cap_reached";

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        currency: String,
        period: TopUpCapPeriod,
        cap_minor: i64,
        attempted_amount_minor: i64,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Self {
        let instant = Uuid::from_u64_pair(
            u64::try_from(period_start.timestamp()).unwrap_or_default(),
            0,
        );
        Self {
            id: derived_id(Self::ROUTING_KEY, &[wallet_id.as_uuid(), instant]),
            tenant_id,
            customer_id,
            wallet_id,
            currency,
            period,
            cap_minor,
            attempted_amount_minor,
            period_start,
            period_end,
        }
    }
}

/// An automatic top-up charge failed (`wallet.auto_top_up_failed`), or its failures suspended the
/// top-up (`wallet.auto_top_up_suspended`). The charge equals the credit it would have added.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletAutoTopUpEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub operation_id: CreditOperationId,
    pub currency: String,
    pub amount_minor: i64,
    pub consecutive_failures: i32,
    pub max_consecutive_failures: i32,
    pub suspended_at: Option<DateTime<Utc>>,
    /// Operator-facing failure text; may carry provider detail and is never shown to a customer.
    pub error: String,
    /// Customer-safe rendering of `error`.
    pub customer_message: String,
}

#[derive(Debug, Clone)]
pub struct WalletAutoTopUpEventMeta {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub operation_id: CreditOperationId,
    pub currency: String,
    pub amount_minor: i64,
    pub consecutive_failures: i32,
    pub max_consecutive_failures: i32,
    pub suspended_at: Option<DateTime<Utc>>,
    pub error: String,
    pub customer_message: String,
}

impl WalletAutoTopUpEvent {
    pub const FAILED_ROUTING_KEY: &'static str = "wallet.auto_top_up_failed";
    pub const SUSPENDED_ROUTING_KEY: &'static str = "wallet.auto_top_up_suspended";

    pub fn failed(meta: &WalletAutoTopUpEventMeta) -> Self {
        Self::new(Self::FAILED_ROUTING_KEY, meta)
    }

    pub fn suspended(meta: &WalletAutoTopUpEventMeta) -> Self {
        Self::new(Self::SUSPENDED_ROUTING_KEY, meta)
    }

    fn new(routing_key: &str, meta: &WalletAutoTopUpEventMeta) -> Self {
        Self {
            id: derived_id(
                routing_key,
                &[meta.wallet_id.as_uuid(), meta.operation_id.as_uuid()],
            ),
            tenant_id: meta.tenant_id,
            customer_id: meta.customer_id,
            wallet_id: meta.wallet_id,
            operation_id: meta.operation_id,
            currency: meta.currency.clone(),
            amount_minor: meta.amount_minor,
            consecutive_failures: meta.consecutive_failures,
            max_consecutive_failures: meta.max_consecutive_failures,
            suspended_at: meta.suspended_at,
            error: meta.error.clone(),
            customer_message: meta.customer_message.clone(),
        }
    }
}

/// The authorization of a wallet's automatic top-up was given, changed or withdrawn. Revisions
/// identify the authorized terms without disclosing payment details.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletAutoTopUpConsentChangedEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub currency: String,
    pub enabled: bool,
    pub previous_configuration_revision: Option<String>,
    pub configuration_revision: Option<String>,
    /// `customer:<id>`, `user:<id>`, `api_token:<id>`, `oauth_app:<id>` or `system`; on
    /// withdrawal, who withdrew it.
    pub actor: String,
    pub changed_at: DateTime<Utc>,
    /// The operator who attested the customer's agreement to the terms now stored, and when.
    #[serde(default)]
    pub consent_attested_by: Option<String>,
    #[serde(default)]
    pub consent_attested_at: Option<DateTime<Utc>>,
}

impl WalletAutoTopUpConsentChangedEvent {
    pub const ROUTING_KEY: &'static str = "wallet.auto_top_up_consent_changed";

    pub fn new(
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        currency: String,
        previous_configuration_revision: Option<String>,
        configuration_revision: Option<String>,
        actor: String,
        changed_at: DateTime<Utc>,
    ) -> Self {
        let instant = Uuid::from_u64_pair(
            u64::try_from(changed_at.timestamp_micros()).unwrap_or_default(),
            0,
        );
        Self {
            id: derived_id(Self::ROUTING_KEY, &[wallet_id.as_uuid(), instant]),
            tenant_id,
            customer_id,
            wallet_id,
            currency,
            enabled: configuration_revision.is_some(),
            previous_configuration_revision,
            configuration_revision,
            actor,
            changed_at,
            consent_attested_by: None,
            consent_attested_at: None,
        }
    }

    pub fn attested(mut self, by: Option<String>, at: Option<DateTime<Utc>>) -> Self {
        self.consent_attested_by = by;
        self.consent_attested_at = at;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ledger_movement_is_identified_by_its_entry_and_event_type() {
        let entry = LedgerEntryId::new();
        let funded = |entry| {
            WalletFundedEvent::new(
                TenantId::new(),
                CustomerId::new(),
                WalletId::new(),
                &GrantScope::customer_wide(),
                entry,
                CreditGrantId::new(),
                None,
                None,
                100,
                "EUR".to_owned(),
                CreditGrantSource::Manual,
                GrantClassification::Promotional,
            )
            .id
        };
        assert_eq!(funded(entry), funded(entry));
        assert_ne!(funded(entry), funded(LedgerEntryId::new()));
        let debited = WalletDebitedEvent::new(
            TenantId::new(),
            CustomerId::new(),
            WalletId::new(),
            &GrantScope::customer_wide(),
            entry,
            CreditGrantId::new(),
            None,
            None,
            100,
            "EUR".to_owned(),
            "Applied to invoice".to_owned(),
            None,
        );
        assert_ne!(funded(entry), debited.id);
    }

    fn notice(
        invoice: InvoiceId,
        trigger: CreditUnrecoveredTrigger,
        sequence: i32,
        total_minor: i64,
    ) -> CreditLossNotice {
        CreditLossNotice {
            tenant_id: TenantId::from_const(Uuid::nil()),
            customer_id: CustomerId::from_const(Uuid::nil()),
            wallet_id: WalletId::from_const(Uuid::nil()),
            invoice_id: Some(invoice),
            payment_id: None,
            trigger,
            source_id: trigger.source_id(Some(invoice)).expect("a flow key"),
            currency: "EUR".to_owned(),
            credit_unit_precision: None,
            sequence,
            total_minor,
        }
    }

    /// Lost, recovered, then lost again to the same total: a total alone would give the third
    /// notice the first one's id and the webhook layer would drop it as a duplicate.
    #[test]
    fn notices_of_one_flow_are_distinct_and_a_replay_is_not() {
        let invoice = InvoiceId::new();
        let lost = |sequence, total| {
            WalletCreditUnrecoveredEvent::new(
                notice(
                    invoice,
                    CreditUnrecoveredTrigger::Payments(CreditUnrecoveredCause::PaymentDisputed),
                    sequence,
                    total,
                ),
                0,
                1_000,
            )
            .id
        };
        assert_eq!(lost(1, 1_000), lost(1, 1_000));
        assert_ne!(lost(1, 1_000), lost(3, 1_000));
        assert_ne!(lost(1, 1_000), lost(2, 2_000));
        let recovered = WalletCreditRecoveredEvent::new(
            notice(
                invoice,
                CreditUnrecoveredTrigger::Payments(CreditUnrecoveredCause::PaymentDisputed),
                1,
                1_000,
            ),
            1_000,
        );
        assert_ne!(
            recovered.id,
            lost(1, 1_000),
            "each event type has its own ids"
        );
        let note = CreditNoteId::new();
        let by_note = WalletCreditUnrecoveredEvent::new(
            notice(
                invoice,
                CreditUnrecoveredTrigger::CreditNote(note),
                1,
                1_000,
            ),
            0,
            1_000,
        );
        assert_ne!(by_note.id, lost(1, 1_000), "each flow has its own ids");
        assert_eq!(by_note.credit_note_id, Some(note));
        assert_eq!(by_note.cause, CreditUnrecoveredCause::CreditNoteRefunded);
    }

    #[test]
    fn an_auto_top_up_failure_notifies_once_per_operation() {
        let meta = WalletAutoTopUpEventMeta {
            tenant_id: TenantId::new(),
            customer_id: CustomerId::new(),
            wallet_id: WalletId::new(),
            operation_id: CreditOperationId::new(),
            currency: "EUR".to_owned(),
            amount_minor: 1_000,
            consecutive_failures: 3,
            max_consecutive_failures: 3,
            suspended_at: None,
            error: "card_declined".to_owned(),
            customer_message: "Your card was declined.".to_owned(),
        };
        assert_eq!(
            WalletAutoTopUpEvent::failed(&meta).id,
            WalletAutoTopUpEvent::failed(&meta).id
        );
        assert_ne!(
            WalletAutoTopUpEvent::failed(&meta).id,
            WalletAutoTopUpEvent::suspended(&meta).id
        );
    }
}

macro_rules! in_credit_unit {
    ($($event:ty),+) => {$(
        impl $event {
            /// Marks the amounts as minor units of a credit unit of this precision.
            pub fn in_credit_unit(mut self, precision: Option<u8>) -> Self {
                self.credit_unit_precision = precision;
                self
            }
        }
    )+};
}

in_credit_unit!(
    WalletFundedEvent,
    WalletDebitedEvent,
    CreditGrantExpiredEvent,
    WalletBalanceLowEvent,
    WalletDepletedEvent,
    WalletRecoveredEvent
);
