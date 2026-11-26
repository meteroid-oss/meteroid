//! An invoice's collection as it stands: the payments attempted, the retries the dunning policy
//! still plans, and what follows once they run out.
use chrono::{DateTime, Utc};
use common_domain::ids::PaymentTransactionId;

use crate::domain::enums::DeclineKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcome {
    Succeeded,
    Failed,
    Pending,
    Cancelled,
    Refunded,
}

#[derive(Debug, Clone)]
pub struct CollectionAttempt {
    pub id: PaymentTransactionId,
    pub at: DateTime<Utc>,
    pub outcome: AttemptOutcome,
    /// The customer paid from the portal: it spends no retry.
    pub by_customer: bool,
    pub decline: Option<DeclineKind>,
    /// "Visa •••• 4242", or what the method is.
    pub method: Option<String>,
    pub amount: i64,
    pub currency: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionEnd {
    /// The invoice stays due, for the customer or a person to collect.
    StaysDue,
    CancelsSubscription,
}

#[derive(Debug, Clone, Default)]
pub struct InvoiceCollection {
    /// Oldest first.
    pub attempts: Vec<CollectionAttempt>,
    /// The retries still planned, the first the scheduled one; each runs if the one before fails.
    pub retries: Vec<DateTime<Utc>>,
    /// A hard decline stopped the retries on the card; a new card starts them again.
    pub stopped_by: Option<DeclineKind>,
    /// What follows once the retries are spent, and when; `None` outside dunning.
    pub end: Option<(CollectionEnd, Option<DateTime<Utc>>)>,
}
