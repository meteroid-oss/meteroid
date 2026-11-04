//! Invoice-time customer credit, kept beside the documents that move it: every movement commits in
//! the same transaction as its invoice, credit note or operator request.
use crate::StoreResult;
use crate::domain::TaxDetail;
use crate::domain::credit::{
    CreditApplication, CreditCost, CreditGrantSource, DrawSource, GrantClassification,
    GrantRollover, GrantScope, LedgerEntryKind,
};
use crate::domain::credit_units::DenominationCode;
use crate::domain::outbox_event::{
    CreditGrantExpiredEvent, OutboxEvent, WalletCreditRecoveredEvent, WalletCreditUnrecoveredEvent,
    WalletDebitedEvent, WalletFundedEvent,
};
use crate::domain::wallet_events::{
    CreditLossNotice, CreditUnrecoveredCause, CreditUnrecoveredTrigger, GrantLapseReason,
};
use crate::errors::StoreError;
use crate::hooks::CreditControlHooks;
use crate::repositories::credit::allocation::{
    Allocation, CreditDemand, CreditRequest, GrantCandidate, GrantPool, allocate, assign,
    rollover_stand_in,
};
use crate::repositories::outbox::publish_outbox_events_tx;
use crate::services::credit::{invalid, precondition};
use crate::store::PgConn;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use common_domain::ids::{
    BaseId, CreditGrantId, CreditNoteId, CustomerId, InvoiceId, LedgerEntryId,
    PaymentTransactionId, ProductId, SubscriptionId, TenantId, UsageSettlementId, WalletId,
};
use diesel_models::customer_credit::{
    CreditUnrecoveredRow, CreditUsageSettlementRow, CreditUsageSettlementRowNew,
    CustomerCreditAccountRow, CustomerCreditAccountRowNew, CustomerCreditEntryRow,
    CustomerCreditEntryRowNew, CustomerCreditGrantProductRow, CustomerCreditGrantRow,
    CustomerCreditGrantRowNew, ScopedGrantRow, SubscriptionFilter,
};
use diesel_models::customers::CustomerRow;
use diesel_models::lock::CustomerLock;
use diesel_models::payment_reversal_movements::PaymentReversalMovementRow;
use error_stack::Report;
use meteroid_tax::{LineToTax, TaxToApply, apply_taxes};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

pub const DEFAULT_PRIORITY: i32 = 100;

/// Why a grant exists; `(kind, source_id)` identifies it once per tenant.
pub mod source {
    pub const CUSTOMER_BALANCE: &str = "CUSTOMER_BALANCE";
    pub const CREDIT_PURCHASE: &str = "CREDIT_PURCHASE";
    pub const INCLUDED_CREDIT: &str = "INCLUDED_CREDIT";
    pub const CREDIT_NOTE: &str = "CREDIT_NOTE";
    pub const NEGATIVE_INVOICE: &str = "NEGATIVE_INVOICE";
    pub const INVOICE_VOID_RESTORE: &str = "INVOICE_VOID_RESTORE";
    pub const MANUAL: &str = "MANUAL";
    /// Collected above what an invoice asked, or a payment no invoice took.
    pub const OVERPAYMENT: &str = "OVERPAYMENT";
    /// Carried from an expiring grant (`source_id` = that grant), exactly once.
    pub const ROLLOVER: &str = "ROLLOVER";
    /// Redeemed with a promotional credit coupon (`source_id` = the applied coupon), once.
    pub const COUPON: &str = "COUPON";
}

/// One movement of a grant, dated by when it happened in business time. Only operator removals
/// carry a reason and an author.
struct Movement {
    kind: LedgerEntryKind,
    amount: i64,
    source_id: Uuid,
    occurred_at: DateTime<Utc>,
    /// What a draw or its reversal pays for; other movements carry none.
    draw_source: Option<DrawSource>,
    invoice_id: Option<InvoiceId>,
    credit_note_id: Option<CreditNoteId>,
    reason: Option<String>,
    created_by: Option<Uuid>,
}

impl Movement {
    fn document(
        kind: LedgerEntryKind,
        amount: i64,
        source_id: Uuid,
        occurred_at: DateTime<Utc>,
        invoice_id: Option<InvoiceId>,
        credit_note_id: Option<CreditNoteId>,
    ) -> Self {
        Self {
            kind,
            amount,
            source_id,
            occurred_at,
            draw_source: None,
            invoice_id,
            credit_note_id,
            reason: None,
            created_by: None,
        }
    }

    fn paying(mut self, source: DrawSource) -> Self {
        self.draw_source = Some(source);
        self
    }

    fn because(mut self, reason: &str) -> Self {
        self.reason = Some(reason.to_owned());
        self
    }
}

/// When a movement happened in business time, and the present its consequences (balance alerts,
/// automatic top-ups) are judged at: the clock, or a replay's simulated moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MovementTime {
    pub at: DateTime<Utc>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub(crate) struct CreditFunding {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub currency: String,
    /// What the grant pays for; a bonus split from it inherits it.
    pub scope: GrantScope,
    pub amount_minor: i64,
    pub classification: GrantClassification,
    pub priority: i32,
    pub effective_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// When the funding happened: the payment, the issue of the document, the operator's request.
    pub occurred_at: DateTime<Utc>,
    /// The present the balance change is judged at.
    pub now: DateTime<Utc>,
    pub source_kind: &'static str,
    pub source_id: Uuid,
    /// What the grant's whole amount was paid, net of tax.
    pub cost: Option<CreditCost>,
    /// Only on included credit; never on a rolled grant.
    pub rollover: Option<GrantRollover>,
    pub invoice_id: Option<InvoiceId>,
    pub line_local_id: Option<String>,
    pub credit_note_id: Option<CreditNoteId>,
    pub reason: String,
    pub reference: Option<String>,
    pub created_by: Option<Uuid>,
    /// The start of the usage it may pay when not `effective_at`; a bonus split from it inherits it.
    pub pays_usage_from: Option<DateTime<Utc>>,
}

/// What paid credit sold below its face mints beside its paid grant.
pub(crate) const BONUS_CREDIT: &str = "Bonus credit";

/// The source of the bonus credit funded beside the paid grant of `source_id`.
pub(crate) fn bonus_source(source_id: Uuid) -> Uuid {
    Uuid::new_v5(&source_id, b"bonus-credit")
}

/// A funding's grant, and the bonus credit minted beside it.
#[derive(Debug, Clone)]
pub(crate) struct FundedCredit {
    pub grant: CustomerCreditGrantRow,
    pub bonus: Option<CustomerCreditGrantRow>,
    pub replayed: bool,
}

/// Tax follows what was paid: paid currency credit bought below its face is minted as paid credit
/// worth its cost, plus promotional "Bonus credit" for the rest at no cost (taken before tax,
/// recognized at nothing), with the same terms and a linked source. Paid credit that cost nothing
/// is promotional; paid credit dearer than its face is refused.
pub(crate) async fn fund_paid_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    funding: CreditFunding,
) -> StoreResult<FundedCredit> {
    let (funding, bonus) = split_paid(funding)?;
    let (grant, replayed) = fund_or_replay_tx(conn, controls, funding).await?;
    let bonus = match bonus {
        Some(bonus) => Some(fund_or_replay_tx(conn, controls, bonus).await?.0),
        None => None,
    };
    Ok(FundedCredit {
        grant,
        bonus,
        replayed,
    })
}

/// `funding` as `fund_paid_tx` mints it, and the bonus credit beside it.
pub(crate) fn split_paid(
    funding: CreditFunding,
) -> StoreResult<(CreditFunding, Option<CreditFunding>)> {
    let Some(cost) = paid_currency_cost(&funding)? else {
        return Ok((funding, None));
    };
    if cost > funding.amount_minor {
        return Err(invalid("paid credit cannot cost more than its face value"));
    }
    if cost == funding.amount_minor {
        return Ok((funding, None));
    }
    if cost == 0 {
        return Ok((
            CreditFunding {
                classification: GrantClassification::Promotional,
                ..funding
            },
            None,
        ));
    }
    let bonus = CreditFunding {
        amount_minor: funding.amount_minor - cost,
        classification: GrantClassification::Promotional,
        source_id: bonus_source(funding.source_id),
        cost: Some(CreditCost::given(&funding.currency)),
        rollover: None,
        reason: BONUS_CREDIT.to_owned(),
        ..funding.clone()
    };
    Ok((
        CreditFunding {
            amount_minor: cost,
            ..funding
        },
        Some(bonus),
    ))
}

/// What paid credit in a currency cost, stated in that currency; None for promotional credit,
/// credit units and credit of no stated cost.
fn paid_currency_cost(funding: &CreditFunding) -> StoreResult<Option<i64>> {
    if funding.classification != GrantClassification::Paid {
        return Ok(None);
    }
    let Some(cost) = &funding.cost else {
        return Ok(None);
    };
    let DenominationCode::Currency(wallet) = DenominationCode::parse(&funding.currency)? else {
        return Ok(None);
    };
    if normalized_currency(&cost.currency) != wallet {
        return Err(invalid(format!(
            "the cost of paid {wallet} credit must be stated in {wallet}"
        )));
    }
    Ok(Some(cost.amount_minor))
}

pub fn normalized_currency(currency: &str) -> String {
    currency.trim().to_uppercase()
}

fn overflow() -> Report<StoreError> {
    invalid("customer credit exceeds the supported range")
}

/// The customer's one wallet of a denomination (a currency or a credit unit code), opened on
/// first use.
pub(crate) async fn account_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    currency: &str,
) -> StoreResult<CustomerCreditAccountRow> {
    let (tenant_id, customer_id) = (lock.tenant_id(), lock.customer_id());
    let denomination =
        super::denomination::resolve_denomination(conn, tenant_id, &normalized_currency(currency))
            .await?;
    let (account, created) = CustomerCreditAccountRow::find_or_create_reporting(
        conn,
        &CustomerCreditAccountRowNew {
            id: WalletId::new(),
            tenant_id,
            customer_id,
            currency: denomination.code().to_owned(),
            credit_unit_id: denomination.credit_unit_id(),
        },
    )
    .await?;
    if created {
        diesel_models::query::credit_inputs::bump_customer(conn, lock).await?;
    }
    Ok(account)
}

/// What `funding` may pay, as stored. A credit unit grant only ever pays usage priced in its unit,
/// so it pays any charge and every product; a subscription's money credit is in its currency.
/// Returns the end of the grant's subscription when it already ended: the grant ends with it.
async fn stored_scope_tx(
    conn: &mut PgConn,
    funding: &CreditFunding,
    account: &CustomerCreditAccountRow,
) -> StoreResult<(GrantScope, Option<DateTime<Utc>>)> {
    let mut scope = funding
        .scope
        .clone()
        .with_products(funding.scope.product_ids.clone());
    if account.credit_unit_id.is_some() {
        if !scope.product_ids.is_empty() {
            return Err(invalid(
                "credit in a credit unit pays for every product; it cannot be restricted to products",
            ));
        }
        scope.application = CreditApplication::Invoice;
    }
    let Some(subscription_id) = scope.subscription_id else {
        return Ok((scope, None));
    };
    let subscription = crate::services::credit::scope::owning_subscription_tx(
        conn,
        funding.tenant_id,
        funding.customer_id,
        subscription_id,
    )
    .await?;
    if account.credit_unit_id.is_none()
        && normalized_currency(&subscription.currency) != account.currency
    {
        return Err(invalid(format!(
            "the subscription is billed in {}; its credit cannot be in {}",
            normalized_currency(&subscription.currency),
            account.currency
        )));
    }
    Ok((scope, subscription.ended_at))
}

/// Idempotent per `(source_kind, source_id)`: a replay returns the recorded grant, a different
/// request for the same source is refused.
pub(crate) async fn fund_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    funding: CreditFunding,
) -> StoreResult<CustomerCreditGrantRow> {
    fund_or_replay_tx(conn, controls, funding)
        .await
        .map(|(grant, _)| grant)
}

/// `fund_tx`, also telling whether the grant already existed.
pub(crate) async fn fund_or_replay_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    funding: CreditFunding,
) -> StoreResult<(CustomerCreditGrantRow, bool)> {
    if funding.amount_minor <= 0 {
        return Err(invalid("customer credit must be positive"));
    }
    if paid_currency_cost(&funding)?.is_some_and(|cost| cost < funding.amount_minor) {
        return Err(precondition(
            "paid credit below its face value is funded with its bonus credit",
        ));
    }
    if funding
        .expires_at
        .is_some_and(|expires_at| expires_at <= funding.effective_at)
    {
        return Err(invalid("customer credit must expire after it takes effect"));
    }
    // Every caller holds the customer's lock already: re-taking it is free and yields the token.
    let lock = lock_customer_tx(conn, funding.tenant_id, funding.customer_id).await?;
    let account = account_tx(conn, &lock, &funding.currency).await?;
    let (scope, ended_at) = stored_scope_tx(conn, &funding, &account).await?;
    let rollover = funding
        .rollover
        .as_ref()
        .map(GrantRollover::to_json)
        .transpose()?;
    let inserted = CustomerCreditGrantRow::insert_new(
        conn,
        &CustomerCreditGrantRowNew {
            id: CreditGrantId::new(),
            tenant_id: funding.tenant_id,
            customer_id: funding.customer_id,
            account_id: account.id,
            amount: funding.amount_minor,
            remaining: funding.amount_minor,
            priority: funding.priority,
            classification: funding.classification.as_db_str().to_owned(),
            effective_at: funding.effective_at,
            expires_at: funding.expires_at,
            source_kind: funding.source_kind.to_owned(),
            source_id: funding.source_id,
            invoice_id: funding.invoice_id,
            line_local_id: funding.line_local_id.clone(),
            credit_note_id: funding.credit_note_id,
            reason: funding.reason.clone(),
            reference: funding.reference.clone(),
            created_by: funding.created_by,
            cost_minor: funding.cost.as_ref().map(|cost| cost.amount_minor),
            cost_currency: funding.cost.as_ref().map(|cost| cost.currency.clone()),
            rollover: rollover.clone(),
            subscription_id: scope.subscription_id,
            application: scope.application.as_db_str().to_owned(),
            // Funded after its subscription ended (a pack paid late): it lapses, paying no other.
            ends_at: ended_at,
            end_reason: ended_at.map(|_| {
                crate::domain::credit::GrantEndReason::SubscriptionEnded
                    .as_db_str()
                    .to_owned()
            }),
            pays_usage_from: funding.pays_usage_from,
        },
    )
    .await?;
    let Some(grant) = inserted else {
        let existing = CustomerCreditGrantRow::find_by_source(
            conn,
            funding.tenant_id,
            funding.source_kind,
            funding.source_id,
        )
        .await?
        .ok_or_else(|| precondition("customer credit grant disappeared"))?;
        if existing.customer_id != funding.customer_id
            || existing.account_id != account.id
            || existing.amount != funding.amount_minor
            || existing.classification != funding.classification.as_db_str()
            || existing.priority != funding.priority
            || !same_instant(Some(existing.effective_at), Some(funding.effective_at))
            || !same_instant(existing.expires_at, funding.expires_at)
            || existing.cost_minor != funding.cost.as_ref().map(|cost| cost.amount_minor)
            || existing.cost_currency.as_deref()
                != funding.cost.as_ref().map(|cost| cost.currency.as_str())
            || existing.rollover != rollover
            || existing.invoice_id != funding.invoice_id
            || existing.line_local_id != funding.line_local_id
            || existing.credit_note_id != funding.credit_note_id
            || existing.reference != funding.reference
            || existing.subscription_id != scope.subscription_id
            || existing.application != scope.application.as_db_str()
            || grant_products_tx(conn, existing.tenant_id, &[existing.id])
                .await?
                .remove(&existing.id)
                .unwrap_or_default()
                != scope.product_ids
        {
            return Err(Report::new(StoreError::Conflict(
                "this credit source already funded a different grant".to_owned(),
            )));
        }
        return Ok((existing, true));
    };
    CustomerCreditGrantProductRow::insert_all(
        conn,
        &scope
            .product_ids
            .iter()
            .map(|product_id| CustomerCreditGrantProductRow {
                tenant_id: grant.tenant_id,
                grant_id: grant.id,
                product_id: *product_id,
            })
            .collect::<Vec<_>>(),
    )
    .await?;
    record_entry(
        conn,
        &grant,
        Movement::document(
            LedgerEntryKind::Fund,
            grant.amount,
            funding.source_id,
            funding.occurred_at,
            funding.invoice_id,
            funding.credit_note_id,
        ),
    )
    .await?;
    balance_changed_tx(
        conn,
        controls,
        grant.tenant_id,
        &[grant.account_id],
        funding.now,
    )
    .await?;
    Ok((grant, false))
}

