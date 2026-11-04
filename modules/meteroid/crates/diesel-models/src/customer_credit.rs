//! Invoice-time customer credit: per-currency accounts, grants and their append-only entries.
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use common_domain::ids::{
    CreditGrantId, CreditNoteId, CreditOperationId, CreditUnitId, CustomerId, InvoiceId,
    LedgerEntryId, PaymentTransactionId, ProductId, SubscriptionId, TenantId, UsageSettlementId,
    WalletId,
};
use diesel::{Identifiable, Insertable, Queryable, QueryableByName, Selectable};
use rust_decimal::Decimal;
use uuid::Uuid;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::customer_credit_account)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerCreditAccountRow {
    pub id: WalletId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub currency: String,
    pub created_at: DateTime<Utc>,
    /// Ascending, distinct, at most 5. Empty means no balance alert is configured.
    #[diesel(deserialize_as = crate::extend::non_null_array::NonNullArray<i64>)]
    pub low_balance_thresholds_minor: Vec<i64>,
    /// Set iff `currency` is a credit unit code rather than an ISO currency.
    pub credit_unit_id: Option<CreditUnitId>,
    /// The customer's agreed price per credit, in major units of `unit_price_currency`.
    pub unit_price: Option<Decimal>,
    pub unit_price_currency: Option<String>,
    /// Bumped on every wallet of the customer by each ledger movement.
    pub ledger_version: i64,
    /// Overrides, for this customer, what the plans say usage does once the credit runs out.
    pub usage_at_zero: crate::enums::UsageAtZeroEnum,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::customer_credit_account)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerCreditAccountRowNew {
    pub id: WalletId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub currency: String,
    pub credit_unit_id: Option<CreditUnitId>,
}

/// A product a grant pays for; a grant without any pays for every product.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::customer_credit_grant_product)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerCreditGrantProductRow {
    pub tenant_id: TenantId,
    pub grant_id: CreditGrantId,
    pub product_id: ProductId,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, diesel::QueryableByName)]
#[diesel(table_name = crate::schema::customer_credit_grant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerCreditGrantRow {
    pub id: CreditGrantId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub account_id: WalletId,
    pub amount: i64,
    pub remaining: i64,
    pub priority: i32,
    pub classification: String,
    pub effective_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub source_kind: String,
    pub source_id: Uuid,
    pub invoice_id: Option<InvoiceId>,
    pub line_local_id: Option<String>,
    pub credit_note_id: Option<CreditNoteId>,
    pub reason: String,
    pub reference: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    /// Money paid for `amount`, net of tax, in `cost_currency`.
    pub cost_minor: Option<i64>,
    pub cost_currency: Option<String>,
    pub rollover: Option<serde_json::Value>,
    /// None pays any of the customer's subscriptions.
    pub subscription_id: Option<SubscriptionId>,
    /// USAGE pays usage charges only; INVOICE any charge.
    pub application: String,
    /// Set once, when the subscription it belongs to ended.
    pub ends_at: Option<DateTime<Utc>>,
    pub end_reason: Option<String>,
    /// The earlier of `expires_at` and `ends_at`: when its value lapses.
    pub lapses_at: Option<DateTime<Utc>>,
    /// The start of the usage it may pay when not `effective_at` (`usage_start` reads both).
    pub pays_usage_from: Option<DateTime<Utc>>,
}

/// Sources holding value main kept (the balance carried over, credit a void of a pre-release
/// invoice restored): they pay usage of any date, as main applied them to any invoice.
pub const UNDATED_SOURCES: [&str; 2] = ["CUSTOMER_BALANCE", "INVOICE_VOID_RESTORE"];

impl CustomerCreditGrantRow {
    /// The start of the usage it may pay: usage whose line ended by then it never pays.
    pub fn usage_start(&self) -> DateTime<Utc> {
        if UNDATED_SOURCES.contains(&self.source_kind.as_str()) {
            return DateTime::<Utc>::MIN_UTC;
        }
        self.pays_usage_from.unwrap_or(self.effective_at)
    }
}

/// A grant with the products it pays for, sorted; none pays for every product.
#[derive(Debug, Clone, diesel::QueryableByName)]
pub struct ScopedGrantRow {
    #[diesel(embed)]
    pub grant: CustomerCreditGrantRow,
    #[diesel(sql_type = diesel::sql_types::Array<diesel::sql_types::Uuid>)]
    pub product_ids: Vec<ProductId>,
}

