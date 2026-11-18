//! Invoice-time customer credit: one wallet per customer and denomination, the grants it holds
//! (each with what it pays for) and its statement. Amounts are minor units of the wallet currency.
use crate::StoreResult;
use crate::errors::StoreError;
use chrono::{DateTime, Utc};
use common_domain::ids::{
    BaseId, CreditGrantId, CreditNoteId, CustomerId, CustomerPaymentMethodId, InvoiceId,
    LedgerEntryId, ProductId, SubscriptionId, TenantId, WalletId,
};
use error_stack::Report;
use uuid::Uuid;

/// What a grant pays: usage credit only usage charges, invoice credit any charge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreditApplication {
    Invoice,
    Usage,
}

impl CreditApplication {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Invoice => "INVOICE",
            Self::Usage => "USAGE",
        }
    }

    pub fn from_db_str(value: &str) -> StoreResult<Self> {
        match value {
            "INVOICE" => Ok(Self::Invoice),
            "USAGE" => Ok(Self::Usage),
            other => Err(Report::new(StoreError::InvalidArgument(format!(
                "unknown credit application {other}"
            )))),
        }
    }
}

/// Paid credit was bought; promotional credit was given. Promotional credit is spent first among
/// grants of equal priority and expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GrantClassification {
    Paid,
    Promotional,
}

impl GrantClassification {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Paid => "PAID",
            Self::Promotional => "PROMOTIONAL",
        }
    }

    pub fn from_db_str(value: &str) -> StoreResult<Self> {
        match value {
            "PAID" => Ok(Self::Paid),
            "PROMOTIONAL" => Ok(Self::Promotional),
            other => Err(Report::new(StoreError::InvalidArgument(format!(
                "unknown credit classification {other}"
            )))),
        }
    }
}

/// What a grant may pay: the charges of one subscription or of any (`subscription_id` None), usage
/// only or any charge, and the products it is restricted to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GrantScope {
    pub subscription_id: Option<SubscriptionId>,
    pub application: CreditApplication,
    /// Sorted; empty pays for every product.
    pub product_ids: Vec<ProductId>,
}

impl GrantScope {
    /// Narrower than customer-wide: one subscription, usage only, or some products.
    pub fn is_scoped(&self) -> bool {
        self.subscription_id.is_some()
            || matches!(self.application, CreditApplication::Usage)
            || !self.product_ids.is_empty()
    }

    /// Pays any charge of any of the customer's subscriptions.
    pub fn customer_wide() -> Self {
        Self {
            subscription_id: None,
            application: CreditApplication::Invoice,
            product_ids: Vec::new(),
        }
    }

    /// Pays only the usage of `subscription_id`.
    pub fn subscription_usage(subscription_id: SubscriptionId) -> Self {
        Self {
            subscription_id: Some(subscription_id),
            application: CreditApplication::Usage,
            product_ids: Vec::new(),
        }
    }

    /// `product_ids` sorted and deduplicated, as stored.
    pub fn with_products(mut self, mut product_ids: Vec<ProductId>) -> Self {
        product_ids.sort_by_key(|id| id.as_uuid());
        product_ids.dedup();
        self.product_ids = product_ids;
        self
    }
}

/// Grant listing filter by the subscription a grant belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantSubscriptionFilter {
    /// The grants that belong to it.
    Of(SubscriptionId),
    /// The grants that belong to it, and customer-wide ones.
    UsableBy(SubscriptionId),
}

/// Why a grant ended before its expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GrantEndReason {
    SubscriptionEnded,
    PlanChange,
}