/// Timestamps as stored: the database keeps microseconds.
fn same_instant(left: Option<DateTime<Utc>>, right: Option<DateTime<Utc>>) -> bool {
    left.map(|at| at.timestamp_micros()) == right.map(|at| at.timestamp_micros())
}

/// Records a movement and publishes its wallet event in the same transaction. `grant` is the row
/// as the movement left it, which the entry's share of the cost is split from. A movement already
/// recorded for the same grant, kind and source is a Conflict: the grant has moved, so replays
/// look for their entry before moving it.
async fn record_entry(
    conn: &mut PgConn,
    grant: &CustomerCreditGrantRow,
    movement: Movement,
) -> StoreResult<CustomerCreditEntryRow> {
    record_linked_entry(conn, grant, movement, None).await
}

/// `record_entry` for a movement following a payment reversal or reinstatement.
async fn record_linked_entry(
    conn: &mut PgConn,
    grant: &CustomerCreditGrantRow,
    movement: Movement,
    payment_movement_id: Option<Uuid>,
) -> StoreResult<CustomerCreditEntryRow> {
    let cost_minor = super::cost::entry_cost(grant, movement.amount)?;
    let recorded = CustomerCreditEntryRow::insert_new(
        conn,
        &CustomerCreditEntryRowNew {
            id: LedgerEntryId::new(),
            tenant_id: grant.tenant_id,
            customer_id: grant.customer_id,
            grant_id: grant.id,
            kind: movement.kind.as_db_str().to_owned(),
            amount: movement.amount,
            source_id: movement.source_id,
            source_kind: movement
                .draw_source
                .map(|source| source.as_db_str().to_owned()),
            invoice_id: movement.invoice_id,
            credit_note_id: movement.credit_note_id,
            occurred_at: movement.occurred_at,
            reason: movement.reason,
            created_by: movement.created_by,
            cost_minor,
            payment_movement_id,
        },
    )
    .await?
    .ok_or_else(|| {
        Report::new(StoreError::Conflict(
            "this credit movement is already recorded".to_owned(),
        ))
    })?;
    // Every caller already holds the customer's lock: re-taking it is free and yields the token.
    // Every wallet: a unit wallet's draws change what money pays for the overage it leaves.
    let lock = lock_customer_tx(conn, grant.tenant_id, grant.customer_id).await?;
    CustomerCreditAccountRow::bump_ledger_version(conn, &lock).await?;
    let account = CustomerCreditAccountRow::find_by_id(conn, grant.tenant_id, grant.account_id)
        .await?
        .ok_or_else(|| precondition("customer credit wallet is missing"))?;
    let precision = super::denomination::account_unit_precision(conn, &account).await?;
    let products = grant_products_tx(conn, grant.tenant_id, &[grant.id]).await?;
    let scope = grant_scope(
        grant,
        products.get(&grant.id).map_or(&[][..], Vec::as_slice),
    )?;
    let event = movement_event(&account, precision, grant, &scope, &recorded)?;
    publish_outbox_events_tx(conn, vec![event]).await?;
    Ok(recorded)
}

fn movement_event(
    account: &CustomerCreditAccountRow,
    unit_precision: Option<u8>,
    grant: &CustomerCreditGrantRow,
    scope: &GrantScope,
    recorded: &CustomerCreditEntryRow,
) -> StoreResult<OutboxEvent> {
    let amount = recorded.amount.checked_abs().ok_or_else(overflow)?;
    Ok(match LedgerEntryKind::from_db_str(&recorded.kind)? {
        kind @ (LedgerEntryKind::Fund
        | LedgerEntryKind::DrawReversal
        | LedgerEntryKind::Reinstatement) => OutboxEvent::wallet_funded(
            WalletFundedEvent::new(
                grant.tenant_id,
                grant.customer_id,
                account.id,
                scope,
                recorded.id,
                grant.id,
                recorded.invoice_id,
                recorded.credit_note_id,
                amount,
                account.currency.clone(),
                match (kind, recorded.credit_note_id) {
                    (LedgerEntryKind::Fund | LedgerEntryKind::Reinstatement, _) => {
                        CreditGrantSource::from_db_str(&grant.source_kind)?
                    }
                    (_, Some(_)) => CreditGrantSource::CreditNote,
                    (_, None) if recorded.invoice_id.is_none() => {
                        CreditGrantSource::UsageSettlementReturn
                    }
                    (_, None) => CreditGrantSource::InvoiceVoid,
                },
                GrantClassification::from_db_str(&grant.classification)?,
            )
            .in_credit_unit(unit_precision),
        ),
        LedgerEntryKind::Expiry
            if !recorded
                .reason
                .as_deref()
                .is_some_and(crate::domain::credit::is_removal_reason) =>
        {
            OutboxEvent::credit_grant_expired(
                CreditGrantExpiredEvent::new(
                    grant.tenant_id,
                    grant.customer_id,
                    account.id,
                    scope,
                    grant.id,
                    recorded.id,
                    amount,
                    account.currency.clone(),
                    grant.lapses_at.unwrap_or(recorded.created_at),
                    lapse_reason(grant),
                )
                .in_credit_unit(unit_precision),
            )
        }
        // Draws, reversals, rollovers, and paid credit a plan change forfeits: it lapses
        // (breakage) but was removed, not expired.
        kind => {
            let reason = match (&recorded.reason, kind) {
                (_, LedgerEntryKind::Rollover) => "Rolled over to a new grant".to_owned(),
                (Some(reason), _) => reason.clone(),
                (None, LedgerEntryKind::Draw)
                    if recorded.source_kind.as_deref()
                        == Some(DrawSource::UsageSettlement.as_db_str()) =>
                {
                    "Usage settled at expiry".to_owned()
                }
                (None, LedgerEntryKind::Draw) => "Applied to invoice".to_owned(),
                (None, _) => "Reversed by credit note".to_owned(),
            };
            OutboxEvent::wallet_debited(
                WalletDebitedEvent::new(
                    grant.tenant_id,
                    grant.customer_id,
                    account.id,
                    scope,
                    recorded.id,
                    grant.id,
                    recorded.invoice_id,
                    recorded.credit_note_id,
                    amount,
                    account.currency.clone(),
                    reason,
                    recorded.created_by,
                )
                .in_credit_unit(unit_precision),
            )
        }
    })
}

/// The ledger's reason (`ended_reason`), so the webhook and the statement never disagree.
fn lapse_reason(grant: &CustomerCreditGrantRow) -> GrantLapseReason {
    if ended_reason(grant) == Some(crate::domain::credit::ENDED_WITH_SUBSCRIPTION) {
        GrantLapseReason::SubscriptionEnded
    } else {
        GrantLapseReason::Expired
    }
}

/// Runs after every movement of the accounts' invoiced value, for the usage controls to judge the
/// balance alerts and the automatic top-up.
pub(crate) async fn balance_changed_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant_id: TenantId,
    account_ids: &[WalletId],
    now: DateTime<Utc>,
) -> StoreResult<()> {
    let mut ids = account_ids.to_vec();
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();
    controls.balance_changed(conn, tenant_id, &ids, now).await
}

/// A pending demand as the projection stores it (`credit_balance_projection.pending_by_product`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDemand {
    pub subscription_id: Option<uuid::Uuid>,
    pub product_id: Option<uuid::Uuid>,
    /// The end of the dates of its usage; absent from projections stored before usage was dated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dated_before: Option<DateTime<Utc>>,
    pub usage_minor: i64,
    /// Of `usage_minor`, the current periods' usage (not yet drafted).
    pub current_usage_minor: i64,
    pub taxed_minor: i64,
    pub owed_minor: i64,
    pub fresh_minor: i64,
    pub uncovered_minor: i64,
}

/// Spendable value of one account at `now`, counting what expired credit will roll over once the
/// sweep reaches it: the wallet never reads low between an expiry and its rollover.
pub async fn available_minor_tx(
    conn: &mut PgConn,
    account_id: WalletId,
    now: DateTime<Utc>,
) -> StoreResult<i64> {
    let live = CustomerCreditGrantRow::available_amounts_for_account(conn, account_id, now)
        .await?
        .into_iter()
        .try_fold(0_i64, |sum, amount| sum.checked_add(amount))
        .ok_or_else(overflow)?;
    unswept_rollovers_tx(conn, account_id, now)
        .await?
        .into_iter()
        .try_fold(live, |sum, (_, carried)| sum.checked_add(carried))
        .ok_or_else(overflow)
}

/// Per expired grant of the account the sweep has not rolled over yet, the most it will carry,
/// while the rolled credit would still be spendable at `now`.
pub(crate) async fn unswept_rollovers_tx(
    conn: &mut PgConn,
    account_id: WalletId,
    now: DateTime<Utc>,
) -> StoreResult<Vec<(CreditGrantId, i64)>> {
    let expired =
        CustomerCreditGrantRow::list_unswept_rollovers_for_account(conn, account_id, now).await?;
    let Some(tenant_id) = expired.first().map(|grant| grant.tenant_id) else {
        return Ok(Vec::new());
    };
    let ids = expired.iter().map(|grant| grant.id).collect::<Vec<_>>();
    let rolled = CustomerCreditGrantRow::list_rolled_over(conn, tenant_id, &ids).await?;
    let mut carried = Vec::new();
    for grant in expired.iter().filter(|grant| !rolled.contains(&grant.id)) {
        let Some(rollover) = grant
            .rollover
            .as_ref()
            .map(GrantRollover::from_json)
            .transpose()?
        else {
            continue;
        };
        let still_spendable = grant
            .expires_at
            .and_then(|at| rollover.rolled_expiry(at))
            .is_some_and(|at| at > now);
        if still_spendable {
            carried.push((grant.id, rollover.carried(grant.remaining)));
        }
    }
    Ok(carried)
}

/// Takes unspent value back out of one grant for a credit note (a refunded purchase, a voided
/// credit). Idempotent per note; refuses rather than leaving value that was already spent.
pub(crate) async fn reverse_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    grant_id: CreditGrantId,
    amount_minor: i64,
    credit_note_id: CreditNoteId,
    now: DateTime<Utc>,
) -> StoreResult<()> {
    let tenant_id = lock.tenant_id();
    if amount_minor <= 0 {
        return Err(invalid("a credit reversal must be positive"));
    }
    let grant = CustomerCreditGrantRow::lock_by_ids(conn, lock, &[grant_id])
        .await?
        .pop()
        .ok_or_else(|| precondition("customer credit grant is missing"))?;
    let reversed = CustomerCreditEntryRow::list_by_note(conn, tenant_id, credit_note_id)
        .await?
        .into_iter()
        .any(|e| e.grant_id == grant_id && e.kind == LedgerEntryKind::Reversal.as_db_str());
    if reversed {
        return Ok(());
    }
    if amount_minor > grant.remaining {
        return Err(precondition(format!(
            "only {} of this credit remains unspent; it cannot be reversed by {amount_minor}",
            grant.remaining
        )));
    }
    let grant = CustomerCreditGrantRow::move_remaining(conn, grant.id, -amount_minor).await?;
    let invoice_id = grant.invoice_id;
    record_entry(
        conn,
        &grant,
        Movement::document(
            LedgerEntryKind::Reversal,
            -amount_minor,
            credit_note_id.as_uuid(),
            now,
            invoice_id,
            Some(credit_note_id),
        ),
    )
    .await?;
    balance_changed_tx(conn, controls, tenant_id, &[grant.account_id], now).await?;
    Ok(())
}

/// Why credit tied to a period is removed before the period ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PeriodCreditRemoval {
    /// The rest ends with the old plan: it pays the usage dated before the change that arrives
    /// within the settlement grace, then lapses.
    PlanChange,
    /// The rest stays, and ends with the subscription.
    Cancellation,
    /// Paid after its period was invoiced: the rest lapses now.
    LatePayment,
}

impl PeriodCreditRemoval {
    fn reason(self) -> &'static str {
        match self {
            Self::PlanChange => crate::domain::credit::PLAN_CHANGE_REMOVAL,
            Self::Cancellation => crate::domain::credit::CANCELLATION_REMOVAL,
            Self::LatePayment => crate::domain::credit::LATE_PAYMENT_REMOVAL,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::PlanChange => "plan-change",
            Self::Cancellation => "cancellation",
            Self::LatePayment => "late-payment",
        }
    }
}

/// Removes credit of a grant whose period ends early: `refunded` of it as a reversal the credit
/// note crediting its fee carries (un-billed). On a plan change the grant ends at `at`, so the
/// sweep lapses the rest (breakage at its cost); paid late, the rest lapses now. The caller holds
/// the grant lock and runs `balance_changed_tx` afterwards.
pub(crate) async fn remove_period_credit_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    grant: &CustomerCreditGrantRow,
    refunded: i64,
    credit_note_id: Option<CreditNoteId>,
    at: DateTime<Utc>,
    removal: PeriodCreditRemoval,
) -> StoreResult<CustomerCreditGrantRow> {
    let left = grant.remaining.max(0);
    let refunded = refunded.clamp(0, left);
    let lapsed = match removal {
        PeriodCreditRemoval::LatePayment => left - refunded,
        PeriodCreditRemoval::PlanChange | PeriodCreditRemoval::Cancellation => 0,
    };
    let occurred_at = not_before_last_movement_tx(conn, grant.id, at).await?;
    let source = |kind: &str| {
        Uuid::new_v5(
            &grant.id.as_uuid(),
            format!("{}-{kind}:{}", removal.key(), at.timestamp_micros()).as_bytes(),
        )
    };
    for (kind, amount, note, invoice_id) in [
        (
            LedgerEntryKind::Reversal,
            refunded,
            credit_note_id,
            grant.invoice_id,
        ),
        (LedgerEntryKind::Expiry, lapsed, None, None),
    ] {
        if amount <= 0 {
            continue;
        }
        let updated = CustomerCreditGrantRow::move_remaining(conn, grant.id, -amount).await?;
        record_entry(
            conn,
            &updated,
            Movement {
                kind,
                amount: -amount,
                source_id: source(kind.as_db_str()),
                occurred_at,
                draw_source: None,
                invoice_id,
                credit_note_id: note,
                reason: Some(removal.reason().to_owned()),
                created_by: None,
            },
        )
        .await?;
    }
    if removal == PeriodCreditRemoval::PlanChange {
        CustomerCreditGrantRow::end_for_plan_change(conn, grant.tenant_id, &[grant.id], at).await?;
        // Ending the grant moves no value but changes what it can pay: a movement for readers.
        CustomerCreditAccountRow::bump_ledger_version(conn, lock).await?;
    }
    CustomerCreditGrantRow::lock_by_ids(conn, lock, &[grant.id])
        .await?
        .pop()
        .ok_or_else(|| precondition("customer credit grant is missing"))
}

