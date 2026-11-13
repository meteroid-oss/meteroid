use crate::currencies::mapping::from_str as currency_from_str;
use crate::currencies::model::Currency;
use chrono::{DateTime, Utc};
use common_domain::ids::{
    CreditGrantId, CreditNoteId, CustomerPaymentMethodId, InvoiceId, LedgerEntryId,
    PaymentTransactionId, ProductId, SubscriptionId, string_serde_opt, string_serde_vec,
    string_serde_vec_opt,
};
use meteroid_app::errors::RestApiError;
use meteroid_app::rest::denomination::Denomination;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_rest_common::model::{PaginatedRequest, PaginationResponse};
use meteroid_store::domain::credit as domain;
use meteroid_store::domain::credit_groups::{self as groups, GrantPlans};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

#[derive(Debug, Clone, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CreditGrantListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    /// Only grants in this currency.
    #[param(value_type = Option<Currency>)]
    pub currency: Option<Currency>,
    /// Only grants in this credit unit (its code); excludes `currency`.
    pub credit_unit: Option<String>,
    #[param(inline)]
    pub status: Option<CreditGrantStatus>,
    /// Only the grants that belong to this subscription (`sub_…`); excludes
    /// `usable_by_subscription_id`.
    #[serde(default, with = "string_serde_opt")]
    #[param(value_type = Option<String>)]
    pub subscription_id: Option<SubscriptionId>,
    /// Only the credit this subscription (`sub_…`) can use: its own grants and the customer's
    /// grants that belong to no subscription. Excludes `subscription_id`.
    #[serde(default, with = "string_serde_opt")]
    #[param(value_type = Option<String>)]
    pub usable_by_subscription_id: Option<SubscriptionId>,
}

impl CreditGrantListRequest {
    pub fn subscription_filter(
        &self,
    ) -> Result<Option<domain::GrantSubscriptionFilter>, RestApiError> {
        match (self.subscription_id, self.usable_by_subscription_id) {
            (Some(_), Some(_)) => Err(RestApiError::InvalidInput(
                "subscription_id and usable_by_subscription_id cannot be combined".to_owned(),
            )),
            (Some(id), None) => Ok(Some(domain::GrantSubscriptionFilter::Of(id))),
            (None, Some(id)) => Ok(Some(domain::GrantSubscriptionFilter::UsableBy(id))),
            (None, None) => Ok(None),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CreditLedgerListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    /// Defaults to the customer's currency.
    #[param(value_type = Option<Currency>)]
    pub currency: Option<Currency>,
    /// The ledger of this credit unit (its code) instead of a currency; excludes `currency`.
    pub credit_unit: Option<String>,
}

/// What a customer's usage does once the credit it can use runs out: what the subscription's plan
/// says (`FOLLOW_PLAN`), or always or never refused for this customer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UsageAtZero {
    #[default]
    FollowPlan,
    AlwaysPause,
    NeverPause,
}

impl From<meteroid_store::domain::credit_exhausted::UsageAtZero> for UsageAtZero {
    fn from(value: meteroid_store::domain::credit_exhausted::UsageAtZero) -> Self {
        use meteroid_store::domain::credit_exhausted::UsageAtZero as Domain;
        match value {
            Domain::FollowPlan => Self::FollowPlan,
            Domain::AlwaysPause => Self::AlwaysPause,
            Domain::NeverPause => Self::NeverPause,
        }
    }
}

impl From<UsageAtZero> for meteroid_store::domain::credit_exhausted::UsageAtZero {
    fn from(value: UsageAtZero) -> Self {
        match value {
            UsageAtZero::FollowPlan => Self::FollowPlan,
            UsageAtZero::AlwaysPause => Self::AlwaysPause,
            UsageAtZero::NeverPause => Self::NeverPause,
        }
    }
}

/// Which charges credit pays: ALL pays any part of an invoice, USAGE only its usage charges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreditAppliesTo {
    #[default]
    All,
    Usage,
}

impl From<domain::CreditApplication> for CreditAppliesTo {
    fn from(value: domain::CreditApplication) -> Self {
        match value {
            domain::CreditApplication::Invoice => Self::All,
            domain::CreditApplication::Usage => Self::Usage,
        }
    }
}

impl From<CreditAppliesTo> for domain::CreditApplication {
    fn from(value: CreditAppliesTo) -> Self {
        match value {
            CreditAppliesTo::All => Self::Invoice,
            CreditAppliesTo::Usage => Self::Usage,
        }
    }
}

/// The group a grant is shown in, from where it came from and what it pays:
/// - INCLUDED: credit a subscription's plan includes, which renews with it;
/// - PREPAID: credit bought (purchases, top-ups, paid packs, credit paid for outside Meteroid);
/// - BONUS: credit given (the bonus part of discounted credit, promo codes, welcome credit);
/// - ACCOUNT_BALANCE: the customer's money that pays any invoice (credit notes, returned invoices,
///   the balance carried from before credit grants, credit added for any charge).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreditGrantKind {
    Included,
    Prepaid,
    Bonus,
    AccountBalance,
}

impl From<groups::CreditGrantKind> for CreditGrantKind {
    fn from(value: groups::CreditGrantKind) -> Self {
        match value {
            groups::CreditGrantKind::Included => Self::Included,
            groups::CreditGrantKind::Prepaid => Self::Prepaid,
            groups::CreditGrantKind::Bonus => Self::Bonus,
            groups::CreditGrantKind::AccountBalance => Self::AccountBalance,
        }
    }
}

/// PAID credit was bought, through Meteroid or outside it, and applies after tax like a payment.
/// PROMOTIONAL credit was given (included with a plan, a coupon, bonus credit, a goodwill grant):
/// it applies before tax, like a discount, so it is spent before paid credit. Credit-unit grants
/// are drawn together, whatever their type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreditGrantType {
    Paid,
    Promotional,
}

