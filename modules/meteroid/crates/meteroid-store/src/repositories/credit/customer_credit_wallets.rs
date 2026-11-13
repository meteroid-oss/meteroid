//! Customer credit presented as wallets: balances, statements and operator adjustments.
use crate::domain::credit::{
    CreditApplication, CreditCost, CreditGrant, CreditGrantSource, CreditGrantState,
    CreditLedgerEntry, CreditWallet, CustomerCreditGrant, ENDED_WITH_SUBSCRIPTION,
    FundingReference, GrantClassification, GrantEndReason, GrantRollover, GrantScope,
    GrantSubscriptionFilter, LedgerEntryKind,
};
use crate::domain::credit_exhausted::{WalletPlanRule, wallet_plan_rules};
use crate::domain::credit_units::CreditUnit;
use crate::errors::StoreError;
use crate::repositories::credit::cost::manual_grant_cost;
use crate::repositories::credit::customer_credit::{
    CreditFunding, CustomerCreditBalance, GrantProducts, ManualDebit, account_tx, balances_tx,
    debit_manual_grant_tx, fund_paid_tx, grant_products_tx, grant_scope, normalized_currency,
    source, unswept_rollovers_tx,
};
use crate::services::credit::invalid;
use crate::store::PgConn;
use crate::{Store, StoreResult};
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{
    BaseId, CreditGrantId, CreditUnitId, CustomerId, InvoiceId, TenantId, WalletId,
};
use diesel_models::credit_units::CreditUnitRow;
use diesel_models::customer_credit::{
    CustomerCreditAccountRow, CustomerCreditEntryRow, CustomerCreditGrantProductRow,
    CustomerCreditGrantRow, CustomerCreditGrantStateFilter, CustomerCreditStatementRow,
    GrantSubscriptionFilter as DbGrantSubscriptionFilter, PlanCreditRuleRow,
};
use diesel_models::customers::CustomerRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use std::collections::HashMap;
use uuid::Uuid;

const MAX_REASON_BYTES: usize = 1024;

#[derive(Debug, Clone)]
pub struct FundCustomerCreditWallet {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    pub amount_minor: i64,
    /// PAID records credit the customer paid for outside of Meteroid; nothing is invoiced.
    pub classification: GrantClassification,
    pub priority: i32,
    pub expires_at: Option<DateTime<Utc>>,
    /// Identifies the grant: a retry with the same key returns it instead of funding again.
    pub idempotency_key: Uuid,
    pub reason: String,
    pub reference: Option<FundingReference>,
    pub actor_id: Option<Uuid>,
    /// What the grant cost, net of tax, in a currency. Required in a credit unit wallet; absent
    /// elsewhere, promotional credit costs nothing and paid credit its face value.
    pub cost: Option<CreditCost>,
}

#[derive(Debug, Clone)]
pub struct FundedCustomerCreditWallet {
    pub wallet: CreditWallet,
    pub grant_id: CreditGrantId,
    /// Paid credit stated to cost less than its face is funded at its cost, beside this bonus
    /// credit (promotional, at no cost) for the rest.
    pub bonus_grant_id: Option<CreditGrantId>,
    pub replayed: bool,
}

/// A page of one wallet's grants.
#[derive(Debug, Clone)]
pub struct WalletGrantsPage {
    pub grants: Vec<CreditGrant>,
    /// How many grants match, over every page.
    pub total: u64,
    /// The wallet holds credits of a unit.
    pub credit_unit: bool,
}