impl GrantEndReason {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::SubscriptionEnded => diesel_models::query::customer_credit::SUBSCRIPTION_ENDED,
            Self::PlanChange => diesel_models::query::customer_credit::PLAN_CHANGE_ENDED,
        }
    }

    pub fn from_db_str(value: &str) -> StoreResult<Self> {
        match value {
            diesel_models::query::customer_credit::SUBSCRIPTION_ENDED => {
                Ok(Self::SubscriptionEnded)
            }
            diesel_models::query::customer_credit::PLAN_CHANGE_ENDED => Ok(Self::PlanChange),
            other => Err(Report::new(StoreError::InvalidArgument(format!(
                "unknown credit grant end reason {other}"
            )))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreditGrantState {
    /// Takes effect later.
    Pending,
    Active,
    Depleted,
    /// Lapsed: expired, or ended with its subscription.
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditGrant {
    pub id: CreditGrantId,
    pub wallet_id: WalletId,
    pub amount_minor: i64,
    /// Spendable (or, while pending, future) value; zero once lapsed.
    pub remaining_minor: i64,
    /// Value that lapsed unspent, once the grant lapsed.
    pub expired_minor: Option<i64>,
    pub classification: GrantClassification,
    pub priority: i32,
    pub source: CreditGrantSource,
    pub source_id: Uuid,
    pub invoice_id: Option<InvoiceId>,
    pub credit_note_id: Option<CreditNoteId>,
    pub effective_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub state: CreditGrantState,
    pub reason: String,
    pub reference: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    /// None for credit granted before costs were recorded.
    pub cost: Option<CreditCost>,
    pub scope: GrantScope,
    /// When the subscription it belongs to ended, if it did.
    pub ends_at: Option<DateTime<Utc>>,
    pub end_reason: Option<GrantEndReason>,
    /// What is left at expiry rolls over into a new grant, up to its cap.
    pub rollover: Option<GrantRollover>,
}

/// A grant with the currency of the wallet that holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerCreditGrant {
    pub grant: CreditGrant,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditWallet {
    pub id: WalletId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub currency: String,
    /// Spendable now.
    pub available_minor: i64,
    /// Granted but not yet effective.
    pub pending_minor: i64,
    /// Ascending, distinct, at most 5. Each fires `wallet.balance_low` once per crossing.
    pub low_balance_thresholds_minor: Vec<i64>,
    /// Overrides, for this customer, what the plans do once the credit runs out.
    pub usage_at_zero: crate::domain::credit_exhausted::UsageAtZero,
    /// What the plan of each active subscription charging usage in `currency` does then.
    pub plan_rules: Vec<crate::domain::credit_exhausted::WalletPlanRule>,
    pub grants: Vec<CreditGrant>,
    pub created_at: DateTime<Utc>,
    /// The credit unit `currency` names, when the wallet holds credits rather than money.
    pub credit_unit: Option<crate::domain::credit_units::CreditUnit>,
    /// A credit unit wallet: the price this customer agreed per credit, which purchases and
    /// automatic top-ups in its currency use instead of the list price.
    pub unit_price: Option<CustomerUnitPrice>,
}

impl CreditWallet {
    /// Usage stops as soon as the credit is used up, for at least one subscription.
    pub fn pauses_at_zero(&self) -> bool {
        crate::domain::credit_exhausted::pauses_at_zero(
            self.usage_at_zero,
            &self.plan_rules,
            &self.currency,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerUnitPrice {
    /// Major units of `currency` per credit.
    pub unit_price: rust_decimal::Decimal,
    pub currency: String,
}

/// What a customer can spend in one currency once the usage not yet invoiced is taken out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveCreditBalance {
    pub currency: String,
    pub projected_minor: i64,
    pub estimated_unbilled_minor: i64,
    pub as_of: DateTime<Utc>,
    /// False when no projection exists yet and `projected_minor` is the ledger's balance.
    pub projected: bool,
}

/// How a wallet stands once the usage not yet invoiced is paid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletStatus {
    Healthy,
    /// Below a low-balance threshold, or at or below the automatic top-up threshold.
    Low,
    Depleted,
    /// Exhausted in a currency where a wallet refuses usage once empty.
    UsageBlocked,
}

impl WalletStatus {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Healthy => "HEALTHY",
            Self::Low => "LOW",
            Self::Depleted => "DEPLETED",
            Self::UsageBlocked => "USAGE_BLOCKED",
        }
    }

    pub fn from_db_str(value: &str) -> StoreResult<Self> {
        match value {
            "HEALTHY" => Ok(Self::Healthy),
            "LOW" => Ok(Self::Low),
            "DEPLETED" => Ok(Self::Depleted),
            "USAGE_BLOCKED" => Ok(Self::UsageBlocked),
            other => Err(Report::new(StoreError::InvalidArgument(format!(
                "unknown wallet status {other}"
            )))),
        }
    }

    /// `balance_minor` is the wallet's own, `currency_balance_minor` that of all the customer's
    /// wallets in its currency. Mirrors the SQL of the tenant-wide wallet roster.
    pub fn classify(
        balance_minor: i64,
        low_balance_thresholds_minor: &[i64],
        auto_top_up_threshold_minor: Option<i64>,
        currency_balance_minor: i64,
        currency_blocks: bool,
    ) -> Self {
        if currency_blocks && currency_balance_minor <= 0 {
            Self::UsageBlocked
        } else if balance_minor <= 0 {
            Self::Depleted
        } else if low_balance_thresholds_minor
            .iter()
            .any(|threshold| balance_minor < *threshold)
            || auto_top_up_threshold_minor.is_some_and(|threshold| balance_minor <= threshold)
        {
            Self::Low
        } else {
            Self::Healthy
        }
    }
}

/// One of the customer's wallets as one subscription can spend it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionWalletBalance {
    pub wallet_id: WalletId,
    pub denomination: String,
    /// The credit the subscription can spend once every subscription's unbilled usage is paid,
    /// less its own usage no credit pays. Credit restricted to products it is not charged for is
    /// left out.
    pub balance_minor: i64,
    /// Of the credit counted in `balance_minor`, the part that belongs to no subscription: every
    /// other subscription of the customer counts it too.
    pub shared_minor: i64,
    /// What its usage asks of credit not drawn yet, tax included: usage no credit pays included,
    /// what a draft invoice already drew left out (as the wallet's own unbilled usage).
    pub unbilled_minor: i64,
    /// Of `unbilled_minor`, what no credit it can use pays.
    pub uncovered_minor: i64,
    /// `balance_minor` is at or below zero, or no credit it can use is left for some of its usage.
    pub exhausted: bool,
    /// The rule in effect for its usage (the wallet's override, else its plan's) pauses it as soon
    /// as no credit it can use is left.
    pub pauses_at_zero: bool,
    /// The access gate refuses some of its usage (of the product asked): no credit it can use is
    /// left for it and the rule in effect pauses it, at zero or past its cap.
    pub refused: bool,
    pub as_of: DateTime<Utc>,
    /// False before the live estimate is computed: the credit as the ledger holds it.
    pub projected: bool,
}

/// One wallet's share of the live balance of its currency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletLiveBalance {
    /// What is left once the usage not yet invoiced is paid.
    pub live_balance_minor: i64,
    /// The usage not yet invoiced this wallet is expected to pay.
    pub pending_usage_minor: i64,
    /// Usage not yet invoiced that no credit in the currency covers.
    pub uncovered_usage_minor: i64,
    pub as_of: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletHealth {
    pub wallet_id: WalletId,
    /// Absent until every wallet of the currency has a live estimate.
    pub live: Option<WalletLiveBalance>,
    pub status: WalletStatus,
    /// A ledger change landed after `live.as_of`: the live numbers are still catching up with it.
    pub live_stale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerEntryKind {
    /// A grant was funded.
    Fund,
    /// Credit applied to an invoice.
    Draw,
    /// Credit an invoice gave back: voided, or returned by a credit note.
    DrawReversal,
    /// Unspent credit taken back by a credit note, an operator or a reversed payment.
    Reversal,
    /// Credit a reversed payment took back, restored once its money is recovered.
    Reinstatement,
    /// Credit that lapsed unspent.
    Expiry,
    /// Unspent credit carried out of an expiring grant into the grant it rolled over to.
    Rollover,
}

impl LedgerEntryKind {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Fund => "FUND",
            Self::Draw => "DRAW",
            Self::DrawReversal => "DRAW_REVERSAL",
            Self::Reversal => "REVERSAL",
            Self::Reinstatement => "REINSTATEMENT",
            Self::Expiry => "EXPIRY",
            Self::Rollover => "ROLLOVER",
        }
    }

    pub fn from_db_str(value: &str) -> StoreResult<Self> {
        match value {
            "FUND" => Ok(Self::Fund),
            "DRAW" => Ok(Self::Draw),
            "DRAW_REVERSAL" => Ok(Self::DrawReversal),
            "REVERSAL" => Ok(Self::Reversal),
            "REINSTATEMENT" => Ok(Self::Reinstatement),
            "EXPIRY" => Ok(Self::Expiry),
            "ROLLOVER" => Ok(Self::Rollover),
            other => Err(Report::new(StoreError::InvalidArgument(format!(
                "unknown customer credit movement {other}"
            )))),
        }
    }
}

/// The ledger reason of credit that lapsed because the subscription it belongs to ended.
pub const ENDED_WITH_SUBSCRIPTION: &str = "Ended with subscription";

/// The reason on every note a plan change issues: it marks the lines the change settled.
pub const PLAN_CHANGE_NOTE_REASON: &str = "Plan change";
/// The reason on the notes an immediate cancellation issues.
pub const CANCELLATION_NOTE_REASON: &str = "Subscription cancellation";
/// The reason on the note crediting a period pack paid after its period was invoiced.
pub const LATE_PAYMENT_NOTE_REASON: &str = "Credit pack paid after its period was invoiced";

/// The ledger reason of credit an immediate plan change removes, shown as is in both ledgers.
pub const PLAN_CHANGE_REMOVAL: &str = "Removed: plan change";

/// The ledger reason of paid pack credit an immediate cancellation credits back with its fee.
pub const CANCELLATION_REMOVAL: &str = "Removed: subscription cancelled";

/// The ledger reason of a period pack's credit taken back with its fee because it was paid after
/// its period was invoiced.
pub const LATE_PAYMENT_REMOVAL: &str = "Removed: paid after its period was invoiced";

/// Whether a ledger reason marks credit removed with the period it was bought for.
pub fn is_removal_reason(reason: &str) -> bool {
    reason == PLAN_CHANGE_REMOVAL
        || reason == CANCELLATION_REMOVAL
        || reason == LATE_PAYMENT_REMOVAL
}

/// One statement line with the wallet balance after it.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditLedgerEntry {
    pub id: LedgerEntryId,
    pub wallet_id: WalletId,
    pub grant_id: CreditGrantId,
    /// Chronological position within the wallet, starting at 1.
    pub sequence: i64,
    pub kind: LedgerEntryKind,
    /// Signed: positive adds credit, negative takes it.
    pub amount_minor: i64,
    pub balance_after_minor: i64,
    pub invoice_id: Option<InvoiceId>,
    pub credit_note_id: Option<CreditNoteId>,
    pub reason: String,
    pub reference: Option<String>,
    pub created_by: Option<Uuid>,
    pub effective_at: DateTime<Utc>,
    pub recorded_at: DateTime<Utc>,
    /// Signed like `amount_minor`: a draw's is recognized revenue, an expiry's breakage, a
    /// reversal's a refund. None when its grant has no cost.
    pub cost: Option<CreditCost>,
    /// What the grant it moved pays for.
    pub application: CreditApplication,
    pub subscription_id: Option<SubscriptionId>,
}

/// An operator's reference for a credit adjustment, such as the invoice or ticket it compensates.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FundingReference(String);

impl FundingReference {
    pub const MAX_BYTES: usize = 255;

    pub fn new(value: impl Into<String>) -> StoreResult<Self> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.len() > Self::MAX_BYTES {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "a credit reference is 1 to {} bytes",
                Self::MAX_BYTES
            ))));
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

