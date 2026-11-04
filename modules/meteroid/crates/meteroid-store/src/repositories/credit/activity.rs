//! Dashboard reads of a customer's credit: how each grant was consumed, what each wallet added,
//! used and lost over a window, and the statement labelled by where each grant came from.
use crate::domain::credit::{
    CreditGrant, CreditGrantSource, CreditGrantState, CreditLedgerEntry, CreditWallet,
    GrantClassification,
};
use crate::domain::credit_units::CreditUnit;
use crate::errors::StoreError;
use crate::repositories::credit::CustomerCreditWalletsInterface;
use crate::repositories::credit::customer_credit_wallets::{rolling_tx, statement_entry};
use crate::{Store, StoreResult};
use chrono::{DateTime, Utc};
use common_domain::ids::{
    BaseId, CreditGrantId, CreditUnitId, CustomerId, SubscriptionId, TenantId, WalletId,
};
use diesel_models::credit_units::CreditUnitRow;
use diesel_models::customer_credit::CustomerCreditAccountRow;
use diesel_models::query::customer_credit_activity::{
    CreditAccountActivityRow, CreditAddedGroupRow, CreditGrantActivityRow,
    CustomerCreditLabelledStatementRow,
};
use error_stack::Report;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct WalletActivity {
    pub wallet_id: WalletId,
    /// Funded, rolled-over credit excepted.
    pub credit_added_minor: i64,
    pub credit_added_groups: Vec<CreditAddedGroup>,
    /// Applied to invoices, net of what was given back.
    pub invoiced_minor: i64,
    /// Paid for usage at expiry, before an invoice took that usage over.
    pub settled_at_expiry_minor: i64,
    pub expired_minor: i64,
    pub removed_minor: i64,
    /// The soonest expiry of spendable credit.
    pub next_expiry: Option<DateTime<Utc>>,
    pub grants: Vec<GrantActivity>,
}

#[derive(Debug, Clone)]
pub struct CreditAddedGroup {
    pub source: CreditGrantSource,
    pub classification: GrantClassification,
    pub added_by_member: bool,
    pub name: String,
    pub grants: u32,
    pub amount_minor: i64,
}

#[derive(Debug, Clone)]
pub struct GrantActivity {
    pub grant_id: CreditGrantId,
    /// The invoice line that funded it, else its reason.
    pub name: String,
    pub invoiced_minor: i64,
    pub settled_at_expiry_minor: i64,
    /// Usage not yet invoiced the grant is projected to pay.
    pub unbilled_minor: i64,
    pub removed_minor: i64,
    pub rolled_over_minor: i64,
    pub invoice_number: Option<String>,
    pub automatic_top_up: bool,
    /// Depleted or expired.
    pub used_up: bool,
}

#[derive(Debug, Clone)]
pub struct LabelledLedgerEntry {
    pub entry: CreditLedgerEntry,
    pub currency: String,
    pub credit_unit: Option<CreditUnit>,
    pub grant_source: CreditGrantSource,
    pub grant_classification: GrantClassification,
    pub grant_created_by: Option<Uuid>,
    /// The invoice line that funded the grant, else its reason.
    pub grant_name: String,
    /// The invoice the entry moved credit for, else the one that funded its grant.
    pub invoice_number: Option<String>,
    pub credit_note_number: Option<String>,
    pub automatic_top_up: bool,
    /// The bonus half of paid credit sold below its face.
    pub grant_bonus: bool,
}

#[async_trait::async_trait]
pub trait CustomerCreditActivityInterface {
    /// One entry per wallet of the customer; totals count movements from `since` (all when None),
    /// grants are listed whatever `since`. With `subscription_id`, the totals and the next expiry
    /// count the credit that subscription can spend: its own and the customer-wide credit.
    async fn customer_credit_activity(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        since: Option<DateTime<Utc>>,
        subscription_id: Option<SubscriptionId>,
    ) -> StoreResult<Vec<WalletActivity>>;

    /// How each of `grants`, all of the customer's, was consumed.
    async fn credit_grants_activity(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        grants: &[CreditGrant],
    ) -> StoreResult<Vec<GrantActivity>>;

    /// The statement of the customer's wallets (or of `wallet_id` only), newest first, with its
    /// total number of lines. Running balances are per wallet. With `subscription_id`, the
    /// statement of the credit that subscription can spend (its own and the customer-wide
    /// credit), its running balances counting those movements only.
    async fn customer_credit_labelled_ledger(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: Option<WalletId>,
        subscription_id: Option<SubscriptionId>,
        limit: i64,
        offset: i64,
    ) -> StoreResult<(Vec<LabelledLedgerEntry>, u64)>;
}