/// Moves a purchased grant's value with the money behind its invoice: a negative `delta` takes
/// unspent value back after `cause` took the money, a positive one restores it once recovered.
/// The caller holds the invoice and grant locks and runs `balance_changed_tx` afterwards.
pub(crate) async fn follow_payment_tx(
    conn: &mut PgConn,
    grant_id: CreditGrantId,
    delta: i64,
    invoice_id: Option<InvoiceId>,
    moved: Option<&PaymentMovement>,
    cause: CreditUnrecoveredCause,
    now: DateTime<Utc>,
) -> StoreResult<WalletId> {
    let (kind, reason) = match delta {
        0 => return Err(invalid("a payment-driven credit movement cannot be zero")),
        d if d < 0 => (
            LedgerEntryKind::Reversal,
            match cause {
                CreditUnrecoveredCause::PaymentRefunded => {
                    crate::domain::credit_ledger_labels::PAYMENT_REFUNDED
                }
                CreditUnrecoveredCause::PaymentDisputed => {
                    crate::domain::credit_ledger_labels::PAYMENT_DISPUTE_LOST
                }
                CreditUnrecoveredCause::PaymentReversed
                | CreditUnrecoveredCause::CreditNoteRefunded => {
                    crate::domain::credit_ledger_labels::PAYMENT_REVERSED
                }
            },
        ),
        _ => (
            LedgerEntryKind::Reinstatement,
            crate::domain::credit_ledger_labels::PAYMENT_RECOVERED,
        ),
    };
    // Dated at the provider's event, never before the value it moves.
    let at = match moved {
        Some(moved) => not_before_last_movement_tx(conn, grant_id, moved.at.min(now)).await?,
        None => now,
    };
    let grant = CustomerCreditGrantRow::move_remaining(conn, grant_id, delta).await?;
    record_linked_entry(
        conn,
        &grant,
        Movement {
            kind,
            amount: delta,
            // Each movement is its own source: the invoice lock makes the cumulative target exact.
            source_id: Uuid::now_v7(),
            occurred_at: at,
            draw_source: None,
            invoice_id,
            credit_note_id: None,
            reason: Some(reason.to_owned()),
            created_by: None,
        },
        moved.and_then(|moved| moved.movement),
    )
    .await?;
    let grant = lapse_given_back_tx(conn, grant, delta, GivenBack::Funding, at).await?;
    Ok(grant.account_id)
}

/// What the credit a payment funded follows: one of its applied reversals or reinstatements
/// (`payment_reversal_movement`), a refund of it without a credit note, a collection paying a
/// reopened invoice again, or credit taken back or restored later on a flow a dispute took money
/// from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PaymentMovement {
    pub payment: PaymentTransactionId,
    /// The movement the ledger entries cite, so dispute analytics leave out what the ledger books.
    pub movement: Option<Uuid>,
    /// When the credit moves (the provider's event, the collection, or now for a follow-up).
    pub at: DateTime<Utc>,
    pub disputed: bool,
    /// A later movement on a disputed flow, not the payment event itself.
    pub follow_up: bool,
    /// A refund without a credit note issued (or failed), and the money it moved.
    pub refunded: Option<i64>,
}

impl PaymentMovement {
    /// A collection paying a reopened invoice again: what it restores is dated when it was paid.
    pub(crate) fn collection(payment: PaymentTransactionId, at: DateTime<Utc>) -> Self {
        Self {
            payment,
            movement: None,
            at,
            disputed: false,
            follow_up: false,
            refunded: None,
        }
    }

    /// Refunds without a credit note of `payment` issued, or failed, for `amount_minor` in all.
    pub(crate) fn refund(
        payment: PaymentTransactionId,
        amount_minor: i64,
        at: DateTime<Utc>,
    ) -> Self {
        Self {
            refunded: Some(amount_minor),
            ..Self::collection(payment, at)
        }
    }

    pub(crate) fn cause(&self) -> CreditUnrecoveredCause {
        if self.refunded.is_some() {
            CreditUnrecoveredCause::PaymentRefunded
        } else if self.disputed {
            CreditUnrecoveredCause::PaymentDisputed
        } else {
            CreditUnrecoveredCause::PaymentReversed
        }
    }
}

/// Credit given back to a flow a dispute took money from, and taken back again (or the reverse),
/// follows that dispute: linked to its latest withdrawal, dated now.
pub(crate) async fn dispute_follow_up_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: Option<InvoiceId>,
    payment: Option<PaymentTransactionId>,
    now: DateTime<Utc>,
) -> StoreResult<Option<PaymentMovement>> {
    Ok(
        PaymentReversalMovementRow::latest_disputed_withdrawal(
            conn, tenant_id, invoice_id, payment,
        )
        .await?
        .map(|withdrawal| PaymentMovement {
            payment: withdrawal.payment_transaction_id,
            movement: Some(withdrawal.id),
            at: now,
            disputed: true,
            follow_up: true,
            refunded: None,
        }),
    )
}

/// Moves the wallet's reported loss on one money flow to `unrecovered`, the loss now: a rise is
/// reported as unrecovered credit, a fall as recovered, and a replay publishes nothing. The caller
/// holds the customer. `payment_id` is the payment whose reversal or reinstatement moved it.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn report_unrecovered_credit_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    grant: &CustomerCreditGrantRow,
    invoice_id: Option<InvoiceId>,
    payment_id: Option<PaymentTransactionId>,
    trigger: CreditUnrecoveredTrigger,
    taken_back: i64,
    unrecovered: i64,
) -> StoreResult<()> {
    if unrecovered < 0 {
        return Err(invalid("an unrecovered credit amount cannot be negative"));
    }
    let source_id = trigger
        .source_id(invoice_id)
        .ok_or_else(|| invalid("an invoice's payment loss needs the invoice"))?;
    let previous =
        CreditUnrecoveredRow::find_for_update(conn, lock, grant.account_id, source_id).await?;
    let (reported, notices) = previous
        .as_ref()
        .map_or((0, 0), |row| (row.reported, row.notices));
    let change = unrecovered.checked_sub(reported).ok_or_else(overflow)?;
    if change == 0 {
        return Ok(());
    }
    let sequence = notices.checked_add(1).ok_or_else(overflow)?;
    CreditUnrecoveredRow {
        tenant_id: grant.tenant_id,
        account_id: grant.account_id,
        invoice_id,
        // Only a flow no invoice owns is keyed by its payment.
        payment_id: match invoice_id {
            Some(_) => None,
            None => previous
                .as_ref()
                .and_then(|row| row.payment_id)
                .or(payment_id),
        },
        source_id,
        reported: unrecovered,
        notices: sequence,
    }
    .upsert(conn)
    .await?;

    let account = CustomerCreditAccountRow::find_by_id(conn, grant.tenant_id, grant.account_id)
        .await?
        .ok_or_else(|| precondition("customer credit wallet is missing"))?;
    let credit_unit_precision = super::denomination::account_unit_precision(conn, &account).await?;
    let notice = CreditLossNotice {
        tenant_id: grant.tenant_id,
        customer_id: grant.customer_id,
        wallet_id: account.id,
        invoice_id,
        payment_id,
        trigger,
        source_id,
        currency: account.currency.clone(),
        credit_unit_precision,
        sequence,
        total_minor: unrecovered,
    };
    let flow = invoice_id.map_or_else(
        || {
            format!(
                "payment {}",
                payment_id.map(|p| p.to_string()).unwrap_or_default()
            )
        },
        |invoice| format!("invoice {invoice}"),
    );
    let event = if change > 0 {
        log::error!(
            "{flow} of tenant {}: {} leaves {change} more {} minor units of purchased credit spent and unrecovered ({unrecovered} in all)",
            grant.tenant_id,
            trigger.cause().as_str(),
            account.currency
        );
        OutboxEvent::wallet_credit_unrecovered(WalletCreditUnrecoveredEvent::new(
            notice, taken_back, change,
        ))
    } else {
        log::warn!(
            "{flow} of tenant {}: {} minor units of {} purchased credit reported unrecovered are backed again ({unrecovered} still unrecovered)",
            grant.tenant_id,
            -change,
            account.currency
        );
        OutboxEvent::wallet_credit_recovered(WalletCreditRecoveredEvent::new(notice, -change))
    };
    publish_outbox_events_tx(conn, vec![event]).await
}

/// An operator's removal of credit added by hand: `amount_minor` of the unspent value, or all of
/// it. A replay of the same request returns what it removed.
#[derive(Debug, Clone)]
pub(crate) struct ManualDebit {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub grant_id: CreditGrantId,
    pub amount_minor: Option<i64>,
    pub request_id: Uuid,
    pub reason: String,
    pub actor_id: Option<Uuid>,
}

/// Returns the debited grant, the amount removed and whether the request was a replay.
pub(crate) async fn debit_manual_grant_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    debit: ManualDebit,
    now: DateTime<Utc>,
) -> StoreResult<(CustomerCreditGrantRow, i64, bool)> {
    let grant = CustomerCreditGrantRow::lock_by_ids(conn, lock, &[debit.grant_id])
        .await?
        .pop()
        .filter(|grant| grant.customer_id == debit.customer_id)
        .ok_or_else(|| Report::new(StoreError::ValueNotFound("credit grant".to_owned())))?;
    if let Some(done) = CustomerCreditEntryRow::find_by_source(
        conn,
        grant.id,
        LedgerEntryKind::Reversal.as_db_str(),
        debit.request_id,
    )
    .await?
    {
        let removed = done.amount.checked_neg().ok_or_else(overflow)?;
        if debit.amount_minor.is_some_and(|amount| amount != removed) {
            return Err(Report::new(StoreError::Conflict(
                "this request already removed a different amount".to_owned(),
            )));
        }
        return Ok((grant, removed, true));
    }
    if grant.source_kind != source::MANUAL {
        return Err(precondition(
            "only credit added by hand can be removed; purchased or returned credit is reversed with a credit note",
        ));
    }
    if grant.lapses_at.is_some_and(|at| at <= now) {
        return Err(precondition(
            "this credit expired; nothing is left to remove",
        ));
    }
    let amount = debit.amount_minor.unwrap_or(grant.remaining);
    if amount <= 0 {
        return Err(if debit.amount_minor.is_some() {
            invalid("the amount to remove must be positive")
        } else {
            precondition("this credit is fully spent; nothing is left to remove")
        });
    }
    if amount > grant.remaining {
        return Err(precondition(format!(
            "only {} of this credit remains unspent; {amount} cannot be removed",
            grant.remaining
        )));
    }
    let grant = CustomerCreditGrantRow::move_remaining(conn, grant.id, -amount).await?;
    record_entry(
        conn,
        &grant,
        Movement {
            kind: LedgerEntryKind::Reversal,
            amount: -amount,
            source_id: debit.request_id,
            occurred_at: now,
            draw_source: None,
            invoice_id: None,
            credit_note_id: None,
            reason: Some(debit.reason),
            created_by: debit.actor_id,
        },
    )
    .await?;
    balance_changed_tx(conn, controls, debit.tenant_id, &[grant.account_id], now).await?;
    Ok((grant, amount, false))
}

/// The least time after credit lapses before the usage dated earlier is settled from it.
pub(crate) const MIN_SETTLEMENT_GRACE: Duration = Duration::minutes(30);

/// How long after credit lapses the usage dated before is settled from it: as long as the period's
/// invoice waits for late usage (the invoicing entity's grace). Events arriving later are priced
/// on the invoice but no longer covered by the lapsed credit.
pub(crate) fn settlement_grace(grace_period_hours: i32, consolidate: bool) -> Duration {
    let hours =
        crate::services::invoices::bill::recurring_grace_hours(grace_period_hours, consolidate);
    MIN_SETTLEMENT_GRACE.max(Duration::hours(i64::from(hours)))
}

/// `settlement_grace` of the customer's invoicing entity.
pub(crate) async fn customer_settlement_grace_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> StoreResult<Duration> {
    let (grace_period_hours, consolidate) =
        diesel_models::invoicing_entities::InvoicingEntityRow::recurring_grace_of_customer(
            conn,
            tenant_id,
            customer_id,
        )
        .await?;
    Ok(settlement_grace(grace_period_hours, consolidate))
}

/// `at`, or the grant's latest movement if later: a backdated movement never precedes the value
/// it moves (credit a void gave back later, value returned after an expiry).
async fn not_before_last_movement_tx(
    conn: &mut PgConn,
    grant_id: CreditGrantId,
    at: DateTime<Utc>,
) -> StoreResult<DateTime<Utc>> {
    Ok(CustomerCreditEntryRow::last_occurred_at(conn, grant_id)
        .await?
        .map_or(at, |last| last.max(at)))
}

/// When value of a lapsed grant (expired, or ended with its subscription) lapses or rolls over.
async fn lapse_moment_tx(
    conn: &mut PgConn,
    grant: &CustomerCreditGrantRow,
) -> StoreResult<DateTime<Utc>> {
    let lapsed_at = grant
        .lapses_at
        .ok_or_else(|| precondition("only expiring credit lapses"))?;
    not_before_last_movement_tx(conn, grant.id, lapsed_at).await
}

/// Whether the grant lapses by its subscription ending, or by a plan change removing it, at or
/// before its own expiry: no rollover happens then.
pub fn ended_first(grant: &CustomerCreditGrantRow) -> bool {
    grant.ends_at.is_some_and(|ends_at| {
        grant
            .expires_at
            .is_none_or(|expires_at| ends_at <= expires_at)
    })
}

/// Records the lapse of `amount` of a lapsed grant's value. A fresh source per lapse: a voided
/// invoice can return value to a lapsed grant later.
pub(crate) async fn expire_value_tx(
    conn: &mut PgConn,
    grant: &CustomerCreditGrantRow,
    amount: i64,
) -> StoreResult<CustomerCreditGrantRow> {
    let lapsed_at = lapse_moment_tx(conn, grant).await?;
    let updated = CustomerCreditGrantRow::move_remaining(conn, grant.id, -amount).await?;
    let movement = Movement::document(
        LedgerEntryKind::Expiry,
        -amount,
        Uuid::now_v7(),
        lapsed_at,
        None,
        None,
    );
    let movement = match ended_reason(grant) {
        Some(reason) => movement.because(reason),
        None => movement,
    };
    record_entry(conn, &updated, movement).await?;
    Ok(updated)
}