impl From<domain::GrantClassification> for CreditGrantType {
    fn from(value: domain::GrantClassification) -> Self {
        match value {
            domain::GrantClassification::Paid => Self::Paid,
            domain::GrantClassification::Promotional => Self::Promotional,
        }
    }
}

impl From<CreditGrantType> for domain::GrantClassification {
    fn from(value: CreditGrantType) -> Self {
        match value {
            CreditGrantType::Paid => Self::Paid,
            CreditGrantType::Promotional => Self::Promotional,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreditGrantStatus {
    /// Takes effect later.
    Pending,
    Active,
    Depleted,
    Expired,
}

impl From<domain::CreditGrantState> for CreditGrantStatus {
    fn from(value: domain::CreditGrantState) -> Self {
        match value {
            domain::CreditGrantState::Pending => Self::Pending,
            domain::CreditGrantState::Active => Self::Active,
            domain::CreditGrantState::Depleted => Self::Depleted,
            domain::CreditGrantState::Expired => Self::Expired,
        }
    }
}

impl From<CreditGrantStatus> for domain::CreditGrantState {
    fn from(value: CreditGrantStatus) -> Self {
        match value {
            CreditGrantStatus::Pending => Self::Pending,
            CreditGrantStatus::Active => Self::Active,
            CreditGrantStatus::Depleted => Self::Depleted,
            CreditGrantStatus::Expired => Self::Expired,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreditGrantSource {
    /// Granted by hand or through the API ("Credit added"); a PAID one was paid outside Meteroid.
    Manual,
    /// Bought: a top-up, an automatic top-up, a credit purchase invoice or a plan's paid credit
    /// pack ("Credit purchased").
    Purchase,
    /// Included with a subscription's plan ("Included with plan").
    Plan,
    /// Given back by a credit note.
    CreditNote,
    /// Returned to the customer: the negative total of an invoice, or a voided invoice.
    Adjustment,
    /// Carried over from the customer balance that preceded credit grants.
    Migrated,
    /// Unspent included credit carried over when it expired.
    Rollover,
    /// Redeemed with a promotional credit coupon.
    Coupon,
    /// Money collected above what an invoice asked, kept as credit ("Overpayment"). Paid, usable
    /// on any charge, never expires; it follows the payments: a reversal takes it back.
    Overpayment,
}

impl From<domain::CreditGrantSource> for CreditGrantSource {
    fn from(value: domain::CreditGrantSource) -> Self {
        match value {
            domain::CreditGrantSource::Manual => Self::Manual,
            domain::CreditGrantSource::Purchase => Self::Purchase,
            domain::CreditGrantSource::Included => Self::Plan,
            domain::CreditGrantSource::CreditNote => Self::CreditNote,
            domain::CreditGrantSource::NegativeInvoice
            | domain::CreditGrantSource::InvoiceVoid
            | domain::CreditGrantSource::UsageSettlementReturn => Self::Adjustment,
            domain::CreditGrantSource::CustomerBalance => Self::Migrated,
            domain::CreditGrantSource::Rollover => Self::Rollover,
            domain::CreditGrantSource::Coupon => Self::Coupon,
            domain::CreditGrantSource::Overpayment => Self::Overpayment,
        }
    }
}

/// Money in minor units of `currency`, net of tax.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreditCost {
    /// In minor units of `currency`.
    pub amount: i64,
    pub currency: Currency,
}

impl CreditCost {
    fn new(cost: &domain::CreditCost) -> Result<Self, RestApiError> {
        Ok(Self {
            amount: cost.amount_minor,
            currency: currency_from_str(&cost.currency)?,
        })
    }
}

/// Money in minor units of `currency`, net of tax.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreditCostRequest {
    /// In minor units of `currency`; zero or positive.
    pub amount: i64,
    pub currency: Currency,
}

impl From<&CreditCostRequest> for domain::CreditCost {
    fn from(value: &CreditCostRequest) -> Self {
        Self::at_face(value.amount, &value.currency.to_string())
    }
}

/// A lot of credit given to or bought by a customer. An invoice applies promotional currency
/// credit before tax and paid credit after it. Within each, it draws the grants that belong to
/// the invoiced subscription first, then grants restricted to products (the fewest products
/// first), then usage-only grants, then the others; then by lowest `priority`, then the soonest
/// to lapse.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreditGrant {
    /// The grant's id (`cgr_…`).
    #[serde(with = "common_domain::ids::string_serde")]
    #[schema(value_type = String)]
    pub id: CreditGrantId,
    /// PAID or PROMOTIONAL: whether it applies after or before tax.
    #[serde(rename = "type")]
    pub grant_type: CreditGrantType,
    /// PENDING until `effective_at`, then ACTIVE, DEPLETED once spent, or EXPIRED.
    pub status: CreditGrantStatus,
    /// Where the credit came from.
    pub source: CreditGrantSource,
    /// Absent when the grant is in a credit unit.
    pub currency: Option<Currency>,
    /// The credit unit (its code) the amounts are in; absent for a currency.
    pub credit_unit: Option<String>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    pub credit_unit_precision: Option<u8>,
    /// ALL pays any charge on an invoice; USAGE only usage charges.
    pub applies_to: CreditAppliesTo,
    /// The products whose charges the credit pays, combined with `applies_to`; empty when it
    /// applies to every product.
    #[serde(with = "string_serde_vec")]
    pub applicable_product_ids: Vec<ProductId>,
    /// The subscription the grant belongs to (`sub_…`): it pays only that subscription's
    /// charges, and lapses when the subscription ends. Null when it pays any of the customer's
    /// subscriptions.
    #[serde(with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub subscription_id: Option<SubscriptionId>,
    /// The group the grant is shown in: INCLUDED, PREPAID, BONUS or ACCOUNT_BALANCE.
    pub kind: CreditGrantKind,
    /// The group's name as Meteroid shows it: "Included with <plan>", "Purchased credit", "Free
    /// credit" or "Account balance".
    pub kind_label: String,
    /// Amount granted, in minor units of `currency` or `credit_unit`.
    pub amount: i64,
    /// Not spent yet, in the same minor units.
    pub remaining: i64,
    /// Lost to expiry; present only on an expired grant.
    pub expired_amount: Option<i64>,
    /// Lower values are drawn first among grants of the same type that pay the same charges; 100
    /// by default.
    pub priority: i32,
    /// When the credit became spendable.
    pub effective_at: ApiDateTime,
    /// When unspent credit lapses; absent never expires.
    pub expires_at: Option<ApiDateTime>,
    /// When the subscription the grant belongs to ended, if it ended. Unspent credit lapses then
    /// (status EXPIRED), even before `expires_at`.
    pub ended_at: Option<ApiDateTime>,
    /// The invoice that sold the credit, or whose void or negative total gave it back.
    #[serde(with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub invoice_id: Option<InvoiceId>,
    /// The credit note that gave the credit back.
    #[serde(with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub credit_note_id: Option<CreditNoteId>,
    /// Shown on the credit ledger.
    pub reason: String,
    /// Your own reference, e.g. a contract or support ticket.
    pub reference: Option<String>,
    /// When the grant was recorded.
    pub created_at: ApiDateTime,
    /// What the whole grant was paid, net of tax: zero for promotional or included credit. A
    /// PAID grant created through the API without an invoice ("Paid outside Meteroid") carries
    /// the cost it was given. Absent for credit granted before costs were recorded.
    pub cost: Option<CreditCost>,
    /// `cost` per one major unit of credit, in major units of the cost currency (e.g. "0.008"),
    /// rounded to 12 decimal places, for display only.
    pub cost_basis: Option<String>,
    /// Only on the response that created a PAID currency grant whose `cost` was below its
    /// amount: the PROMOTIONAL "Bonus credit" grant created beside it for the difference, with
    /// the same expiry, scope and priority.
    #[serde(skip_serializing_if = "Option::is_none", with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub bonus_grant_id: Option<CreditGrantId>,
}

impl CreditGrant {
    pub fn new(
        grant: &domain::CreditGrant,
        denomination: &Denomination,
        plans: &GrantPlans,
    ) -> Result<Self, RestApiError> {
        let grant_kind = grant.kind(denomination.credit_unit.is_some());
        let kind: CreditGrantKind = grant_kind.into();
        Ok(Self {
            id: grant.id,
            grant_type: grant.classification.into(),
            status: grant.state.into(),
            source: grant.source.into(),
            currency: denomination.currency.clone(),
            credit_unit: denomination.credit_unit.clone(),
            credit_unit_precision: denomination.credit_unit_precision,
            applies_to: grant.scope.application.into(),
            applicable_product_ids: grant.scope.product_ids.clone(),
            subscription_id: grant.scope.subscription_id,
            kind,
            kind_label: groups::kind_label(grant_kind, grant.scope.subscription_id, plans),
            amount: grant.amount_minor,
            remaining: grant.remaining_minor,
            expired_amount: grant.expired_minor,
            priority: grant.priority,
            effective_at: grant.effective_at.into(),
            expires_at: grant.expires_at.map(Into::into),
            ended_at: grant.ends_at.map(Into::into),
            invoice_id: grant.invoice_id,
            credit_note_id: grant.credit_note_id,
            reason: grant.reason.clone(),
            reference: grant.reference.clone(),
            created_at: grant.created_at.into(),
            cost: grant.cost.as_ref().map(CreditCost::new).transpose()?,
            cost_basis: grant
                .cost
                .as_ref()
                .zip(denomination.precision())
                .and_then(|(cost, precision)| cost.per_unit(grant.amount_minor, precision))
                .map(|basis| basis.to_string()),
            bonus_grant_id: None,
        })
    }
}

/// A page of credit grants.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreditGrantListResponse {
    pub data: Vec<CreditGrant>,
    pub pagination_meta: PaginationResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AutoTopUpStatus {
    Active,
    /// Turned off; the terms are kept.
    Disabled,
    /// Stopped: after failed or reversed payments, after a price increase or a change of the
    /// customer's currency, or until the customer confirms terms an operator changed. See
    /// `auto_top_up` on the wallet patch for what resumes it.
    Suspended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AutoTopUpMethod {
    /// Buys `amount` each time the balance falls below the threshold.
    Fixed,
    /// Buys enough to bring the balance back to `target`.
    ToTarget,
}

impl From<AutoTopUpMethod> for domain::AutomaticTopUpMethod {
    fn from(value: AutoTopUpMethod) -> Self {
        match value {
            AutoTopUpMethod::Fixed => Self::Fixed,
            AutoTopUpMethod::ToTarget => Self::ToTarget,
        }
    }
}

/// Automatic top-up of the credit that applies to ALL charges: it charges the customer's payment
/// method when the balance falls below `threshold`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AutoTopUp {
    /// ACTIVE, DISABLED (terms kept) or SUSPENDED after repeated failed charges.
    pub status: AutoTopUpStatus,
    /// FIXED buys `amount` each time; TO_TARGET buys back up to `target`.
    pub method: AutoTopUpMethod,
    /// Tops up once the balance is at or below this, in minor units of the wallet.
    pub threshold: i64,
    /// FIXED only: credit bought each time, in minor units of the wallet.
    pub amount: Option<i64>,
    /// TO_TARGET only: the balance a top-up brings the wallet back to, in minor units.
    pub target: Option<i64>,
    /// Smallest single charge. For a credit unit wallet it is money (the price before tax, in
    /// minor units of the customer's currency), like the caps; the threshold, amount and target
    /// are credits.
    pub minimum: Option<i64>,
    /// Largest single charge, in the same money as `minimum`.
    pub maximum: Option<i64>,
    /// Most charged in a rolling day, tax included. For a credit unit wallet caps are money, in
    /// minor units of the customer's currency.
    pub daily_cap: Option<i64>,
    /// Most charged in a calendar month (UTC), tax included.
    pub monthly_cap: Option<i64>,
    /// Absent charges the customer's default payment method.
    #[serde(with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub payment_method_id: Option<CustomerPaymentMethodId>,
    /// The last charge that succeeded.
    pub last_succeeded_at: Option<ApiDateTime>,
    /// Set while SUSPENDED: since when.
    pub suspended_at: Option<ApiDateTime>,
    /// Why the last charge failed; may carry payment provider detail.
    pub last_error: Option<String>,
    /// When an operator attested that the customer agreed to automatic charges on exactly these
    /// terms. Absent when the customer confirmed them in their portal, or the terms changed since.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_consent_attested_at: Option<ApiDateTime>,
}

impl From<&domain::AutomaticTopUp> for AutoTopUp {
    fn from(value: &domain::AutomaticTopUp) -> Self {
        let config = &value.config;
        Self {
            status: if value.parked {
                AutoTopUpStatus::Disabled
            } else if config.suspended_at.is_some() {
                AutoTopUpStatus::Suspended
            } else {
                AutoTopUpStatus::Active
            },
            method: match config.method {
                domain::AutomaticTopUpMethod::Fixed => AutoTopUpMethod::Fixed,
                domain::AutomaticTopUpMethod::ToTarget => AutoTopUpMethod::ToTarget,
            },
            threshold: config.threshold_minor,
            amount: config.amount_minor,
            target: config.target_minor,
            minimum: config.minimum_minor,
            maximum: config.maximum_minor,
            daily_cap: config.daily_cap_minor,
            monthly_cap: config.monthly_cap_minor,
            payment_method_id: config.payment_method_id,
            last_succeeded_at: config.last_succeeded_at.map(Into::into),
            suspended_at: config.suspended_at.map(Into::into),
            last_error: config.last_error.clone(),
            customer_consent_attested_at: value
                .consent_attestation
                .as_ref()
                .map(|attestation| attestation.attested_at.into()),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct WalletListRequest {
    /// Only the credit this subscription (`sub_…`) can use: the credit that belongs to it and
    /// the customer's credit that belongs to no subscription. Every wallet is still listed.
    #[serde(default, with = "string_serde_opt")]
    pub subscription_id: Option<SubscriptionId>,
}

/// A customer's credit in one currency or credit unit. Amounts are minor units of it. Read with
/// `subscription_id`, every amount counts only the credit that subscription can use.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Wallet {
    /// Absent for a credit unit wallet.
    pub currency: Option<Currency>,
    /// The credit unit (its code) the wallet holds; absent for a currency.
    pub credit_unit: Option<String>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    pub credit_unit_precision: Option<u8>,
    /// The ledger total: credit granted and effective, not yet applied to an invoice or expired,
    /// whatever it applies to. Usage not invoiced yet is not deducted (see `balance`).
    pub credit_on_account: i64,
    /// The part of `credit_on_account` that pays only usage charges.
    pub credit_on_account_usage_only: i64,
    /// The part of `credit_on_account` that belongs to a subscription, which only that
    /// subscription's charges can use, per subscription. The rest pays any subscription.
    pub subscription_credit: Vec<SubscriptionCredit>,
    /// Credit granted that takes effect later.
    pub upcoming_credit: i64,
    /// Ascending, distinct, at most 5. `wallet.balance_low` fires once when `balance` falls below
    /// one; a drop below several at once fires once, for the lowest. All credit counts, including
    /// credit that belongs to a subscription or is limited to usage or products, which
    /// `wallet.depleted` leaves out: a wallet can be depleted without an alert firing.
    pub low_balance_thresholds: Vec<i64>,
    /// What this customer's usage does once the credit a subscription can use for it is used up
    /// (its own credit, credit that belongs to no subscription, and credit limited to the product:
    /// the `balance` of `GET /wallets/check`). `FOLLOW_PLAN` applies the "when credit runs out"
    /// rule of each subscription's plan.
    pub usage_at_zero: UsageAtZero,
    /// Usage stops as soon as the credit is used up, for at least one active subscription: by
    /// `usage_at_zero`, or by the plan of a subscription charging usage in this wallet.
    pub pauses_at_zero: bool,
    /// Automatic top-up of the credit that applies to ALL charges; absent when never set.
    pub auto_top_up: Option<AutoTopUp>,
    /// Credit unit wallets: the price this customer agreed for one credit, in major units of
    /// `unit_price_currency`, as a decimal string (e.g. "0.0008"). Purchases and automatic
    /// top-ups in that currency charge it instead of the list price, unless a purchase sets its
    /// own `unit_price`. Absent: the list price applies.
    #[serde(
        with = "rust_decimal::serde::str_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>)]
    pub unit_price: Option<rust_decimal::Decimal>,
    /// The currency of `unit_price`: the customer's currency when it was set. Absent with it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price_currency: Option<Currency>,
    /// What the customer has left: `credit_on_account` minus the part of `unbilled_usage` that
    /// credit will pay. Never negative: usage beyond the credit stays in `unbilled_usage`, to be
    /// invoiced. Low-balance alerts use it; automatic top-ups watch only its part that pays any
    /// charge (credit that belongs to no subscription and is not limited to usage or products).
    /// Absent, with `unbilled_usage` and `as_of`, until the customer's usage has been priced once.
    /// With `subscription_id`: the credit that subscription can spend once every subscription's
    /// unbilled usage is paid, minus its own unbilled usage no credit pays (negative then), as
    /// `GET /wallets/check` reads it. Credit restricted to products it is not charged for is
    /// left out. Credit that belongs to no subscription counts in the balance of every
    /// subscription that can use it (see `shared_balance`): never add subscriptions' balances up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<i64>,
    /// With `subscription_id` only: the part of `balance` that belongs to no subscription, which
    /// the customer's other subscriptions can spend too.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_balance: Option<i64>,
    /// Usage recorded and priced but not invoiced yet, tax included, in minor units: what the
    /// next invoices will charge for it, paid from credit as far as the credit goes. With
    /// `subscription_id`, that subscription's usage only; other subscriptions' usage paid from
    /// shared credit still lowers `balance`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unbilled_usage: Option<i64>,
    /// When `balance` and `unbilled_usage` were computed. They follow usage within a few seconds
    /// while the balance is near zero or a threshold, and within a minute otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<ApiDateTime>,
}

/// Credit on account that only one subscription can use.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SubscriptionCredit {
    #[serde(with = "common_domain::ids::string_serde")]
    #[schema(value_type = String)]
    pub subscription_id: SubscriptionId,
    /// In minor units of the wallet.
    pub credit_on_account: i64,
}

impl Wallet {
    pub fn apply_live_balance(&mut self, balance: &domain::LiveCreditBalance) {
        self.balance = Some(balance.projected_minor);
        self.unbilled_usage = Some(balance.estimated_unbilled_minor);
        self.as_of = Some(balance.as_of.into());
    }

    pub fn apply_subscription_balance(
        &mut self,
        balance_minor: i64,
        shared_minor: i64,
        unbilled_minor: i64,
        as_of: DateTime<Utc>,
    ) {
        self.balance = Some(balance_minor);
        self.shared_balance = Some(shared_minor);
        self.unbilled_usage = Some(unbilled_minor);
        self.as_of = Some(as_of.into());
    }
}

/// A customer's wallets, the customer's own currency first.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WalletListResponse {
    pub data: Vec<Wallet>,
}

/// An automatic top-up's terms, replaced as a whole. FIXED needs `amount` and no `target`;
/// TO_TARGET needs `target` above `threshold` and no `amount`; amounts are positive.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AutoTopUpWriteRequest {
    /// FIXED buys `amount` each time; TO_TARGET buys back up to `target`.
    pub method: AutoTopUpMethod,
    /// Tops up once the balance is at or below this, in minor units of the wallet.
    pub threshold: i64,
    /// FIXED only: credit bought each time, in minor units of the wallet.
    pub amount: Option<i64>,
    /// TO_TARGET only: the balance a top-up brings the wallet back to; must exceed `threshold`.
    pub target: Option<i64>,
    /// Smallest single charge. For a credit unit wallet it is money (the price before tax, in
    /// minor units of the customer's currency), like the caps; the threshold, amount and target
    /// are credits.
    pub minimum: Option<i64>,
    /// Largest single charge, in the same money as `minimum`.
    pub maximum: Option<i64>,
    /// Most charged in a rolling day, tax included. For a credit unit wallet caps are money, in
    /// minor units of the customer's currency.
    pub daily_cap: Option<i64>,
    /// Most charged in a calendar month (UTC), tax included.
    pub monthly_cap: Option<i64>,
    /// Defaults to the customer's default payment method: the customer's agreement, and your
    /// attestation of it, then cover whichever method is their default at each top-up.
    #[serde(default, with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub payment_method_id: Option<CustomerPaymentMethodId>,
    /// Credit unit wallets: authorize today's price per credit for these terms, which lifts a
    /// stop caused by a price increase. When false (the default) the terms keep the price they
    /// were authorized at, and a rule stopped by a price increase stays stopped.
    /// A price at or below the authorized one never stops the rule: top-ups charge it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub at_current_price: bool,
    /// Attests that the customer agreed to automatic charges on these terms; recorded with the
    /// API key and the time. Required (true) when the terms turn automatic top-up on, including
    /// one the customer turned off, or change what it charges. Not needed to resend the stored
    /// terms unchanged, nor to change terms the customer confirmed in their portal: those stop
    /// until the customer confirms the new ones.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub customer_consent_attested: bool,
}

impl From<AutoTopUpWriteRequest> for domain::AutomaticTopUpConfig {
    fn from(value: AutoTopUpWriteRequest) -> Self {
        Self {
            minimum_minor: value.minimum,
            maximum_minor: value.maximum,
            daily_cap_minor: value.daily_cap,
            monthly_cap_minor: value.monthly_cap,
            ..domain::AutomaticTopUpConfig::new(
                value.method.into(),
                value.threshold,
                value.amount,
                value.target,
                value.payment_method_id,
            )
        }
    }
}

/// Mirrors the store's threshold rule, but strictly: the store silently sorts and dedupes an
/// operator's input, while this endpoint rejects input that is not already ascending and
/// distinct, so a caller never silently gets a different list back than it sent.
fn validate_low_balance_thresholds(thresholds: &[i64]) -> Result<(), validator::ValidationError> {
    if thresholds.iter().any(|threshold| *threshold < 0) {
        let mut err = validator::ValidationError::new("negative_threshold");
        err.message = Some("a low-balance threshold must not be negative".into());
        return Err(err);
    }
    if thresholds.len() > 5 {
        let mut err = validator::ValidationError::new("too_many_thresholds");
        err.message = Some("a wallet may have at most 5 balance alert thresholds".into());
        return Err(err);
    }
    if !thresholds.windows(2).all(|pair| pair[0] < pair[1]) {
        let mut err = validator::ValidationError::new("unsorted_thresholds");
        err.message = Some("balance alert thresholds must be ascending with no duplicates".into());
        return Err(err);
    }
    Ok(())
}

/// Partial update of a wallet's configuration: omit a field to leave it unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchWalletRequest {
    /// See `Wallet.usage_at_zero`. Omit to leave unchanged.
    pub usage_at_zero: Option<UsageAtZero>,
    /// Ascending, distinct, at most 5, non-negative. Omit to leave unchanged; an empty list
    /// clears every threshold.
    #[validate(custom(function = "validate_low_balance_thresholds"))]
    pub low_balance_thresholds: Option<Vec<i64>>,
    /// Omit to leave the automatic top-up unchanged; `null` turns it off and removes its terms and
    /// the customer's confirmation; an object replaces the terms. Sending the stored terms
    /// unchanged changes nothing: a stopped rule stays stopped. Terms the customer confirmed in
    /// their portal and that this request changes stop until the customer confirms them again.
    /// Terms you created run as sent, and changing them lifts a stop caused by failed or reversed
    /// payments; otherwise such a stop lifts when the customer confirms the terms in their portal.
    /// Terms sent after a `null` are yours: an earlier confirmation does not carry over. Terms
    /// that turn automatic top-up on, or change what it charges, without the customer's
    /// confirmation need `customer_consent_attested`.
    // `skip_serializing_if` keeps an omitted field out of the idempotency fingerprint, so an
    // omitted `auto_top_up` and an explicit `null` hash differently.
    #[serde(
        default,
        with = "::serde_with::rust::double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<AutoTopUpWriteRequest>)]
    pub auto_top_up: Option<Option<AutoTopUpWriteRequest>>,
    /// Credit unit wallets: the price this customer agreed for one credit, in major units of the
    /// customer's currency, as a decimal string (positive, at most 12 decimal places). Purchases
    /// and automatic top-ups default to it instead of the list price. Omit to leave it unchanged;
    /// `null` returns the customer to the list price. A change applies to purchases quoted after
    /// it. An automatic top-up keeps running at the new price when it is at or below the price
    /// its terms were authorized at, and stops until they are confirmed again when it is above.
    #[serde(
        default,
        with = "optional_decimal_patch",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, nullable)]
    pub unit_price: Option<Option<rust_decimal::Decimal>>,
}