/// What a grant's whole amount was paid, net of tax, in an ISO currency. Zero for given credit;
/// the per-unit cost basis is derived (`amount_minor / grant amount`) so no rounding is stored.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CreditCost {
    pub amount_minor: i64,
    pub currency: String,
}

impl CreditCost {
    pub fn given(currency: &str) -> Self {
        Self {
            amount_minor: 0,
            currency: currency.trim().to_uppercase(),
        }
    }

    /// Credit that is the customer's own money: worth its face in that currency.
    pub fn at_face(amount_minor: i64, currency: &str) -> Self {
        Self {
            amount_minor,
            currency: currency.trim().to_uppercase(),
        }
    }

    /// Major units of `currency` paid per major unit of a grant of `granted_minor`, rounded to 12
    /// places. Display only: ledger costs are always split from the exact total.
    pub fn per_unit(
        &self,
        granted_minor: i64,
        denomination_precision: u8,
    ) -> Option<rust_decimal::Decimal> {
        use rust_decimal::Decimal;
        let cost_precision =
            crate::constants::Currencies::resolve_currency_precision(&self.currency)?;
        if granted_minor <= 0 {
            return None;
        }
        let paid = Decimal::new(self.amount_minor, u32::from(cost_precision));
        let granted = Decimal::new(granted_minor, u32::from(denomination_precision));
        paid.checked_div(granted)
            .map(|basis| basis.round_dp(12).normalize())
    }
}

/// A grant's capped rollover, frozen in the grant's minor units on the included-credit line and
/// stored in `customer_credit_grant.rollover`. A rolled grant carries none (single hop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GrantRollover {
    /// None rolls over everything left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_minor: Option<i64>,
    /// Term of the rolled credit from the original expiry; `Day` or `Month` only.
    pub expiry: super::price_components::CreditExpiry,
}

impl GrantRollover {
    /// What rolls over of `remaining`: all of it, or up to the cap.
    pub fn carried(&self, remaining_minor: i64) -> i64 {
        let remaining_minor = remaining_minor.max(0);
        self.max_minor
            .map_or(remaining_minor, |max| remaining_minor.min(max.max(0)))
    }

    /// When the rolled credit expires, for a grant that expired at `expired_at`.
    pub fn rolled_expiry(&self, expired_at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self.expiry {
            super::price_components::CreditExpiry::BillingPeriod => None,
            term => term.resolve(expired_at),
        }
    }

    pub fn to_json(&self) -> StoreResult<serde_json::Value> {
        serde_json::to_value(self).map_err(|e| {
            Report::new(StoreError::SerdeError(
                "Failed to serialize GrantRollover".to_owned(),
                e,
            ))
        })
    }

    pub fn from_json(value: &serde_json::Value) -> StoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|e| {
            Report::new(StoreError::SerdeError(
                "Failed to deserialize GrantRollover".to_owned(),
                e,
            ))
        })
    }
}

/// How an automatic top-up sizes its charge: a fixed amount, or whatever brings the balance back
/// to a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AutomaticTopUpMethod {
    Fixed,
    ToTarget,
}

impl AutomaticTopUpMethod {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Fixed => "FIXED",
            Self::ToTarget => "TO_TARGET",
        }
    }

    pub fn from_db_str(value: &str) -> StoreResult<Self> {
        match value {
            "FIXED" => Ok(Self::Fixed),
            "TO_TARGET" => Ok(Self::ToTarget),
            other => Err(Report::new(StoreError::InvalidArgument(format!(
                "unknown automatic top-up method {other}"
            )))),
        }
    }
}

/// Minimum spacing between two successful automatic top-ups of one wallet. It also covers a
/// low-balance signal racing the funding of the previous top-up.
pub const AUTOMATIC_TOP_UP_COOLDOWN_SECONDS: i64 = 300;
/// Failed charges in a row after which automatic top-up is suspended until the next edit.
pub const AUTOMATIC_TOP_UP_MAX_CONSECUTIVE_FAILURES: i32 = 3;
/// A fixed refill must add at least this fraction (1/n) of the balance that triggers it.
const MIN_FIXED_AMOUNT_THRESHOLD_RATIO: i64 = 10;
/// Ceiling on one self-service charge, in minor units: a manual top-up, or an automatic top-up
/// whose terms the customer authored and whose maximum the operator left unset.
pub const MAX_SELF_SERVICE_CHARGE_MINOR: i64 = 1_000_000;

/// The maximum per charge of terms a customer wrote when the operator set none: the ceiling,
/// never below the operator's minimum.
pub fn self_service_maximum(minimum_minor: Option<i64>) -> i64 {
    MAX_SELF_SERVICE_CHARGE_MINOR.max(minimum_minor.unwrap_or(0))
}

/// Why a suspended automatic top-up stopped, which decides what resumes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoTopUpStopReason {
    /// The price per credit rose above the one the terms were authorized at, or the customer's
    /// currency changed.
    PriceChanged,
    /// The credit unit cannot be sold any more (no price in the currency, no product).
    Unsellable,
    PaymentReversed,
    /// A hard decline, or repeated failed charges.
    PaymentFailed,
    /// Terms the customer authorized were changed by an operator, or one top-up would charge
    /// more than the self-service ceiling: only the customer confirming terms resumes it.
    AwaitingCustomer,
}

/// An invoice-credit wallet's automatic top-up, in minor units of the wallet currency, with the
/// runtime state the executor keeps beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomaticTopUpConfig {
    pub method: AutomaticTopUpMethod,
    /// Tops up when the available balance is at or below this.
    pub threshold_minor: i64,
    pub amount_minor: Option<i64>,
    pub target_minor: Option<i64>,
    /// Bounds on one top-up.
    pub minimum_minor: Option<i64>,
    pub maximum_minor: Option<i64>,
    /// Bound on what settled top-ups added in the last 24 hours. Caps are money: a credit unit
    /// wallet's top-ups count their price before tax, in the customer's currency.
    pub daily_cap_minor: Option<i64>,
    /// Bound on what top-ups add in a calendar month (UTC).
    pub monthly_cap_minor: Option<i64>,
    /// None charges the customer's default payment method.
    pub payment_method_id: Option<CustomerPaymentMethodId>,
    pub consecutive_failures: i32,
    /// When the last failed charge was attempted; paces the retry backoff.
    pub last_attempted_at: Option<DateTime<Utc>>,
    pub last_succeeded_at: Option<DateTime<Utc>>,
    pub suspended_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    /// A credit unit wallet: the price per credit the rule was authorized at, kept when the price
    /// falls. A price above it needs new consent, so it is part of the authorization revision.
    pub agreed_price: Option<AgreedCreditPrice>,
    /// The daily cap is the default a customer's own terms set, not one the operator chose: the
    /// customer's next edit recomputes it.
    pub daily_cap_is_default: bool,
    /// The maximum per top-up is the self-service ceiling a customer's own terms got, not one the
    /// operator chose.
    pub maximum_is_default: bool,
}

/// The selling price, and its currency, a credit unit wallet's automatic top-up was authorized
/// at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgreedCreditPrice {
    pub price_id: common_domain::ids::CreditUnitPriceId,
    /// Major units of `currency` per credit.
    pub unit_price: rust_decimal::Decimal,
    pub currency: String,
}

impl AgreedCreditPrice {
    /// Whether these terms still cover charging `unit_price` in `currency`: the authorized price
    /// or less (a price decrease never charges more than the customer agreed), in its currency.
    pub fn covers(&self, unit_price: rust_decimal::Decimal, currency: &str) -> bool {
        unit_price <= self.unit_price && self.currency == currency
    }
}

/// How a credit unit wallet's top-up is priced: its per-charge bounds are money before tax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopUpPricing {
    pub unit_precision: u8,
    pub unit_price: rust_decimal::Decimal,
    pub currency_precision: u8,
}