/// The ledger reason of a lapse caused by the grant's end rather than its expiry.
fn ended_reason(grant: &CustomerCreditGrantRow) -> Option<&'static str> {
    if !ended_first(grant) {
        return None;
    }
    Some(
        if grant.end_reason.as_deref()
            == Some(diesel_models::query::customer_credit::PLAN_CHANGE_ENDED)
        {
            crate::domain::credit::PLAN_CHANGE_REMOVAL
        } else {
            crate::domain::credit::ENDED_WITH_SUBSCRIPTION
        },
    )
}

/// `expire_value_tx` for value that lapses without the usage it covered having been settled.
pub(crate) async fn lapse_unsettled_tx(
    conn: &mut PgConn,
    grant: &CustomerCreditGrantRow,
    amount: i64,
) -> StoreResult<CustomerCreditGrantRow> {
    let lapsed_at = lapse_moment_tx(conn, grant).await?;
    let updated = CustomerCreditGrantRow::move_remaining(conn, grant.id, -amount).await?;
    record_entry(
        conn,
        &updated,
        Movement::document(
            LedgerEntryKind::Expiry,
            -amount,
            Uuid::now_v7(),
            lapsed_at,
            None,
            None,
        )
        .because("Lapsed without settling the usage it covered"),
    )
    .await?;
    Ok(updated)
}

/// Lapses what is left of an expired grant, first carrying up to its rollover cap into a new grant
/// with the same scope that burns before fresh credit (priority one lower) for the rollover term.
/// A grant rolls once, and never once its subscription ended; value returned to it later only
/// lapses. The caller holds the grant locks.
pub(crate) async fn roll_over_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    scoped: &ScopedGrantRow,
    currency: &str,
    rollover: &GrantRollover,
    now: DateTime<Utc>,
) -> StoreResult<(CustomerCreditGrantRow, Option<CustomerCreditGrantRow>)> {
    let grant = &scoped.grant;
    let expired_at = grant
        .expires_at
        .ok_or_else(|| precondition("only expiring credit rolls over"))?;
    let rolled_once = CustomerCreditGrantRow::find_by_source(
        conn,
        grant.tenant_id,
        source::ROLLOVER,
        grant.id.as_uuid(),
    )
    .await?
    .is_some();
    let carried = if rolled_once || ended_first(grant) {
        0
    } else {
        rollover.carried(grant.remaining)
    };
    if carried == 0 {
        let lapsed = match grant.remaining {
            0 => grant.clone(),
            remaining => expire_value_tx(conn, grant, remaining).await?,
        };
        return Ok((lapsed, None));
    }
    let expires_at = rollover
        .rolled_expiry(expired_at)
        .ok_or_else(|| invalid("rolled credit expires a number of days or months later"))?;
    let cost = super::cost::carried_cost(grant, carried)?;
    let rolled_at = lapse_moment_tx(conn, grant).await?;
    let mut updated = CustomerCreditGrantRow::move_remaining(conn, grant.id, -carried).await?;
    record_entry(
        conn,
        &updated,
        Movement::document(
            LedgerEntryKind::Rollover,
            -carried,
            grant.id.as_uuid(),
            rolled_at,
            None,
            None,
        ),
    )
    .await?;
    if updated.remaining > 0 {
        updated = expire_value_tx(conn, &updated, updated.remaining).await?;
    }
    let rolled = fund_tx(
        conn,
        controls,
        rolled_over_funding(scoped, currency, carried, expires_at, rolled_at, now, cost)?,
    )
    .await?;
    Ok((updated, Some(rolled)))
}

/// The grant `carried` of `scoped` rolls over into: its subscription, application and products.
fn rolled_over_funding(
    scoped: &ScopedGrantRow,
    currency: &str,
    carried: i64,
    expires_at: DateTime<Utc>,
    rolled_at: DateTime<Utc>,
    now: DateTime<Utc>,
    cost: Option<CreditCost>,
) -> StoreResult<CreditFunding> {
    let grant = &scoped.grant;
    let expired_at = grant
        .expires_at
        .ok_or_else(|| precondition("only expiring credit rolls over"))?;
    Ok(CreditFunding {
        tenant_id: grant.tenant_id,
        customer_id: grant.customer_id,
        currency: currency.to_owned(),
        scope: grant_scope(grant, &scoped.product_ids)?,
        amount_minor: carried,
        classification: GrantClassification::from_db_str(&grant.classification)?,
        priority: grant.priority.checked_sub(1).ok_or_else(overflow)?,
        effective_at: expired_at,
        expires_at: Some(expires_at),
        occurred_at: rolled_at,
        now,
        source_kind: source::ROLLOVER,
        source_id: grant.id.as_uuid(),
        cost,
        rollover: None,
        invoice_id: None,
        line_local_id: None,
        credit_note_id: None,
        reason: "Rolled over from expiring credit".to_owned(),
        reference: None,
        created_by: None,
        // Rollovers never chain (a rolled grant is funded without one): one hop is the whole origin.
        pays_usage_from: Some(grant.pays_usage_from.unwrap_or(grant.effective_at)),
    })
}

/// Whether value back on this expired grant is left for the sweep to roll over rather than lapsed.
pub(crate) async fn awaits_rollover_tx(
    conn: &mut PgConn,
    grant: &CustomerCreditGrantRow,
) -> StoreResult<bool> {
    if grant.rollover.is_none() || ended_first(grant) {
        return Ok(false);
    }
    Ok(CustomerCreditGrantRow::find_by_source(
        conn,
        grant.tenant_id,
        source::ROLLOVER,
        grant.id.as_uuid(),
    )
    .await?
    .is_none())
}

/// The customer lock every credit money path takes first (`diesel_models::lock`).
pub async fn lock_customer_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> StoreResult<CustomerLock> {
    Ok(CustomerRow::lock(conn, tenant_id, customer_id).await?)
}

/// Locks every grant of the customer in a currency, with its products.
pub(crate) async fn lock_grants_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    currency: &str,
) -> StoreResult<LockedGrants> {
    Ok(CustomerCreditGrantRow::lock_all(conn, lock, &normalized_currency(currency)).await?)
}

/// The customer's unspent grants of a denomination, with their products, locked in id order.
pub(crate) async fn lock_drawable_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    currency: &str,
) -> StoreResult<LockedGrants> {
    drawable_tx(
        conn,
        lock.tenant_id(),
        lock.customer_id(),
        currency,
        Some(lock),
    )
    .await
}

/// `lock_drawable_tx` without locking anything.
pub(crate) async fn list_drawable_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
) -> StoreResult<LockedGrants> {
    drawable_tx(conn, tenant_id, customer_id, currency, None).await
}

async fn drawable_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
    lock: Option<&CustomerLock>,
) -> StoreResult<LockedGrants> {
    let currency = normalized_currency(currency);
    let Some(account) =
        CustomerCreditAccountRow::find_by_currency(conn, tenant_id, customer_id, &currency).await?
    else {
        return Ok(Vec::new());
    };
    let filter = SubscriptionFilter::Any;
    Ok(match lock {
        Some(lock) => CustomerCreditGrantRow::lock_drawable(conn, lock, account.id, filter).await?,
        None => {
            CustomerCreditGrantRow::list_drawable(conn, tenant_id, customer_id, account.id, filter)
                .await?
        }
    })
}

/// A grant list held under `lock_drawable` or `lock_all`, kept current as movements land.
pub(crate) type LockedGrants = Vec<ScopedGrantRow>;

fn refresh_locked(grants: &mut LockedGrants, updated: &CustomerCreditGrantRow) {
    if let Some(slot) = grants.iter_mut().find(|slot| slot.grant.id == updated.id) {
        slot.grant = updated.clone();
    }
}

/// What a settlement pays for one product: its usage before tax, and the taxes its invoice line
/// will levy. Paid credit is a means of payment and pays the usage with its tax; promotional
/// credit comes off before tax, as on the invoice.
#[derive(Debug, Clone)]
pub struct UsageOwed {
    pub product_id: Option<ProductId>,
    pub base_minor: i64,
    pub tax_inclusive: bool,
    pub taxes: Vec<TaxDetail>,
}

impl UsageOwed {
    /// What `base` of it is invoiced at, tax included.
    pub(crate) fn invoiced(&self, base: i64) -> i64 {
        if self.tax_inclusive || self.taxes.is_empty() || base <= 0 {
            return base;
        }
        base + self.levied(base, false)
    }

    /// The tax in what `covered` of `base`'s invoiced figure pays, as its invoice line levies it,
    /// `base` being invoiced at `full` (`invoiced_together`).
    pub(crate) fn tax_in_within(&self, base: i64, covered: i64, full: i64) -> i64 {
        if self.taxes.is_empty() || covered <= 0 {
            0
        } else if self.tax_inclusive {
            self.levied(covered, true)
        } else if covered >= full {
            full - base.max(0)
        } else {
            let usage = self.usage_paid_by_within(base, covered, full);
            self.invoiced(usage) - usage
        }
    }

    fn tax_keys(&self) -> Vec<TaxToApply> {
        self.taxes
            .iter()
            .map(|tax| TaxToApply {
                rate: tax.tax_rate,
                key: format!("{}:{}", tax.tax_name, tax.tax_rate),
                compound: false,
            })
            .collect()
    }

    fn levied(&self, amount: i64, tax_inclusive: bool) -> i64 {
        let line = LineToTax {
            taxable_amount: amount,
            tax_inclusive,
            taxes: self.tax_keys(),
        };
        apply_taxes(&[line]).into_iter().flatten().sum::<i64>()
    }

    /// What each of `owed` is invoiced at for `bases`, tax included, each rate rounded once over
    /// all of them as the invoice taking them over rounds it (OQ14), not line by line.
    pub(crate) fn invoiced_together(owed: &[Self], bases: &[i64]) -> Vec<i64> {
        let lines: Vec<LineToTax> = owed
            .iter()
            .zip(bases)
            .map(|(owed, base)| LineToTax {
                taxable_amount: if owed.tax_inclusive {
                    0
                } else {
                    (*base).max(0)
                },
                tax_inclusive: false,
                taxes: owed.tax_keys(),
            })
            .collect();
        apply_taxes(&lines)
            .into_iter()
            .zip(bases)
            .map(|(taxes, base)| base + taxes.iter().sum::<i64>())
            .collect()
    }

    /// `usage_paid_by` where `base` is invoiced at `full`: all of it only once `paid` reaches it.
    pub(crate) fn usage_paid_by_within(&self, base: i64, paid: i64, full: i64) -> i64 {
        let base = base.max(0);
        if paid >= full {
            return base;
        }
        self.usage_paid_by(base, paid).min((base - 1).max(0))
    }

    /// The usage before tax that `paid` of `base`'s invoiced figure pays: all of it once `paid`
    /// reaches that figure, else the most whose invoiced figure `paid` covers. Not every amount is
    /// an invoiced figure (3 at 20% invoices at 4), so up to one minor unit per tax pays no usage.
    pub(crate) fn usage_paid_by(&self, base: i64, paid: i64) -> i64 {
        let base = base.max(0);
        if paid <= 0 {
            return 0;
        }
        if paid >= self.invoiced(base) {
            return base;
        }
        // The invoiced figure rises with the usage: `low` is paid whole, `high` is not.
        let (mut low, mut high) = (0_i64, base);
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            if self.invoiced(middle) <= paid {
                low = middle;
            } else {
                high = middle;
            }
        }
        low
    }
}

/// A usage charge of a promotional request (`demand`), with the rates its invoice line levies.
#[derive(Debug, Clone)]
pub struct UsageCharge {
    pub demand: usize,
    pub owed: UsageOwed,
}

/// What promotional credit may take of `request` once the paid settlements the invoice keeps
/// are placed: each pays its charges tax included, as it settled them, and the usage that pays
/// (by the charge's rates) is never paid again before tax.
pub fn after_paid_settlements(
    request: &CreditRequest,
    charges: &[UsageCharge],
    paid_settled: &[(GrantCandidate, i64)],
) -> StoreResult<CreditRequest> {
    if paid_settled.is_empty() || charges.is_empty() {
        return Ok(request.clone());
    }
    let invoiced = UsageOwed::invoiced_together(
        &charges
            .iter()
            .map(|charge| charge.owed.clone())
            .collect::<Vec<_>>(),
        &charges
            .iter()
            .map(|charge| charge.owed.base_minor)
            .collect::<Vec<_>>(),
    );
    let placed = assign(
        paid_settled,
        &CreditRequest {
            requested_minor: invoiced
                .iter()
                .try_fold(0_i64, |sum, amount| sum.checked_add(*amount))
                .ok_or_else(overflow)?,
            demands: charges
                .iter()
                .zip(&invoiced)
                .map(|(charge, amount)| CreditDemand {
                    subscription_id: request
                        .demands
                        .get(charge.demand)
                        .and_then(|demand| demand.subscription_id),
                    product_id: charge.owed.product_id,
                    usage: true,
                    amount_minor: *amount,
                    dated_before: request
                        .demands
                        .get(charge.demand)
                        .and_then(|demand| demand.dated_before),
                })
                .collect(),
        },
    )?;
    let (mut base, mut paid) = (
        vec![0_i64; request.demands.len()],
        vec![0_i64; request.demands.len()],
    );
    for ((charge, covered), full) in charges.iter().zip(&placed.covered).zip(&invoiced) {
        let (Some(base), Some(paid)) = (base.get_mut(charge.demand), paid.get_mut(charge.demand))
        else {
            return Err(precondition(
                "a usage charge names no demand of the request",
            ));
        };
        *base = base
            .checked_add(charge.owed.base_minor.max(0))
            .ok_or_else(overflow)?;
        *paid = paid
            .checked_add(
                charge
                    .owed
                    .usage_paid_by_within(charge.owed.base_minor, *covered, *full),
            )
            .ok_or_else(overflow)?;
    }
    let mut left = request.clone();
    let mut taken = 0_i64;
    for ((demand, base), paid) in left.demands.iter_mut().zip(base).zip(paid) {
        if paid <= 0 {
            continue;
        }
        let amount = demand.amount_minor.min(base - paid).max(0);
        taken = taken
            .checked_add(demand.amount_minor - amount)
            .ok_or_else(overflow)?;
        demand.amount_minor = amount;
    }
    left.requested_minor = (left.requested_minor - taken).max(0);
    Ok(left)
}