#[derive(Debug, Clone)]
pub struct DebitCustomerCreditGrant {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub grant_id: CreditGrantId,
    /// None removes everything the grant has left.
    pub amount_minor: Option<i64>,
    /// Identifies the removal: a retry with the same key returns it instead of removing again.
    pub idempotency_key: Uuid,
    pub reason: String,
    pub actor_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct DebitedCustomerCreditGrant {
    pub wallet: CreditWallet,
    pub removed_minor: i64,
    pub replayed: bool,
}

#[async_trait::async_trait]
pub trait CustomerCreditWalletsInterface {
    async fn customer_credit_wallets(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Vec<CreditWallet>>;

    /// None when the id is not a wallet of this tenant.
    async fn customer_credit_wallet(
        &self,
        tenant_id: TenantId,
        wallet_id: WalletId,
    ) -> StoreResult<Option<CreditWallet>>;

    /// The wallet's statement, newest first, with its total number of lines.
    async fn customer_credit_ledger(
        &self,
        tenant_id: TenantId,
        wallet_id: WalletId,
        limit: i64,
        offset: i64,
    ) -> StoreResult<(Vec<CreditLedgerEntry>, u64)>;

    /// A page of the customer's grants, newest first, and the number matching the filters.
    #[allow(clippy::too_many_arguments)]
    async fn customer_credit_grants(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        currency: Option<&str>,
        subscription: Option<GrantSubscriptionFilter>,
        state: Option<CreditGrantState>,
        limit: i64,
        offset: i64,
    ) -> StoreResult<(Vec<CustomerCreditGrant>, u64)>;

    /// A page of the wallet's grants (only the depleted or expired ones when `used_up`), newest
    /// first, and the number matching. None when the wallet is not one of the customer's.
    #[allow(clippy::too_many_arguments)]
    async fn customer_wallet_grants(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        subscription: Option<GrantSubscriptionFilter>,
        used_up: bool,
        limit: i64,
        offset: i64,
    ) -> StoreResult<Option<WalletGrantsPage>>;

    /// Whether the wallet is one of the customer's.
    async fn is_customer_wallet(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
    ) -> StoreResult<bool>;

    /// The start of the earliest current period among the customer's live subscriptions.
    async fn customer_current_period_start(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Option<NaiveDate>>;

    /// The statement of the customer's wallet in `currency`, newest first, each line with what its
    /// grant pays for, and the total number of lines.
    async fn customer_credit_currency_ledger(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        currency: &str,
        limit: i64,
        offset: i64,
    ) -> StoreResult<(Vec<(CreditLedgerEntry, CreditApplication)>, u64)>;

    /// Opens (or returns) the customer's wallet for a currency or credit unit.
    async fn create_customer_credit_wallet(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        currency: &str,
    ) -> StoreResult<CreditWallet>;

    /// Adds credit by hand that pays any charge of any of the customer's subscriptions.
    async fn fund_customer_credit_wallet(
        &self,
        command: FundCustomerCreditWallet,
    ) -> StoreResult<FundedCustomerCreditWallet>;

    /// `fund_customer_credit_wallet` recorded as added at `at`, a past moment the credit takes
    /// effect from and its expiry must follow.
    async fn fund_customer_credit_wallet_at(
        &self,
        command: FundCustomerCreditWallet,
        at: DateTime<Utc>,
    ) -> StoreResult<FundedCustomerCreditWallet>;

    /// `fund_customer_credit_wallet` paying what `scope` names: one live subscription of the
    /// customer (in its currency) or any, usage only or any charge, some products or every one.
    /// Credit in a credit unit cannot be restricted to products.
    async fn fund_scoped_customer_credit_wallet(
        &self,
        command: FundCustomerCreditWallet,
        scope: GrantScope,
    ) -> StoreResult<FundedCustomerCreditWallet>;

    /// `fund_scoped_customer_credit_wallet` recorded as added at `at`, a past moment, as
    /// `fund_customer_credit_wallet_at`.
    async fn fund_scoped_customer_credit_wallet_at(
        &self,
        command: FundCustomerCreditWallet,
        scope: GrantScope,
        at: DateTime<Utc>,
    ) -> StoreResult<FundedCustomerCreditWallet>;

    /// Removes credit an operator added by hand: the given amount, or all it has left.
    async fn debit_customer_credit_grant(
        &self,
        command: DebitCustomerCreditGrant,
    ) -> StoreResult<DebitedCustomerCreditGrant>;

    /// Pays the usage expired credit covered, then records its lapse, `limit` customers per page;
    /// returns how many grants lapsed.
    async fn expire_customer_credit(&self, limit: i64) -> StoreResult<usize>;

    /// Ends the credit of terminal subscriptions whose termination did not (the sweep's first
    /// step); returns how many subscriptions it repaired.
    async fn end_credit_of_ended_subscriptions(&self, page: i64) -> StoreResult<usize>;

    /// Settles and lapses (or rolls over) the customer's credit that expired by `as_of`, as the
    /// sweep would at that moment; returns how many grants lapsed. Errors are returned, not retried.
    async fn expire_customer_credit_for(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        as_of: DateTime<Utc>,
    ) -> StoreResult<usize>;

    /// Takes back the credit of purchases whose payment or cash refund the previous release
    /// reversed without the ledger; returns how many were reconciled. Idempotent.
    async fn reconcile_reversed_credit_purchases(&self, page: i64) -> StoreResult<usize>;

    /// Credit the invoice holds, by grant: its draws and the usage settlements it took over, net
    /// of what was given back.
    async fn invoice_credit_by_grant(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Vec<(CreditGrantId, i64)>>;
}

#[async_trait::async_trait]
impl CustomerCreditWalletsInterface for Store {
    async fn invoice_credit_by_grant(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Vec<(CreditGrantId, i64)>> {
        let mut conn = self.get_conn().await?;
        crate::repositories::credit::customer_credit::invoice_credit_by_grant_tx(
            &mut conn, tenant_id, invoice_id,
        )
        .await
    }

    async fn customer_credit_wallets(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Vec<CreditWallet>> {
        let mut conn = self.get_conn().await?;
        CustomerRow::find_by_id(&mut conn, &customer_id, &tenant_id).await?;
        let now = common_domain::clock::now();
        let balances = balances_tx(&mut conn, tenant_id, customer_id, now).await?;
        let units = wallet_units(&mut conn, tenant_id, &balances).await?;
        let rules = plan_rules_of(&mut conn, tenant_id, customer_id).await?;
        balances
            .iter()
            .map(|balance| wallet_view(balance, &units, &rules, now))
            .collect()
    }

    async fn customer_credit_wallet(
        &self,
        tenant_id: TenantId,
        wallet_id: WalletId,
    ) -> StoreResult<Option<CreditWallet>> {
        let mut conn = self.get_conn().await?;
        let Some(account) =
            CustomerCreditAccountRow::find_by_id(&mut conn, tenant_id, wallet_id).await?
        else {
            return Ok(None);
        };
        let now = common_domain::clock::now();
        let balances = balances_tx(&mut conn, tenant_id, account.customer_id, now).await?;
        let Some(balance) = balances
            .iter()
            .find(|balance| balance.account.id == wallet_id)
        else {
            return Ok(None);
        };
        let units = wallet_units(&mut conn, tenant_id, std::slice::from_ref(balance)).await?;
        let rules = plan_rules_of(&mut conn, tenant_id, account.customer_id).await?;
        wallet_view(balance, &units, &rules, now).map(Some)
    }

    async fn customer_credit_ledger(
        &self,
        tenant_id: TenantId,
        wallet_id: WalletId,
        limit: i64,
        offset: i64,
    ) -> StoreResult<(Vec<CreditLedgerEntry>, u64)> {
        let mut conn = self.get_conn().await?;
        CustomerCreditAccountRow::find_by_id(&mut conn, tenant_id, wallet_id)
            .await?
            .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        let now = common_domain::clock::now();
        let rolling = rolling_tx(&mut conn, &[wallet_id], now).await?;
        let rows = CustomerCreditStatementRow::list_for_accounts(
            &mut conn,
            tenant_id,
            &[wallet_id],
            &rolling,
            now,
            limit.clamp(1, 1000),
            offset.max(0),
        )
        .await?;
        let total = CustomerCreditStatementRow::count_for_accounts(
            &mut conn,
            tenant_id,
            &[wallet_id],
            &rolling,
            now,
        )
        .await?;
        let entries = rows
            .into_iter()
            .map(statement_entry)
            .collect::<StoreResult<Vec<_>>>()?;
        Ok((entries, u64::try_from(total).unwrap_or_default()))
    }

    async fn customer_credit_grants(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        currency: Option<&str>,
        subscription: Option<GrantSubscriptionFilter>,
        state: Option<CreditGrantState>,
        limit: i64,
        offset: i64,
    ) -> StoreResult<(Vec<CustomerCreditGrant>, u64)> {
        let mut conn = self.get_conn().await?;
        CustomerRow::find_by_id(&mut conn, &customer_id, &tenant_id).await?;
        let currency = currency.map(|currency| currency.trim().to_ascii_uppercase());
        let state = state.map(|state| match state {
            CreditGrantState::Pending => CustomerCreditGrantStateFilter::Pending,
            CreditGrantState::Active => CustomerCreditGrantStateFilter::Active,
            CreditGrantState::Depleted => CustomerCreditGrantStateFilter::Depleted,
            CreditGrantState::Expired => CustomerCreditGrantStateFilter::Expired,
        });
        grants_page(
            &mut conn,
            tenant_id,
            customer_id,
            currency.as_deref(),
            subscription,
            state,
            limit,
            offset,
        )
        .await
    }

    async fn customer_wallet_grants(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        subscription: Option<GrantSubscriptionFilter>,
        used_up: bool,
        limit: i64,
        offset: i64,
    ) -> StoreResult<Option<WalletGrantsPage>> {
        let mut conn = self.get_conn().await?;
        let Some(account) = CustomerCreditAccountRow::find_by_id(&mut conn, tenant_id, wallet_id)
            .await?
            .filter(|account| account.customer_id == customer_id)
        else {
            return Ok(None);
        };
        let (grants, total) = grants_page(
            &mut conn,
            tenant_id,
            customer_id,
            Some(account.currency.as_str()),
            subscription,
            used_up.then_some(CustomerCreditGrantStateFilter::UsedUp),
            limit,
            offset,
        )
        .await?;
        Ok(Some(WalletGrantsPage {
            grants: grants.into_iter().map(|grant| grant.grant).collect(),
            total,
            credit_unit: account.credit_unit_id.is_some(),
        }))
    }

    async fn is_customer_wallet(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
    ) -> StoreResult<bool> {
        let mut conn = self.get_conn().await?;
        Ok(
            CustomerCreditAccountRow::find_by_id(&mut conn, tenant_id, wallet_id)
                .await?
                .is_some_and(|account| account.customer_id == customer_id),
        )
    }

    async fn customer_current_period_start(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Option<NaiveDate>> {
        let mut conn = self.get_conn().await?;
        Ok(
            SubscriptionRow::find_active_by_customer(&mut conn, customer_id, tenant_id)
                .await?
                .iter()
                .map(|subscription| subscription.current_period_start)
                .min(),
        )
    }

    async fn customer_credit_currency_ledger(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        currency: &str,
        limit: i64,
        offset: i64,
    ) -> StoreResult<(Vec<(CreditLedgerEntry, CreditApplication)>, u64)> {
        let mut conn = self.get_conn().await?;
        CustomerRow::find_by_id(&mut conn, &customer_id, &tenant_id).await?;
        let currency = currency.trim().to_ascii_uppercase();
        let Some(account) = CustomerCreditAccountRow::find_by_currency(
            &mut conn,
            tenant_id,
            customer_id,
            &currency,
        )
        .await?
        else {
            return Ok((Vec::new(), 0));
        };
        let ids = vec![account.id];
        let now = common_domain::clock::now();
        let rolling = rolling_tx(&mut conn, &ids, now).await?;
        let rows = CustomerCreditStatementRow::list_for_accounts(
            &mut conn,
            tenant_id,
            &ids,
            &rolling,
            now,
            limit.clamp(1, 1000),
            offset.max(0),
        )
        .await?;
        let total = CustomerCreditStatementRow::count_for_accounts(
            &mut conn, tenant_id, &ids, &rolling, now,
        )
        .await?;
        let entries = rows
            .into_iter()
            .map(|row| {
                let entry = statement_entry(row)?;
                let application = entry.application;
                Ok((entry, application))
            })
            .collect::<StoreResult<Vec<_>>>()?;
        Ok((entries, u64::try_from(total).unwrap_or_default()))
    }

    async fn create_customer_credit_wallet(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        currency: &str,
    ) -> StoreResult<CreditWallet> {
        let account = self
            .transaction(|conn| {
                async move {
                    let lock = CustomerRow::lock(conn, tenant_id, customer_id).await?;
                    account_tx(conn, &lock, currency).await
                }
                .scope_boxed()
            })
            .await?;
        self.customer_credit_wallet(tenant_id, account.id)
            .await?
            .ok_or_else(|| invalid("the created wallet is missing"))
    }

    async fn fund_customer_credit_wallet(
        &self,
        command: FundCustomerCreditWallet,
    ) -> StoreResult<FundedCustomerCreditWallet> {
        fund_wallet(self, command, GrantScope::customer_wide(), None).await
    }

    async fn fund_customer_credit_wallet_at(
        &self,
        command: FundCustomerCreditWallet,
        at: DateTime<Utc>,
    ) -> StoreResult<FundedCustomerCreditWallet> {
        fund_wallet(self, command, GrantScope::customer_wide(), Some(at)).await
    }

    async fn fund_scoped_customer_credit_wallet(
        &self,
        command: FundCustomerCreditWallet,
        scope: GrantScope,
    ) -> StoreResult<FundedCustomerCreditWallet> {
        fund_wallet(self, command, scope, None).await
    }

    async fn fund_scoped_customer_credit_wallet_at(
        &self,
        command: FundCustomerCreditWallet,
        scope: GrantScope,
        at: DateTime<Utc>,
    ) -> StoreResult<FundedCustomerCreditWallet> {
        fund_wallet(self, command, scope, Some(at)).await
    }

    async fn debit_customer_credit_grant(
        &self,
        command: DebitCustomerCreditGrant,
    ) -> StoreResult<DebitedCustomerCreditGrant> {
        let reason = command.reason.trim().to_owned();
        if reason.is_empty() {
            return Err(invalid("removing customer credit needs a reason"));
        }
        if reason.len() > MAX_REASON_BYTES {
            return Err(invalid("the reason is too long"));
        }
        let (grant, removed_minor, replayed) = self
            .transaction(|conn| {
                async move {
                    let lock = super::customer_credit::lock_customer_tx(
                        conn,
                        command.tenant_id,
                        command.customer_id,
                    )
                    .await?;
                    debit_manual_grant_tx(
                        conn,
                        &*self.hooks.credit,
                        &lock,
                        ManualDebit {
                            tenant_id: command.tenant_id,
                            customer_id: command.customer_id,
                            grant_id: command.grant_id,
                            amount_minor: command.amount_minor,
                            request_id: command.idempotency_key,
                            reason,
                            actor_id: command.actor_id,
                        },
                        common_domain::clock::now(),
                    )
                    .await
                }
                .scope_boxed()
            })
            .await?;
        let wallet = self
            .customer_credit_wallet(command.tenant_id, grant.account_id)
            .await?
            .ok_or_else(|| invalid("the debited wallet is missing"))?;
        Ok(DebitedCustomerCreditGrant {
            wallet,
            removed_minor,
            replayed,
        })
    }

    async fn expire_customer_credit(&self, limit: i64) -> StoreResult<usize> {
        crate::services::credit::expiry_settlement::lapse_expired_credit(self, limit).await
    }

    async fn end_credit_of_ended_subscriptions(&self, page: i64) -> StoreResult<usize> {
        crate::services::credit::subscription_end::end_credit_of_ended_subscriptions(self, page)
            .await
    }

    async fn expire_customer_credit_for(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        as_of: DateTime<Utc>,
    ) -> StoreResult<usize> {
        crate::services::credit::expiry_settlement::lapse_expired_credit_for(
            self,
            tenant_id,
            customer_id,
            as_of,
        )
        .await
    }

    async fn reconcile_reversed_credit_purchases(&self, page: i64) -> StoreResult<usize> {
        crate::services::credit::reversal_sweep::reconcile_reversed_credit_purchases(self, page)
            .await
    }
}

/// The credit units the balances' wallets hold, by id.
async fn wallet_units(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    balances: &[CustomerCreditBalance],
) -> StoreResult<HashMap<CreditUnitId, CreditUnit>> {
    let mut ids: Vec<CreditUnitId> = balances
        .iter()
        .filter_map(|balance| balance.account.credit_unit_id)
        .collect();
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

/// What the sweep will roll over of each expired grant of `account_ids`: the statement keeps it in
/// the balance, as `available_minor_tx` does.
pub(crate) async fn rolling_tx(
    conn: &mut crate::store::PgConn,
    account_ids: &[WalletId],
    now: DateTime<Utc>,
) -> StoreResult<Vec<(CreditGrantId, i64)>> {
    let mut rolling = Vec::new();
    for account_id in account_ids {
        rolling.extend(unswept_rollovers_tx(conn, *account_id, now).await?);
    }
    Ok(rolling)
}

async fn fund_wallet(
    store: &Store,
    command: FundCustomerCreditWallet,
    scope: GrantScope,
    explicit_at: Option<DateTime<Utc>>,
) -> StoreResult<FundedCustomerCreditWallet> {
    let at = explicit_at.unwrap_or_else(common_domain::clock::now);
    if at > common_domain::clock::now() {
        return Err(invalid("credit cannot be added at a future moment"));
    }
    let reason = command.reason.trim().to_owned();
    if reason.is_empty() {
        return Err(invalid("customer credit needs a reason"));
    }
    if reason.len() > MAX_REASON_BYTES {
        return Err(invalid("the reason is too long"));
    }
    if command.priority < 0 {
        return Err(invalid("priority must not be negative"));
    }
    if command.amount_minor <= 0 {
        return Err(invalid("customer credit must be positive"));
    }
    let mut conn = store.get_conn().await?;
    let account =
        CustomerCreditAccountRow::find_by_id(&mut conn, command.tenant_id, command.wallet_id)
            .await?
            .filter(|account| account.customer_id == command.customer_id)
            .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
    let replayed = CustomerCreditGrantRow::find_by_source(
        &mut conn,
        command.tenant_id,
        source::MANUAL,
        command.idempotency_key,
    )
    .await?
    // Another customer's grant is no replay: the funding below refuses the reused key.
    .filter(|grant| grant.customer_id == command.customer_id);
    // A replay is funded at its first request's moment, unless the caller names one.
    let at = explicit_at.unwrap_or(replayed.as_ref().map_or(at, |grant| grant.effective_at));
    // Given credit units cost nothing, stated in the customer's currency.
    let stated = match (
        &command.cost,
        account.credit_unit_id,
        command.classification,
    ) {
        (None, Some(_), GrantClassification::Promotional) => Some(CreditCost::given(
            &CustomerRow::find_by_id(&mut conn, &command.customer_id, &command.tenant_id)
                .await?
                .currency,
        )),
        _ => command.cost.clone(),
    };
    let scope = scope.clone().with_products(scope.product_ids);
    if account.credit_unit_id.is_some() && scope.application == CreditApplication::Usage {
        return Err(invalid(format!(
            "a {} wallet pays the usage priced in its credit unit; it cannot be limited to usage",
            account.currency
        )));
    }
    if !scope.product_ids.is_empty() {
        if account.credit_unit_id.is_some() {
            return Err(invalid(
                "credit in a credit unit pays for every product; it cannot be restricted to products",
            ));
        }
        let known = CustomerCreditGrantProductRow::existing_products(
            &mut conn,
            command.tenant_id,
            &scope.product_ids,
        )
        .await?;
        if let Some(unknown) = scope.product_ids.iter().find(|id| !known.contains(id)) {
            return Err(invalid(format!("product {unknown} does not exist")));
        }
    }
    // A replay returns what its first request granted, even once the subscription has ended.
    if let Some(subscription_id) = scope.subscription_id.filter(|_| replayed.is_none()) {
        let subscription = crate::services::credit::scope::owning_subscription_tx(
            &mut conn,
            command.tenant_id,
            command.customer_id,
            subscription_id,
        )
        .await?;
        if subscription.ended {
            return Err(invalid(
                "the subscription has ended; credit can no longer be added to it",
            ));
        }
    }
    drop(conn);
    let cost = manual_grant_cost(
        command.classification,
        command.amount_minor,
        &account.currency,
        stated,
    )?;
    let (grant, bonus, replayed) = store
        .transaction(|conn| {
            async move {
                let funded = fund_paid_tx(
                    conn,
                    &*store.hooks.credit,
                    CreditFunding {
                        tenant_id: command.tenant_id,
                        customer_id: command.customer_id,
                        currency: account.currency,
                        scope,
                        amount_minor: command.amount_minor,
                        classification: command.classification,
                        priority: command.priority,
                        effective_at: at,
                        expires_at: command.expires_at,
                        occurred_at: at,
                        now: at,
                        source_kind: source::MANUAL,
                        source_id: command.idempotency_key,
                        cost: Some(cost),
                        rollover: None,
                        invoice_id: None,
                        line_local_id: None,
                        credit_note_id: None,
                        reason,
                        reference: command.reference.map(FundingReference::into_inner),
                        created_by: command.actor_id,
                        pays_usage_from: None,
                    },
                )
                .await?;
                Ok::<_, Report<StoreError>>((
                    funded.grant,
                    funded.bonus.map(|bonus| bonus.id),
                    funded.replayed,
                ))
            }
            .scope_boxed()
        })
        .await?;
    let wallet = store
        .customer_credit_wallet(command.tenant_id, command.wallet_id)
        .await?
        .ok_or_else(|| invalid("the funded wallet is missing"))?;
    Ok(FundedCustomerCreditWallet {
        wallet,
        grant_id: grant.id,
        bonus_grant_id: bonus,
        replayed,
    })
}

/// A page of the customer's grants with the currency of their wallet, newest first, and the
/// number matching.
#[allow(clippy::too_many_arguments)]
async fn grants_page(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: Option<&str>,
    subscription: Option<GrantSubscriptionFilter>,
    state: Option<CustomerCreditGrantStateFilter>,
    limit: i64,
    offset: i64,
) -> StoreResult<(Vec<CustomerCreditGrant>, u64)> {
    let now = common_domain::clock::now();
    let subscription = subscription.map(|filter| match filter {
        GrantSubscriptionFilter::Of(id) => DbGrantSubscriptionFilter::Of(id),
        GrantSubscriptionFilter::UsableBy(id) => DbGrantSubscriptionFilter::UsableBy(id),
    });
    let rows = CustomerCreditGrantRow::page_for_customer(
        conn,
        tenant_id,
        customer_id,
        currency,
        subscription,
        state,
        now,
        limit.clamp(1, 1000),
        offset.max(0),
    )
    .await?;
    let total = CustomerCreditGrantRow::count_for_customer(
        conn,
        tenant_id,
        customer_id,
        currency,
        subscription,
        state,
        now,
    )
    .await?;
    let expiries = CustomerCreditEntryRow::list_amounts_for_customer(
        conn,
        tenant_id,
        customer_id,
        LedgerEntryKind::Expiry.as_db_str(),
    )
    .await?;
    let products = grant_products_tx(
        conn,
        tenant_id,
        &rows.iter().map(|(grant, _)| grant.id).collect::<Vec<_>>(),
    )
    .await?;
    let grants = rows
        .into_iter()
        .map(|(grant, currency)| {
            let lapsed = expiries
                .iter()
                .filter(|(grant_id, _)| *grant_id == grant.id)
                .try_fold(0_i64, |sum, (_, amount)| sum.checked_sub(*amount))
                .ok_or_else(|| invalid("customer credit exceeds the supported range"))?;
            Ok(CustomerCreditGrant {
                grant: grant_view(&grant, lapsed, now, &products)?,
                currency,
            })
        })
        .collect::<StoreResult<Vec<_>>>()?;
    Ok((grants, u64::try_from(total).unwrap_or_default()))
}

/// What the plans of the customer's active subscriptions do once each denomination's credit
/// runs out.
pub(crate) async fn plan_rules_of(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> StoreResult<HashMap<String, Vec<WalletPlanRule>>> {
    Ok(wallet_plan_rules(
        PlanCreditRuleRow::list_for_customers(conn, tenant_id, &[customer_id]).await?,
    ))
}

pub(crate) fn wallet_view(
    balance: &CustomerCreditBalance,
    units: &HashMap<CreditUnitId, CreditUnit>,
    rules: &HashMap<String, Vec<WalletPlanRule>>,
    now: DateTime<Utc>,
) -> StoreResult<CreditWallet> {
    let account = &balance.account;
    let grants = balance
        .grants
        .iter()
        .map(|(grant, lapsed)| grant_view(grant, *lapsed, now, &balance.products))
        .collect::<StoreResult<Vec<_>>>()?;
    Ok(CreditWallet {
        id: account.id,
        tenant_id: account.tenant_id,
        customer_id: account.customer_id,
        currency: account.currency.clone(),
        available_minor: balance.available_minor,
        pending_minor: balance.pending_minor,
        low_balance_thresholds_minor: account.low_balance_thresholds_minor.clone(),
        usage_at_zero: account.usage_at_zero.into(),
        plan_rules: rules
            .get(&normalized_currency(&account.currency))
            .cloned()
            .unwrap_or_default(),
        grants,
        created_at: account.created_at,
        credit_unit: account
            .credit_unit_id
            .map(|id| {
                units
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| invalid(format!("credit unit {id} of a wallet is missing")))
            })
            .transpose()?,
        unit_price: account
            .unit_price
            .zip(account.unit_price_currency.clone())
            .map(
                |(unit_price, currency)| crate::domain::credit::CustomerUnitPrice {
                    unit_price: unit_price.normalize(),
                    currency,
                },
            ),
    })
}

/// `lapsed_minor` is what recorded expiries already took; value past its lapse that the sweep has
/// not recorded yet counts as lapsed too.
fn grant_view(
    grant: &CustomerCreditGrantRow,
    lapsed_minor: i64,
    now: DateTime<Utc>,
    products: &GrantProducts,
) -> StoreResult<CreditGrant> {
    let expired = grant.lapses_at.is_some_and(|at| at <= now);
    let state = if expired {
        CreditGrantState::Expired
    } else if grant.effective_at > now {
        CreditGrantState::Pending
    } else if grant.remaining == 0 {
        CreditGrantState::Depleted
    } else {
        CreditGrantState::Active
    };
    let expired_minor = if expired {
        Some(
            lapsed_minor
                .checked_add(grant.remaining)
                .ok_or_else(|| invalid("customer credit exceeds the supported range"))?,
        )
    } else {
        None
    };
    Ok(CreditGrant {
        id: grant.id,
        wallet_id: grant.account_id,
        amount_minor: grant.amount,
        remaining_minor: if expired { 0 } else { grant.remaining },
        expired_minor,
        classification: GrantClassification::from_db_str(&grant.classification)?,
        priority: grant.priority,
        source: CreditGrantSource::from_db_str(&grant.source_kind)?,
        source_id: grant.source_id,
        invoice_id: grant.invoice_id,
        credit_note_id: grant.credit_note_id,
        effective_at: grant.effective_at,
        expires_at: grant.expires_at,
        state,
        reason: grant.reason.clone(),
        reference: grant.reference.clone(),
        created_by: grant.created_by,
        created_at: grant.created_at,
        cost: cost_of(grant.cost_minor, grant.cost_currency.as_deref()),
        scope: grant_scope(
            grant,
            products.get(&grant.id).map_or(&[][..], Vec::as_slice),
        )?,
        ends_at: grant.ends_at,
        end_reason: grant
            .end_reason
            .as_deref()
            .map(GrantEndReason::from_db_str)
            .transpose()?,
        rollover: grant
            .rollover
            .as_ref()
            .map(GrantRollover::from_json)
            .transpose()?,
    })
}

fn cost_of(amount_minor: Option<i64>, currency: Option<&str>) -> Option<CreditCost> {
    amount_minor
        .zip(currency)
        .map(|(amount_minor, currency)| CreditCost::at_face(amount_minor, currency))
}

pub(crate) fn statement_entry(row: CustomerCreditStatementRow) -> StoreResult<CreditLedgerEntry> {
    let kind = LedgerEntryKind::from_db_str(&row.kind)?;
    let reference = (kind == LedgerEntryKind::Fund)
        .then_some(row.grant_reference)
        .flatten();
    let reason = match kind {
        LedgerEntryKind::Fund => row.grant_reason,
        LedgerEntryKind::Draw => "Applied to invoice".to_owned(),
        LedgerEntryKind::DrawReversal if row.credit_note_id.is_some() => {
            "Returned by credit note".to_owned()
        }
        LedgerEntryKind::DrawReversal if row.invoice_id.is_none() => {
            "Returned, the usage was never invoiced".to_owned()
        }
        LedgerEntryKind::DrawReversal => "Returned by invoice void".to_owned(),
        LedgerEntryKind::Reversal => row
            .entry_reason
            .unwrap_or_else(|| "Reversed by credit note".to_owned()),
        LedgerEntryKind::Reinstatement => row
            .entry_reason
            .unwrap_or_else(|| "Restored after a reversed payment was recovered".to_owned()),
        LedgerEntryKind::Expiry => row
            .entry_reason
            .clone()
            .filter(|reason| {
                crate::domain::credit::is_removal_reason(reason)
                    || reason == ENDED_WITH_SUBSCRIPTION
            })
            .unwrap_or_else(|| "Expired unused".to_owned()),
        LedgerEntryKind::Rollover => "Rolled over to a new grant".to_owned(),
    };
    Ok(CreditLedgerEntry {
        id: row.id,
        wallet_id: row.account_id,
        grant_id: row.grant_id,
        sequence: row.sequence,
        kind,
        amount_minor: row.amount,
        balance_after_minor: row.balance_after,
        invoice_id: row.invoice_id,
        credit_note_id: row.credit_note_id,
        reason,
        reference,
        created_by: row.created_by,
        effective_at: row.occurred_at,
        recorded_at: row.recorded_at,
        cost: cost_of(row.cost_minor, row.cost_currency.as_deref()),
        application: CreditApplication::from_db_str(&row.grant_application)?,
        subscription_id: row.grant_subscription_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use common_domain::ids::{BaseId, LedgerEntryId};

    fn grant(remaining: i64, expires_at: Option<DateTime<Utc>>) -> CustomerCreditGrantRow {
        let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        CustomerCreditGrantRow {
            id: CreditGrantId::new(),
            tenant_id: TenantId::new(),
            customer_id: CustomerId::new(),
            account_id: WalletId::new(),
            amount: 1_000,
            remaining,
            priority: 100,
            classification: "PAID".to_owned(),
            effective_at: at,
            expires_at,
            source_kind: "MANUAL".to_owned(),
            source_id: Uuid::nil(),
            invoice_id: None,
            line_local_id: None,
            credit_note_id: None,
            reason: "goodwill".to_owned(),
            reference: None,
            created_by: None,
            created_at: at,
            cost_minor: None,
            cost_currency: None,
            rollover: None,
            subscription_id: None,
            application: "INVOICE".to_owned(),
            ends_at: None,
            end_reason: None,
            lapses_at: expires_at,
            pays_usage_from: None,
        }
    }

    #[test]
    fn expired_credit_shows_what_lapsed_and_nothing_spendable() {
        let now = Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap();
        let lapsed = grant_view(
            &grant(250, Some(now - chrono::Duration::days(1))),
            0,
            now,
            &GrantProducts::new(),
        )
        .unwrap();
        assert_eq!(lapsed.state, CreditGrantState::Expired);
        assert_eq!(lapsed.remaining_minor, 0);
        assert_eq!(lapsed.expired_minor, Some(250));

        // Recorded by the sweep: the grant holds nothing and the entry carries the lapse.
        let swept = grant_view(
            &grant(0, Some(now - chrono::Duration::days(1))),
            250,
            now,
            &GrantProducts::new(),
        )
        .unwrap();
        assert_eq!(swept.expired_minor, Some(250));

        let live = grant_view(&grant(250, None), 0, now, &GrantProducts::new()).unwrap();
        assert_eq!(live.state, CreditGrantState::Active);
        assert_eq!(live.remaining_minor, 250);
        assert_eq!(live.amount_minor, 1_000);
        assert_eq!(live.expired_minor, None);

        assert_eq!(
            grant_view(&grant(0, None), 0, now, &GrantProducts::new())
                .unwrap()
                .state,
            CreditGrantState::Depleted
        );
    }

    #[test]
    fn statement_lines_carry_their_meaning() {
        let wallet = WalletId::new();
        let row = |kind: &str, amount: i64| CustomerCreditStatementRow {
            id: LedgerEntryId::new(),
            account_id: wallet,
            grant_id: CreditGrantId::new(),
            kind: kind.to_owned(),
            amount,
            invoice_id: None,
            credit_note_id: None,
            grant_reason: "Credit note".to_owned(),
            entry_reason: None,
            grant_reference: Some("REF-1".to_owned()),
            created_by: None,
            occurred_at: common_domain::clock::now(),
            recorded_at: common_domain::clock::now(),
            sequence: 1,
            balance_after: 40,
            cost_minor: Some(-24),
            cost_currency: Some("EUR".to_owned()),
            grant_application: "USAGE".to_owned(),
            grant_subscription_id: None,
        };
        for kind in [
            "FUND",
            "DRAW",
            "DRAW_REVERSAL",
            "REVERSAL",
            "REINSTATEMENT",
            "EXPIRY",
            "ROLLOVER",
        ] {
            let entry = statement_entry(row(kind, -60)).unwrap();
            assert_eq!(entry.kind.as_db_str(), kind);
            assert_eq!(entry.amount_minor, -60);
            assert_eq!(entry.balance_after_minor, 40);
            assert_eq!(entry.reference.is_some(), kind == "FUND");
            assert_eq!(entry.cost, Some(CreditCost::at_face(-24, "EUR")));
        }
        assert!(statement_entry(row("HOLD", 1)).is_err());
        let removed = CustomerCreditStatementRow {
            entry_reason: Some("Over-granted goodwill".to_owned()),
            ..row("REVERSAL", -60)
        };
        assert_eq!(
            statement_entry(removed).unwrap().reason,
            "Over-granted goodwill"
        );
    }
}
