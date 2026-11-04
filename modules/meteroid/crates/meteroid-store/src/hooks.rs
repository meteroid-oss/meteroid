use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use common_domain::ids::{
    BudgetId, CreditGrantId, CreditUnitId, CustomerId, OrganizationId, ProductId, SubscriptionId,
    TenantId, UserId, WalletId,
};
use diesel_models::PgConn;
use diesel_models::customer_credit::{CustomerCreditAccountRow, CustomerCreditOperationRow};
use diesel_models::enums::UsageAtZeroEnum;
use diesel_models::lock::CustomerLock;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::domain::credit::{
    AutoTopUpChange, AutoTopUpMonthSpend, AutomaticTopUp, ConfigureCreditWallet, CreditWallet,
    LiveCreditBalance, SubscriptionWalletBalance, WalletHealth, WalletStatus,
};
use crate::domain::credit_outlook::CreditOutlook;
use crate::domain::customer_signals::OpenSignals;
use crate::domain::enums::DeclineKind;
use crate::domain::fx::FxTable;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::{BillableMetric, FullPlan, Organization};
use crate::errors::StoreError;
use crate::services::credit::forecast::ForecastDocument;
use crate::{Services, Store, StoreResult};

/// Edition behaviour around organizations and tenants. Core's own does nothing.
#[async_trait::async_trait]
pub trait OrganizationHooks: Send + Sync {
    /// After an organization and its first tenant are committed; an error fails the request.
    async fn organization_created(&self, _organization: &Organization) -> StoreResult<()> {
        Ok(())
    }

    /// Before a sign-up or an invitation is accepted. Core accepts every address.
    async fn accepts_signup_email(&self, _email: &str) -> bool {
        true
    }

    /// Before a Production tenant is inserted.
    async fn ensure_production_tenant_allowed(
        &self,
        _organization_id: OrganizationId,
    ) -> StoreResult<()> {
        Ok(())
    }
}

pub struct NoOrganizationHooks;

impl OrganizationHooks for NoOrganizationHooks {}

pub fn no_organization_hooks() -> Arc<dyn OrganizationHooks> {
    Arc::new(NoOrganizationHooks)
}

/// What the edition knows about this installation. OSS: not Meteroid Cloud, one organization.
pub trait InstanceEdition: Send + Sync + 'static {
    /// Meteroid Cloud; fixed for the process.
    fn is_cloud(&self) -> bool {
        false
    }

    /// The edition name this install reports in its telemetry.
    fn edition(&self) -> &'static str {
        "oss"
    }

    /// Whether `ENABLE_MULTI_ORGANIZATION` takes effect; may change while the process runs.
    fn allows_multiple_organizations(&self) -> bool {
        false
    }

    /// Lets the edition reach its own state (e.g. the license) from a `Store`.
    fn as_any(&self) -> &dyn Any;
}

pub struct CoreInstance;

impl InstanceEdition for CoreInstance {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn core_instance() -> Arc<dyn InstanceEdition> {
    Arc::new(CoreInstance)
}

/// What a sandbox seed created.
pub struct SeededSandbox<'a> {
    pub tenant_id: TenantId,
    /// By plan name.
    pub plans: &'a HashMap<String, FullPlan>,
    /// In creation order: the first is the scenario's primary usage metric.
    pub metrics: &'a [BillableMetric],
}

/// Told when a tenant's billing history is written after the fact, so derived analytics catch up,
/// and of the sandbox and tenant lifecycle steps the edition's analytics take part in.
#[async_trait::async_trait]
pub trait AnalyticsHook: Send + Sync {
    /// History of `tenant_id` from `from` on was written out of order (a seed, an import).
    /// Best effort: never fails the write.
    async fn tenant_history_written(&self, store: &Store, tenant_id: TenantId, from: NaiveDate);

    /// A sandbox seed created its catalog and customers. Best effort: never fails the seed.
    async fn sandbox_seeded(&self, _store: &Store, _sandbox: &SeededSandbox<'_>) {}

    /// A seeded sandbox's history is billed and settled. Best effort.
    async fn sandbox_warmed(
        &self,
        _store: &Store,
        _services: &Services,
        _tenant_id: TenantId,
        _today: NaiveDate,
    ) {
    }

    /// The tenants are being hard-deleted; an error leaves them pending deletion, retried later.
    async fn tenants_purged(&self, _tenant_ids: &[TenantId]) -> StoreResult<()> {
        Ok(())
    }
}

pub struct NoAnalyticsHook;

#[async_trait::async_trait]
impl AnalyticsHook for NoAnalyticsHook {
    async fn tenant_history_written(&self, _store: &Store, _tenant_id: TenantId, _from: NaiveDate) {
    }
}

pub fn no_analytics_hook() -> Arc<dyn AnalyticsHook> {
    Arc::new(NoAnalyticsHook)
}

/// Signals core screens and billing take part in, kept by the edition's alerting. Core's own
/// has none.
#[async_trait::async_trait]
pub trait SignalHooks: Send + Sync {
    /// Open signals of each of `customer_ids` that has some, keyed by customer.
    async fn open_signals_by_customer(
        &self,
        _conn: &mut PgConn,
        _tenant_id: TenantId,
        _customer_ids: &[CustomerId],
        _fx: &FxTable,
    ) -> StoreResult<HashMap<CustomerId, OpenSignals>> {
        Ok(HashMap::new())
    }