#[async_trait::async_trait]
impl CustomerCreditActivityInterface for Store {
    async fn customer_credit_activity(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        since: Option<DateTime<Utc>>,
        subscription_id: Option<SubscriptionId>,
    ) -> StoreResult<Vec<WalletActivity>> {
        let wallets = self.customer_credit_wallets(tenant_id, customer_id).await?;
        let mut conn = self.get_conn().await?;
        let now = Utc::now();
        let account_ids = wallets.iter().map(|wallet| wallet.id).collect::<Vec<_>>();
        let rolling = rolling_tx(&mut conn, &account_ids, now).await?;
        let totals: HashMap<WalletId, CreditAccountActivityRow> =
            CreditAccountActivityRow::list_for_accounts(
                &mut conn,
                tenant_id,
                &account_ids,
                &rolling,
                now,
                since,
                subscription_id,
            )
            .await?
            .into_iter()
            .map(|row| (row.account_id, row))
            .collect();
        let mut groups: HashMap<WalletId, Vec<CreditAddedGroup>> = HashMap::new();
        for row in CreditAddedGroupRow::list_for_customer(
            &mut conn,
            tenant_id,
            customer_id,
            since,
            subscription_id,
        )
        .await?
        {
            groups
                .entry(row.account_id)
                .or_default()
                .push(CreditAddedGroup {
                    source: CreditGrantSource::from_db_str(&row.source_kind)?,
                    classification: GrantClassification::from_db_str(&row.classification)?,
                    added_by_member: row.added_by_member,
                    name: row.name,
                    grants: u32::try_from(row.grants).unwrap_or(u32::MAX),
                    amount_minor: row.amount,
                });
        }
        let consumption: HashMap<CreditGrantId, CreditGrantActivityRow> =
            CreditGrantActivityRow::list_for_customer(&mut conn, tenant_id, customer_id, None)
                .await?
                .into_iter()
                .map(|row| (row.grant_id, row))
                .collect();
        let projected = self
            .hooks
            .credit
            .projected_remaining_by_grant(&mut conn, tenant_id, customer_id)
            .await?;

        Ok(wallets
            .iter()
            .map(|wallet| {
                let total = totals.get(&wallet.id);
                WalletActivity {
                    wallet_id: wallet.id,
                    credit_added_minor: total.map_or(0, |row| row.added),
                    credit_added_groups: groups.remove(&wallet.id).unwrap_or_default(),
                    invoiced_minor: total.map_or(0, |row| row.invoiced),
                    settled_at_expiry_minor: total.map_or(0, |row| row.settled_at_expiry),
                    expired_minor: total.map_or(0, |row| row.expired),
                    removed_minor: total.map_or(0, |row| row.removed),
                    next_expiry: next_expiry(wallet, subscription_id),
                    grants: grant_activity(&wallet.grants, &consumption, &projected),
                }
            })
            .collect())
    }

    async fn credit_grants_activity(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        grants: &[CreditGrant],
    ) -> StoreResult<Vec<GrantActivity>> {
        if grants.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.get_conn().await?;
        let ids = grants.iter().map(|grant| grant.id).collect::<Vec<_>>();
        let consumption: HashMap<CreditGrantId, CreditGrantActivityRow> =
            CreditGrantActivityRow::list_for_customer(
                &mut conn,
                tenant_id,
                customer_id,
                Some(&ids),
            )
            .await?
            .into_iter()
            .map(|row| (row.grant_id, row))
            .collect();
        // Only active credit has usage not invoiced yet to project.
        let projected = if grants
            .iter()
            .any(|grant| grant.state == CreditGrantState::Active)
        {
            self.hooks
                .credit
                .projected_remaining_by_grant(&mut conn, tenant_id, customer_id)
                .await?
        } else {
            HashMap::new()
        };
        Ok(grant_activity(grants, &consumption, &projected))
    }

