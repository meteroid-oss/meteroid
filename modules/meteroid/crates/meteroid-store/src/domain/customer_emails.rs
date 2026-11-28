use crate::domain::InvoicingEntity;
use crate::domain::payment_reminders::{Audience, SendWindow};
use crate::errors::{StoreError, StoreErrorReport};
use chrono::NaiveDateTime;
use common_domain::ids::{
    BudgetId, CreditNoteId, CreditOperationId, CustomerId, CustomerPaymentMethodId, InvoiceId,
    InvoicingEntityId, PaymentReminderSequenceId, PaymentTransactionId, SubscriptionId, WalletId,
};
use diesel_models::email_kind_audience::EmailKindAudienceRow;
use diesel_models::enums::EmailKindAudienceModeEnum;
use diesel_models::invoicing_entity_email_settings::InvoicingEntityEmailSettingsRow;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use strum::{Display, EnumIter, EnumString, IntoEnumIterator};

pub const DEFAULT_REMINDER_SEND_HOUR: u8 = 9;
pub const MAX_BCC: usize = 5;

/// Closed set of customer-facing email kinds; the snake_case string is the wire and column value.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Display,
    EnumString,
    EnumIter,
    Serialize,
    Deserialize,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EmailKind {
    InvoiceReady,
    InvoicePaid,
    CreditNoteIssued,
    PaymentFailed,
    AuthenticationRequired,
    PaymentReminder,
    QuoteReady,
    /// Every party signed the quote.
    QuoteAccepted,
    WalletBalanceLow,
    AutoTopUpFailed,
    BudgetThreshold,
    BudgetExceeded,
    /// A cancellation was recorded: the customer's confirmation in text form (§ 312k BGB).
    SubscriptionCancelled,
    /// The card a subscription is charged on expires soon; the link updates it.
    PaymentMethodExpiring,
    /// Billing moved from another provider: the customer adds a payment method Meteroid charges.
    PaymentMethodRequested,
    /// A subscription renews or ends soon (R-161), sent when the tenant sets a reminder lead.
    RenewalReminder,
    /// A trial ends soon: what follows it, sent with the trial-ending notice (R-158).
    TrialEnding,
    /// A trial started with a payment method on file: what is charged once it ends.
    TrialStarted,
    /// A trial ended without converting: cancelled, switched or awaiting payment.
    TrialEnded,
}

impl EmailKind {
    /// Only reminder steps carry operator-written copy; fiscal bodies are fixed.
    pub fn editable_copy(&self) -> bool {
        matches!(self, Self::PaymentReminder)
    }

    pub fn attaches_document(&self) -> bool {
        matches!(
            self,
            Self::InvoiceReady
                | Self::InvoicePaid
                | Self::CreditNoteIssued
                | Self::PaymentFailed
                | Self::PaymentReminder
        )
    }

    /// Kinds switched off for every entity until an operator enables them; must equal the
    /// `invoicing_entity_email_settings.disabled_kinds` column default.
    pub fn default_disabled() -> BTreeSet<EmailKind> {
        EmailKind::iter().filter(|k| !k.default_enabled()).collect()
    }

    /// Reminders target audiences through their sequences instead of a "Send to" rule.
    pub fn takes_audience_rule(&self) -> bool {
        !matches!(self, Self::PaymentReminder)
    }

    /// A quote, a wallet or a subscription has no invoice: its rule may only use customer-level
    /// predicates.
    pub fn customer_level_audience(&self) -> bool {
        matches!(
            self,
            Self::QuoteReady
                | Self::QuoteAccepted
                | Self::WalletBalanceLow
                | Self::AutoTopUpFailed
                | Self::BudgetThreshold
                | Self::BudgetExceeded
                | Self::SubscriptionCancelled
                | Self::PaymentMethodExpiring
                | Self::PaymentMethodRequested
                | Self::RenewalReminder
                | Self::TrialEnding
                | Self::TrialStarted
                | Self::TrialEnded
        )
    }

    fn default_enabled(&self) -> bool {
        // Reminders need a deliberate sequence and copy, and an expiring-card notice is a new
        // email to every customer: tenants opt in. They opt out of the rest.
        !matches!(self, Self::PaymentReminder | Self::PaymentMethodExpiring)
    }
}