    /// In the plan change's transaction, which holds the customer.
    async fn subscription_plan_changed(
        &self,
        _conn: &mut PgConn,
        _tenant_id: TenantId,
        _customer_id: CustomerId,
        _subscription_id: SubscriptionId,
        _at: NaiveDateTime,
    ) -> StoreResult<()> {
        Ok(())
    }
}

pub struct NoSignalHooks;

impl SignalHooks for NoSignalHooks {}

/// What the edition does when a member leaves an organization.
#[async_trait::async_trait]
pub trait MemberHooks: Send + Sync {
    /// In the removal transaction: what the departing member owns becomes unowned.
    async fn member_released(
        &self,
        _conn: &mut PgConn,
        _organization_id: OrganizationId,
        _user_id: UserId,
    ) -> StoreResult<()> {
        Ok(())
    }
}

pub struct NoMemberHooks;

impl MemberHooks for NoMemberHooks {}

/// Refused by core's own [`CreditControlHooks`]: the usage controls are a module.
pub const USAGE_CONTROLS_UNAVAILABLE: &str = "Usage controls are not available in this edition";

/// The money a refund or dispute took back from a top-up payment, as the customer's notice
/// states it.
pub struct ReversedPayment {
    pub currency: String,
    pub amount_minor: i64,
}

/// An entitlement's monitor as the caller derives it from the current entitlement and window.
#[derive(Debug, Clone, Copy)]
pub struct ExpectedMonitor {
    pub id: Uuid,
    pub window_start: DateTime<Utc>,
    pub limit: Decimal,
    pub grouped: bool,
}

/// A usage monitor's crossing of its limit, still held (announced, not re-armed since).
#[derive(Debug, Clone)]
pub struct LimitCrossing {
    pub monitor_id: Uuid,
    pub window_start: Option<DateTime<Utc>>,
    pub fired_at: DateTime<Utc>,
}

/// What the live usage controls hold of all of a customer's wallets.
#[derive(Debug, Clone, Default)]
pub struct LiveWallets {
    /// One per wallet, in the order given.
    pub health: Vec<WalletHealth>,
    /// Since when usage is refused, for each wallet whose usage is paused.
    pub usage_paused_at: HashMap<WalletId, DateTime<Utc>>,
    /// Each grant's remaining credit once the usage not invoiced yet is paid.
    pub remaining_by_grant: HashMap<CreditGrantId, i64>,
}

/// The documents of a credit forecast already run for the customer, and whether some were left
/// out, which a credit outlook reads instead of forecasting again.
pub struct ForecastRead<'a> {
    pub(crate) documents: &'a [ForecastDocument],
    pub(crate) partial: bool,
}

impl<'a> ForecastRead<'a> {
    pub(crate) fn new(documents: &'a [ForecastDocument], partial: bool) -> Self {
        Self { documents, partial }
    }

    pub fn documents(&self) -> &'a [ForecastDocument] {
        self.documents
    }

    pub fn partial(&self) -> bool {
        self.partial
    }
}

/// A budget a sandbox scenario seeds for a customer: its currency's whole spend, alerting at the
/// default thresholds.
#[derive(Debug, Clone)]
pub struct SeededBudget {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub currency: String,
    pub amount_minor: i64,
    /// Refuses usage once reached, rather than only alerting.
    pub hard: bool,
}

/// The usage controls that watch usage live: automatic top-ups, low-balance alerts, pausing usage
/// at zero, live balances and usage monitors. Write methods run in the caller's transaction, at
/// its position, under the customer lock it holds. Core's own runs none of them.
#[async_trait::async_trait]
pub trait CreditControlHooks: Send + Sync {
    /// Whether the edition runs the usage controls: pausing usage, invoicing it up to a cap.
    fn available(&self) -> bool {
        false
    }

    /// After a movement of the wallets' invoiced value; `wallet_ids` are sorted and distinct.
    async fn balance_changed(
        &self,
        _conn: &mut PgConn,
        _tenant_id: TenantId,
        _wallet_ids: &[WalletId],
        _now: DateTime<Utc>,
    ) -> StoreResult<()> {
        Ok(())
    }