impl TopUpPricing {
    pub fn net_minor(&self, credits_minor: i64) -> StoreResult<i64> {
        super::credit_units::credits_to_money_minor(
            credits_minor,
            self.unit_precision,
            self.unit_price,
            self.currency_precision,
        )
    }

    /// Credits per minor unit of money, unrounded: the starting point of the exact searches.
    fn credits_per_money_minor(&self) -> Option<rust_decimal::Decimal> {
        let exponent = i32::from(self.unit_precision) - i32::from(self.currency_precision);
        let scale = rust_decimal::Decimal::from(10_i64.checked_pow(exponent.unsigned_abs())?);
        let per_major = if exponent >= 0 {
            scale.checked_div(self.unit_price)?
        } else {
            rust_decimal::Decimal::ONE
                .checked_div(self.unit_price)?
                .checked_div(scale)?
        };
        Some(per_major)
    }

    /// The most credits whose price, rounded as it is charged, stays within `money_minor`.
    pub fn most_credits_within(&self, money_minor: i64) -> StoreResult<i64> {
        let estimate = self.estimate(money_minor)?.floor();
        let mut credits = rust_decimal::prelude::ToPrimitive::to_i64(&estimate)
            .ok_or_else(|| invalid_top_up("the top-up bound exceeds the supported range"))?
            .max(0);
        while self.net_minor(credits + 1)? <= money_minor {
            credits += 1;
        }
        while credits > 0 && self.net_minor(credits)? > money_minor {
            credits -= 1;
        }
        Ok(credits)
    }

    /// The fewest credits whose price, rounded as it is charged, reaches `money_minor`.
    pub fn fewest_credits_reaching(&self, money_minor: i64) -> StoreResult<i64> {
        let estimate = self.estimate(money_minor)?.ceil();
        let mut credits = rust_decimal::prelude::ToPrimitive::to_i64(&estimate)
            .ok_or_else(|| invalid_top_up("the top-up bound exceeds the supported range"))?
            .max(0);
        while credits > 0 && self.net_minor(credits - 1)? >= money_minor {
            credits -= 1;
        }
        while self.net_minor(credits)? < money_minor {
            credits += 1;
        }
        Ok(credits)
    }

    fn estimate(&self, money_minor: i64) -> StoreResult<rust_decimal::Decimal> {
        self.credits_per_money_minor()
            .and_then(|rate| rate.checked_mul(rust_decimal::Decimal::from(money_minor)))
            .ok_or_else(|| invalid_top_up("the top-up bound exceeds the supported range"))
    }
}

impl AutomaticTopUpConfig {
    /// A rule as authored: no runtime state yet.
    pub fn new(
        method: AutomaticTopUpMethod,
        threshold_minor: i64,
        amount_minor: Option<i64>,
        target_minor: Option<i64>,
        payment_method_id: Option<CustomerPaymentMethodId>,
    ) -> Self {
        Self {
            method,
            threshold_minor,
            amount_minor,
            target_minor,
            minimum_minor: None,
            maximum_minor: None,
            daily_cap_minor: None,
            monthly_cap_minor: None,
            payment_method_id,
            consecutive_failures: 0,
            last_attempted_at: None,
            last_succeeded_at: None,
            suspended_at: None,
            last_error: None,
            agreed_price: None,
            daily_cap_is_default: false,
            maximum_is_default: false,
        }
    }

    /// Identifies the consent terms only: runtime counters never invalidate an authorization.
    pub fn authorization_revision(&self) -> String {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(self.revision_input().as_bytes()))
    }

    /// `authorization_revision` of a credit unit wallet's rule also covers the agreed price; a
    /// money wallet's revision is unchanged by it.
    fn revision_input(&self) -> String {
        let terms = format!(
            "{:?}",
            (
                self.method,
                self.threshold_minor,
                self.amount_minor,
                self.target_minor,
                self.minimum_minor,
                self.maximum_minor,
                self.daily_cap_minor,
                self.monthly_cap_minor,
                self.payment_method_id,
            )
        );
        match &self.agreed_price {
            Some(price) => format!(
                "{terms}:price:{}:{}:{}",
                price.price_id,
                price.unit_price.normalize(),
                price.currency
            ),
            None => terms,
        }
    }

    /// Credits a credit unit wallet's top-up adds when its balance is `available_minor`: the
    /// per-charge bounds are money before tax, so they become the credits that price fits.
    pub fn top_up_credits(
        &self,
        available_minor: i64,
        pricing: &TopUpPricing,
    ) -> StoreResult<Option<i64>> {
        let raw = match self.method {
            AutomaticTopUpMethod::Fixed => self.amount_minor,
            AutomaticTopUpMethod::ToTarget => self
                .target_minor
                .and_then(|target| target.checked_sub(available_minor)),
        };
        let Some(raw) = raw.filter(|raw| *raw > 0) else {
            return Ok(None);
        };
        let minimum = match self.minimum_minor {
            Some(money) => pricing.fewest_credits_reaching(money)?,
            None => 1,
        };
        let maximum = match self.maximum_minor {
            Some(money) => pricing.most_credits_within(money)?,
            None => i64::MAX,
        }
        .max(minimum);
        Ok(Some(raw.clamp(minimum, maximum)))
    }

    /// Write-time validation: structure plus the refill-size policy.
    pub fn validate(&self) -> StoreResult<()> {
        self.validate_structure()?;
        // A fixed refill far below its own trigger re-arms right after every charge.
        if let (AutomaticTopUpMethod::Fixed, Some(amount)) = (self.method, self.amount_minor)
            && amount < self.threshold_minor / MIN_FIXED_AMOUNT_THRESHOLD_RATIO
        {
            return Err(invalid_top_up(
                "A fixed top-up must add at least a tenth of its threshold.",
            ));
        }
        Ok(())
    }

    /// Read-time validation of a stored rule: a policy added after it was saved must never make it
    /// unreadable.
    pub fn validate_structure(&self) -> StoreResult<()> {
        if self.threshold_minor < 0 {
            return Err(invalid_top_up("The threshold can't be negative."));
        }
        let positive = [
            self.amount_minor,
            self.target_minor,
            self.minimum_minor,
            self.maximum_minor,
            self.daily_cap_minor,
            self.monthly_cap_minor,
        ]
        .into_iter()
        .flatten()
        .all(|value| value > 0);
        if !positive || self.consecutive_failures < 0 {
            return Err(invalid_top_up(
                "Amounts, bounds and caps of an automatic top-up must be above zero.",
            ));
        }
        match self.method {
            AutomaticTopUpMethod::Fixed
                if self.amount_minor.is_none() || self.target_minor.is_some() =>
            {
                return Err(invalid_top_up(
                    "A fixed automatic top-up needs an amount, and no target.",
                ));
            }
            AutomaticTopUpMethod::ToTarget
                if self.amount_minor.is_some()
                    || self
                        .target_minor
                        .is_none_or(|target| target <= self.threshold_minor) =>
            {
                return Err(invalid_top_up(
                    "A top-up to a target needs a target above its threshold, and no amount.",
                ));
            }
            _ => {}
        }
        if self
            .minimum_minor
            .zip(self.maximum_minor)
            .is_some_and(|(min, max)| min > max)
        {
            return Err(invalid_top_up(
                "The minimum per top-up can't be above the maximum.",
            ));
        }
        Ok(())
    }

    /// The credit one top-up adds at `available_minor`, within its bounds; None when nothing is due.
    pub fn top_up_amount(&self, available_minor: i64) -> Option<i64> {
        let raw = match self.method {
            AutomaticTopUpMethod::Fixed => self.amount_minor?,
            AutomaticTopUpMethod::ToTarget => self.target_minor?.checked_sub(available_minor)?,
        };
        if raw <= 0 {
            return None;
        }
        let minimum = self.minimum_minor.unwrap_or(1);
        let maximum = self.maximum_minor.unwrap_or(i64::MAX).max(minimum);
        Some(raw.clamp(minimum, maximum))
    }
}