/// A workspace's delivery mode with the addresses `Test` mode may reach, read per send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailDelivery {
    pub mode: crate::domain::enums::EmailDeliveryMode,
    /// The organization's active members.
    pub members: Vec<String>,
    pub allowlist: Vec<String>,
}

impl EmailDelivery {
    pub fn is_off(&self) -> bool {
        self.mode == crate::domain::enums::EmailDeliveryMode::Off
    }

    /// The addresses an email to `recipients` reaches: those this mode delivers to, less the
    /// reserved test domains no transport sends to.
    pub fn reaches(&self, recipients: &[String]) -> Vec<String> {
        let sendable: Vec<String> = recipients
            .iter()
            .filter(|r| !meteroid_mailer::model::is_test_address(r))
            .cloned()
            .collect();
        self.deliverable(&sendable)
    }

    /// The addresses this mode delivers to, in order: all in `Live`, the team's in `Test`.
    pub fn deliverable(&self, recipients: &[String]) -> Vec<String> {
        use crate::domain::enums::EmailDeliveryMode;
        match self.mode {
            EmailDeliveryMode::Live => recipients.to_vec(),
            EmailDeliveryMode::Off => vec![],
            EmailDeliveryMode::Test => recipients
                .iter()
                .filter(|r| {
                    crate::domain::tenants::is_test_recipient(r, &self.members, &self.allowlist)
                })
                .cloned()
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailSettings {
    pub invoicing_entity_id: InvoicingEntityId,
    pub sender_name: Option<String>,
    pub reply_to: Option<String>,
    pub bcc: Vec<String>,
    pub disabled_kinds: BTreeSet<EmailKind>,
    /// IANA zone for the reminder send hour; `None` = UTC.
    pub timezone: Option<String>,
    pub reminder_send_hour: u8,
}

impl EmailSettings {
    pub fn defaults(invoicing_entity_id: InvoicingEntityId) -> Self {
        Self {
            invoicing_entity_id,
            sender_name: None,
            reply_to: None,
            bcc: vec![],
            disabled_kinds: EmailKind::default_disabled(),
            timezone: None,
            reminder_send_hour: DEFAULT_REMINDER_SEND_HOUR,
        }
    }

    pub fn is_enabled(&self, kind: EmailKind) -> bool {
        !self.disabled_kinds.contains(&kind)
    }

    /// The From header: the configured sender name, else the entity's legal name.
    pub fn sender(&self, entity: &InvoicingEntity, from_addr: &str) -> String {
        let name = self
            .sender_name
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&entity.legal_name);
        format!("{name} <{from_addr}>")
    }

    /// Falls back to UTC (with a warning) on an unparsable zone, so a bad setting never blocks sends.
    pub fn send_window(&self) -> SendWindow {
        let tz = match self.timezone.as_deref() {
            None => chrono_tz::UTC,
            Some(tz) => common_domain::identifiers::parse_timezone(tz).unwrap_or_else(|_| {
                tracing::warn!(
                    invoicing_entity_id = %self.invoicing_entity_id,
                    timezone = tz,
                    "invalid email settings timezone, falling back to UTC"
                );
                chrono_tz::UTC
            }),
        };
        SendWindow {
            tz,
            hour: self.reminder_send_hour,
        }
    }
}

impl From<InvoicingEntityEmailSettingsRow> for EmailSettings {
    fn from(row: InvoicingEntityEmailSettingsRow) -> Self {
        Self {
            invoicing_entity_id: row.invoicing_entity_id,
            sender_name: row.sender_name,
            reply_to: row.reply_to,
            bcc: row.bcc.into_iter().flatten().collect(),
            // Unknown values (a kind removed from the enum) are dropped rather than failing the read.
            disabled_kinds: row
                .disabled_kinds
                .into_iter()
                .flatten()
                .filter_map(|k| k.parse().ok())
                .collect(),
            timezone: row.timezone,
            reminder_send_hour: u8::try_from(row.reminder_send_hour)
                .unwrap_or(DEFAULT_REMINDER_SEND_HOUR),
        }
    }
}

#[derive(o2o, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[map_owned(EmailKindAudienceModeEnum)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EmailAudienceMode {
    /// Everyone except the audience.
    Exclude,
    /// Only the audience.
    Only,
}

impl EmailAudienceMode {
    pub fn suppresses(self, matched: bool) -> bool {
        match self {
            Self::Exclude => matched,
            Self::Only => !matched,
        }
    }
}

/// "Send to" rule of one kind for one invoicing entity; no rule = everyone.
#[derive(Debug, Clone, PartialEq)]
pub struct EmailKindAudience {
    pub invoicing_entity_id: InvoicingEntityId,
    pub kind: EmailKind,
    pub mode: EmailAudienceMode,
    pub audience: Audience,
    pub updated_at: NaiveDateTime,
}

impl TryFrom<EmailKindAudienceRow> for EmailKindAudience {
    type Error = StoreErrorReport;