    /// An automatic top-up's payment funded `wallet`.
    async fn automatic_purchase_settled(
        &self,
        _conn: &mut PgConn,
        _wallet: &CustomerCreditAccountRow,
        _now: DateTime<Utc>,
    ) -> StoreResult<()> {
        Ok(())
    }

    /// An automatic top-up ended without capturing anything; `operation` is its anchor as it was
    /// before it failed. A `replayed` failure happened at `now`, in the past.
    #[allow(clippy::too_many_arguments)]
    async fn automatic_purchase_failed(
        &self,
        _conn: &mut PgConn,
        _lock: &CustomerLock,
        _operation: &CustomerCreditOperationRow,
        _reason: String,
        _decline_kind: Option<DeclineKind>,
        _now: DateTime<Utc>,
        _replayed: bool,
    ) -> StoreResult<()> {
        Ok(())
    }

    /// The payment of a top-up (`operation`) was refunded or disputed.
    async fn purchase_payment_reversed(
        &self,
        _conn: &mut PgConn,
        _lock: &CustomerLock,
        _operation: &CustomerCreditOperationRow,
        _reversed: ReversedPayment,
        _now: DateTime<Utc>,
    ) -> StoreResult<()> {
        Ok(())
    }

    /// The price the credit of the locked customers' wallets sells at changed with their currency.
    async fn wallet_price_changed(
        &self,
        _conn: &mut PgConn,
        _locks: &[&CustomerLock],
        _now: DateTime<Utc>,
    ) -> StoreResult<()> {
        Ok(())
    }

    /// After a credit unit's price or sellability change committed, outside its transaction.
    async fn credit_unit_price_changed(
        &self,
        _store: &Store,
        _tenant_id: TenantId,
        _unit_id: CreditUnitId,
    ) -> StoreResult<()> {
        Ok(())
    }

    /// Applies the live settings of an operator's wallet configuration, after its unit price
    /// (`price_changed` when that moved). Returns the wallet's automatic top-up as it now stands.
    async fn configure_wallet_controls(
        &self,
        conn: &mut PgConn,
        _lock: &CustomerLock,
        command: &ConfigureCreditWallet,
        _price_changed: bool,
        _now: DateTime<Utc>,
    ) -> StoreResult<Option<AutomaticTopUp>> {
        let account =
            CustomerCreditAccountRow::find_by_id(conn, command.tenant_id, command.wallet_id)
                .await?
                .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        // A request repeating the stored settings changes nothing.
        let changes = !matches!(command.auto_top_up, AutoTopUpChange::Keep)
            || command
                .low_balance_thresholds_minor
                .as_ref()
                .is_some_and(|thresholds| *thresholds != account.low_balance_thresholds_minor)
            || command
                .usage_at_zero
                .is_some_and(|usage| UsageAtZeroEnum::from(usage) != account.usage_at_zero);
        if changes {
            return Err(Report::new(StoreError::InvalidArgument(
                USAGE_CONTROLS_UNAVAILABLE.to_owned(),
            )));
        }
        Ok(None)
    }

    /// Some(reason) when an automatic top-up's payment must not be confirmed. Core's own never
    /// confirms one: nothing re-checks its terms.
    async fn confirmation_cancelled(
        &self,
        _store: &Store,
        _tenant_id: TenantId,
        _customer_id: CustomerId,
        transaction: &PaymentTransaction,
    ) -> StoreResult<Option<String>> {
        Ok(
            (transaction.payment_type == crate::domain::enums::PaymentTypeEnum::AutoTopup)
                .then(|| USAGE_CONTROLS_UNAVAILABLE.to_owned()),
        )
    }

    async fn automatic_top_up(
        &self,
        _conn: &mut PgConn,
        _wallet_id: WalletId,
    ) -> StoreResult<Option<AutomaticTopUp>> {
        Ok(None)
    }

    /// What automatic top-ups charged this calendar month, per wallet with a top-up rule.
    async fn automatic_top_up_month_spend(
        &self,
        _services: &Services,
        _tenant_id: TenantId,
        _customer_id: CustomerId,
        _wallets: &[(CreditWallet, Option<AutomaticTopUp>)],
    ) -> StoreResult<HashMap<WalletId, AutoTopUpMonthSpend>> {
        Ok(HashMap::new())
    }