/// Pays a period's usage (`owed`, net of what `earlier` settlements of it paid) from the locked
/// grants as they stand just before `bound`: those expiring at `bound` first whatever their pool
/// (what they leave lapses), then the others by `pools` in turn. Records it as one settlement per
/// paying account, after any it already recorded at `bound` (usage ingested since). The caller
/// holds the grant locks. Returns the accounts that paid and the value settled.
/// `ending` names grants a plan change ends at `bound`: only they pay, as if they expired then.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn settle_usage_tx(
    conn: &mut PgConn,
    settlement: &UsageSettlementTerms,
    owed: &[UsageOwed],
    earlier: &[(GrantCandidate, i64)],
    pools: &[GrantPool],
    grants: &mut LockedGrants,
    ending: &[CreditGrantId],
) -> StoreResult<(Vec<WalletId>, i64)> {
    let at = settlement.bound - Duration::microseconds(1);
    let (expiring, mut others): (Vec<_>, Vec<_>) =
        candidates(grants)?.into_iter().partition(|grant| {
            if ending.is_empty() {
                grant.lapses_at == Some(settlement.bound)
            } else {
                ending.contains(&grant.id)
            }
        });
    if !ending.is_empty() {
        others.clear();
    }
    let mut left = owed
        .iter()
        .map(|owed| owed.base_minor.max(0))
        .collect::<Vec<_>>();
    let (subscription_id, dated_before) = (settlement.subscription_id, settlement.dated_before);
    for pool in pools.iter().rev() {
        let held = earlier
            .iter()
            .filter(|(grant, _)| pool.admits(grant.classification))
            .cloned()
            .collect::<Vec<_>>();
        let asked = owed_request(owed, &left, *pool, subscription_id, dated_before);
        let placed = assign(&held, &asked)?;
        pay_owed(owed, &mut left, *pool, &placed);
    }
    let mut taxed: HashMap<CreditGrantId, i64> = HashMap::new();
    let plan = draw_owed(
        owed,
        &mut left,
        (subscription_id, dated_before),
        &[&expiring, &others],
        pools,
        at,
        &mut taxed,
    )?;
    let mut by_account: BTreeMap<Uuid, (WalletId, Vec<(CreditGrantId, i64)>)> = BTreeMap::new();
    for (grant_id, amount) in plan {
        let account_id = grants
            .iter()
            .find(|slot| slot.grant.id == grant_id)
            .map(|slot| slot.grant.account_id)
            .ok_or_else(|| precondition("customer credit grant is missing"))?;
        by_account
            .entry(account_id.as_uuid())
            .or_insert_with(|| (account_id, Vec::new()))
            .1
            .push((grant_id, amount));
    }
    // Frozen with the settlement: the tax its quoted usage price carried.
    let tax_inclusive_rate = diesel_models::subscriptions::SubscriptionRow::get_tax_inclusive_rate(
        conn,
        settlement.tenant_id,
        settlement.subscription_id,
    )
    .await?;
    let (mut paid_by, mut settled) = (Vec::new(), 0_i64);
    for (account_id, draws) in by_account.into_values() {
        let total = draws
            .iter()
            .try_fold(0_i64, |sum, (_, amount)| sum.checked_add(*amount))
            .ok_or_else(overflow)?;
        let tax_minor = draws
            .iter()
            .try_fold(0_i64, |sum, (grant_id, _)| {
                sum.checked_add(taxed.get(grant_id).copied().unwrap_or(0))
            })
            .ok_or_else(overflow)?;
        let Some(recorded) = CreditUsageSettlementRow::insert_new(
            conn,
            &CreditUsageSettlementRowNew {
                id: UsageSettlementId::new(),
                tenant_id: settlement.tenant_id,
                customer_id: settlement.customer_id,
                account_id,
                subscription_id: settlement.subscription_id,
                period_start: settlement.period_start,
                event_time_bound: settlement.bound,
                amount: total,
                tax_inclusive_rate,
                tax_minor,
            },
        )
        .await?
        else {
            continue;
        };
        for (grant_id, amount) in draws {
            let drawn_at = not_before_last_movement_tx(conn, grant_id, settlement.bound).await?;
            let grant = CustomerCreditGrantRow::move_remaining(conn, grant_id, -amount).await?;
            refresh_locked(grants, &grant);
            record_entry(
                conn,
                &grant,
                Movement::document(
                    LedgerEntryKind::Draw,
                    -amount,
                    recorded.id.as_uuid(),
                    drawn_at,
                    None,
                    None,
                )
                .paying(DrawSource::UsageSettlement),
            )
            .await?;
        }
        paid_by.push(account_id);
        settled = settled.checked_add(total).ok_or_else(overflow)?;
    }
    Ok((paid_by, settled))
}

/// Adds the tax paid credit settled in `drawn` to its draws, split by what each drew so the parts
/// sum to it exactly.
/// What `owed` (`left` of it) asks of `pool`: promotional credit pays its usage before tax, paid
/// credit its invoiced figure.
fn owed_request(
    owed: &[UsageOwed],
    left: &[i64],
    pool: GrantPool,
    subscription_id: SubscriptionId,
    dated_before: DateTime<Utc>,
) -> CreditRequest {
    let amounts = if pool == GrantPool::Paid {
        UsageOwed::invoiced_together(owed, left)
    } else {
        left.to_vec()
    };
    CreditRequest {
        requested_minor: amounts.iter().sum(),
        demands: owed
            .iter()
            .zip(amounts)
            .map(|(owed, amount)| CreditDemand {
                subscription_id: Some(subscription_id),
                product_id: owed.product_id,
                usage: true,
                amount_minor: amount,
                dated_before: Some(dated_before),
            })
            .collect(),
    }
}

/// Takes off `left` the usage `paid` (drawn from `pool`) pays: paid credit covers invoiced
/// figures, so what it pays comes off the usage they invoice.
fn pay_owed(owed: &[UsageOwed], left: &mut [i64], pool: GrantPool, paid: &Allocation) {
    let full = UsageOwed::invoiced_together(owed, left);
    for (index, base) in left.iter_mut().enumerate() {
        let covered = paid.covered.get(index).copied().unwrap_or(0);
        *base = if pool == GrantPool::Paid {
            *base - owed[index].usage_paid_by_within(*base, covered, full[index])
        } else {
            (*base - covered).max(0)
        };
    }
}

/// Draws what is `left` of `owed` (one subscription's usage dated before a bound) from each of
/// `sets` in turn, each by `pools` in turn, as they stand at `at`; `taxed` gathers, by grant, the
/// tax the paid draws carry. Returns the draws.
pub(crate) fn draw_owed(
    owed: &[UsageOwed],
    left: &mut [i64],
    (subscription_id, dated_before): (SubscriptionId, DateTime<Utc>),
    sets: &[&[GrantCandidate]],
    pools: &[GrantPool],
    at: DateTime<Utc>,
    taxed: &mut HashMap<CreditGrantId, i64>,
) -> StoreResult<Vec<(CreditGrantId, i64)>> {
    let mut plan = Allocation::default();
    for (set, pool) in sets
        .iter()
        .flat_map(|set| pools.iter().map(move |pool| (*set, *pool)))
    {
        let asked = owed_request(owed, left, pool, subscription_id, dated_before);
        if asked.requested_minor <= 0 {
            continue;
        }
        let drawn = allocate(set, &asked, at, pool)?;
        if pool == GrantPool::Paid {
            carried_tax(owed, left, &drawn, taxed)?;
        }
        pay_owed(owed, left, pool, &drawn);
        plan = plan.then(Allocation {
            draws: drawn.draws,
            covered: Vec::new(),
            placed: Vec::new(),
        })?;
    }
    Ok(plan.draws)
}

fn carried_tax(
    owed: &[UsageOwed],
    left: &[i64],
    drawn: &Allocation,
    taxed: &mut HashMap<CreditGrantId, i64>,
) -> StoreResult<()> {
    let full = UsageOwed::invoiced_together(owed, left);
    let tax = owed
        .iter()
        .zip(left)
        .enumerate()
        .try_fold(0_i64, |sum, (index, (owed, base))| {
            sum.checked_add(owed.tax_in_within(
                *base,
                drawn.covered.get(index).copied().unwrap_or(0),
                full[index],
            ))
        })
        .ok_or_else(overflow)?;
    let total = drawn.total()?;
    if tax == 0 || total <= 0 {
        return Ok(());
    }
    let (mut cumulative, mut assigned) = (0_i128, 0_i64);
    for (grant_id, amount) in &drawn.draws {
        cumulative += i128::from(*amount);
        let share = i64::try_from(cumulative * i128::from(tax) / i128::from(total))
            .map_err(|_| overflow())?;
        *taxed.entry(*grant_id).or_default() += share - assigned;
        assigned = share;
    }
    Ok(())
}

/// The usage a settlement pays: one subscription period, up to an event-time bound.
#[derive(Debug, Clone)]
pub(crate) struct UsageSettlementTerms {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: SubscriptionId,
    pub period_start: NaiveDate,
    /// When its usage lines end (an immediate cancellation's instant on its last day): only
    /// credit effective before it pays them, as the invoice draws.
    pub dated_before: DateTime<Utc>,
    pub bound: DateTime<Utc>,
}

/// The products each grant pays for, sorted; a grant left out pays for every product. For
/// read-only views over many grants: a draw reads them with its grants (`ScopedGrantRow`).
pub type GrantProducts = HashMap<CreditGrantId, Vec<ProductId>>;

pub async fn grant_products_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    grant_ids: &[CreditGrantId],
) -> StoreResult<GrantProducts> {
    let mut products = GrantProducts::new();
    for row in CustomerCreditGrantProductRow::list_by_grants(conn, tenant_id, grant_ids).await? {
        products
            .entry(row.grant_id)
            .or_default()
            .push(row.product_id);
    }
    for products in products.values_mut() {
        products.sort_by_key(|id| id.as_uuid());
    }
    Ok(products)
}

/// What a stored grant pays for.
pub(crate) fn grant_scope(
    grant: &CustomerCreditGrantRow,
    product_ids: &[ProductId],
) -> StoreResult<GrantScope> {
    Ok(GrantScope {
        subscription_id: grant.subscription_id,
        application: CreditApplication::from_db_str(&grant.application)?,
        product_ids: product_ids.to_vec(),
    })
}

pub fn candidate(scoped: &ScopedGrantRow) -> StoreResult<GrantCandidate> {
    let grant = &scoped.grant;
    Ok(GrantCandidate {
        id: grant.id,
        scope: grant_scope(grant, &scoped.product_ids)?,
        remaining_minor: grant.remaining,
        priority: grant.priority,
        classification: GrantClassification::from_db_str(&grant.classification)?,
        effective_at: grant.effective_at,
        pays_usage_from: grant.usage_start(),
        lapses_at: grant.lapses_at,
        created_at: grant.created_at,
    })
}

/// What a give-back returns to its grant: draws it paid (a void, a note, a returned settlement),
/// or funding a payment reversal took back. Restored funding lapses as the funding itself does: at
/// the sweep for expired credit, which settles the period's usage from it first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GivenBack {
    Draws,
    Funding,
}

/// `amount` given back at `now` (the business moment of the give-back) to a grant that lapsed by
/// then (ended, or for draws also expired) lapses once the settlement grace after its lapse has
/// passed; before, the sweep settles the usage dated earlier from it first.
async fn lapse_given_back_tx(
    conn: &mut PgConn,
    grant: CustomerCreditGrantRow,
    amount: i64,
    given_back: GivenBack,
    now: DateTime<Utc>,
) -> StoreResult<CustomerCreditGrantRow> {
    if grant.remaining <= 0 || amount <= 0 {
        return Ok(grant);
    }
    let ended = crate::services::credit::mint::grant_ended_at_tx(conn, &grant).await?;
    let ended_on_grant = grant.ends_at.filter(|_| ended_first(&grant));
    let lapses_at = match given_back {
        GivenBack::Draws => grant.lapses_at,
        GivenBack::Funding => ended_on_grant,
    };
    let Some(lapsed) = [lapses_at, ended]
        .into_iter()
        .flatten()
        .min()
        .filter(|lapsed| *lapsed <= now)
    else {
        return Ok(grant);
    };
    let grace = customer_settlement_grace_tx(conn, grant.tenant_id, grant.customer_id).await?;
    if lapsed + grace > now || awaits_rollover_tx(conn, &grant).await? {
        return Ok(grant);
    }
    let amount = amount.min(grant.remaining);
    if lapses_at.is_some_and(|lapses| lapses <= now) {
        return expire_value_tx(conn, &grant, amount).await;
    }
    // Ended by a plan change read from its line (`grant_ended_at_tx`), not recorded on the grant.
    let lapsed_at = not_before_last_movement_tx(conn, grant.id, lapsed).await?;
    let updated = CustomerCreditGrantRow::move_remaining(conn, grant.id, -amount).await?;
    record_entry(
        conn,
        &updated,
        Movement::document(
            LedgerEntryKind::Expiry,
            -amount,
            Uuid::now_v7(),
            lapsed_at,
            None,
            None,
        )
        .because(crate::domain::credit::PLAN_CHANGE_REMOVAL),
    )
    .await?;
    Ok(updated)
}

fn candidates(grants: &[ScopedGrantRow]) -> StoreResult<Vec<GrantCandidate>> {
    grants.iter().map(candidate).collect()
}

fn pays(entry: &CustomerCreditEntryRow, source: DrawSource) -> bool {
    entry.source_kind.as_deref() == Some(source.as_db_str())
}

fn signed_sum<'a>(
    entries: impl Iterator<Item = &'a CustomerCreditEntryRow>,
    kinds: &[LedgerEntryKind],
) -> StoreResult<i64> {
    entries
        .filter(|e| kinds.iter().any(|kind| kind.as_db_str() == e.kind))
        .try_fold(0_i64, |sum, e| sum.checked_sub(e.amount))
        .ok_or_else(overflow)
}

/// What a promotional draw may still take once the paid settlements the invoice keeps are placed
/// ([`after_paid_settlements`]): usage they paid is never paid again by promotional credit and
/// their grant never lapses it.
fn ahead_of(
    pool: GrantPool,
    request: &CreditRequest,
    charges: &[UsageCharge],
    paid_settled: &[(GrantCandidate, i64)],
) -> StoreResult<CreditRequest> {
    if pool != GrantPool::Promotional {
        return Ok(request.clone());
    }
    after_paid_settlements(request, charges, paid_settled)
}

/// What each grant of `pool` among `movements` (one invoice's) holds for `draw_source`, net of
/// what was given back, with the grant as a candidate.
fn held_in_pool(
    movements: &[CustomerCreditEntryRow],
    draw_source: DrawSource,
    grants: &[ScopedGrantRow],
    pool: GrantPool,
) -> StoreResult<Vec<(GrantCandidate, i64)>> {
    let mut held = Vec::new();
    for (grant_id, source, amount) in held_by_grant(movements)? {
        if source != draw_source || amount <= 0 {
            continue;
        }
        let scoped = grants
            .iter()
            .find(|scoped| scoped.grant.id == grant_id)
            .ok_or_else(|| precondition("customer credit grant is missing"))?;
        let candidate = candidate(scoped)?;
        if pool.admits(candidate.classification) {
            held.push((candidate, amount));
        }
    }
    Ok(held)
}