    fn try_from(row: EmailKindAudienceRow) -> Result<Self, Self::Error> {
        let kind = row.kind.parse().map_err(|_| {
            StoreError::InvalidArgument(format!("unknown email kind {} in audience rule", row.kind))
        })?;
        let audience = serde_json::from_value(row.audience).map_err(|e| {
            StoreError::SerdeError("Failed to deserialize email_kind_audience".to_string(), e)
        })?;
        Ok(Self {
            invoicing_entity_id: row.invoicing_entity_id,
            kind,
            mode: row.mode.into(),
            audience,
            updated_at: row.updated_at,
        })
    }
}

/// What a "Send to" rule is evaluated against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudienceSubject {
    Invoice(InvoiceId),
    Customer(CustomerId),
}

/// Counts behind the "Send to" live preview; invoices are 0 for customer-level kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EmailKindAudiencePreview {
    pub scanned_invoices: usize,
    pub scanned_customers: usize,
    pub matched_invoices: usize,
    pub matched_customers: usize,
    pub receiving_invoices: usize,
    pub receiving_customers: usize,
}

/// `None` = leave untouched; `Some(None)` = clear.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmailSettingsPatch {
    pub sender_name: Option<Option<String>>,
    pub reply_to: Option<Option<String>>,
    pub bcc: Option<Vec<String>>,
    pub timezone: Option<Option<String>>,
    pub reminder_send_hour: Option<u8>,
}

/// What a `sent_email` row needs to re-render itself; also the payload of the send job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EmailContext {
    Invoice {
        invoice_id: InvoiceId,
    },
    CreditNote {
        credit_note_id: CreditNoteId,
    },
    PaymentFailed {
        invoice_id: InvoiceId,
        payment_transaction_id: PaymentTransactionId,
    },
    AuthenticationRequired {
        invoice_id: InvoiceId,
        payment_transaction_id: PaymentTransactionId,
    },
    PaymentReminder {
        invoice_id: InvoiceId,
        sequence_id: PaymentReminderSequenceId,
        offset_days: i32,
        generation: i32,
        #[serde(default)]
        manual: bool,
    },
    /// The balance at the crossing, as the `wallet.balance_low` event reported it.
    WalletBalanceLow {
        customer_id: CustomerId,
        wallet_id: WalletId,
        currency: String,
        available_minor: i64,
        threshold_minor: i64,
    },
    /// `suspended`: the failures (or a reversed payment) paused the top-up. `payment_reversed`:
    /// a top-up payment of `amount_minor` (`operation_id`) was refunded or disputed.
    AutoTopUpFailed {
        customer_id: CustomerId,
        wallet_id: WalletId,
        operation_id: CreditOperationId,
        currency: String,
        amount_minor: i64,
        suspended: bool,
        #[serde(default)]
        payment_reversed: bool,
    },
    /// The spend at the crossing, as the `budget.*` event reported it.
    BudgetAlert {
        customer_id: CustomerId,
        budget_id: BudgetId,
        currency: String,
        level_pct: u32,
        used_minor: i64,
        amount_minor: i64,
        window_start: NaiveDateTime,
        window_end: Option<NaiveDateTime>,
    },
    /// The cancellation as recorded: requested at `received_at`, ending the subscription at
    /// `ends_at` (an instant, or the midnight starting its end date).
    SubscriptionCancelled {
        customer_id: CustomerId,
        subscription_id: SubscriptionId,
        plan_name: String,
        received_at: NaiveDateTime,
        ends_at: NaiveDateTime,
    },
    /// The card on file, which an automatically charged subscription uses, expires at the end of
    /// `exp_month` of `exp_year`.
    PaymentMethodExpiring {
        customer_id: CustomerId,
        payment_method_id: CustomerPaymentMethodId,
        method: String,
        exp_month: u32,
        exp_year: i32,
    },
    /// `update_by` is the day the payment method is needed; the link is minted when it is sent.
    PaymentMethodRequested {
        customer_id: CustomerId,
        update_by: Option<chrono::NaiveDate>,
        previous_provider: String,
        /// The day billing moves, while it hasn't: the email announces the move.
        #[serde(default)]
        moves_on: Option<chrono::NaiveDate>,
        /// The previous provider keeps its customers' cards (merchant of record).
        #[serde(default)]
        merchant_of_record: bool,
        #[serde(default)]
        previous_method: PreviousPaymentMethod,
        /// One-click opt-out of these reminders.
        #[serde(default)]
        stop_url: Option<String>,
    },
    /// The renewal as noticed: renewing (else ending) on `date`, its opt-out taken until the
    /// deadline when the term has one.
    RenewalReminder {
        customer_id: CustomerId,
        subscription_id: SubscriptionId,
        plan_name: String,
        date: chrono::NaiveDate,
        renews: bool,
        opt_out_deadline: Option<chrono::NaiveDate>,
        renewal_months: Option<u32>,
        uplift_percent: Option<rust_decimal::Decimal>,
    },
    /// The trial ending on `ends_on` (exclusive: billing starts on it). What follows it, and the
    /// first charge, are read when the email is built.
    TrialEnding {
        customer_id: CustomerId,
        subscription_id: SubscriptionId,
        ends_on: chrono::NaiveDate,
    },
    /// A trial from `starts_on` to `ends_on` (exclusive); the payment method and the amount
    /// after it are read when the email is built.
    TrialStarted {
        customer_id: CustomerId,
        subscription_id: SubscriptionId,
        starts_on: chrono::NaiveDate,
        ends_on: chrono::NaiveDate,
    },
    TrialEnded {
        customer_id: CustomerId,
        subscription_id: SubscriptionId,
        ended_on: chrono::NaiveDate,
        outcome: TrialEndedOutcome,
    },
}