fn invalid_top_up(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.into()))
}

/// Consent evidence stored beside a top-up: the revision of the exact terms authorized, who
/// authorized them (`customer:<id>`, `user:<id>` or `system`) and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomaticTopUpAuthorization {
    pub revision: String,
    pub actor: String,
    pub recorded_at: DateTime<Utc>,
}

/// Refuses an operator's terms that would charge without the customer's confirmation and carry no
/// attestation of their agreement.
pub const ATTESTATION_REQUIRED: &str = "These terms turn automatic top-up on, or change what it charges, without the customer confirming them in their portal: attest that the customer agreed to automatic charges on them.";

/// An operator's statement that the customer agreed to automatic charges on terms the customer did
/// not confirm in their portal. It covers the terms of `revision` only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentAttestation {
    pub revision: String,
    pub attested_by: common_domain::actor::Actor,
    pub attested_at: DateTime<Utc>,
}

/// Starts the reason a credit unit's top-up stopped because the unit cannot be sold.
pub const UNSELLABLE_PREFIX: &str = "Top-up stopped: ";
/// Why a credit unit's top-up stopped when its price rose above the one the rule was authorized
/// at, or its currency changed.
pub const PRICE_CHANGED: &str = "Price changed: review your automatic top-up";
/// Why a rule stopped after one of its payments was reversed (refunded or disputed).
pub const PAYMENT_REVERSED: &str = "A top-up payment was reversed";
/// Why terms the customer authorized stopped after an operator changed them.
pub const AWAITING_CUSTOMER: &str = "Terms changed by the merchant: confirm your automatic top-up";
/// Why a customer's rule stopped when one top-up, tax included, came above the ceiling.
pub const ABOVE_SELF_SERVICE_LIMIT: &str =
    "A top-up would charge more than the self-service limit: review your automatic top-up";

/// Whether a suspension waits for the customer to confirm terms, which nothing else lifts.
pub fn is_awaiting_customer(last_error: Option<&str>) -> bool {
    last_error.is_some_and(|error| error == AWAITING_CUSTOMER || error == ABOVE_SELF_SERVICE_LIMIT)
}

/// Why a rule stopped, from its suspension and last error; None while it runs.
pub fn stop_reason_of(suspended: bool, last_error: Option<&str>) -> Option<AutoTopUpStopReason> {
    if !suspended {
        return None;
    }
    Some(match last_error {
        Some(PRICE_CHANGED) => AutoTopUpStopReason::PriceChanged,
        Some(error) if error.starts_with(UNSELLABLE_PREFIX) => AutoTopUpStopReason::Unsellable,
        Some(PAYMENT_REVERSED) => AutoTopUpStopReason::PaymentReversed,
        error if is_awaiting_customer(error) => AutoTopUpStopReason::AwaitingCustomer,
        _ => AutoTopUpStopReason::PaymentFailed,
    })
}

impl AutomaticTopUpConfig {
    /// Why a suspended rule stopped; None while it runs.
    pub fn stop_reason(&self) -> Option<AutoTopUpStopReason> {
        stop_reason_of(self.suspended_at.is_some(), self.last_error.as_deref())
    }
}

/// A wallet's automatic top-up as stored: live, or parked by the customer so that turning it back
/// on restores its guardrails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomaticTopUp {
    pub wallet_id: WalletId,
    pub config: AutomaticTopUpConfig,
    pub parked: bool,
    pub authorization: Option<AutomaticTopUpAuthorization>,
    /// Set only while it covers the stored terms.
    pub consent_attestation: Option<ConsentAttestation>,
}

/// How a customer's own authorization of automatic top-up terms is recorded.
pub const CUSTOMER_ACTOR_PREFIX: &str = "customer:";

impl AutomaticTopUp {
    /// The rule that may charge a card now; a parked one never does.
    pub fn live(&self) -> Option<&AutomaticTopUpConfig> {
        (!self.parked).then_some(&self.config)
    }

    /// Whether the customer's own confirmation covers the stored terms.
    pub fn authorized_by_customer(&self) -> bool {
        self.authorization
            .as_ref()
            .is_some_and(|authorization| authorization.actor.starts_with(CUSTOMER_ACTOR_PREFIX))
    }

    /// The terms a charge is sized by: a customer's own terms never charge past the self-service
    /// ceiling, including terms stored before the ceiling existed (no maximum).
    pub fn chargeable(self) -> Option<AutomaticTopUpConfig> {
        if self.parked {
            return None;
        }
        let customer = self.authorized_by_customer();
        let mut config = self.config;
        if customer && config.maximum_minor.is_none() {
            config.maximum_minor = Some(self_service_maximum(config.minimum_minor));
            config.maximum_is_default = true;
        }
        Some(config)
    }
}

/// What a wallet's automatic top-ups charged this calendar month (UTC), as its monthly cap counts
/// it: the settled card charges, tax included, in minor units of `currency`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoTopUpMonthSpend {
    pub charged_minor: i64,
    /// The currency the card is charged in, which the caps count: the wallet's, or the customer's
    /// for a credit unit wallet.
    pub currency: String,
}

/// What a wallet draw pays for: an invoice, usage settled ahead of one, or a consumed request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawSource {
    Invoice,
    UsageSettlement,
    Consume,
}

impl DrawSource {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Invoice => "INVOICE",
            Self::UsageSettlement => "USAGE_SETTLEMENT",
            Self::Consume => "CONSUME",
        }
    }

    pub fn from_db_str(value: &str) -> StoreResult<Self> {
        match value {
            "INVOICE" => Ok(Self::Invoice),
            "USAGE_SETTLEMENT" => Ok(Self::UsageSettlement),
            "CONSUME" => Ok(Self::Consume),
            other => Err(Report::new(StoreError::InvalidArgument(format!(
                "unknown customer credit draw source {other}"
            )))),
        }
    }
}

/// An operator's instruction for a wallet's automatic top-up: `Keep` leaves the stored rule
/// exactly as it is (parked or live), `Remove` turns it off, `Set` replaces it, guardrails
/// included. `Set` keeps the price per credit the stored terms were authorized at, and a stop on
/// a price change; only `SetAtCurrentPrice` authorizes today's price and lifts that stop.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum AutoTopUpChange {
    Keep,
    Remove,
    Set(AutomaticTopUpConfig),
    SetAtCurrentPrice(AutomaticTopUpConfig),
    /// `Set`, or `SetAtCurrentPrice` when `reprice`, from an organization whose plan excludes
    /// automatic top-ups: applied only when it resends the stored live terms, else forbidden.
    Resend {
        config: AutomaticTopUpConfig,
        reprice: bool,
    },
}

/// Refuses an automatic top-up being turned on or changed without the `auto_top_ups` gate.
pub const AUTO_TOP_UPS_NOT_IN_PLAN: &str =
    "Automatic top-ups are not enabled for this organization.";

impl AutoTopUpChange {
    /// This change from an organization whose plan excludes automatic top-ups: keeping or
    /// removing one stays open, setting terms may only resend the stored ones.
    pub fn resend_only(self) -> Self {
        match self {
            Self::Set(config) => Self::Resend {
                config,
                reprice: false,
            },
            Self::SetAtCurrentPrice(config) => Self::Resend {
                config,
                reprice: true,
            },
            change => change,
        }
    }

    pub fn sets_terms(&self) -> bool {
        matches!(
            self,
            Self::Set(_) | Self::SetAtCurrentPrice(_) | Self::Resend { .. }
        )
    }
}