    /// `wallets` are all the customer's wallets: a status depends on every wallet of its currency.
    /// Core's own reads the ledger: no live balance, and usage is never paused.
    async fn live_wallets(
        &self,
        _conn: &mut PgConn,
        _tenant_id: TenantId,
        _customer_id: CustomerId,
        wallets: &[(CreditWallet, Option<AutomaticTopUp>)],
    ) -> StoreResult<LiveWallets> {
        let mut by_currency: HashMap<String, i64> = HashMap::new();
        for (wallet, _) in wallets {
            let sum = by_currency
                .entry(wallet.currency.trim().to_uppercase())
                .or_default();
            *sum = sum.saturating_add(wallet.available_minor);
        }
        let health = wallets
            .iter()
            .map(|(wallet, _)| WalletHealth {
                wallet_id: wallet.id,
                live: None,
                status: WalletStatus::classify(
                    wallet.available_minor,
                    &wallet.low_balance_thresholds_minor,
                    None,
                    by_currency[&wallet.currency.trim().to_uppercase()],
                    false,
                ),
                live_stale: false,
            })
            .collect();
        Ok(LiveWallets {
            health,
            ..LiveWallets::default()
        })
    }

    /// Each grant's remaining credit once the usage not invoiced yet is paid.
    async fn projected_remaining_by_grant(
        &self,
        _conn: &mut PgConn,
        _tenant_id: TenantId,
        _customer_id: CustomerId,
    ) -> StoreResult<HashMap<CreditGrantId, i64>> {
        Ok(HashMap::new())
    }

    /// One per currency the customer holds a wallet in.
    async fn live_credit_balances(
        &self,
        _services: &Services,
        _tenant_id: TenantId,
        _customer_id: CustomerId,
    ) -> StoreResult<Vec<LiveCreditBalance>> {
        Ok(Vec::new())
    }

    /// Every wallet of the customer as `subscription_id` can spend it, for its usage of
    /// `product_id` when given; refused when the subscription is another customer's.
    async fn subscription_wallet_balances(
        &self,
        services: &Services,
        tenant_id: TenantId,
        customer_id: CustomerId,
        subscription_id: SubscriptionId,
        _product_id: Option<ProductId>,
    ) -> StoreResult<Vec<SubscriptionWalletBalance>> {
        let mut conn = services.store().get_conn().await?;
        let subscription =
            SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, subscription_id)
                .await?
                .subscription;
        if subscription.customer_id != customer_id {
            return Err(Report::new(StoreError::InvalidArgument(
                "the subscription belongs to another customer".to_owned(),
            )));
        }
        Ok(Vec::new())
    }

    /// The credit outlooks of the customer's live subscriptions, from a forecast already run.
    async fn credit_outlooks(
        &self,
        _services: &Services,
        _tenant_id: TenantId,
        _customer_id: CustomerId,
        _now: DateTime<Utc>,
        _forecast: ForecastRead<'_>,
    ) -> StoreResult<Vec<CreditOutlook>> {
        Ok(Vec::new())
    }

    /// The usage the live monitors last computed for the expected entitlement monitors, by
    /// monitor id; the caller reads metering for the others.
    async fn entitlement_monitor_usage(
        &self,
        _conn: &mut PgConn,
        _tenant_id: TenantId,
        _customer_id: CustomerId,
        _expected: &[ExpectedMonitor],
        _now: DateTime<Utc>,
    ) -> StoreResult<HashMap<Uuid, Decimal>> {
        Ok(HashMap::new())
    }

    /// The limit crossings `monitor_ids` still hold.
    async fn held_limit_crossings(
        &self,
        _conn: &mut PgConn,
        _tenant_id: TenantId,
        _monitor_ids: &[Uuid],
    ) -> StoreResult<Vec<LimitCrossing>> {
        Ok(Vec::new())
    }

    /// Whether the budget refuses usage once reached, as it is set now.
    async fn budget_refuses_usage(
        &self,
        _conn: &mut PgConn,
        _tenant_id: TenantId,
        _budget_id: BudgetId,
    ) -> StoreResult<bool> {
        Ok(false)
    }

    /// Seeds a sandbox scenario's budget. Core's own seeds none: budgets are usage controls.
    async fn seed_budget(&self, _services: &Services, _budget: SeededBudget) -> StoreResult<()> {
        Ok(())
    }
}

pub struct NoCreditControls;

impl CreditControlHooks for NoCreditControls {}

/// The hooks modules give the store, each with core's own when no module does.
#[derive(Clone)]
pub struct StoreHooks {
    pub signals: Arc<dyn SignalHooks>,
    pub members: Arc<dyn MemberHooks>,
    pub credit: Arc<dyn CreditControlHooks>,
}

impl Default for StoreHooks {
    fn default() -> Self {
        Self {
            signals: Arc::new(NoSignalHooks),
            members: Arc::new(NoMemberHooks),
            credit: Arc::new(NoCreditControls),
        }
    }
}