/// How a trial that did not convert ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialEndedOutcome {
    Cancelled,
    /// Moved to another plan; the subscription's plan is the one it moved to.
    Switched,
    /// Its opening invoice waits for payment.
    AwaitingPayment,
}

impl From<TrialEndedOutcome> for meteroid_mailer::render::TrialEndedOutcome {
    fn from(outcome: TrialEndedOutcome) -> Self {
        match outcome {
            TrialEndedOutcome::Cancelled => Self::Cancelled,
            TrialEndedOutcome::Switched => Self::Switched,
            TrialEndedOutcome::AwaitingPayment => Self::AwaitingPayment,
        }
    }
}

/// How the customer paid the previous provider: says why a new payment method is needed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviousPaymentMethod {
    #[default]
    Card,
    PayPal,
    Invoice,
}

impl From<PreviousPaymentMethod> for meteroid_mailer::render::PreviousMethod {
    fn from(method: PreviousPaymentMethod) -> Self {
        match method {
            PreviousPaymentMethod::Card => Self::Card,
            PreviousPaymentMethod::PayPal => Self::PayPal,
            PreviousPaymentMethod::Invoice => Self::Invoice,
        }
    }
}

impl EmailContext {
    pub fn budget_alert(event: &crate::domain::usage_alert_events::BudgetAlertEvent) -> Self {
        Self::BudgetAlert {
            customer_id: event.customer_id,
            budget_id: event.budget_id,
            currency: event.currency.clone(),
            level_pct: event.threshold_pct,
            used_minor: event.used_minor,
            amount_minor: event.amount_minor,
            window_start: event.window_start,
            window_end: event.window_end,
        }
    }

    pub fn invoice_id(&self) -> Option<InvoiceId> {
        match self {
            Self::Invoice { invoice_id }
            | Self::PaymentFailed { invoice_id, .. }
            | Self::AuthenticationRequired { invoice_id, .. }
            | Self::PaymentReminder { invoice_id, .. } => Some(*invoice_id),
            Self::CreditNote { .. }
            | Self::WalletBalanceLow { .. }
            | Self::AutoTopUpFailed { .. }
            | Self::BudgetAlert { .. }
            | Self::SubscriptionCancelled { .. }
            | Self::PaymentMethodExpiring { .. }
            | Self::PaymentMethodRequested { .. }
            | Self::RenewalReminder { .. }
            | Self::TrialEnding { .. }
            | Self::TrialStarted { .. }
            | Self::TrialEnded { .. } => None,
        }
    }