/// Which grants a draw may lock: every one, or the customer-wide ones and those of the listed
/// subscriptions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionFilter {
    Any,
    CustomerWideOr(Vec<SubscriptionId>),
}

/// Grant listing filter by the subscription a grant belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantSubscriptionFilter {
    /// Grants that belong to it.
    Of(SubscriptionId),
    /// Grants that belong to it, and customer-wide ones.
    UsableBy(SubscriptionId),
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::customer_credit_grant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerCreditGrantRowNew {
    pub id: CreditGrantId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub account_id: WalletId,
    pub amount: i64,
    pub remaining: i64,
    pub priority: i32,
    pub classification: String,
    pub effective_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub source_kind: String,
    pub source_id: Uuid,
    pub invoice_id: Option<InvoiceId>,
    pub line_local_id: Option<String>,
    pub credit_note_id: Option<CreditNoteId>,
    pub reason: String,
    pub reference: Option<String>,
    pub created_by: Option<Uuid>,
    pub cost_minor: Option<i64>,
    pub cost_currency: Option<String>,
    pub rollover: Option<serde_json::Value>,
    pub subscription_id: Option<SubscriptionId>,
    pub application: String,
    pub ends_at: Option<DateTime<Utc>>,
    pub end_reason: Option<String>,
    pub pays_usage_from: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::customer_credit_entry)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerCreditEntryRow {
    pub id: LedgerEntryId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub grant_id: CreditGrantId,
    pub kind: String,
    pub amount: i64,
    pub source_id: Uuid,
    pub source_kind: Option<String>,
    pub invoice_id: Option<InvoiceId>,
    pub credit_note_id: Option<CreditNoteId>,
    pub created_at: DateTime<Utc>,
    pub occurred_at: DateTime<Utc>,
    pub reason: Option<String>,
    pub created_by: Option<Uuid>,
    /// Money value the movement carried, same sign as `amount`; None iff the grant has no cost.
    pub cost_minor: Option<i64>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::customer_credit_entry)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerCreditEntryRowNew {
    pub id: LedgerEntryId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub grant_id: CreditGrantId,
    pub kind: String,
    pub amount: i64,
    pub source_id: Uuid,
    pub source_kind: Option<String>,
    pub invoice_id: Option<InvoiceId>,
    pub credit_note_id: Option<CreditNoteId>,
    pub occurred_at: DateTime<Utc>,
    pub reason: Option<String>,
    pub created_by: Option<Uuid>,
    pub cost_minor: Option<i64>,
    /// The payment reversal or reinstatement this movement follows.
    pub payment_movement_id: Option<Uuid>,
}

/// What one grant paid of one invoice line (None: of no line), when the invoice drew it or a
/// credit note (`credit_note_id`) gave it back.
#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::customer_credit_line_allocation)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditLineAllocationRowNew {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub invoice_id: InvoiceId,
    pub credit_note_id: Option<CreditNoteId>,
    pub grant_id: CreditGrantId,
    pub line_local_id: Option<String>,
    pub amount: i64,
}

/// A credit purchase awaiting payment of its invoice; the grant it funds cites this row.
#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::customer_credit_purchase)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditPurchaseRow {
    pub expires_at: Option<DateTime<Utc>>,
    pub id: Uuid,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub amount_cents: i64,
    pub note: Option<String>,
    pub invoice_id: InvoiceId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub created_by: Option<Uuid>,
    /// A credit unit sale: the credits sold; `amount_cents` is then their price before tax.
    pub amount_credits: Option<i64>,
    /// A credit unit sale at an operator's negotiated price per credit.
    pub negotiated_unit_price: Option<Decimal>,
    /// A credit unit sale: `LIST`, `CUSTOMER` (the wallet's agreed price) or `OVERRIDE`.
    pub unit_price_source: Option<String>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::customer_credit_purchase)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditPurchaseRowNew {
    pub expires_at: Option<DateTime<Utc>>,
    pub id: Uuid,
    /// None records the purchase now.
    pub created_at: Option<NaiveDateTime>,
    pub amount_cents: i64,
    pub note: Option<String>,
    pub invoice_id: InvoiceId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub created_by: Option<Uuid>,
    pub amount_credits: Option<i64>,
    pub negotiated_unit_price: Option<Decimal>,
    pub unit_price_source: Option<String>,
}