/// Applies credit of `pool` to an invoice, whole minor units only. The caller holds the invoice
/// lock; a replay returns the credit already drawn from that pool instead of drawing again.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn draw_for_invoice_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    invoice_id: InvoiceId,
    currency: &str,
    pool: GrantPool,
    request: &CreditRequest,
    time: MovementTime,
) -> StoreResult<Allocation> {
    let (tenant_id, customer_id) = (lock.tenant_id(), lock.customer_id());
    let currency = normalized_currency(currency);
    let movements =
        CustomerCreditEntryRow::list_by_invoice_in_currency(conn, tenant_id, invoice_id, &currency)
            .await?
            .into_iter()
            .filter(|e| pays(e, DrawSource::Invoice))
            .collect::<Vec<_>>();
    if movements
        .iter()
        .any(|e| e.kind == LedgerEntryKind::DrawReversal.as_db_str())
    {
        return Err(precondition(
            "this invoice's credit was already returned; it cannot draw again",
        ));
    }
    if !movements.is_empty() {
        let mut ids = movements.iter().map(|e| e.grant_id).collect::<Vec<_>>();
        ids.sort_by_key(|id| id.as_uuid());
        ids.dedup();
        let drew = CustomerCreditGrantRow::list_by_ids_scoped(conn, tenant_id, &ids).await?;
        let held = held_in_pool(&movements, DrawSource::Invoice, &drew, pool)?;
        if !held.is_empty() {
            return assign(&held, request);
        }
    }
    if request.requested_minor <= 0 {
        return Ok(Allocation::default());
    }
    let Some(account) =
        CustomerCreditAccountRow::find_by_currency(conn, tenant_id, customer_id, &currency).await?
    else {
        return Ok(Allocation::default());
    };
    // Another subscription's credit could pay nothing here: it stays unlocked.
    let grants = CustomerCreditGrantRow::lock_drawable(
        conn,
        lock,
        account.id,
        demand_subscriptions(request),
    )
    .await?;
    let allocation = allocate(&candidates(&grants)?, request, time.at, pool)?;
    let mut touched = Vec::with_capacity(allocation.draws.len());
    for (grant_id, amount) in &allocation.draws {
        let drawn_at = not_before_last_movement_tx(conn, *grant_id, time.at).await?;
        let grant = CustomerCreditGrantRow::move_remaining(conn, *grant_id, -amount).await?;
        touched.push(grant.account_id);
        record_entry(
            conn,
            &grant,
            Movement::document(
                LedgerEntryKind::Draw,
                -amount,
                invoice_id.as_uuid(),
                drawn_at,
                Some(invoice_id),
                None,
            )
            .paying(DrawSource::Invoice),
        )
        .await?;
    }
    balance_changed_tx(conn, controls, tenant_id, &touched, time.now).await?;
    Ok(allocation)
}

/// Customer-wide grants and those of the subscriptions `request` names: all that can pay it.
fn demand_subscriptions(request: &CreditRequest) -> SubscriptionFilter {
    let mut subscriptions = request
        .demands
        .iter()
        .filter_map(|demand| demand.subscription_id)
        .collect::<Vec<_>>();
    subscriptions.sort_by_key(|id| id.as_uuid());
    subscriptions.dedup();
    SubscriptionFilter::CustomerWideOr(subscriptions)
}

/// Source of the returns of settled credit an invoice did not need, distinct from the void's own.
fn excess_source(invoice_id: InvoiceId) -> Uuid {
    Uuid::new_v5(&invoice_id.as_uuid(), b"usage-settlement-excess")
}

/// The `(subscription, period start)` usage periods an invoice bills, whose settlements it takes
/// over: several subscriptions' for a consolidated invoice.
#[derive(Debug, Clone)]
pub(crate) struct BilledPeriods {
    pub periods: Vec<(SubscriptionId, NaiveDate)>,
}

/// Finishes the `pool` credit of an invoice that bills subscription periods, under the grant
/// locks the caller took before deciding anything (`lock_grants_tx`). The invoice takes over the
/// usage settlements of those periods, keeps what the request can use of the ones `pool` paid and
/// returns the rest to the grants they came from; only the remainder of the request is drawn.
/// Returns what the invoice now holds from `pool`. The caller holds the invoice lock.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn net_settlements_then_draw_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    invoice_id: InvoiceId,
    currency: &str,
    grants: &LockedGrants,
    periods: &BilledPeriods,
    pool: GrantPool,
    request: &CreditRequest,
    charges: &[UsageCharge],
    time: MovementTime,
) -> StoreResult<Allocation> {
    let tenant_id = lock.tenant_id();
    let currency = normalized_currency(currency);
    let mut accounts = grants
        .iter()
        .map(|scoped| scoped.grant.account_id)
        .collect::<Vec<_>>();
    accounts.sort_by_key(|id| id.as_uuid());
    accounts.dedup();
    for (subscription_id, period_start) in &periods.periods {
        CreditUsageSettlementRow::attach_open_to_invoice(
            conn,
            tenant_id,
            *subscription_id,
            *period_start,
            &accounts,
            invoice_id,
        )
        .await?;
    }
    // Open settlements are in the live set, never cached: a ledger change, not an inputs one.
    diesel_models::query::credit_inputs::record_ledger_change(conn, lock).await?;
    // Only this denomination's movements: the invoice may also hold credit of another one.
    let movements = CustomerCreditEntryRow::list_by_invoice(conn, tenant_id, invoice_id)
        .await?
        .into_iter()
        .filter(|e| grants.iter().any(|scoped| scoped.grant.id == e.grant_id))
        .collect::<Vec<_>>();
    let settled = held_in_pool(&movements, DrawSource::UsageSettlement, grants, pool)?;
    let drawn = held_in_pool(&movements, DrawSource::Invoice, grants, pool)?;
    let request = &ahead_of(
        pool,
        request,
        charges,
        &held_in_pool(
            &movements,
            DrawSource::UsageSettlement,
            grants,
            GrantPool::Paid,
        )?,
    )?;
    if !drawn.is_empty() {
        let mut held = settled;
        for (grant, amount) in drawn {
            match held.iter_mut().find(|(kept, _)| kept.id == grant.id) {
                Some((_, kept)) => *kept = kept.checked_add(amount).ok_or_else(overflow)?,
                None => held.push((grant, amount)),
            }
        }
        return assign(&held, request);
    }

    // A settlement paid usage dated before its grant lapsed: it keeps only what the invoice's
    // usage asks (units bought since may cover more of it), never a later fee; the rest goes back.
    let usage_only = CreditRequest {
        requested_minor: request
            .demands
            .iter()
            .filter(|demand| demand.usage)
            .try_fold(0_i64, |sum, demand| {
                sum.checked_add(demand.amount_minor.max(0))
            })
            .ok_or_else(overflow)?
            .min(request.requested_minor.max(0)),
        demands: request
            .demands
            .iter()
            .map(|demand| CreditDemand {
                amount_minor: if demand.usage { demand.amount_minor } else { 0 },
                ..*demand
            })
            .collect(),
    };
    let kept = assign(&settled, &usage_only)?;
    let mut touched = Vec::new();
    let mut given_to = Vec::new();
    for (grant, held) in &settled {
        let keeps = kept
            .draws
            .iter()
            .find(|(id, _)| *id == grant.id)
            .map_or(0, |(_, amount)| *amount);
        let amount = held - keeps;
        if amount <= 0 {
            continue;
        }
        let grant = CustomerCreditGrantRow::move_remaining(conn, grant.id, amount).await?;
        record_entry(
            conn,
            &grant,
            Movement::document(
                LedgerEntryKind::DrawReversal,
                amount,
                excess_source(invoice_id),
                time.at,
                Some(invoice_id),
                None,
            )
            .paying(DrawSource::UsageSettlement),
        )
        .await?;
        let grant = lapse_given_back_tx(conn, grant, amount, GivenBack::Draws, time.at).await?;
        touched.push(grant.account_id);
        given_to.push(grant.id);
    }
    balance_changed_tx(conn, controls, tenant_id, &touched, time.now).await?;

    let fresh = draw_for_invoice_tx(
        conn,
        controls,
        lock,
        invoice_id,
        &currency,
        pool,
        &request.remainder(&kept)?,
        time,
    )
    .await?;
    crate::services::credit::follow_grants_given_back_tx(
        conn,
        controls,
        lock,
        given_to,
        Some(invoice_id),
    )
    .await?;
    kept.then(fresh)
}

/// Gives an open usage settlement back to the grants that paid it, lapsing what came back to a
/// grant already expired. None when an invoice took the settlement over, or it was returned
/// already. The caller holds the customer's grant locks (`lock_grants_tx`) and announces the
/// balance change of the returned wallet.
pub(crate) async fn return_settlement_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    settlement_id: UsageSettlementId,
    now: DateTime<Utc>,
) -> StoreResult<Option<(WalletId, i64)>> {
    let tenant_id = lock.tenant_id();
    let Some(settlement) =
        CreditUsageSettlementRow::mark_returned(conn, tenant_id, settlement_id).await?
    else {
        return Ok(None);
    };
    let source = give_back_source(settlement.id.as_uuid(), DrawSource::UsageSettlement);
    let draws =
        CustomerCreditEntryRow::list_settlement_draws(conn, tenant_id, &[settlement.id.as_uuid()])
            .await?;
    let mut returned = 0_i64;
    let mut given_to = Vec::new();
    for draw in draws {
        let amount = draw.amount.checked_neg().ok_or_else(overflow)?;
        returned = returned.checked_add(amount).ok_or_else(overflow)?;
        let grant = CustomerCreditGrantRow::move_remaining(conn, draw.grant_id, amount).await?;
        record_entry(
            conn,
            &grant,
            Movement::document(
                LedgerEntryKind::DrawReversal,
                amount,
                source,
                now,
                None,
                None,
            )
            .paying(DrawSource::UsageSettlement),
        )
        .await?;
        let grant = lapse_given_back_tx(conn, grant, amount, GivenBack::Draws, now).await?;
        given_to.push(grant.id);
    }
    crate::services::credit::follow_grants_given_back_tx(conn, controls, lock, given_to, None)
        .await?;
    Ok(Some((settlement.account_id, returned)))
}

/// The grants that paid usage settlements, each with what it still holds of them.
pub(crate) async fn settlement_holders_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    settlement_ids: &[Uuid],
) -> StoreResult<Vec<(GrantCandidate, i64)>> {
    if settlement_ids.is_empty() {
        return Ok(Vec::new());
    }
    let draws =
        CustomerCreditEntryRow::list_settlement_draws(conn, tenant_id, settlement_ids).await?;
    let mut ids = draws.iter().map(|e| e.grant_id).collect::<Vec<_>>();
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();
    let paid_by = CustomerCreditGrantRow::list_by_ids_scoped(conn, tenant_id, &ids).await?;
    held_in_pool(
        &draws,
        DrawSource::UsageSettlement,
        &paid_by,
        GrantPool::All,
    )
}

/// Read-only estimate of `draw_for_invoice_tx`, for drafts and previews.
pub(crate) async fn preview_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
    pool: GrantPool,
    request: &CreditRequest,
    now: DateTime<Utc>,
) -> StoreResult<Allocation> {
    if request.requested_minor <= 0 {
        return Ok(Allocation::default());
    }
    let currency = normalized_currency(currency);
    let Some(account) =
        CustomerCreditAccountRow::find_by_currency(conn, tenant_id, customer_id, &currency).await?
    else {
        return Ok(Allocation::default());
    };
    let grants = CustomerCreditGrantRow::list_drawable(
        conn,
        tenant_id,
        customer_id,
        account.id,
        demand_subscriptions(request),
    )
    .await?;
    allocate(&candidates(&grants)?, request, now, pool)
}

/// `preview_tx` for an estimate of a document finalizing at `now` (`estimate_candidates_tx`).
pub(crate) async fn preview_estimate_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
    pool: GrantPool,
    request: &CreditRequest,
    now: DateTime<Utc>,
) -> StoreResult<Allocation> {
    if request.requested_minor <= 0 {
        return Ok(Allocation::default());
    }
    let currency = normalized_currency(currency);
    let Some(account) =
        CustomerCreditAccountRow::find_by_currency(conn, tenant_id, customer_id, &currency).await?
    else {
        return Ok(Allocation::default());
    };
    let grants = CustomerCreditGrantRow::list_drawable(
        conn,
        tenant_id,
        customer_id,
        account.id,
        demand_subscriptions(request),
    )
    .await?;
    let candidates = estimate_candidates_tx(conn, tenant_id, &grants, now).await?;
    allocate(&candidates, request, now, pool)
}

/// The candidates of `grants`, in order, for estimating a document finalizing at `at`: expired
/// credit not rolled over yet stands in for what it rolls over to, as finalizing rolls it first
/// (`settle_due_expiries_tx`). The usage it pays before its expiry is unknown: the cap stays.
pub(crate) async fn estimate_candidates_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    grants: &[ScopedGrantRow],
    at: DateTime<Utc>,
) -> StoreResult<Vec<GrantCandidate>> {
    let expired = grants
        .iter()
        .filter(|scoped| {
            scoped.grant.rollover.is_some()
                && !ended_first(&scoped.grant)
                && scoped.grant.expires_at.is_some_and(|expiry| expiry <= at)
        })
        .map(|scoped| scoped.grant.id)
        .collect::<Vec<_>>();
    let rolled = if expired.is_empty() {
        Vec::new()
    } else {
        CustomerCreditGrantRow::list_rolled_over(conn, tenant_id, &expired).await?
    };
    grants
        .iter()
        .map(|scoped| {
            let candidate = candidate(scoped)?;
            let pending = scoped.grant.rollover.as_ref().filter(|_| {
                expired.contains(&scoped.grant.id) && !rolled.contains(&scoped.grant.id)
            });
            Ok(match pending {
                Some(rollover) => {
                    rollover_stand_in(&candidate, &GrantRollover::from_json(rollover)?, 0, at)
                        .unwrap_or(candidate)
                }
                None => candidate,
            })
        })
        .collect()
}

/// Read-only estimate of what finalizing an invoice billing `periods` would hold from `pool`: the
/// open settlements of those periods it would keep, then fresh draws on the rest.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn estimate_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
    invoice_id: Option<InvoiceId>,
    periods: &[(SubscriptionId, NaiveDate)],
    pool: GrantPool,
    request: &CreditRequest,
    charges: &[UsageCharge],
    now: DateTime<Utc>,
) -> StoreResult<Allocation> {
    if request.requested_minor <= 0 {
        return Ok(Allocation::default());
    }
    let (ahead, kept) = estimate_kept_tx(
        conn, tenant_id, currency, invoice_id, periods, pool, request, charges,
    )
    .await?;
    let fresh = preview_estimate_tx(
        conn,
        tenant_id,
        customer_id,
        currency,
        pool,
        &ahead.remainder(&kept)?,
        now,
    )
    .await?;
    kept.then(fresh)
}

/// The open settlements of `periods` an invoice finalizing would keep from `pool`, and what the
/// request still asks of fresh draws once they are placed.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn estimate_kept_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    currency: &str,
    invoice_id: Option<InvoiceId>,
    periods: &[(SubscriptionId, NaiveDate)],
    pool: GrantPool,
    request: &CreditRequest,
    charges: &[UsageCharge],
) -> StoreResult<(CreditRequest, Allocation)> {
    let currency = normalized_currency(currency);
    let mut periods = periods.to_vec();
    periods.sort_by_key(|(subscription_id, start)| (subscription_id.as_uuid(), *start));
    periods.dedup();
    let mut settlements = Vec::new();
    for (subscription_id, period_start) in periods {
        for settlement in CreditUsageSettlementRow::list_for_period(
            conn,
            tenant_id,
            subscription_id,
            period_start,
            &currency,
        )
        .await?
        {
            if settlement.invoice_id.is_none() || settlement.invoice_id == invoice_id {
                settlements.push(settlement.id.as_uuid());
            }
        }
    }
    Ok(if settlements.is_empty() {
        (request.clone(), Allocation::default())
    } else {
        let draws =
            CustomerCreditEntryRow::list_settlement_draws(conn, tenant_id, &settlements).await?;
        let mut ids = draws.iter().map(|e| e.grant_id).collect::<Vec<_>>();
        ids.sort_by_key(|id| id.as_uuid());
        ids.dedup();
        let paid_by = CustomerCreditGrantRow::list_by_ids_scoped(conn, tenant_id, &ids).await?;
        let paid_settled = held_in_pool(
            &draws,
            DrawSource::UsageSettlement,
            &paid_by,
            GrantPool::Paid,
        )?;
        let ahead = ahead_of(pool, request, charges, &paid_settled)?;
        let kept = assign(
            &held_in_pool(&draws, DrawSource::UsageSettlement, &paid_by, pool)?,
            &ahead,
        )?;
        (ahead, kept)
    })
}