/// A change to a credit unit wallet's agreed price per credit, set in the customer's currency.
#[derive(Debug, Clone, Default)]
pub enum UnitPriceChange {
    #[default]
    Keep,
    Clear,
    Set(rust_decimal::Decimal),
}

/// An operator's patch of an invoice-credit wallet. A field left `Keep`/`None` stays exactly as
/// stored, so a caller can change one field without resending (and racing on) the others.
#[derive(Debug, Clone)]
pub struct ConfigureCreditWallet {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub auto_top_up: AutoTopUpChange,
    /// Normalized (sorted, deduplicated, bounded to 5) before it is stored.
    pub low_balance_thresholds_minor: Option<Vec<i64>>,
    pub usage_at_zero: Option<crate::domain::credit_exhausted::UsageAtZero>,
    pub unit_price: UnitPriceChange,
    pub actor_id: Option<Uuid>,
    /// The operator attesting that the customer agreed to automatic charges on the terms
    /// `auto_top_up` sets. Required when those terms turn automatic top-up on or change what it
    /// charges without the customer confirming them in their portal.
    pub consent_attested_by: Option<common_domain::actor::Actor>,
}

/// Everything a customer may author on their own automatic top-up. The operator's guardrails
/// (bounds per top-up, daily cap) are not theirs: an edit merges into the stored rule.
#[derive(Debug, Clone)]
pub struct CustomerAutomaticTopUpEdit {
    pub method: AutomaticTopUpMethod,
    pub threshold_minor: i64,
    pub amount_minor: Option<i64>,
    pub target_minor: Option<i64>,
    pub payment_method_id: CustomerPaymentMethodId,
}

/// The daily cap a customer's terms get when the operator set none: this many top-ups.
const DEFAULT_DAILY_CAP_MULTIPLIER: i64 = 5;

impl CustomerAutomaticTopUpEdit {
    /// The daily cap these terms get when the operator set none, in the credit it adds: a credit
    /// unit wallet's cap is the money price of this many credits.
    pub fn default_daily_cap(&self) -> Option<i64> {
        match self.method {
            AutomaticTopUpMethod::Fixed => self.amount_minor,
            AutomaticTopUpMethod::ToTarget => self
                .target_minor
                .map(|target| target.saturating_sub(self.threshold_minor)),
        }
        .map(|top_up| top_up.saturating_mul(DEFAULT_DAILY_CAP_MULTIPLIER))
        .filter(|cap| *cap > 0)
    }

    /// The customer's trigger, amount and card win; the operator's guardrails and the counters
    /// are carried over. Unset by the operator, the daily cap follows these terms and one charge
    /// is capped at the self-service ceiling. Suspension lifts (the edit is the customer's fix)
    /// and with it the failure streak and error; on a running rule they reset only with a new card.
    pub fn merge(&self, stored: Option<&AutomaticTopUpConfig>) -> AutomaticTopUpConfig {
        let payment_method_changed =
            stored.and_then(|stored| stored.payment_method_id) != Some(self.payment_method_id);
        let fresh_start =
            payment_method_changed || stored.is_some_and(|stored| stored.suspended_at.is_some());
        let minimum_minor = stored.and_then(|stored| stored.minimum_minor);
        let operator_maximum = stored
            .filter(|stored| !stored.maximum_is_default)
            .and_then(|stored| stored.maximum_minor);
        let maximum_minor = operator_maximum.unwrap_or_else(|| self_service_maximum(minimum_minor));
        let operator_daily_cap = stored
            .filter(|stored| !stored.daily_cap_is_default)
            .and_then(|stored| stored.daily_cap_minor);
        let daily_cap_minor = operator_daily_cap.or_else(|| self.default_daily_cap());
        AutomaticTopUpConfig {
            method: self.method,
            threshold_minor: self.threshold_minor,
            amount_minor: self.amount_minor,
            target_minor: self.target_minor,
            minimum_minor,
            maximum_minor: Some(maximum_minor),
            daily_cap_minor,
            monthly_cap_minor: stored.and_then(|stored| stored.monthly_cap_minor),
            payment_method_id: Some(self.payment_method_id),
            consecutive_failures: if fresh_start {
                0
            } else {
                stored.map_or(0, |stored| stored.consecutive_failures)
            },
            // The backoff of a failing rule outlives an edit on the same card.
            last_attempted_at: if fresh_start {
                None
            } else {
                stored.and_then(|stored| stored.last_attempted_at)
            },
            last_succeeded_at: stored.and_then(|stored| stored.last_succeeded_at),
            suspended_at: None,
            last_error: if fresh_start {
                None
            } else {
                stored.and_then(|stored| stored.last_error.clone())
            },
            agreed_price: stored.and_then(|stored| stored.agreed_price.clone()),
            daily_cap_is_default: operator_daily_cap.is_none() && daily_cap_minor.is_some(),
            maximum_is_default: operator_maximum.is_none(),
        }
    }
}

/// A customer's change to their automatic top-up; `edit` None turns it off and keeps its terms
/// for later. Enabling or changing it confirms the exact reviewed terms by their revision.
#[derive(Debug, Clone)]
pub struct ConfigureCustomerAutomaticTopUp {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub edit: Option<CustomerAutomaticTopUpEdit>,
    pub expected_configuration_revision: Option<String>,
}

/// A customer's purchase of invoice credit charged to a saved payment method. The key is the
/// caller's: a retry with the same key never charges twice.
#[derive(Debug, Clone)]
pub struct TopUpWallet {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    /// In minor units of the wallet: money, or credits for a credit unit wallet.
    pub amount_minor: i64,
    pub payment_method_id: CustomerPaymentMethodId,
    pub idempotency_key: String,
    /// Absolute expiry of the purchased credit; unset never expires.
    pub expires_at: Option<DateTime<Utc>>,
    /// A credit unit wallet only: an operator's negotiated price of one credit, in major units of
    /// the customer's currency. Unset sells at the customer's agreed price, else the list price.
    pub unit_price: Option<rust_decimal::Decimal>,
    /// A credit unit wallet only: the price per credit the operator was shown. A top-up that would
    /// charge another one is refused before anything is charged.
    pub expected_unit_price: Option<rust_decimal::Decimal>,
}

/// The price of a top-up as the charge will be made. Credit bought with currency is taxed when it
/// is spent, so the charge is the credit amount and the tax is zero; credits of a unit are sold
/// at their price in the customer's currency, plus tax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopUpWalletQuote {
    pub charge_minor: i64,
    pub currency: String,
    pub net_minor: i64,
    pub tax_minor: i64,
}

/// Where a self-service top-up stands, as the customer may be told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelfServiceTopUpState {
    Succeeded,
    Pending {
        requires_action: bool,
    },
    /// The charge failed; a new top-up is needed. `reason` is safe to show the customer.
    Failed {
        reason: String,
    },
}

/// What funded a grant, as its `source_kind` records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreditGrantSource {
    /// Carried over from the customer balance that preceded the ledger.
    CustomerBalance,
    /// Bought: a credit purchase invoice, a credit pack or a top-up.
    Purchase,
    /// Included with a subscription's credit pack.
    Included,
    CreditNote,
    /// The negative total of a finalized invoice.
    NegativeInvoice,
    /// Given back by the void of an invoice finalized before the ledger.
    InvoiceVoid,
    /// Given back by a usage settlement no invoice took over. Only ever an event's source.
    UsageSettlementReturn,
    /// Added by an operator or through the API.
    Manual,
    /// Collected above what an invoice asked, or a payment no invoice took.
    Overpayment,
    /// Carried over from expiring included credit.
    Rollover,
    /// Redeemed with a promotional credit coupon.
    Coupon,
}