/// A patch field holding a decimal string: absent, `null`, or a value. Numbers are refused, so a
/// price never goes through a float.
mod optional_decimal_patch {
    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        value: &Option<Option<Decimal>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            // Normalized, so "0.0008" and "0.00080" are the same request.
            Some(Some(decimal)) => serializer.serialize_str(&decimal.normalize().to_string()),
            _ => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Option<Decimal>>, D::Error> {
        match Option::<String>::deserialize(deserializer)? {
            None => Ok(Some(None)),
            Some(text) => text
                .trim()
                .parse::<Decimal>()
                .map(|decimal| Some(Some(decimal)))
                .map_err(|_| serde::de::Error::custom("unit_price must be a decimal string")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LedgerEntryType {
    /// Credit granted.
    Credit,
    /// Credit applied to a finalized invoice.
    InvoiceApplication,
    /// Credit an invoice gave back: voided, or returned by a credit note.
    InvoiceReturn,
    /// Credit removed by a credit note, by hand, or by a reversed payment (chargeback).
    CreditReversal,
    /// Credit a reversed payment removed, restored once its money was recovered.
    CreditReinstatement,
    /// Credit that lapsed unspent.
    CreditExpiry,
    /// Unspent credit carried out of an expiring grant into the grant it rolled over to.
    CreditRollover,
}

impl From<domain::LedgerEntryKind> for LedgerEntryType {
    fn from(value: domain::LedgerEntryKind) -> Self {
        match value {
            domain::LedgerEntryKind::Fund => Self::Credit,
            domain::LedgerEntryKind::Draw => Self::InvoiceApplication,
            domain::LedgerEntryKind::DrawReversal => Self::InvoiceReturn,
            domain::LedgerEntryKind::Reversal => Self::CreditReversal,
            domain::LedgerEntryKind::Reinstatement => Self::CreditReinstatement,
            domain::LedgerEntryKind::Expiry => Self::CreditExpiry,
            domain::LedgerEntryKind::Rollover => Self::CreditRollover,
        }
    }
}

/// One movement of a customer's credit.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LedgerEntry {
    /// The entry's id (`wle_…`).
    #[serde(with = "common_domain::ids::string_serde")]
    #[schema(value_type = String)]
    pub id: LedgerEntryId,
    /// The grant the credit moved in or out of.
    #[serde(with = "common_domain::ids::string_serde")]
    #[schema(value_type = String)]
    pub credit_grant_id: CreditGrantId,
    /// Position within the currency's ledger in the order the movements took effect, from 1. A
    /// movement recorded later with an earlier date shifts the positions after it: identify an
    /// entry by its `id`.
    pub sequence: i64,
    /// What moved the credit.
    #[serde(rename = "type")]
    pub entry_type: LedgerEntryType,
    /// Absent in a credit unit ledger.
    pub currency: Option<Currency>,
    /// The credit unit (its code) the amounts are in; absent for a currency.
    pub credit_unit: Option<String>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    pub credit_unit_precision: Option<u8>,
    /// What the moved credit's grant pays: ALL any charge, USAGE only usage charges.
    pub applies_to: CreditAppliesTo,
    /// The subscription the moved credit's grant belongs to; null when it pays any of the
    /// customer's subscriptions.
    #[serde(with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub subscription_id: Option<SubscriptionId>,
    /// Signed change to the credit on account, in minor units.
    pub amount: i64,
    /// The credit on account after this entry, whatever it applies to.
    pub balance_after: i64,
    /// The invoice the credit was applied to, bought on, or given back by.
    #[serde(with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub invoice_id: Option<InvoiceId>,
    /// The credit note that gave the credit back or took it back.
    #[serde(with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub credit_note_id: Option<CreditNoteId>,
    /// Why the credit moved, as shown on the ledger.
    pub reason: String,
    /// Your own reference, e.g. a contract or support ticket.
    pub reference: Option<String>,
    /// When the movement took effect; the ledger is ordered by it.
    pub effective_at: ApiDateTime,
    /// When the movement was recorded.
    pub recorded_at: ApiDateTime,
    /// The share of its grant's cost this entry moved, signed like `amount`: for credit applied
    /// to an invoice it is recognized revenue, for expired credit breakage, for a reversal a
    /// refund. Absent when the grant has no cost.
    pub cost: Option<CreditCost>,
}

impl LedgerEntry {
    pub fn new(
        entry: &domain::CreditLedgerEntry,
        denomination: &Denomination,
        application: domain::CreditApplication,
    ) -> Result<Self, RestApiError> {
        Ok(Self {
            id: entry.id,
            credit_grant_id: entry.grant_id,
            sequence: entry.sequence,
            entry_type: entry.kind.into(),
            currency: denomination.currency.clone(),
            credit_unit: denomination.credit_unit.clone(),
            credit_unit_precision: denomination.credit_unit_precision,
            applies_to: application.into(),
            subscription_id: entry.subscription_id,
            amount: entry.amount_minor,
            balance_after: entry.balance_after_minor,
            invoice_id: entry.invoice_id,
            credit_note_id: entry.credit_note_id,
            reason: entry.reason.clone(),
            reference: entry.reference.clone(),
            effective_at: entry.effective_at.into(),
            recorded_at: entry.recorded_at.into(),
            cost: entry.cost.as_ref().map(CreditCost::new).transpose()?,
        })
    }
}

/// A page of ledger entries, newest first.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LedgerListResponse {
    pub data: Vec<LedgerEntry>,
    pub pagination_meta: PaginationResponse,
}

/// Credit granted without charging or invoicing.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCreditGrantRequest {
    /// Credit granted, in minor units of `currency`, or of `credit_unit` when set (e.g. 500000
    /// is 5,000.00 credits of a unit with precision 2). Positive.
    pub amount: i64,
    /// Defaults to the customer's currency.
    pub currency: Option<Currency>,
    /// Grants in this credit unit (its code) instead of a currency; excludes `currency`. Credit in
    /// a credit unit pays the usage priced in that unit, so `applies_to` must stay ALL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<String>,
    /// What the customer paid for the credit, net of tax. Required for PAID credit in a credit
    /// unit; defaults to zero for PROMOTIONAL credit and to `amount` for PAID currency credit.
    /// PAID currency credit cannot cost more than `amount`. When it costs less, the cost is
    /// granted as PAID credit and the rest as a separate PROMOTIONAL bonus grant with the same
    /// expiry, which applies before tax; PAID credit that cost nothing is granted as PROMOTIONAL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<CreditCostRequest>,
    /// ALL pays any charge on an invoice; USAGE only usage charges.
    #[serde(default)]
    pub applies_to: CreditAppliesTo,
    /// The customer's subscription (`sub_…`) the credit belongs to: it pays only that
    /// subscription's charges and lapses when the subscription ends. It must not have ended,
    /// and its currency must be the grant's. Absent pays any of the customer's subscriptions.
    #[serde(
        default,
        with = "string_serde_opt",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>)]
    pub subscription_id: Option<SubscriptionId>,
    /// Restricts the credit to charges of these products; absent applies to every product.
    /// Combined with `applies_to`. Not available for credit units.
    #[serde(
        default,
        with = "string_serde_vec_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub applicable_product_ids: Option<Vec<ProductId>>,
    /// PROMOTIONAL gives credit for free (a goodwill or marketing grant). PAID records credit
    /// the customer paid for outside of Meteroid: nothing is charged or invoiced, and the grant
    /// is shown as "Paid outside Meteroid". Usage paid with PAID currency credit is taxed on the
    /// invoice that bills it, so do not charge tax on the external sale of currency credit, or
    /// the customer is taxed twice. Credit in a credit unit is the exception: its usage is billed
    /// in credits, untaxed, so tax its sale as you would any other. Use a top-up to sell credit
    /// through Meteroid.
    #[serde(rename = "type")]
    pub grant_type: CreditGrantType,
    /// Lower values are drawn first among grants of the same type that pay the same charges;
    /// defaults to 100.
    pub priority: Option<i32>,
    /// When unspent credit lapses; absent never expires.
    pub expires_at: Option<ApiDateTime>,
    /// Shown on the credit ledger.
    pub reason: Option<String>,
    /// Your own reference, e.g. a contract or support ticket. Required for PAID credit that cost
    /// more than zero when the customer's invoicing entity is in France: the number of the invoice
    /// or receipt for the payment, as French law requires a supporting document for every
    /// accounting entry. Refused with 400 when missing.
    pub reference: Option<String>,
}