/// Credit the invoice holds, by grant: its draws and the usage settlements it took over, net of
/// what voids and credit notes gave back, most recently drawn first.
pub(crate) async fn invoice_credit_by_grant_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
) -> StoreResult<Vec<(CreditGrantId, i64)>> {
    let movements = CustomerCreditEntryRow::list_by_invoice(conn, tenant_id, invoice_id).await?;
    let mut held: Vec<(CreditGrantId, i64)> = Vec::new();
    for (grant_id, _, amount) in held_by_grant(&movements)? {
        match held.iter_mut().find(|(id, _)| *id == grant_id) {
            Some(slot) => slot.1 = slot.1.checked_add(amount).ok_or_else(overflow)?,
            None => held.push((grant_id, amount)),
        }
    }
    held.retain(|(_, amount)| *amount != 0);
    Ok(held)
}

/// Per grant and what it paid for, credit the invoice still holds: its draws net of what voids
/// and credit notes gave back, most recently drawn first.
fn held_by_grant(
    movements: &[CustomerCreditEntryRow],
) -> StoreResult<Vec<(CreditGrantId, DrawSource, i64)>> {
    let mut held: Vec<(CreditGrantId, DrawSource, i64, usize)> = Vec::new();
    for (position, entry) in movements.iter().enumerate() {
        let kind = LedgerEntryKind::from_db_str(&entry.kind)?;
        if !matches!(kind, LedgerEntryKind::Draw | LedgerEntryKind::DrawReversal) {
            continue;
        }
        let draw_source = entry
            .source_kind
            .as_deref()
            .map(DrawSource::from_db_str)
            .transpose()?
            .ok_or_else(|| precondition("a credit draw does not say what it paid for"))?;
        let index = match held
            .iter()
            .position(|(id, source, _, _)| *id == entry.grant_id && *source == draw_source)
        {
            Some(index) => index,
            None => {
                held.push((entry.grant_id, draw_source, 0, position));
                held.len() - 1
            }
        };
        let slot = &mut held[index];
        slot.2 = slot.2.checked_sub(entry.amount).ok_or_else(overflow)?;
        if kind == LedgerEntryKind::Draw {
            slot.3 = position;
        }
    }
    held.sort_by_key(|entry| std::cmp::Reverse(entry.3));
    Ok(held
        .into_iter()
        .map(|(id, source, amount, _)| (id, source, amount))
        .collect())
}

/// The source of the give-back of credit an invoice took from a grant for `draw_source`: distinct
/// per purpose, as a grant records one reversal per source.
pub fn give_back_source(source_id: Uuid, draw_source: DrawSource) -> Uuid {
    match draw_source {
        DrawSource::UsageSettlement => {
            diesel_models::customer_credit::usage_settlement_give_back_source(source_id)
        }
        DrawSource::Invoice | DrawSource::Consume => source_id,
    }
}

fn is_note_return(entry: &CustomerCreditEntryRow, note_source: Uuid) -> bool {
    entry.source_id == note_source
        || entry.source_id == give_back_source(note_source, DrawSource::UsageSettlement)
}

/// Moves `amount` back into a grant the caller locked, as an invoice's credit given back.
async fn give_back_tx(
    conn: &mut PgConn,
    grant_id: CreditGrantId,
    amount: i64,
    source_id: Uuid,
    draw_source: DrawSource,
    invoice_id: InvoiceId,
    credit_note_id: Option<CreditNoteId>,
    now: DateTime<Utc>,
) -> StoreResult<CustomerCreditGrantRow> {
    // Never before the draw it returns, as a draw is never before the grant's latest movement.
    let dated = not_before_last_movement_tx(conn, grant_id, now).await?;
    let grant = CustomerCreditGrantRow::move_remaining(conn, grant_id, amount).await?;
    record_entry(
        conn,
        &grant,
        Movement::document(
            LedgerEntryKind::DrawReversal,
            amount,
            give_back_source(source_id, draw_source),
            dated,
            Some(invoice_id),
            credit_note_id,
        )
        .paying(draw_source),
    )
    .await?;
    lapse_given_back_tx(conn, grant, amount, GivenBack::Draws, now).await
}

/// Returns the credit a voided invoice still holds to the grants it came from, credit units
/// included. Returns what finalization drew in `currency`, the invoice's, `None` when the invoice
/// never drew ledger credit in it.
pub(crate) async fn reverse_invoice_draws_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    invoice_id: InvoiceId,
    currency: &str,
    now: DateTime<Utc>,
) -> StoreResult<Option<i64>> {
    let tenant_id = lock.tenant_id();
    let movements = CustomerCreditEntryRow::list_by_invoice(conn, tenant_id, invoice_id).await?;
    let source = invoice_id.as_uuid();
    let is_original = |e: &CustomerCreditEntryRow| {
        e.kind == LedgerEntryKind::Draw.as_db_str()
            && e.credit_note_id.is_none()
            && (e.source_id == source || pays(e, DrawSource::UsageSettlement))
    };
    if !movements.iter().any(is_original) {
        return Ok(None);
    }
    let in_currency = CustomerCreditEntryRow::list_by_invoice_in_currency(
        conn,
        tenant_id,
        invoice_id,
        &normalized_currency(currency),
    )
    .await?;
    let original = in_currency
        .iter()
        .filter(|e| is_original(e))
        .collect::<Vec<_>>();
    let excess = excess_source(invoice_id);
    let returned_as_excess = in_currency
        .iter()
        .filter(|e| e.kind == LedgerEntryKind::DrawReversal.as_db_str() && e.source_id == excess)
        .try_fold(0_i64, |sum, e| sum.checked_add(e.amount))
        .ok_or_else(overflow)?;
    let drawn = (!original.is_empty())
        .then(|| {
            original
                .iter()
                .try_fold(0_i64, |sum, e| sum.checked_sub(e.amount))
                .and_then(|sum| sum.checked_sub(returned_as_excess))
                .ok_or_else(overflow)
        })
        .transpose()?;
    let held = held_by_grant(&movements)?;
    let mut ids = held.iter().map(|(id, _, _)| *id).collect::<Vec<_>>();
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();
    CustomerCreditGrantRow::lock_by_ids(conn, lock, &ids).await?;
    let mut touched = Vec::new();
    for (grant_id, draw_source, amount) in held {
        let give_source = give_back_source(source, draw_source);
        let already = movements.iter().any(|e| {
            e.kind == LedgerEntryKind::DrawReversal.as_db_str()
                && e.grant_id == grant_id
                && e.source_id == give_source
        });
        if amount <= 0 || already {
            continue;
        }
        touched.push(
            give_back_tx(
                conn,
                grant_id,
                amount,
                source,
                draw_source,
                invoice_id,
                None,
                now,
            )
            .await?
            .account_id,
        );
    }
    balance_changed_tx(conn, controls, tenant_id, &touched, now).await?;
    Ok(drawn)
}

/// What a finalized note gave back to the invoice's grants of `pool`, as the ledger booked it.
pub(crate) async fn returned_by_note(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
    currency: &str,
    credit_note_id: CreditNoteId,
    pool: GrantPool,
) -> StoreResult<i64> {
    let source = credit_note_id.as_uuid();
    let returns = CustomerCreditEntryRow::list_by_invoice_in_currency(
        conn,
        tenant_id,
        invoice_id,
        &normalized_currency(currency),
    )
    .await?
    .into_iter()
    .filter(|e| e.kind == LedgerEntryKind::DrawReversal.as_db_str() && is_note_return(e, source))
    .collect::<Vec<_>>();
    let mut ids = returns.iter().map(|e| e.grant_id).collect::<Vec<_>>();
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();
    let mut in_pool = Vec::new();
    for scoped in CustomerCreditGrantRow::list_by_ids_scoped(conn, tenant_id, &ids).await? {
        if pool.admits(GrantClassification::from_db_str(
            &scoped.grant.classification,
        )?) {
            in_pool.push(scoped.grant.id);
        }
    }
    returns
        .iter()
        .filter(|e| in_pool.contains(&e.grant_id))
        .try_fold(0_i64, |sum, e| sum.checked_add(e.amount))
        .ok_or_else(overflow)
}

/// Gives the `currency` credit of `pool` an invoice holds back to the grants it was drawn from,
/// keeping their terms, for a credit note. `request` holds the credited charges: a grant gets back
/// only what it can pay of them (its products, usage or not), and among grants that can pay the
/// same charges the latest drawn comes back first. Credit units an invoice drew stay drawn: a note
/// credits money. Returns what was given back, and how it was placed on the credited charges
/// (empty on a replay, which gives nothing more).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn return_draws_for_note_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    invoice_id: InvoiceId,
    currency: &str,
    credit_note_id: CreditNoteId,
    pool: GrantPool,
    request: &CreditRequest,
    now: DateTime<Utc>,
) -> StoreResult<(i64, Allocation)> {
    let tenant_id = lock.tenant_id();
    let movements = CustomerCreditEntryRow::list_by_invoice_in_currency(
        conn,
        tenant_id,
        invoice_id,
        &normalized_currency(currency),
    )
    .await?;
    let mut ids = movements.iter().map(|e| e.grant_id).collect::<Vec<_>>();
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();
    let mut in_pool = Vec::new();
    for grant in CustomerCreditGrantRow::lock_by_ids(conn, lock, &ids).await? {
        if pool.admits(GrantClassification::from_db_str(&grant.classification)?) {
            in_pool.push(grant.id);
        }
    }
    let movements = movements
        .into_iter()
        .filter(|e| in_pool.contains(&e.grant_id))
        .collect::<Vec<_>>();
    let source = credit_note_id.as_uuid();
    let replayed = movements
        .iter()
        .filter(|e| {
            e.kind == LedgerEntryKind::DrawReversal.as_db_str() && is_note_return(e, source)
        })
        .try_fold(0_i64, |sum, e| sum.checked_add(e.amount))
        .ok_or_else(overflow)?;
    if replayed > 0 || request.requested_minor <= 0 {
        return Ok((replayed, Allocation::default()));
    }
    let holds = held_by_grant(&movements)?
        .into_iter()
        .filter(|(_, _, amount)| *amount > 0)
        .collect::<Vec<_>>();
    let holders = holds.iter().map(|(id, _, _)| *id).collect::<Vec<_>>();
    let grants = CustomerCreditGrantRow::list_by_ids_scoped(conn, tenant_id, &holders).await?;
    // The rank of a grant's latest draw stands in for its priority, so placement on the credited
    // charges takes the latest drawn first among equally restricted grants.
    let mut held: Vec<(GrantCandidate, i64)> = Vec::new();
    for (grant_id, _, amount) in &holds {
        if let Some((_, total)) = held.iter_mut().find(|(grant, _)| grant.id == *grant_id) {
            *total = total.checked_add(*amount).ok_or_else(overflow)?;
            continue;
        }
        let scoped = grants
            .iter()
            .find(|scoped| scoped.grant.id == *grant_id)
            .ok_or_else(|| precondition("customer credit grant is missing"))?;
        let mut candidate = candidate(scoped)?;
        candidate.priority = i32::try_from(held.len()).map_err(|_| overflow())?;
        held.push((candidate, *amount));
    }
    let allocation = assign(&held, request)?;
    let mut placed = allocation.draws.clone();
    let (mut given, mut touched) = (0_i64, Vec::new());
    for (grant_id, draw_source, amount) in holds {
        let Some((_, left)) = placed.iter_mut().find(|(id, _)| *id == grant_id) else {
            continue;
        };
        let amount = amount.min(*left);
        if amount <= 0 {
            continue;
        }
        *left -= amount;
        let grant = give_back_tx(
            conn,
            grant_id,
            amount,
            source,
            draw_source,
            invoice_id,
            Some(credit_note_id),
            now,
        )
        .await?;
        touched.push(grant.account_id);
        given = given.checked_add(amount).ok_or_else(overflow)?;
    }
    balance_changed_tx(conn, controls, tenant_id, &touched, now).await?;
    Ok((given, allocation))
}

/// Undoes `return_draws_for_note_tx` when its note is voided: the credit applies to the invoice
/// again. Refused once any of it is spent or has lapsed.
pub(crate) async fn redraw_note_returns_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    credit_note_id: CreditNoteId,
    movements: &[CustomerCreditEntryRow],
    now: DateTime<Utc>,
) -> StoreResult<()> {
    let tenant_id = lock.tenant_id();
    let source = credit_note_id.as_uuid();
    let returns = movements
        .iter()
        .filter(|e| {
            e.kind == LedgerEntryKind::DrawReversal.as_db_str() && is_note_return(e, source)
        })
        .collect::<Vec<_>>();
    if returns.is_empty() {
        return Ok(());
    }
    let mut ids = returns.iter().map(|e| e.grant_id).collect::<Vec<_>>();
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();
    let grants = CustomerCreditGrantRow::lock_by_ids(conn, lock, &ids).await?;
    let mut touched = Vec::new();
    for entry in returns {
        let redrawn = movements.iter().any(|e| {
            e.kind == LedgerEntryKind::Draw.as_db_str()
                && e.grant_id == entry.grant_id
                && e.source_id == entry.source_id
        });
        if redrawn {
            continue;
        }
        let grant = grants
            .iter()
            .find(|g| g.id == entry.grant_id)
            .ok_or_else(|| precondition("customer credit grant is missing"))?;
        if grant.remaining < entry.amount || grant.lapses_at.is_some_and(|at| at <= now) {
            return Err(precondition(
                "the credit this note gave back is already spent or has expired; it cannot be voided",
            ));
        }
        let amount = entry.amount.checked_neg().ok_or_else(overflow)?;
        let grant = CustomerCreditGrantRow::move_remaining(conn, grant.id, amount).await?;
        touched.push(grant.account_id);
        record_entry(
            conn,
            &grant,
            Movement::document(
                LedgerEntryKind::Draw,
                amount,
                entry.source_id,
                now,
                entry.invoice_id,
                Some(credit_note_id),
            )
            .paying(DrawSource::from_db_str(
                entry
                    .source_kind
                    .as_deref()
                    .ok_or_else(|| precondition("a credit draw does not say what it paid for"))?,
            )?),
        )
        .await?;
    }
    balance_changed_tx(conn, controls, tenant_id, &touched, now).await?;
    Ok(())
}

/// Whether finalization drew ledger credit in `currency` for this invoice.
pub(crate) async fn invoice_drew_credit_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
    currency: &str,
) -> StoreResult<bool> {
    let movements = CustomerCreditEntryRow::list_by_invoice_in_currency(
        conn,
        tenant_id,
        invoice_id,
        &normalized_currency(currency),
    )
    .await?;
    Ok(movements.iter().any(|e| {
        e.kind == LedgerEntryKind::Draw.as_db_str()
            && (e.source_id == invoice_id.as_uuid() || pays(e, DrawSource::UsageSettlement))
    }))
}

