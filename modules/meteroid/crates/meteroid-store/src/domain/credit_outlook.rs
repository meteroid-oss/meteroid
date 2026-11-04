//! What a subscription's credit is expected to pay, and the kinds of credit a wallet holds, as the
//! admin and the portal show them. Kinds share a denomination but not their expiry or what they
//! pay for, so a wallet's total is never shown alone.
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{CreditGrantId, InvoiceId, PlanVersionId, SubscriptionId, WalletId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CreditGroupKind {
    /// Plan credit of one subscription.
    Included,
    /// Bought: purchases, top-ups and paid packs.
    Prepaid,
    /// Given: the bonus of a purchase, promotion codes and other promotional credit.
    Bonus,
    /// The customer's money that pays any charge: refunds to credit, the carried balance.
    AccountBalance,
}

/// The kind a grant is shown as, with its labels.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CreditGroup {
    pub kind: CreditGroupKind,
    /// "Included with Scale", "Purchased credit", "Free credit" or "Account balance".
    pub label: String,
    /// The subscription it belongs to; None when it pays any of the customer's.
    pub subscription_id: Option<SubscriptionId>,
    /// "Any charge", "Usage only", "This subscription's usage" or "This subscription's charges",
    /// with " · selected products" when restricted to products.
    pub pays_for: String,
}

/// One grant's part of what a document's credit pays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditDeduction {
    pub grant_id: CreditGrantId,
    pub denomination: String,
    pub group: CreditGroup,
    /// Taken off the charges before tax (promotional credit); otherwise paid on the taxed total.
    pub before_tax: bool,
    pub amount_minor: i64,
}

/// A document billed before the previewed one that draws on the same customer-wide credit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BilledFirst {
    /// Set for an invoice already drafted.
    pub invoice_id: Option<InvoiceId>,
    pub subscription_id: Option<SubscriptionId>,
    pub plan_name: Option<String>,
    pub invoice_date: NaiveDate,
}

/// What the next invoice does beyond renewing the period.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpcomingChange {
    /// The subscription is cancelled or reaches its end date: the final invoice. Not `billed` when
    /// it never activated (awaiting checkout): it ends without one.
    Ends { on: NaiveDate, billed: bool },
    /// The subscription is paused: the last invoice before the pause, `billed` as for an ending.
    Pauses { on: NaiveDate, billed: bool },
    /// A plan change takes effect on the invoice date: advance charges are the new plan's.
    PlanChange { plan_version_id: PlanVersionId },
    /// A scheduled amendment takes effect on the invoice date.
    Amendment,
}

/// A subscription's next invoice with the credit it draws once every document billed before it
/// drew its own.
#[derive(Debug, Clone)]
pub struct UpcomingInvoiceForecast {
    pub content: ComputedInvoiceContent,
    pub invoice_date: NaiveDate,
    pub change: Option<UpcomingChange>,
    /// The credit drawn is shared with other pending documents, so it is an estimate ("≈").
    pub credit_estimate_shared: bool,
    pub billed_first: Vec<BilledFirst>,
    pub credit: Vec<CreditDeduction>,
}

/// What credit of one kind a subscription can still use, once unbilled usage is paid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditGroupBalance {
    pub group: CreditGroup,
    pub amount_minor: i64,
    /// The soonest lapse among its grants.
    pub lapses_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditOutlook {
    pub wallet_id: WalletId,
    pub denomination: String,
    pub subscription_id: SubscriptionId,
    pub plan_name: String,
    /// The subscription's current period end.
    pub horizon: NaiveDate,
    pub as_of: DateTime<Utc>,
    pub stale: bool,
    /// Usable by the subscription, after all unbilled usage.
    pub balance_minor: i64,
    /// Of which belongs to the subscription.
    pub included_minor: i64,
    /// Of which any of the customer's subscriptions may use.
    pub shared_minor: i64,
    /// The other live subscriptions drawing on that shared credit.
    pub shared_with: Vec<SubscriptionId>,
    /// The subscription's usage to the horizon at its current rate; None in the period's first
    /// hour.
    pub projected_usage_minor: Option<i64>,
    pub groups: Vec<CreditGroupBalance>,
    pub verdict: OutlookVerdict,
    /// Some of the documents billed before the horizon were left out (past a bound, or not
    /// computable): the verdict leaves out what they charge, so it is shown as an estimate.
    pub partial: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutlookVerdict {
    Stale,
    UsagePaused {
        since: DateTime<Utc>,
    },
    TopUpFailing {
        last_error: String,
    },
    BeyondCredit {
        uncovered_minor: i64,
    },
    RunsOut {
        on: NaiveDate,
        pauses: bool,
    },
    ProjectedBeyond {
        amount_minor: i64,
        by: NaiveDate,
    },
    AutomaticTopUp {
        target_or_amount_minor: i64,
        threshold_minor: i64,
    },
    Covered {
        through: NaiveDate,
        projected_minor: i64,
    },
}