/// A deduction the retired scalar balance recorded for an invoice, in the currency it was taken in.
/// The loss the unrecovered-credit notices of one wallet have reported on one money flow so far.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::customer_credit_unrecovered)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditUnrecoveredRow {
    pub tenant_id: TenantId,
    pub account_id: WalletId,
    pub invoice_id: Option<InvoiceId>,
    pub payment_id: Option<PaymentTransactionId>,
    pub source_id: Uuid,
    pub reported: i64,
    pub notices: i32,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::customer_balance_tx)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct HistoricalBalanceTxRow {
    pub id: Uuid,
    pub amount_cents: i64,
    pub currency: String,
}

/// One line of an account statement with its running balance. Credit that lapsed unspent and is
/// not yet recorded appears as an `EXPIRY` line at its expiry date.
#[derive(Debug, Clone, diesel::QueryableByName)]
pub struct CustomerCreditStatementRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: LedgerEntryId,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub account_id: WalletId,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub grant_id: CreditGrantId,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub kind: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub amount: i64,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
    pub invoice_id: Option<InvoiceId>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
    pub credit_note_id: Option<CreditNoteId>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub grant_reason: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub entry_reason: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub grant_reference: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
    pub created_by: Option<Uuid>,
    #[diesel(sql_type = diesel::sql_types::Timestamptz)]
    pub occurred_at: DateTime<Utc>,
    #[diesel(sql_type = diesel::sql_types::Timestamptz)]
    pub recorded_at: DateTime<Utc>,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub sequence: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub balance_after: i64,
    /// Signed like `amount`; None when the grant has no cost.
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::BigInt>)]
    pub cost_minor: Option<i64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub cost_currency: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub grant_application: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
    pub grant_subscription_id: Option<SubscriptionId>,
}

/// Lifecycle state of a grant at a given instant, as grant listings filter it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomerCreditGrantStateFilter {
    Pending,
    Active,
    Depleted,
    Expired,
    /// Depleted or expired.
    UsedUp,
}

/// A top-up purchase: the anchor that pins one payment and one purchase document per attempt.
#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::customer_credit_operation)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerCreditOperationRow {
    pub id: CreditOperationId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub account_id: WalletId,
    pub state: String,
    pub namespace: String,
    pub idempotency_key: String,
    pub request_fingerprint: Vec<u8>,
    pub amount_minor: i64,
    pub expected_consideration_minor: Option<i64>,
    pub consideration_currency: Option<String>,
    pub payment_transaction_id: Option<PaymentTransactionId>,
    pub invoice_id: Option<InvoiceId>,
    pub reason: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub settled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::customer_credit_operation)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerCreditOperationRowNew {
    pub id: CreditOperationId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub account_id: WalletId,
    pub state: String,
    pub namespace: String,
    pub idempotency_key: String,
    pub request_fingerprint: Vec<u8>,
    pub amount_minor: i64,
    pub expected_consideration_minor: Option<i64>,
    pub consideration_currency: Option<String>,
    pub reason: String,
    pub metadata: serde_json::Value,
}

/// One wallet of the tenant-wide roster, with its customer and live balance.
#[derive(Debug, Clone, diesel::QueryableByName)]
pub struct CustomerCreditRosterRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: WalletId,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub customer_id: CustomerId,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub customer_name: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub customer_alias: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub currency: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
    pub credit_unit_id: Option<CreditUnitId>,
    #[diesel(sql_type = diesel::sql_types::Array<diesel::sql_types::BigInt>)]
    pub low_balance_thresholds_minor: Vec<i64>,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub available_minor: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub pending_minor: i64,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub auto_top_up_enabled: bool,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub auto_top_up_suspended: bool,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub auto_top_up_failing: bool,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::BigInt>)]
    pub auto_top_up_threshold_minor: Option<i64>,
    #[diesel(sql_type = crate::schema::sql_types::UsageAtZeroEnum)]
    pub usage_at_zero: crate::enums::UsageAtZeroEnum,
    /// Usage stops once the wallet's currency is used up: see `pauses_at_zero` in the store.
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub pauses_at_zero: bool,
    /// Set, with the other live fields, once every wallet of the currency has a projection.
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::BigInt>)]
    pub live_balance_minor: Option<i64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::BigInt>)]
    pub pending_usage_minor: Option<i64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::BigInt>)]
    pub uncovered_usage_minor: Option<i64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Timestamptz>)]
    pub live_as_of: Option<DateTime<Utc>>,
    /// The ledger changed after the live fields were computed.
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub live_stale: bool,
    /// HEALTHY, LOW, DEPLETED or USAGE_BLOCKED.
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub status: String,
    #[diesel(sql_type = diesel::sql_types::Timestamptz)]
    pub created_at: DateTime<Utc>,
}