    async fn customer_credit_labelled_ledger(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: Option<WalletId>,
        subscription_id: Option<SubscriptionId>,
        limit: i64,
        offset: i64,
    ) -> StoreResult<(Vec<LabelledLedgerEntry>, u64)> {
        let mut conn = self.get_conn().await?;
        let accounts =
            CustomerCreditAccountRow::list_for_customer(&mut conn, tenant_id, customer_id)
                .await?
                .into_iter()
                .filter(|account| wallet_id.is_none_or(|id| account.id == id))
                .collect::<Vec<_>>();
        if wallet_id.is_some() && accounts.is_empty() {
            return Err(Report::new(StoreError::ValueNotFound("wallet".to_owned())));
        }
        let account_ids = accounts
            .iter()
            .map(|account| account.id)
            .collect::<Vec<_>>();
        let units = credit_units(&mut conn, tenant_id, &accounts).await?;
        let now = Utc::now();
        let rolling = rolling_tx(&mut conn, &account_ids, now).await?;
        let rows = CustomerCreditLabelledStatementRow::list_for_accounts(
            &mut conn,
            tenant_id,
            &account_ids,
            &rolling,
            now,
            subscription_id,
            limit.clamp(1, 1000),
            offset.max(0),
        )
        .await?;
        let total = CustomerCreditLabelledStatementRow::count_for_accounts(
            &mut conn,
            tenant_id,
            &account_ids,
            &rolling,
            now,
            subscription_id,
        )
        .await?;
        let entries = rows
            .into_iter()
            .map(|row| {
                let account = accounts
                    .iter()
                    .find(|account| account.id == row.statement.account_id)
                    .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
                let grant_name = row
                    .grant_line_name
                    .clone()
                    .unwrap_or_else(|| row.statement.grant_reason.clone());
                let grant_classification =
                    GrantClassification::from_db_str(&row.grant_classification)?;
                let grant_bonus = grant_classification == GrantClassification::Promotional
                    && row.statement.grant_reason
                        == crate::repositories::credit::customer_credit::BONUS_CREDIT;
                let entry = statement_entry(row.statement)?;
                Ok(LabelledLedgerEntry {
                    invoice_number: row.invoice_number.or(row.grant_invoice_number),
                    currency: account.currency.clone(),
                    credit_unit: account
                        .credit_unit_id
                        .and_then(|id| units.get(&id).cloned()),
                    grant_source: CreditGrantSource::from_db_str(&row.grant_source_kind)?,
                    grant_classification,
                    grant_bonus,
                    grant_created_by: row.grant_created_by,
                    grant_name,
                    credit_note_number: row.credit_note_number,
                    automatic_top_up: row.automatic_top_up,
                    entry,
                })
            })
            .collect::<StoreResult<Vec<_>>>()?;
        Ok((entries, u64::try_from(total).unwrap_or_default()))
    }
}

async fn credit_units(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    accounts: &[CustomerCreditAccountRow],
) -> StoreResult<HashMap<CreditUnitId, CreditUnit>> {
    let mut ids = accounts
        .iter()
        .filter_map(|account| account.credit_unit_id)
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();
    CreditUnitRow::list_by_ids(conn, tenant_id, &ids)
        .await?
        .into_iter()
        .map(|row| CreditUnit::try_from(row).map(|unit| (unit.id, unit)))
        .collect()
}

fn next_expiry(
    wallet: &CreditWallet,
    subscription_id: Option<SubscriptionId>,
) -> Option<DateTime<Utc>> {
    wallet
        .grants
        .iter()
        .filter(|grant| grant.state == CreditGrantState::Active && grant.remaining_minor > 0)
        .filter(|grant| {
            subscription_id
                .is_none_or(|scoped| grant.scope.subscription_id.is_none_or(|own| own == scoped))
        })
        .filter_map(|grant| grant.lapses_at())
        .min()
}

fn grant_activity(
    grants: &[CreditGrant],
    consumption: &HashMap<CreditGrantId, CreditGrantActivityRow>,
    projected: &HashMap<CreditGrantId, i64>,
) -> Vec<GrantActivity> {
    grants
        .iter()
        .map(|grant| {
            let row = consumption.get(&grant.id);
            // A ledger change after the projection can leave it above what the grant still holds.
            let unbilled = if grant.state == CreditGrantState::Active {
                projected.get(&grant.id).map_or(0, |left| {
                    (grant.remaining_minor - left).clamp(0, grant.remaining_minor)
                })
            } else {
                0
            };
            GrantActivity {
                grant_id: grant.id,
                name: row
                    .and_then(|row| row.line_name.clone())
                    .unwrap_or_else(|| grant.reason.clone()),
                invoiced_minor: row.map_or(0, |row| row.invoiced),
                settled_at_expiry_minor: row.map_or(0, |row| row.settled_at_expiry),
                unbilled_minor: unbilled,
                removed_minor: row.map_or(0, |row| row.removed),
                rolled_over_minor: row.map_or(0, |row| row.rolled_over),
                invoice_number: row.and_then(|row| row.invoice_number.clone()),
                automatic_top_up: row.is_some_and(|row| row.automatic_top_up),
                used_up: grant.used_up(),
            }
        })
        .collect()
}