/// How a top-up is paid: CHARGE a saved payment method now, or SEND_INVOICE and grant the credit
/// once the invoice is paid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TopUpCollection {
    #[default]
    Charge,
    SendInvoice,
}

/// Credit sold to the customer, charged or invoiced.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateTopUpRequest {
    /// Absolute expiry of the purchased credit; unset never expires.
    pub expires_at: Option<ApiDateTime>,
    /// Credit to buy, positive, applying to ALL charges. In minor units of `currency`, charged
    /// one for one without tax: the usage it pays is taxed on the invoice that bills it. With
    /// `credit_unit`, in minor units of that unit (e.g. 500000 is 5,000.00 credits of a unit with
    /// precision 2), charged at the unit's price per credit in the customer's currency, plus tax
    /// under the unit's product.
    pub amount: i64,
    /// Defaults to the customer's currency. SEND_INVOICE sells only in the customer's currency.
    pub currency: Option<Currency>,
    /// CHARGE (default) charges a saved payment method now; SEND_INVOICE issues an invoice and
    /// grants the credit once it is paid.
    #[serde(default)]
    pub collection: TopUpCollection,
    /// CHARGE only; defaults to the customer's default payment method.
    #[serde(default, with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub payment_method_id: Option<CustomerPaymentMethodId>,
    /// Buy credits of this credit unit (its code) instead of money; excludes `currency`. They are
    /// charged in the customer's currency at the customer's agreed price for the unit (the
    /// wallet's `unit_price`), else its list price, plus tax, under the unit's product.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<String>,
    /// With `credit_unit`: the negotiated price of one credit for this purchase, in major units of
    /// the customer's currency, as a decimal string (e.g. "0.0008"). Defaults to the wallet's
    /// `unit_price` when set, else the list price; the unit must still have a list price in that
    /// currency.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::str_option"
    )]
    #[schema(value_type = Option<String>)]
    pub unit_price: Option<rust_decimal::Decimal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TopUpStatus {
    /// Paid; the credit is granted.
    Succeeded,
    /// The charge is settling; the credit is granted when it settles.
    Pending,
    /// The customer authenticates the charge at `checkout_url`.
    RequiresCustomerAction,
    /// The invoice is issued; the credit is granted when it is paid.
    AwaitingPayment,
}