/// Net credit of one grant that invoices still hold.
pub(crate) async fn applied_to_invoices_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    grant_id: CreditGrantId,
) -> StoreResult<i64> {
    let movements = CustomerCreditEntryRow::list_by_grant(conn, tenant_id, grant_id).await?;
    signed_sum(
        movements.iter(),
        &[LedgerEntryKind::Draw, LedgerEntryKind::DrawReversal],
    )
}

#[derive(Debug, Clone)]
pub(crate) struct CustomerCreditBalance {
    pub account: CustomerCreditAccountRow,
    pub available_minor: i64,
    pub pending_minor: i64,
    /// Each grant with the value its recorded expiries took.
    pub grants: Vec<(CustomerCreditGrantRow, i64)>,
    pub products: GrantProducts,
}

/// Per account: its grants, what is spendable now and what takes effect later. Expired value is
/// excluded whether or not the sweep has recorded it, except what the sweep will roll over
/// (`available_minor_tx`).
pub(crate) async fn balances_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    now: DateTime<Utc>,
) -> StoreResult<Vec<CustomerCreditBalance>> {
    let accounts =
        CustomerCreditAccountRow::list_for_customer(conn, tenant_id, customer_id).await?;
    let grants = CustomerCreditGrantRow::list_for_customer(conn, tenant_id, customer_id).await?;
    let products = grant_products_tx(
        conn,
        tenant_id,
        &grants.iter().map(|grant| grant.id).collect::<Vec<_>>(),
    )
    .await?;
    let expiries = CustomerCreditEntryRow::list_amounts_for_customer(
        conn,
        tenant_id,
        customer_id,
        LedgerEntryKind::Expiry.as_db_str(),
    )
    .await?;
    let mut balances = accounts
        .into_iter()
        .map(|account| {
            let grants = grants
                .iter()
                .filter(|g| g.account_id == account.id)
                .map(|g| {
                    expiries
                        .iter()
                        .filter(|(grant_id, _)| *grant_id == g.id)
                        .try_fold(0_i64, |sum, (_, amount)| sum.checked_sub(*amount))
                        .map(|lapsed| (g.clone(), lapsed))
                        .ok_or_else(overflow)
                })
                .collect::<StoreResult<Vec<_>>>()?;
            let live = |g: &&CustomerCreditGrantRow| g.lapses_at.is_none_or(|at| at > now);
            let available_minor = grants
                .iter()
                .map(|(g, _)| g)
                .filter(live)
                .filter(|g| g.effective_at <= now)
                .try_fold(0_i64, |sum, g| sum.checked_add(g.remaining))
                .ok_or_else(overflow)?;
            let pending_minor = grants
                .iter()
                .map(|(g, _)| g)
                .filter(live)
                .filter(|g| g.effective_at > now)
                .try_fold(0_i64, |sum, g| sum.checked_add(g.remaining))
                .ok_or_else(overflow)?;
            let products = grants
                .iter()
                .filter_map(|(g, _)| Some((g.id, products.get(&g.id)?.clone())))
                .collect();
            Ok(CustomerCreditBalance {
                account,
                available_minor,
                pending_minor,
                grants,
                products,
            })
        })
        .collect::<StoreResult<Vec<_>>>()?;
    for balance in &mut balances {
        let awaits_rollover = balance.grants.iter().any(|(g, _)| {
            g.rollover.is_some()
                && !ended_first(g)
                && g.remaining > 0
                && g.expires_at.is_some_and(|at| at <= now)
        });
        if !awaits_rollover {
            continue;
        }
        for (_, carried) in unswept_rollovers_tx(conn, balance.account.id, now).await? {
            balance.available_minor = balance
                .available_minor
                .checked_add(carried)
                .ok_or_else(overflow)?;
        }
    }
    Ok(balances)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn vat(base_minor: i64) -> UsageOwed {
        UsageOwed {
            product_id: None,
            base_minor,
            tax_inclusive: false,
            taxes: vec![TaxDetail {
                tax_rate: rust_decimal::Decimal::new(20, 2),
                tax_name: "VAT".to_owned(),
                tax_amount: 0,
            }],
        }
    }

    /// Two products' usage at 20%, 0.07 and 0.06: settled alone each rounds its tax down (0.01 +
    /// 0.01), while their invoice rounds 20% once (0.026 → 0.03, the cent on the larger
    /// remainder). The settlement owes what the invoice will ask, 0.09 and 0.07 (OQ14).
    #[test]
    fn a_settlement_owes_each_product_its_share_of_the_tax_rounded_per_rate() {
        let owed = [vat(7), vat(6)];
        assert_eq!(owed[0].invoiced(7) + owed[1].invoiced(6), 15);
        let full = UsageOwed::invoiced_together(&owed, &[7, 6]);
        assert_eq!(full, vec![9, 7]);
        assert_eq!(owed[0].tax_in_within(7, 9, full[0]), 2);
        assert_eq!(owed[1].tax_in_within(6, 7, full[1]), 1);
        // One cent short of the invoiced figure never reads as all the usage paid.
        assert_eq!(owed[0].usage_paid_by_within(7, 8, full[0]), 6);
    }

    /// $54 of paid credit settling usage taxed 10% on top pays $49.09 of it and carries its $4.91
    /// of tax, split exactly over the grants that drew it; priced tax included, the tax inside.
    #[test]
    fn settled_paid_credit_carries_the_tax_of_the_usage_it_pays() {
        let ten = |base_minor| UsageOwed {
            taxes: vec![TaxDetail {
                tax_rate: rust_decimal::Decimal::new(10, 2),
                tax_name: "Sales tax".to_owned(),
                tax_amount: 0,
            }],
            ..vat(base_minor)
        };
        assert_eq!(ten(4_909).invoiced(4_909), 5_400);
        assert_eq!(ten(4_909).tax_in_within(4_909, 5_400, 5_400), 491);
        assert_eq!(ten(4_909).tax_in_within(4_909, 0, 5_400), 0);
        let included = UsageOwed {
            tax_inclusive: true,
            ..vat(12_000)
        };
        assert_eq!(included.tax_in_within(12_000, 12_000, 12_000), 2_000);

        let (first, second) = (CreditGrantId::new(), CreditGrantId::new());
        let drawn = Allocation {
            draws: vec![(first, 3_000), (second, 2_400)],
            covered: vec![5_400],
            placed: Vec::new(),
        };
        let mut taxed = HashMap::new();
        carried_tax(&[ten(4_909)], &[4_909], &drawn, &mut taxed).unwrap();
        assert_eq!((taxed[&first], taxed[&second]), (272, 219));
    }

    /// Every figure paid of an invoiced amount converts to the most usage it pays whole; what is
    /// left (3 is no invoiced figure at 20%: 2 invoices at 2, 3 at 4) is under one unit plus tax.
    #[test]
    fn paid_credit_pays_the_most_usage_its_invoiced_figure_covers() {
        let owed = vat(5_000);
        assert_eq!(owed.usage_paid_by(10, 3), 2);
        assert_eq!(owed.usage_paid_by(5_000, 3_003), 2_502);
        assert_eq!(owed.usage_paid_by(5_000, 6_000), 5_000);
        assert_eq!(owed.usage_paid_by(5_000, 9_000), 5_000);
        assert_eq!(owed.usage_paid_by(5_000, 0), 0);
        for paid in 0..=owed.invoiced(997) {
            let usage = owed.usage_paid_by(997, paid);
            assert!(owed.invoiced(usage) <= paid, "{paid}");
            assert!(usage == 997 || owed.invoiced(usage + 1) > paid, "{paid}");
            assert!(paid - owed.invoiced(usage) <= 1, "{paid}");
        }
    }

    /// 30.03 of paid credit settled on 50.00 of usage at 20% pays 25.02 of it: promotional
    /// credit may take the other 24.98, at most the share a capped demand allows.
    #[test]
    fn promotional_credit_takes_the_usage_paid_settlements_leave_before_tax() {
        let at = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let paid = GrantCandidate {
            id: CreditGrantId::new(),
            scope: GrantScope::customer_wide(),
            remaining_minor: 0,
            priority: 0,
            classification: GrantClassification::Paid,
            effective_at: at,
            pays_usage_from: at,
            lapses_at: Some(at),
            created_at: at,
        };
        let request = |amount_minor: i64| CreditRequest {
            requested_minor: amount_minor + 1_000,
            demands: vec![
                CreditDemand {
                    subscription_id: None,
                    product_id: None,
                    usage: false,
                    amount_minor: 1_000,
                    dated_before: None,
                },
                CreditDemand {
                    subscription_id: None,
                    product_id: None,
                    usage: true,
                    amount_minor,
                    dated_before: None,
                },
            ],
        };
        let charges = [UsageCharge {
            demand: 1,
            owed: vat(5_000),
        }];
        let left =
            after_paid_settlements(&request(5_000), &charges, &[(paid.clone(), 3_003)]).unwrap();
        assert_eq!(left.requested_minor, 3_498);
        assert_eq!(
            left.demands[0].amount_minor, 1_000,
            "never placed on fixed fees"
        );
        assert_eq!(left.demands[1].amount_minor, 2_498);
        let capped = after_paid_settlements(&request(2_000), &charges, &[(paid, 3_003)]).unwrap();
        assert_eq!(capped.demands[1].amount_minor, 2_000);
        assert_eq!(capped.requested_minor, 3_000);
    }

    fn paid(currency: &str, amount_minor: i64, cost: i64) -> CreditFunding {
        let at = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        CreditFunding {
            tenant_id: TenantId::new(),
            customer_id: CustomerId::new(),
            currency: currency.to_owned(),
            scope: GrantScope::subscription_usage(SubscriptionId::new()),
            amount_minor,
            classification: GrantClassification::Paid,
            priority: DEFAULT_PRIORITY,
            effective_at: at,
            expires_at: Some(at + Duration::days(365)),
            occurred_at: at,
            now: at,
            source_kind: source::CREDIT_PURCHASE,
            source_id: Uuid::now_v7(),
            cost: Some(CreditCost::at_face(cost, "USD")),
            rollover: None,
            invoice_id: None,
            line_local_id: Some("pack".to_owned()),
            credit_note_id: None,
            reason: "Credit purchased on a paid invoice".to_owned(),
            reference: None,
            created_by: None,
            pays_usage_from: None,
        }
    }

    /// $500 for $550: $500 of paid credit at its cost, $50 of bonus credit at none, same terms.
    #[test]
    fn paid_credit_below_face_splits_into_paid_and_bonus_credit() {
        let funding = paid("USD", 55_000, 50_000);
        let (main, bonus) = split_paid(funding.clone()).unwrap();
        let bonus = bonus.expect("bonus credit");
        assert_eq!(main.amount_minor, 50_000);
        assert_eq!(main.classification, GrantClassification::Paid);
        assert_eq!(main.source_id, funding.source_id);
        assert_eq!(bonus.amount_minor, 5_000);
        assert_eq!(bonus.classification, GrantClassification::Promotional);
        assert_eq!(bonus.cost, Some(CreditCost::given("USD")));
        assert_eq!(bonus.source_id, bonus_source(funding.source_id));
        assert_eq!(bonus.reason, BONUS_CREDIT);
        assert_eq!(
            bonus.scope, funding.scope,
            "the bonus belongs where its paid credit does"
        );
        assert_eq!(
            (bonus.expires_at, bonus.priority, bonus.line_local_id),
            (funding.expires_at, funding.priority, funding.line_local_id)
        );

        let (ninety_off, bonus) = split_paid(paid("USD", 55_000, 5_000)).unwrap();
        assert_eq!(
            (ninety_off.amount_minor, bonus.map(|b| b.amount_minor)),
            (5_000, Some(50_000))
        );
        let (free, none) = split_paid(paid("USD", 1_000, 0)).unwrap();
        assert_eq!(free.classification, GrantClassification::Promotional);
        assert!(none.is_none());
        let (at_face, none) = split_paid(paid("USD", 1_000, 1_000)).unwrap();
        assert_eq!(at_face.classification, GrantClassification::Paid);
        assert!(none.is_none());
        assert!(split_paid(paid("USD", 1_000, 1_200)).is_err());
        assert!(
            split_paid(paid("EUR", 1_000, 800)).is_err(),
            "cost in another currency"
        );
        let (units, none) = split_paid(paid("CREDITS", 5_000, 4_900)).unwrap();
        assert_eq!(units.amount_minor, 5_000, "credit units are unchanged");
        assert!(none.is_none());
    }

    fn stored_grant(at: DateTime<Utc>) -> CustomerCreditGrantRow {
        CustomerCreditGrantRow {
            id: CreditGrantId::new(),
            tenant_id: TenantId::new(),
            customer_id: CustomerId::new(),
            account_id: WalletId::new(),
            amount: 5_000,
            remaining: 1_500,
            priority: DEFAULT_PRIORITY,
            classification: "PROMOTIONAL".to_owned(),
            effective_at: at - Duration::days(30),
            expires_at: Some(at),
            source_kind: source::INCLUDED_CREDIT.to_owned(),
            source_id: Uuid::now_v7(),
            invoice_id: None,
            line_local_id: None,
            credit_note_id: None,
            reason: "Credit included for the subscription period".to_owned(),
            reference: None,
            created_by: None,
            created_at: at - Duration::days(30),
            cost_minor: Some(0),
            cost_currency: Some("USD".to_owned()),
            rollover: None,
            subscription_id: Some(SubscriptionId::new()),
            application: "USAGE".to_owned(),
            ends_at: None,
            end_reason: None,
            lapses_at: Some(at),
            pays_usage_from: None,
        }
    }

    #[test]
    fn rollover_inherits_subscription_application_and_products() {
        let at = Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap();
        let mut products = vec![ProductId::new(), ProductId::new()];
        products.sort_by_key(|id| id.as_uuid());
        let parent = ScopedGrantRow {
            grant: stored_grant(at),
            product_ids: products.clone(),
        };
        let rolled =
            rolled_over_funding(&parent, "USD", 1_000, at + Duration::days(30), at, at, None)
                .unwrap();
        assert_eq!(
            rolled.scope,
            GrantScope {
                subscription_id: parent.grant.subscription_id,
                application: CreditApplication::Usage,
                product_ids: products,
            }
        );
        assert_eq!(
            (rolled.amount_minor, rolled.priority, rolled.effective_at),
            (1_000, DEFAULT_PRIORITY - 1, at)
        );
        assert_eq!(rolled.source_id, parent.grant.id.as_uuid());
    }

    #[test]
    fn credit_ended_with_its_subscription_never_rolls_over() {
        let at = Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap();
        let mut grant = stored_grant(at);
        assert!(!ended_first(&grant));
        grant.ends_at = Some(at - Duration::days(10));
        assert!(ended_first(&grant), "ended before its expiry");
        grant.ends_at = Some(at);
        assert!(ended_first(&grant), "ended with its expiry");
        grant.ends_at = Some(at + Duration::days(1));
        assert!(
            !ended_first(&grant),
            "expired first: it rolls over as usual"
        );
    }
}