/// What the plan of an active subscription does once the credit of one denomination it charges
/// usage in runs out.
#[derive(Debug, Clone, QueryableByName)]
pub struct PlanCreditRuleRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub customer_id: CustomerId,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub subscription_id: SubscriptionId,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub plan_name: String,
    /// Upper-case currency or credit unit code.
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub denomination: String,
    #[diesel(sql_type = crate::schema::sql_types::CreditExhaustedRuleEnum)]
    pub credit_exhausted_rule: crate::enums::CreditExhaustedRuleEnum,
    #[diesel(sql_type = diesel::sql_types::Jsonb)]
    pub credit_exhausted_caps: serde_json::Value,
}

/// The source of the entries giving a usage settlement's credit back, apart from its draws'.
pub fn usage_settlement_give_back_source(settlement_id: Uuid) -> Uuid {
    Uuid::new_v5(&settlement_id, b"usage-settlement")
}

/// Usage paid from credit ahead of the invoice that will finalize its period.
#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::credit_usage_settlement)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditUsageSettlementRow {
    pub id: UsageSettlementId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub account_id: WalletId,
    pub subscription_id: SubscriptionId,
    pub period_start: NaiveDate,
    pub event_time_bound: DateTime<Utc>,
    pub amount: i64,
    pub tax_inclusive_rate: Option<rust_decimal::Decimal>,
    pub invoice_id: Option<InvoiceId>,
    pub created_at: DateTime<Utc>,
    pub returned_at: Option<DateTime<Utc>>,
    /// The tax in what its paid credit settled, exact as the usage's invoice line levies it.
    pub tax_minor: i64,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::credit_usage_settlement)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditUsageSettlementRowNew {
    pub id: UsageSettlementId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub account_id: WalletId,
    pub subscription_id: SubscriptionId,
    pub period_start: NaiveDate,
    pub event_time_bound: DateTime<Utc>,
    pub amount: i64,
    pub tax_inclusive_rate: Option<rust_decimal::Decimal>,
    pub tax_minor: i64,
}

/// A customer whose live credit projection went stale, as of `marked_at`.
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::credit_recompute_mark)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditRecomputeMarkRow {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub marked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::credit_recompute_mark)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditRecomputeMarkRowNew {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
}

/// What one grant holds now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, diesel::QueryableByName)]
pub struct GrantRemaining {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: CreditGrantId,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub remaining: i64,
}

/// A subscription that ended while some of its credit was not ended with it.
#[derive(Debug, Clone, PartialEq, Eq, diesel::QueryableByName)]
pub struct EndedSubscriptionCredit {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub tenant_id: TenantId,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub subscription_id: SubscriptionId,
}

/// A customer and currency holding expired credit not yet lapsed, keyed for the expiry sweep.
#[derive(Debug, Clone, diesel::QueryableByName)]
pub struct DueExpiryGroup {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub tenant_id: TenantId,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub customer_id: CustomerId,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub currency: String,
}

/// A `DueExpiryGroup` with its earliest lapse and its customer's invoicing entity grace, which
/// decides when the sweep may settle it.
#[derive(Debug, Clone, diesel::QueryableByName)]
pub struct DueExpiryCandidate {
    #[diesel(embed)]
    pub group: DueExpiryGroup,
    #[diesel(sql_type = diesel::sql_types::Timestamptz)]
    pub first_lapse: DateTime<Utc>,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub grace_period_hours: i32,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub consolidate_recurring_invoices: bool,
}

/// A customer with a credit account, keyed for the projection sweep.
#[derive(Debug, Clone, diesel::QueryableByName)]
pub struct CreditRecomputeCandidate {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub tenant_id: TenantId,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub customer_id: CustomerId,
}