/// Where a top-up stands. Its credit is granted once it is paid; `wallet.funded` reports it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TopUpResponse {
    /// SUCCEEDED once paid; otherwise what the payment is waiting for.
    pub status: TopUpStatus,
    /// Credit bought, in minor units of `currency`, or of `credit_unit` when set.
    pub amount: i64,
    /// Charged to the payment method, tax included, in minor units of `currency`; absent until a
    /// charge is made.
    pub charged_amount: Option<i64>,
    /// The money the top-up is charged in.
    pub currency: Currency,
    /// Set when credits of this credit unit were bought: `amount` is in its minor units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<String>,
    /// The invoice documenting the purchase.
    #[serde(with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub invoice_id: Option<InvoiceId>,
    /// The charge made on the payment method; absent until a charge is made.
    #[serde(with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub payment_transaction_id: Option<PaymentTransactionId>,
    /// Hosted page where the customer completes the payment; present while it awaits them.
    pub checkout_url: Option<String>,
}

#[cfg(test)]
mod cost_tests {
    use super::*;
    use common_domain::ids::{BaseId, WalletId};

    fn grant(amount_minor: i64, cost: Option<domain::CreditCost>) -> domain::CreditGrant {
        let at = common_domain::clock::now();
        domain::CreditGrant {
            id: CreditGrantId::new(),
            wallet_id: WalletId::new(),
            amount_minor,
            remaining_minor: amount_minor,
            expired_minor: None,
            classification: domain::GrantClassification::Paid,
            priority: 100,
            source: domain::CreditGrantSource::Purchase,
            source_id: uuid::Uuid::nil(),
            invoice_id: None,
            credit_note_id: None,
            effective_at: at,
            expires_at: None,
            state: domain::CreditGrantState::Active,
            reason: "pack".to_owned(),
            reference: None,
            created_by: None,
            created_at: at,
            cost,
            scope: domain::GrantScope::customer_wide(),
            ends_at: None,
            end_reason: None,
            rollover: None,
        }
    }