    /// The customer of a wallet, budget or subscription notice, which has no document behind it.
    pub fn notice_customer_id(&self) -> Option<CustomerId> {
        match self {
            Self::WalletBalanceLow { customer_id, .. }
            | Self::AutoTopUpFailed { customer_id, .. }
            | Self::BudgetAlert { customer_id, .. }
            | Self::SubscriptionCancelled { customer_id, .. }
            | Self::PaymentMethodExpiring { customer_id, .. }
            | Self::PaymentMethodRequested { customer_id, .. }
            | Self::RenewalReminder { customer_id, .. }
            | Self::TrialEnding { customer_id, .. }
            | Self::TrialStarted { customer_id, .. }
            | Self::TrialEnded { customer_id, .. } => Some(*customer_id),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_auto_top_up_notice_queued_before_payment_reversed_parses_as_not_reversed() {
        let json = serde_json::json!({
            "type": "auto_top_up_failed",
            "customer_id": CustomerId::new().as_base62(),
            "wallet_id": WalletId::new().as_base62(),
            "operation_id": CreditOperationId::new().as_base62(),
            "currency": "EUR",
            "amount_minor": 2_500,
            "suspended": true
        });
        let context: EmailContext = serde_json::from_value(json).expect("parses");
        assert!(matches!(
            context,
            EmailContext::AutoTopUpFailed {
                suspended: true,
                payment_reversed: false,
                ..
            }
        ));
    }
    use common_domain::ids::BaseId;

    /// The `disabled_kinds` column default, as migration
    /// `2026-11-27-018493_payment_method_expiry_notice` sets it.
    const COLUMN_DEFAULT: &[&str] = &["payment_reminder", "payment_method_expiring"];

    #[test]
    fn default_disabled_matches_the_column_default() {
        let expected: BTreeSet<EmailKind> = COLUMN_DEFAULT
            .iter()
            .map(|k| k.parse().expect("column default kind is a known EmailKind"))
            .collect();
        assert_eq!(EmailKind::default_disabled(), expected);
        assert_eq!(
            EmailSettings::defaults(InvoicingEntityId::new()).disabled_kinds,
            expected
        );
    }

    #[test]
    fn kind_strings_are_snake_case_on_both_serde_and_strum() {
        for kind in EmailKind::iter() {
            let json = serde_json::to_value(kind).unwrap();
            assert_eq!(json, serde_json::Value::String(kind.to_string()));
            assert_eq!(kind.to_string().parse::<EmailKind>().unwrap(), kind);
        }
    }

    #[test]
    fn audience_modes_suppress_opposite_sides_and_reminders_take_no_rule() {
        assert!(EmailAudienceMode::Exclude.suppresses(true));
        assert!(!EmailAudienceMode::Exclude.suppresses(false));
        assert!(EmailAudienceMode::Only.suppresses(false));
        assert!(!EmailAudienceMode::Only.suppresses(true));
        let without_rule: Vec<EmailKind> = EmailKind::iter()
            .filter(|k| !k.takes_audience_rule())
            .collect();
        assert_eq!(without_rule, vec![EmailKind::PaymentReminder]);
    }

    #[test]
    fn send_window_falls_back_to_utc_on_bad_zone() {
        let mut s = EmailSettings::defaults(InvoicingEntityId::new());
        s.timezone = Some("Not/AZone".to_string());
        assert_eq!(s.send_window().tz, chrono_tz::UTC);
        s.timezone = Some("Europe/Paris".to_string());
        assert_eq!(s.send_window().tz, chrono_tz::Europe::Paris);
        assert_eq!(s.send_window().hour, DEFAULT_REMINDER_SEND_HOUR);
    }

    #[test]
    fn email_context_is_internally_tagged_snake_case() {
        let ctx = EmailContext::PaymentReminder {
            invoice_id: InvoiceId::new(),
            sequence_id: PaymentReminderSequenceId::new(),
            offset_days: 7,
            generation: 2,
            manual: false,
        };
        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["type"], "payment_reminder");
        let back: EmailContext = serde_json::from_value(json).unwrap();
        assert_eq!(back, ctx);
    }
}