impl CreditGrantSource {
    pub fn from_db_str(value: &str) -> StoreResult<Self> {
        use crate::repositories::credit::customer_credit::source;
        match value {
            source::CUSTOMER_BALANCE => Ok(Self::CustomerBalance),
            source::CREDIT_PURCHASE => Ok(Self::Purchase),
            source::INCLUDED_CREDIT => Ok(Self::Included),
            source::CREDIT_NOTE => Ok(Self::CreditNote),
            source::NEGATIVE_INVOICE => Ok(Self::NegativeInvoice),
            source::INVOICE_VOID_RESTORE => Ok(Self::InvoiceVoid),
            source::MANUAL => Ok(Self::Manual),
            source::OVERPAYMENT => Ok(Self::Overpayment),
            source::ROLLOVER => Ok(Self::Rollover),
            source::COUPON => Ok(Self::Coupon),
            other => Err(Report::new(StoreError::InvalidArgument(format!(
                "unknown credit grant source {other}"
            )))),
        }
    }
}

impl CreditGrant {
    /// When its value lapses: its expiry, or the end of its subscription if earlier.
    pub fn lapses_at(&self) -> Option<DateTime<Utc>> {
        match (self.expires_at, self.ends_at) {
            (Some(expires_at), Some(ends_at)) => Some(expires_at.min(ends_at)),
            (expires_at, ends_at) => expires_at.or(ends_at),
        }
    }

    /// The promotional part of paid credit bought below its face.
    pub fn is_bonus(&self) -> bool {
        self.classification == GrantClassification::Promotional
            && self.reason == crate::repositories::credit::customer_credit::BONUS_CREDIT
    }

    /// Depleted or expired: nothing of it can be spent any more.
    pub fn used_up(&self) -> bool {
        matches!(
            self.state,
            CreditGrantState::Depleted | CreditGrantState::Expired
        )
    }

    /// Operators remove only credit added by hand, and only what is left of it.
    pub fn removable(&self) -> bool {
        self.source == CreditGrantSource::Manual
            && self.state == CreditGrantState::Active
            && self.remaining_minor > 0
    }
}

/// One wallet of the tenant-wide roster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditWalletSummary {
    pub id: WalletId,
    pub customer_id: CustomerId,
    pub customer_name: String,
    pub customer_alias: Option<String>,
    pub currency: String,
    pub credit_unit: Option<crate::domain::credit_units::CreditUnit>,
    pub available_minor: i64,
    pub pending_minor: i64,
    pub low_balance_thresholds_minor: Vec<i64>,
    pub auto_top_up_enabled: bool,
    pub auto_top_up_suspended: bool,
    /// Failed at least once since its last success, and not suspended yet.
    pub auto_top_up_failing: bool,
    pub auto_top_up_threshold_minor: Option<i64>,
    pub usage_at_zero: crate::domain::credit_exhausted::UsageAtZero,
    /// See `CreditWallet::pauses_at_zero`.
    pub pauses_at_zero: bool,
    pub live: Option<WalletLiveBalance>,
    pub live_stale: bool,
    pub status: WalletStatus,
    pub created_at: DateTime<Utc>,
}

/// What the invoicing entity lets its customers do from the portal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PortalCreditCapabilities {
    pub top_up_enabled: bool,
    pub auto_top_up_enabled: bool,
}

/// A public API write keyed by the caller's Idempotency-Key. The fingerprint is a digest of the
/// request, so the same key sent with a different request is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiIdempotency {
    pub operation: &'static str,
    pub key: String,
    pub fingerprint: [u8; 32],
}

impl ApiIdempotency {
    pub fn new(operation: &'static str, key: &str, fingerprint: [u8; 32]) -> StoreResult<Self> {
        let key = key.trim();
        if key.is_empty() || key.len() > 255 {
            return Err(Report::new(StoreError::InvalidArgument(
                "an idempotency key is 1 to 255 bytes".to_owned(),
            )));
        }
        Ok(Self {
            operation,
            key: key.to_owned(),
            fingerprint,
        })
    }