    #[test]
    fn a_grant_shows_its_cost_and_the_price_of_one_unit() {
        let shown = CreditGrant::new(
            &grant(1_000, Some(domain::CreditCost::at_face(800, "EUR"))),
            &Denomination::of_currency(Currency::EUR),
            &GrantPlans::new(),
        )
        .unwrap();
        let cost = shown.cost.unwrap();
        assert_eq!(
            (cost.amount, cost.currency.to_string()),
            (800, "EUR".to_owned())
        );
        assert_eq!(shown.cost_basis.as_deref(), Some("0.8"));

        let unknown = CreditGrant::new(
            &grant(1_000, None),
            &Denomination::of_currency(Currency::EUR),
            &GrantPlans::new(),
        )
        .unwrap();
        assert!(unknown.cost.is_none() && unknown.cost_basis.is_none());
    }

    #[test]
    fn a_wallet_reports_credit_on_account_unbilled_usage_and_balance() {
        let mut wallet =
            super::super::router::empty_wallet(Denomination::of_currency(Currency::EUR));
        wallet.credit_on_account = 5_000;
        let unpriced = serde_json::to_value(&wallet).unwrap();
        assert_eq!(unpriced["credit_on_account"], 5_000);
        assert!(unpriced.get("balance").is_none() && unpriced.get("unbilled_usage").is_none());

        wallet.apply_live_balance(&domain::LiveCreditBalance {
            currency: "EUR".to_owned(),
            projected_minor: 3_800,
            estimated_unbilled_minor: 1_200,
            as_of: common_domain::clock::now(),
            projected: true,
        });
        let shown = serde_json::to_value(&wallet).unwrap();
        assert_eq!(shown["balance"], 3_800);
        assert_eq!(shown["unbilled_usage"], 1_200);
        for jargon in [
            "available",
            "available_usage_only",
            "upcoming_grants",
            "live_balance",
            "pending_usage",
        ] {
            assert!(shown.get(jargon).is_none(), "{jargon}");
        }
    }

    #[test]
    fn a_unit_grant_states_its_unit_and_prices_one_credit_at_its_precision() {
        // 5,000.000 credits (precision 3) bought for 40.00 EUR: 0.008 EUR per credit.
        let shown = CreditGrant::new(
            &grant(5_000_000, Some(domain::CreditCost::at_face(4_000, "EUR"))),
            &Denomination::of_unit("CREDITS", 3),
            &GrantPlans::new(),
        )
        .unwrap();
        assert!(shown.currency.is_none());
        assert_eq!(shown.credit_unit.as_deref(), Some("CREDITS"));
        assert_eq!(shown.credit_unit_precision, Some(3));
        assert_eq!(shown.cost_basis.as_deref(), Some("0.008"));
    }
}