    /// Stable per tenant, operation and key, so a retry addresses what its first attempt made.
    pub fn derived_id(&self, tenant_id: TenantId) -> Uuid {
        Uuid::new_v5(
            &tenant_id.as_uuid(),
            format!("api-write:{}:{}", self.operation, self.key).as_bytes(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Consent at €0.0010 per credit covers €0.0008 and €0.0010 again, never €0.0011 nor the
    /// same price in another currency.
    #[test]
    fn consent_covers_its_price_or_less_in_its_currency() {
        use common_domain::ids::BaseId;
        use rust_decimal_macros::dec;
        let agreed = AgreedCreditPrice {
            price_id: common_domain::ids::CreditUnitPriceId::new(),
            unit_price: dec!(0.001),
            currency: "EUR".to_owned(),
        };
        assert!(agreed.covers(dec!(0.0008), "EUR"));
        assert!(agreed.covers(dec!(0.00100), "EUR"));
        assert!(!agreed.covers(dec!(0.0011), "EUR"));
        assert!(!agreed.covers(dec!(0.0008), "USD"));
    }

    #[test]
    fn db_values_round_trip() {
        for application in [CreditApplication::Invoice, CreditApplication::Usage] {
            assert_eq!(
                CreditApplication::from_db_str(application.as_db_str()).unwrap(),
                application
            );
        }
        for classification in [GrantClassification::Paid, GrantClassification::Promotional] {
            assert_eq!(
                GrantClassification::from_db_str(classification.as_db_str()).unwrap(),
                classification
            );
        }
        for kind in [
            LedgerEntryKind::Fund,
            LedgerEntryKind::Draw,
            LedgerEntryKind::DrawReversal,
            LedgerEntryKind::Reversal,
            LedgerEntryKind::Reinstatement,
            LedgerEntryKind::Expiry,
            LedgerEntryKind::Rollover,
        ] {
            assert_eq!(
                LedgerEntryKind::from_db_str(kind.as_db_str()).unwrap(),
                kind
            );
        }
    }

    #[test]
    fn a_top_up_is_sized_by_its_method_within_its_bounds() {
        let fixed =
            AutomaticTopUpConfig::new(AutomaticTopUpMethod::Fixed, 1_000, Some(5_000), None, None);
        assert_eq!(fixed.top_up_amount(0), Some(5_000));
        let target = AutomaticTopUpConfig {
            maximum_minor: Some(7_000),
            minimum_minor: Some(2_000),
            ..AutomaticTopUpConfig::new(
                AutomaticTopUpMethod::ToTarget,
                1_000,
                None,
                Some(10_000),
                None,
            )
        };
        assert_eq!(target.top_up_amount(4_000), Some(6_000));
        assert_eq!(target.top_up_amount(0), Some(7_000));
        assert_eq!(target.top_up_amount(9_500), Some(2_000));
        assert_eq!(target.top_up_amount(10_000), None);
    }

    #[test]
    fn a_rule_is_validated_by_its_structure_and_refill_policy() {
        let fixed = |threshold, amount| {
            AutomaticTopUpConfig::new(AutomaticTopUpMethod::Fixed, threshold, amount, None, None)
        };
        assert!(fixed(1_000, Some(100)).validate().is_ok());
        assert!(fixed(1_000, Some(99)).validate().is_err());
        assert!(fixed(1_000, Some(99)).validate_structure().is_ok());
        assert!(fixed(1_000, None).validate().is_err());
        let target = |target| {
            AutomaticTopUpConfig::new(AutomaticTopUpMethod::ToTarget, 1_000, None, target, None)
        };
        assert!(target(Some(1_000)).validate().is_err());
        assert!(target(Some(1_001)).validate().is_ok());
        let inverted = AutomaticTopUpConfig {
            minimum_minor: Some(500),
            maximum_minor: Some(400),
            ..fixed(1_000, Some(500))
        };
        assert!(inverted.validate().is_err());
    }

    #[test]
    fn the_revision_covers_the_terms_not_the_runtime_state() {
        let rule =
            AutomaticTopUpConfig::new(AutomaticTopUpMethod::Fixed, 1_000, Some(500), None, None);
        let failed = AutomaticTopUpConfig {
            consecutive_failures: 2,
            last_error: Some("declined".to_owned()),
            ..rule.clone()
        };
        assert_eq!(
            rule.authorization_revision(),
            failed.authorization_revision()
        );
        let changed = AutomaticTopUpConfig {
            threshold_minor: 2_000,
            ..rule.clone()
        };
        assert_ne!(
            rule.authorization_revision(),
            changed.authorization_revision()
        );
    }

    #[test]
    fn a_customer_edit_keeps_the_operator_guardrails() {
        let method = CustomerPaymentMethodId::new();
        let stored = AutomaticTopUpConfig {
            minimum_minor: Some(1_000),
            maximum_minor: Some(20_000),
            daily_cap_minor: Some(50_000),
            consecutive_failures: 2,
            suspended_at: Some(common_domain::clock::now()),
            ..AutomaticTopUpConfig::new(
                AutomaticTopUpMethod::Fixed,
                1_000,
                Some(5_000),
                None,
                Some(method),
            )
        };
        let edit = CustomerAutomaticTopUpEdit {
            method: AutomaticTopUpMethod::Fixed,
            threshold_minor: 2_000,
            amount_minor: Some(8_000),
            target_minor: None,
            payment_method_id: method,
        };
        let merged = edit.merge(Some(&stored));
        assert_eq!(merged.daily_cap_minor, Some(50_000));
        assert!(!merged.daily_cap_is_default);
        assert_eq!(merged.maximum_minor, Some(20_000));
        assert!(merged.suspended_at.is_none());
        assert_eq!(
            (merged.consecutive_failures, merged.last_error.as_deref()),
            (0, None),
            "resuming a suspended rule clears its failures"
        );
        let running = AutomaticTopUpConfig {
            suspended_at: None,
            last_error: Some("card_declined".to_owned()),
            ..stored.clone()
        };
        let attempted = common_domain::clock::now();
        let running = AutomaticTopUpConfig {
            last_attempted_at: Some(attempted),
            ..running
        };
        let edited = edit.merge(Some(&running));
        assert_eq!(
            (edited.consecutive_failures, edited.last_error.as_deref()),
            (2, Some("card_declined")),
            "an edit of a running rule keeps the streak on the same card"
        );
        assert_eq!(
            edited.last_attempted_at,
            Some(attempted),
            "and its backoff: the same card is not charged again at once"
        );
        let resumed = edit.merge(Some(&AutomaticTopUpConfig {
            last_attempted_at: Some(attempted),
            ..stored.clone()
        }));
        assert_eq!(resumed.last_attempted_at, None, "resuming starts afresh");
        let new_card = CustomerAutomaticTopUpEdit {
            payment_method_id: CustomerPaymentMethodId::new(),
            ..edit
        };
        assert_eq!(new_card.merge(Some(&stored)).consecutive_failures, 0);
        let first = new_card.merge(None);
        assert_eq!(first.daily_cap_minor, Some(40_000));
        assert!(first.daily_cap_is_default);
        assert_eq!(first.maximum_minor, Some(MAX_SELF_SERVICE_CHARGE_MINOR));
    }

    /// RB-2: a default cap stored with the customer's terms follows their next amount, so raising
    /// it is never refused by the cap the previous amount set.
    #[test]
    fn a_default_daily_cap_follows_the_customer_terms() {
        let method = CustomerPaymentMethodId::new();
        let edit = |amount| CustomerAutomaticTopUpEdit {
            method: AutomaticTopUpMethod::Fixed,
            threshold_minor: 1_000,
            amount_minor: Some(amount),
            target_minor: None,
            payment_method_id: method,
        };
        let first = edit(2_000).merge(None);
        assert_eq!(first.daily_cap_minor, Some(10_000));
        let raised = edit(5_000).merge(Some(&first));
        assert_eq!(raised.daily_cap_minor, Some(25_000));
        assert!(raised.daily_cap_is_default);
        let operator_capped = AutomaticTopUpConfig {
            daily_cap_is_default: false,
            ..first
        };
        assert_eq!(
            edit(5_000).merge(Some(&operator_capped)).daily_cap_minor,
            Some(10_000)
        );
    }

    /// RB-3: the self-service ceiling never lowers an operator's minimum below itself.
    #[test]
    fn the_self_service_ceiling_respects_an_operator_minimum() {
        let stored = AutomaticTopUpConfig {
            minimum_minor: Some(2 * MAX_SELF_SERVICE_CHARGE_MINOR),
            ..AutomaticTopUpConfig::new(AutomaticTopUpMethod::Fixed, 1_000, Some(5_000), None, None)
        };
        let merged = CustomerAutomaticTopUpEdit {
            method: AutomaticTopUpMethod::Fixed,
            threshold_minor: 1_000,
            amount_minor: Some(5_000),
            target_minor: None,
            payment_method_id: CustomerPaymentMethodId::new(),
        }
        .merge(Some(&stored));
        assert_eq!(
            merged.maximum_minor,
            Some(2 * MAX_SELF_SERVICE_CHARGE_MINOR)
        );
    }

    #[test]
    fn a_reference_is_trimmed_and_bounded() {
        assert_eq!(
            FundingReference::new("  INV-42 ").unwrap().as_str(),
            "INV-42"
        );
        assert!(FundingReference::new("   ").is_err());
        assert!(FundingReference::new("x".repeat(256)).is_err());
        assert!(FundingReference::new("x".repeat(255)).is_ok());
    }

    #[test]
    fn a_retry_addresses_the_same_write_and_nothing_else_does() {
        let tenant = TenantId::new();
        let key = |operation, key: &str| ApiIdempotency::new(operation, key, [1; 32]).unwrap();
        let first = key("credit_grant.create", "k1").derived_id(tenant);
        assert_eq!(first, key("credit_grant.create", " k1 ").derived_id(tenant));
        assert_ne!(first, key("credit_grant.create", "k2").derived_id(tenant));
        assert_ne!(first, key("top_up.create", "k1").derived_id(tenant));
        assert_ne!(
            first,
            key("credit_grant.create", "k1").derived_id(TenantId::new())
        );
        assert!(ApiIdempotency::new("top_up.create", "  ", [0; 32]).is_err());
        assert!(ApiIdempotency::new("top_up.create", &"k".repeat(256), [0; 32]).is_err());
    }

    #[test]
    fn a_rollover_carries_up_to_its_cap_and_runs_its_term_from_the_expiry() {
        use super::super::price_components::CreditExpiry;
        use chrono::TimeZone;
        let capped = GrantRollover {
            max_minor: Some(2_000),
            expiry: CreditExpiry::Month(1),
        };
        assert_eq!(capped.carried(5_000), 2_000, "capped");
        assert_eq!(capped.carried(1_500), 1_500, "under the cap");
        assert_eq!(capped.carried(0), 0, "nothing left");
        assert_eq!(capped.carried(-10), 0);
        let uncapped = GrantRollover {
            max_minor: None,
            expiry: CreditExpiry::Day(10),
        };
        assert_eq!(uncapped.carried(5_000), 5_000);
        let expired = Utc.with_ymd_and_hms(2026, 1, 31, 0, 0, 0).unwrap();
        assert_eq!(
            capped.rolled_expiry(expired),
            Some(Utc.with_ymd_and_hms(2026, 2, 28, 0, 0, 0).unwrap())
        );
        assert_eq!(
            uncapped.rolled_expiry(expired),
            Some(Utc.with_ymd_and_hms(2026, 2, 10, 0, 0, 0).unwrap())
        );
        let unbounded = GrantRollover {
            max_minor: None,
            expiry: CreditExpiry::BillingPeriod,
        };
        assert_eq!(unbounded.rolled_expiry(expired), None);
    }
}
