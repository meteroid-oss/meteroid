//! A customer's credit through API v2: wallets (listed, read, configured by merge patch), credit
//! grants (listed, read, granted), the credit ledger and top-ups. Amounts are integers in minor units of the
//! wallet's currency or credit unit; a price per credit is a decimal string. Every computation is
//! v1's (`crate::credits`): the wallet view, the grant and ledger views, the gates, the store's
//! idempotent writes. Granting and selling credit take an `Idempotency-Key`, required.
use crate::AppState;
use crate::credits::model as v1;
use crate::credits::router::{apply_wallet, empty_wallet, wallet_denomination};
use crate::currencies::model::Currency;
use crate::v2::Reader;
use crate::v2::extract::{Body, Path, Query};
use crate::v2::idempotency::{IDEMPOTENCY_KEY_HEADER, committed_write};
use crate::v2::ids;
use crate::v2::pagination::{KeyCursor, PageQuery, refused_cursor};
use crate::v2::problem::{Problem, ProblemCode};
use crate::v2::refs::{
    CreditNoteRef, CreditUnitRef, InvoiceRef, ProductRef, ReferenceIds, Refs, SubscriptionRef,
};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use common_domain::ids::{
    BaseId, CreditGrantId, CustomerId, CustomerPaymentMethodId, InvoiceId, LedgerEntryId,
    PaymentTransactionId, TenantId,
};
use common_grpc::middleware::server::auth::{AuthorizedAsTenant, TenantActor};
use http::{HeaderMap, StatusCode};
use meteroid_app::features::{SCOPED_CREDIT_NOT_IN_PLAN, gates};
use meteroid_app::rest::denomination::{Denomination, Denominations, requested_code};
use meteroid_app::rest::writes::fingerprint;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::credit::{
    ApiIdempotency, AutoTopUpChange, AutomaticTopUpConfig, ConfigureCreditWallet,
    CreditApplication, CreditCost, CreditGrantState, FundingReference, GrantScope,
    GrantSubscriptionFilter, TopUpWallet, UnitPriceChange,
};
use meteroid_store::domain::{Customer, PaymentStatusEnum};
use meteroid_store::hooks::USAGE_CONTROLS_UNAVAILABLE;
use meteroid_store::repositories::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::{
    FundCustomerCreditWallet, GrantPosition, LedgerPosition,
};
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;
use meteroid_store::services::{SellCustomerCredit, TopUpWalletReceipt};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;
use utoipa::{IntoParams, ToSchema};

// ── Wallets ─────────────────────────────────────────────────────────

/// What a customer's usage does once the credit it can use runs out: what each subscription's
/// plan says, or always or never stop, for this customer. `follow_plan`: as each plan says;
/// `always_pause`: usage stops; `never_pause`: usage goes on, billed.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageAtZero {
    FollowPlan,
    AlwaysPause,
    NeverPause,
}

impl From<v1::UsageAtZero> for UsageAtZero {
    fn from(value: v1::UsageAtZero) -> Self {
        match value {
            v1::UsageAtZero::FollowPlan => UsageAtZero::FollowPlan,
            v1::UsageAtZero::AlwaysPause => UsageAtZero::AlwaysPause,
            v1::UsageAtZero::NeverPause => UsageAtZero::NeverPause,
        }
    }
}

impl From<UsageAtZero> for v1::UsageAtZero {
    fn from(value: UsageAtZero) -> Self {
        match value {
            UsageAtZero::FollowPlan => v1::UsageAtZero::FollowPlan,
            UsageAtZero::AlwaysPause => v1::UsageAtZero::AlwaysPause,
            UsageAtZero::NeverPause => v1::UsageAtZero::NeverPause,
        }
    }
}

/// Whether automatic top-up charges: `active` charges at the threshold; `disabled` is turned
/// off, its terms kept; `suspended` stopped after failed or reversed payments, a price increase
/// or a change of the customer's currency, or until the customer confirms terms an operator
/// changed.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutoTopUpStatus {
    Active,
    /// Turned off; the terms are kept.
    Disabled,
    /// Stopped after failed or reversed payments, a price increase or a change of the customer's
    /// currency, or until the customer confirms terms an operator changed.
    Suspended,
}

/// `fixed` buys `amount` each time the balance falls to the threshold; `to_target` buys back up
/// to `target`.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutoTopUpMethod {
    Fixed,
    ToTarget,
}

impl From<v1::AutoTopUpMethod> for AutoTopUpMethod {
    fn from(value: v1::AutoTopUpMethod) -> Self {
        match value {
            v1::AutoTopUpMethod::Fixed => AutoTopUpMethod::Fixed,
            v1::AutoTopUpMethod::ToTarget => AutoTopUpMethod::ToTarget,
        }
    }
}

impl From<AutoTopUpMethod> for v1::AutoTopUpMethod {
    fn from(value: AutoTopUpMethod) -> Self {
        match value {
            AutoTopUpMethod::Fixed => v1::AutoTopUpMethod::Fixed,
            AutoTopUpMethod::ToTarget => v1::AutoTopUpMethod::ToTarget,
        }
    }
}

/// Automatic top-up of the credit that pays any charge: it charges the customer's payment method
/// when the balance falls to `threshold`. Threshold, amount and target are credit in minor units
/// of the wallet; for a credit unit wallet, the minimum, maximum and caps are money in minor
/// units of the customer's currency.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct AutoTopUp {
    /// Whether it charges.
    pub status: AutoTopUpStatus,
    /// How much each top-up buys.
    pub method: AutoTopUpMethod,
    /// The balance a top-up is bought at, once it falls to it.
    pub threshold: i64,
    /// With `fixed`: the credit each top-up buys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<i64>,
    /// With `to_target`: the balance each top-up buys back up to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<i64>,
    /// Smallest single charge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<i64>,
    /// Largest single charge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<i64>,
    /// Most charged in a rolling day, tax included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_cap: Option<i64>,
    /// Most charged in a calendar month (UTC), tax included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly_cap: Option<i64>,
    /// The payment method charged; the customer's default when absent.
    #[serde(
        serialize_with = "common_domain::ids::string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub payment_method: Option<CustomerPaymentMethodId>,
    /// When the last top-up was paid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_succeeded_at: Option<ApiDateTime>,
    /// When it was suspended; absent unless `status` is `suspended`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suspended_at: Option<ApiDateTime>,
    /// Why the last charge failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// When an operator attested the customer agreed to these terms; absent when the customer
    /// confirmed them in the portal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_consent_attested_at: Option<ApiDateTime>,
}

impl From<v1::AutoTopUp> for AutoTopUp {
    fn from(value: v1::AutoTopUp) -> Self {
        AutoTopUp {
            status: match value.status {
                v1::AutoTopUpStatus::Active => AutoTopUpStatus::Active,
                v1::AutoTopUpStatus::Disabled => AutoTopUpStatus::Disabled,
                v1::AutoTopUpStatus::Suspended => AutoTopUpStatus::Suspended,
            },
            method: value.method.into(),
            threshold: value.threshold,
            amount: value.amount,
            target: value.target,
            minimum: value.minimum,
            maximum: value.maximum,
            daily_cap: value.daily_cap,
            monthly_cap: value.monthly_cap,
            payment_method: value.payment_method_id,
            last_succeeded_at: value.last_succeeded_at,
            suspended_at: value.suspended_at,
            last_error: value.last_error,
            customer_consent_attested_at: value.customer_consent_attested_at,
        }
    }
}

/// The price this customer agreed for one credit of a unit, in major units of `currency`.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct UnitPrice {
    /// The price of one credit, a decimal string.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub price: Decimal,
    /// The customer's currency (ISO 4217).
    pub currency: String,
}

/// Credit that only one subscription can use.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct SubscriptionCredit {
    /// The subscription it pays the charges of.
    pub subscription: SubscriptionRef,
    /// Its credit on account, in minor units of the wallet.
    pub credit_on_account: i64,
}

/// A customer's credit in one currency or credit unit. The customer's own currency is always
/// listed, even empty.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct Wallet {
    /// Absent for a credit unit wallet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// The credit unit; absent for a currency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<CreditUnitRef>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_unit_precision: Option<u8>,
    /// Credit granted, in effect, not applied to an invoice nor expired; usage not invoiced yet is
    /// not deducted (see `balance`).
    pub credit_on_account: i64,
    /// The part of `credit_on_account` that pays only usage charges.
    pub usage_only_credit: i64,
    /// The part of `credit_on_account` only one subscription can use, per subscription.
    pub subscription_credit: Vec<SubscriptionCredit>,
    /// Credit granted that takes effect later.
    pub upcoming_credit: i64,
    /// `credit_on_account` less the part of `unbilled_usage` credit will pay; never negative.
    /// Absent until the customer's usage was priced once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<i64>,
    /// Usage priced but not invoiced yet, tax included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unbilled_usage: Option<i64>,
    /// When `balance` and `unbilled_usage` were computed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<ApiDateTime>,
    /// Balances sending `wallet.balance_low` once crossed downward; ascending, at most 5.
    pub low_balance_thresholds: Vec<i64>,
    /// What the customer's usage does once the credit runs out, as set for this customer.
    pub usage_at_zero: UsageAtZero,
    /// Usage stops once the credit is used up, for one active subscription at least.
    pub pauses_at_zero: bool,
    /// Its automatic top-up; absent when none is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_top_up: Option<AutoTopUp>,
    /// A credit unit wallet: the customer's agreed price per credit; the list price when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<UnitPrice>,
}

/// What wallets name, to read in one round.
fn wallet_ids<'a>(wallets: impl IntoIterator<Item = &'a v1::Wallet>) -> ReferenceIds {
    let mut ids = ReferenceIds::default();
    for wallet in wallets {
        ids.credit_unit_codes.extend(wallet.credit_unit.clone());
        ids.subscriptions.extend(
            wallet
                .subscription_credit
                .iter()
                .map(|credit| credit.subscription_id),
        );
    }
    ids
}

impl Wallet {
    fn new(wallet: v1::Wallet, refs: &Refs) -> Self {
        Wallet {
            currency: wallet.currency.map(|currency| currency.to_string()),
            credit_unit: wallet
                .credit_unit
                .as_deref()
                .and_then(|code| refs.credit_unit_coded(code)),
            credit_unit_precision: wallet.credit_unit_precision,
            credit_on_account: wallet.credit_on_account,
            usage_only_credit: wallet.credit_on_account_usage_only,
            subscription_credit: wallet
                .subscription_credit
                .into_iter()
                .map(|credit| SubscriptionCredit {
                    subscription: refs.subscription(credit.subscription_id),
                    credit_on_account: credit.credit_on_account,
                })
                .collect(),
            upcoming_credit: wallet.upcoming_credit,
            balance: wallet.balance,
            unbilled_usage: wallet.unbilled_usage,
            as_of: wallet.as_of,
            low_balance_thresholds: wallet.low_balance_thresholds,
            usage_at_zero: wallet.usage_at_zero.into(),
            pauses_at_zero: wallet.pauses_at_zero,
            auto_top_up: wallet.auto_top_up.map(Into::into),
            unit_price: wallet.unit_price.zip(wallet.unit_price_currency).map(
                |(price, currency)| UnitPrice {
                    price,
                    currency: currency.to_string(),
                },
            ),
        }
    }
}

/// Every wallet of the customer, one per currency or credit unit: a list never paged.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct WalletList {
    /// The wallets, the customer's own currency first.
    pub data: Vec<Wallet>,
}

/// The terms of an automatic top-up, replaced whole. `fixed` takes `amount`, `to_target` a
/// `target` above `threshold`.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct AutoTopUpInput {
    /// How much each top-up buys.
    pub method: AutoTopUpMethod,
    /// The balance a top-up is bought at, in minor units of the wallet.
    pub threshold: i64,
    /// With `fixed`: the credit each top-up buys, in minor units of the wallet.
    #[serde(default)]
    pub amount: Option<i64>,
    /// With `to_target`: the balance each top-up buys back up to, above `threshold`.
    #[serde(default)]
    pub target: Option<i64>,
    /// Smallest single charge.
    #[serde(default)]
    pub minimum: Option<i64>,
    /// Largest single charge.
    #[serde(default)]
    pub maximum: Option<i64>,
    /// Most charged in a rolling day, tax included.
    #[serde(default)]
    pub daily_cap: Option<i64>,
    /// Most charged in a calendar month (UTC), tax included.
    #[serde(default)]
    pub monthly_cap: Option<i64>,
    /// A payment method id; the customer's default at each top-up when left out.
    #[serde(default)]
    pub payment_method: Option<String>,
    /// Credit unit wallets: authorize today's price per credit, lifting a stop a price increase
    /// caused.
    #[serde(default)]
    pub at_current_price: bool,
    /// Attests the customer agreed to automatic charges on these terms. Required to turn
    /// automatic top-up on, or to change what it charges, without the customer confirming in the
    /// portal.
    #[serde(default)]
    pub customer_consent_attested: bool,
}

/// A merge patch of a wallet's settings: a field left out is unchanged. `null` clears the
/// thresholds, turns automatic top-up off (removing its terms) and returns the customer to the
/// unit's list price; an `auto_top_up` object replaces the terms whole.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateWallet {
    /// What the customer's usage does once the credit runs out.
    #[serde(default)]
    pub usage_at_zero: Option<UsageAtZero>,
    /// Ascending, distinct, non-negative, at most 5.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<Vec<i64>>)]
    pub low_balance_thresholds: Option<Option<Vec<i64>>>,
    /// The terms of its automatic top-up, replaced whole; `null` turns it off and removes them.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<AutoTopUpInput>)]
    pub auto_top_up: Option<Option<AutoTopUpInput>>,
    /// Credit unit wallets: a decimal string, in major units of the customer's currency.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub unit_price: Option<Option<String>>,
}

/// The customer's wallets as v1 computes them: per currency or credit unit, the customer's own
/// currency first, with their live balance.
async fn wallets(
    state: &impl Reader,
    tenant_id: TenantId,
    customer: &Customer,
) -> Result<Vec<v1::Wallet>, Problem> {
    let wallets = state
        .services()
        .customer_credit_wallets_with_auto_top_up(tenant_id, customer.id)
        .await?;
    let customer_currency = customer.currency.trim().to_ascii_uppercase();
    let mut by_currency: BTreeMap<String, v1::Wallet> = BTreeMap::new();
    by_currency.insert(
        customer_currency.clone(),
        empty_wallet(Denomination::of_currency(
            crate::currencies::mapping::from_str(&customer_currency)?,
        )),
    );
    for (wallet, auto_top_up) in &wallets {
        let entry = match by_currency.entry(wallet.currency.clone()) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(empty_wallet(wallet_denomination(wallet)?))
            }
        };
        apply_wallet(entry, wallet, auto_top_up.as_ref(), None)?;
    }
    let live = state
        .store()
        .hooks
        .credit
        .live_credit_balances(state.services(), tenant_id, customer.id)
        .await?;
    for balance in live.iter().filter(|balance| balance.projected) {
        if let Some(entry) = by_currency.get_mut(&balance.currency) {
            entry.apply_live_balance(balance);
        }
    }
    let mut data = Vec::with_capacity(by_currency.len());
    if let Some(own) = by_currency.remove(&customer_currency) {
        data.push(own);
    }
    data.extend(by_currency.into_values());
    Ok(data)
}

/// List a customer's wallets
///
/// The customer's credit per currency and credit unit: on account, upcoming, its balance after
/// the usage not invoiced yet, with each wallet's alerts and automatic top-up.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/wallets",
    params(("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`")),
    responses(
        (status = 200, description = "The customer's wallets", body = WalletList),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_wallets(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
) -> Result<Json<WalletList>, Problem> {
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let wallets = wallets(&state, auth.tenant_id, &customer).await?;
    let refs = Refs::load(&state.store, auth.tenant_id, &wallet_ids(&wallets)).await?;
    let data = wallets
        .into_iter()
        .map(|wallet| Wallet::new(wallet, &refs))
        .collect();
    Ok(Json(WalletList { data }))
}

/// The wallet of a currency or credit unit.
fn in_denomination(wallet: &v1::Wallet, denomination: &Denomination) -> bool {
    let same_currency = wallet.currency.as_ref().map(ToString::to_string)
        == denomination.currency.as_ref().map(ToString::to_string);
    same_currency && wallet.credit_unit == denomination.credit_unit
}

/// Get a customer's wallet
///
/// The customer's credit in one currency or credit unit, as its wallets list it. The customer's
/// own currency always has one, even empty; another currency or credit unit has one once credit
/// was granted, bought or configured in it.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/wallets/{wallet}",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("wallet" = String, Path, description = "An ISO 4217 currency, or a credit unit code"),
    ),
    responses(
        (status = 200, description = "The wallet", body = Wallet),
        (status = 404, description = "The customer, or a wallet in that currency or credit unit, is not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_wallet(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_alias, wallet)): Path<(String, String)>,
) -> Result<Json<Wallet>, Problem> {
    let tenant_id = auth.tenant_id;
    let customer = ids::customer(&state, tenant_id, &id_or_alias).await?;
    // A path naming no currency nor credit unit names no wallet.
    let code = wallet_code(&wallet, &customer)
        .map_err(|_| Problem::not_found(format!("No wallet in '{}'", wallet.trim())))?;
    Ok(Json(wallet_in(&state, tenant_id, &customer, &code).await?))
}

/// The customer's wallet in `code` (a currency or a credit unit code), as `GET
/// /customers/{id}/wallets/{code}` reads it.
pub(crate) async fn wallet_in(
    state: &impl Reader,
    tenant_id: TenantId,
    customer: &Customer,
    code: &str,
) -> Result<Wallet, Problem> {
    let no_wallet = || Problem::not_found(format!("The customer has no wallet in {code}"));
    let denomination = Denominations::load(state.store(), tenant_id, [code])
        .await?
        .of(code)
        .map_err(|_| no_wallet())?;
    let found = wallets(state, tenant_id, customer)
        .await?
        .into_iter()
        .find(|wallet| in_denomination(wallet, &denomination))
        .ok_or_else(no_wallet)?;
    let refs = Refs::load(state.store(), tenant_id, &wallet_ids([&found])).await?;
    Ok(Wallet::new(found, &refs))
}

/// The wallet code a path names: an ISO currency (three letters) or a credit unit code.
fn wallet_code(raw: &str, customer: &Customer) -> Result<String, Problem> {
    let raw = raw.trim();
    if raw.len() == 3 {
        return Currency::from_str(&raw.to_ascii_uppercase())
            .map(|currency| currency.to_string())
            .map_err(|_| Problem::not_found(format!("No wallet in '{raw}': not a currency")));
    }
    Ok(requested_code(None, Some(raw), &customer.currency)?)
}

fn actor_user(auth: &AuthorizedAsTenant) -> Option<uuid::Uuid> {
    match auth.actor {
        TenantActor::User { .. } => Some(auth.actor.id()),
        TenantActor::ApiKey(_) | TenantActor::OAuthApp(_) => None,
    }
}

fn check_thresholds(thresholds: &[i64]) -> Result<(), Problem> {
    let refused = |message: &str| Problem::field("/low_balance_thresholds", "invalid", message);
    if thresholds.iter().any(|threshold| *threshold < 0) {
        return Err(refused("A threshold is not negative"));
    }
    if thresholds.len() > 5 {
        return Err(refused("A wallet has 5 thresholds at most"));
    }
    if !thresholds.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(refused("Thresholds are ascending, without duplicates"));
    }
    Ok(())
}

/// Configure a wallet
///
/// A merge patch of the settings of the customer's wallet in a currency or a credit unit: whether
/// usage stops once its credit is used up, the balances alerting, its automatic top-up and agreed
/// price per credit. Opens the wallet when it does not exist yet.
#[utoipa::path(
    patch,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/wallets/{wallet}",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("wallet" = String, Path, description = "An ISO 4217 currency, or a credit unit code"),
    ),
    request_body(content = UpdateWallet, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "The wallet as configured", body = Wallet),
        (status = 403, description = "Automatic top-ups are not in the organization's plan", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_wallet(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_alias, wallet)): Path<(String, String)>,
    Body(body): Body<UpdateWallet>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    let customer = ids::customer(&state, tenant_id, &id_or_alias).await?;
    let code = wallet_code(&wallet, &customer)?;
    let denomination = Denominations::load(&state.store, tenant_id, [code.as_str()])
        .await?
        .of(&code)?;
    let thresholds = body
        .low_balance_thresholds
        .clone()
        .map(Option::unwrap_or_default);
    if let Some(thresholds) = &thresholds {
        check_thresholds(thresholds)?;
    }
    let unit_price = match &body.unit_price {
        None => UnitPriceChange::Keep,
        Some(None) => UnitPriceChange::Clear,
        Some(Some(raw)) => UnitPriceChange::Set(Decimal::from_str(raw.trim()).map_err(|_| {
            Problem::field(
                "/unit_price",
                "invalid",
                "A decimal string, like \"0.0008\"",
            )
        })?),
    };
    if !state.store.hooks.credit.available()
        && (body.auto_top_up.is_some() || thresholds.is_some() || body.usage_at_zero.is_some())
    {
        return Err(Problem::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::ValidationFailed,
            USAGE_CONTROLS_UNAVAILABLE,
        ));
    }
    let auto_top_up = match body.auto_top_up.clone() {
        None => AutoTopUpChange::Keep,
        Some(None) => AutoTopUpChange::Remove,
        Some(Some(terms)) => {
            let payment_method_id = terms
                .payment_method
                .as_deref()
                .map(|raw| {
                    CustomerPaymentMethodId::from_str(raw.trim()).map_err(|_| {
                        Problem::field(
                            "/auto_top_up/payment_method",
                            "invalid",
                            format!("Not a payment method id: {raw}"),
                        )
                    })
                })
                .transpose()?;
            let config = AutomaticTopUpConfig::from(v1::AutoTopUpWriteRequest {
                method: terms.method.into(),
                threshold: terms.threshold,
                amount: terms.amount,
                target: terms.target,
                minimum: terms.minimum,
                maximum: terms.maximum,
                daily_cap: terms.daily_cap,
                monthly_cap: terms.monthly_cap,
                payment_method_id,
                at_current_price: terms.at_current_price,
                customer_consent_attested: terms.customer_consent_attested,
            });
            if terms.at_current_price {
                AutoTopUpChange::SetAtCurrentPrice(config)
            } else {
                AutoTopUpChange::Set(config)
            }
        }
    };
    // Without the plan's automatic top-ups, resending the stored terms still passes.
    let auto_top_up = if auto_top_up.sets_terms()
        && !state
            .feature_resolver
            .resolve(auth.organization_id)
            .await
            .is_enabled(gates::AUTO_TOP_UPS)
    {
        auto_top_up.resend_only()
    } else {
        auto_top_up
    };
    let attested =
        matches!(&body.auto_top_up, Some(Some(terms)) if terms.customer_consent_attested);
    // Read before anything is written: what is answered after names the unit by it.
    let unit_ref = match denomination.credit_unit.as_deref() {
        Some(unit) => top_up_refs(&state, tenant_id, Some(unit), None)
            .await?
            .credit_unit_coded(unit),
        None => None,
    };
    let opened = state
        .store
        .create_customer_credit_wallet(tenant_id, customer.id, &code)
        .await?;
    state
        .services
        .configure_credit_wallet(ConfigureCreditWallet {
            tenant_id,
            customer_id: customer.id,
            wallet_id: opened.id,
            auto_top_up,
            low_balance_thresholds_minor: thresholds,
            usage_at_zero: body
                .usage_at_zero
                .map(|usage| v1::UsageAtZero::from(usage).into()),
            unit_price,
            actor_id: actor_user(&auth),
            consent_attested_by: attested.then(|| auth.as_actor()),
        })
        .await?;
    // Configured: what it answers from here is what it did, whatever is read back.
    let known = serde_json::json!({
        "currency": denomination.currency.as_ref().map(ToString::to_string),
        "credit_unit": unit_ref,
    });
    // Read back as the list reads it, so a patch and the next list agree.
    let read = async {
        let configured = wallets(&state, tenant_id, &customer)
            .await?
            .into_iter()
            .find(|wallet| in_denomination(wallet, &denomination))
            .unwrap_or_else(|| empty_wallet(denomination));
        let refs = Refs::load(&state.store, tenant_id, &wallet_ids([&configured])).await?;
        Ok::<_, Problem>(Wallet::new(configured, &refs))
    }
    .await;
    Ok(committed_write(StatusCode::OK, read, || known))
}

// ── Credit grants ───────────────────────────────────────────────────

/// `paid` credit was bought and applies after tax, like a payment; `promotional` credit was given
/// and applies before tax, like a discount.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GrantType {
    Paid,
    Promotional,
}

impl From<v1::CreditGrantType> for GrantType {
    fn from(value: v1::CreditGrantType) -> Self {
        match value {
            v1::CreditGrantType::Paid => GrantType::Paid,
            v1::CreditGrantType::Promotional => GrantType::Promotional,
        }
    }
}

impl From<GrantType> for v1::CreditGrantType {
    fn from(value: GrantType) -> Self {
        match value {
            GrantType::Paid => v1::CreditGrantType::Paid,
            GrantType::Promotional => v1::CreditGrantType::Promotional,
        }
    }
}

/// Where a grant is: `pending` until its `effective_at`, then `active`, `depleted` once spent,
/// or `expired` once lapsed.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GrantStatus {
    /// Takes effect later.
    Pending,
    Active,
    Depleted,
    Expired,
}

impl From<v1::CreditGrantStatus> for GrantStatus {
    fn from(value: v1::CreditGrantStatus) -> Self {
        match value {
            v1::CreditGrantStatus::Pending => GrantStatus::Pending,
            v1::CreditGrantStatus::Active => GrantStatus::Active,
            v1::CreditGrantStatus::Depleted => GrantStatus::Depleted,
            v1::CreditGrantStatus::Expired => GrantStatus::Expired,
        }
    }
}

impl From<GrantStatus> for CreditGrantState {
    fn from(value: GrantStatus) -> Self {
        match value {
            GrantStatus::Pending => CreditGrantState::Pending,
            GrantStatus::Active => CreditGrantState::Active,
            GrantStatus::Depleted => CreditGrantState::Depleted,
            GrantStatus::Expired => CreditGrantState::Expired,
        }
    }
}

/// Where the credit came from: `manual` (granted by hand or through the API), `purchase` (a
/// top-up, an automatic top-up, a credit purchase or a plan's paid credit pack), `plan`
/// (included with a subscription's plan), `credit_note` (given back by a credit note),
/// `adjustment` (returned by a negative or voided invoice), `migrated` (the balance that
/// preceded credit grants), `rollover` (unspent included credit carried over when it expired),
/// `coupon` (redeemed with a promotional credit coupon) or `overpayment` (money collected above
/// what an invoice asked).
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GrantSource {
    /// Granted by hand or through the API.
    Manual,
    /// Bought: a top-up, an automatic top-up, a credit purchase or a plan's paid credit pack.
    Purchase,
    /// Included with a subscription's plan.
    Plan,
    /// Given back by a credit note.
    CreditNote,
    /// Returned by a negative or voided invoice.
    Adjustment,
    /// Carried over from the balance that preceded credit grants.
    Migrated,
    /// Unspent included credit carried over when it expired.
    Rollover,
    /// Redeemed with a promotional credit coupon.
    Coupon,
    /// Money collected above what an invoice asked.
    Overpayment,
}

impl From<v1::CreditGrantSource> for GrantSource {
    fn from(value: v1::CreditGrantSource) -> Self {
        match value {
            v1::CreditGrantSource::Manual => GrantSource::Manual,
            v1::CreditGrantSource::Purchase => GrantSource::Purchase,
            v1::CreditGrantSource::Plan => GrantSource::Plan,
            v1::CreditGrantSource::CreditNote => GrantSource::CreditNote,
            v1::CreditGrantSource::Adjustment => GrantSource::Adjustment,
            v1::CreditGrantSource::Migrated => GrantSource::Migrated,
            v1::CreditGrantSource::Rollover => GrantSource::Rollover,
            v1::CreditGrantSource::Coupon => GrantSource::Coupon,
            v1::CreditGrantSource::Overpayment => GrantSource::Overpayment,
        }
    }
}

/// What the credit pays: `all`, any charge (the default), or `usage` charges only.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppliesTo {
    #[default]
    All,
    Usage,
}

impl From<v1::CreditAppliesTo> for AppliesTo {
    fn from(value: v1::CreditAppliesTo) -> Self {
        match value {
            v1::CreditAppliesTo::All => AppliesTo::All,
            v1::CreditAppliesTo::Usage => AppliesTo::Usage,
        }
    }
}

impl From<AppliesTo> for CreditApplication {
    fn from(value: AppliesTo) -> Self {
        match value {
            AppliesTo::All => CreditApplication::Invoice,
            AppliesTo::Usage => CreditApplication::Usage,
        }
    }
}

/// The group a grant is shown in: `included` (credit a plan includes, renewed with it),
/// `prepaid` (credit bought), `bonus` (credit given) or `account_balance` (the customer's money
/// that pays any invoice: credit notes, returned invoices, the balance from before grants).
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GrantKind {
    Included,
    Prepaid,
    Bonus,
    AccountBalance,
}

impl From<v1::CreditGrantKind> for GrantKind {
    fn from(value: v1::CreditGrantKind) -> Self {
        match value {
            v1::CreditGrantKind::Included => GrantKind::Included,
            v1::CreditGrantKind::Prepaid => GrantKind::Prepaid,
            v1::CreditGrantKind::Bonus => GrantKind::Bonus,
            v1::CreditGrantKind::AccountBalance => GrantKind::AccountBalance,
        }
    }
}

/// An amount in minor units of its currency.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreditMoney {
    /// In minor units of `currency`.
    pub amount: i64,
    /// ISO 4217.
    pub currency: String,
}

/// A lot of credit given to or bought by the customer.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct CustomerCreditGrant {
    /// The grant's id.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: CreditGrantId,
    /// Whether it applies after tax (`paid`) or before (`promotional`).
    #[serde(rename = "type")]
    pub grant_type: GrantType,
    /// Where the grant is.
    pub status: GrantStatus,
    /// Where the credit came from.
    pub source: GrantSource,
    /// The currency of its amounts (ISO 4217); absent for a credit unit grant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// The credit unit its amounts are in; absent for a currency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<CreditUnitRef>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_unit_precision: Option<u8>,
    /// What the credit pays, combined with `products`.
    pub applies_to: AppliesTo,
    /// The products whose charges it pays; empty for every product.
    pub products: Vec<ProductRef>,
    /// The subscription it belongs to: it pays that subscription's charges only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionRef>,
    /// The group the grant is shown in.
    pub kind: GrantKind,
    /// The group's name as shown: "Included with Pro", "Purchased credit"…
    pub kind_label: String,
    /// Credit granted, in minor units of `currency` or `credit_unit`.
    pub amount: i64,
    /// Not spent yet, in the same minor units.
    pub remaining: i64,
    /// Lost to expiry; present only on an expired grant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expired_amount: Option<i64>,
    /// Lower is drawn first among grants paying the same charges.
    pub priority: i32,
    /// When the credit became, or becomes, spendable.
    pub effective_at: ApiDateTime,
    /// When unspent credit lapses; absent when it never does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<ApiDateTime>,
    /// When the subscription it belongs to ended: unspent credit lapses then, even before
    /// `expires_at`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<ApiDateTime>,
    /// The invoice that sold the credit, or whose void or negative total gave it back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice: Option<InvoiceRef>,
    /// The credit note that gave the credit back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_note: Option<CreditNoteRef>,
    /// Shown on the credit ledger.
    pub reason: String,
    /// Your own reference, e.g. a contract or support ticket.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// What the whole grant was paid, net of tax.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<CreditMoney>,
    /// `cost` per one credit, in major units of its currency (`"0.008"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(format = "decimal")]
    pub cost_basis: Option<String>,
    /// On the response that created paid credit costing less than its amount: the promotional
    /// grant made beside it for the difference.
    #[serde(
        serialize_with = "common_domain::ids::string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub bonus_grant: Option<CreditGrantId>,
    /// When the grant was recorded.
    pub created_at: ApiDateTime,
}

impl CustomerCreditGrant {
    fn new(grant: v1::CreditGrant, refs: &Refs) -> Self {
        CustomerCreditGrant {
            id: grant.id,
            grant_type: grant.grant_type.into(),
            status: grant.status.into(),
            source: grant.source.into(),
            currency: grant.currency.map(|currency| currency.to_string()),
            credit_unit: grant
                .credit_unit
                .as_deref()
                .and_then(|code| refs.credit_unit_coded(code)),
            credit_unit_precision: grant.credit_unit_precision,
            applies_to: grant.applies_to.into(),
            products: grant
                .applicable_product_ids
                .iter()
                .map(|id| refs.product(*id))
                .collect(),
            subscription: grant.subscription_id.map(|id| refs.subscription(id)),
            kind: grant.kind.into(),
            kind_label: grant.kind_label,
            amount: grant.amount,
            remaining: grant.remaining,
            expired_amount: grant.expired_amount,
            priority: grant.priority,
            effective_at: grant.effective_at,
            expires_at: grant.expires_at,
            ended_at: grant.ended_at,
            invoice: grant.invoice_id.map(|id| refs.invoice(id)),
            credit_note: grant.credit_note_id.map(|id| refs.credit_note(id)),
            reason: grant.reason,
            reference: grant.reference,
            cost: grant.cost.map(|cost| CreditMoney {
                amount: cost.amount,
                currency: cost.currency.to_string(),
            }),
            cost_basis: grant.cost_basis,
            bonus_grant: grant.bonus_grant_id,
            created_at: grant.created_at,
        }
    }
}

/// What the grants name, read in one round.
async fn grant_refs(
    state: &impl Reader,
    tenant_id: TenantId,
    grants: &[v1::CreditGrant],
) -> Result<Refs, Problem> {
    let mut ids = ReferenceIds::default();
    for grant in grants {
        ids.credit_unit_codes.extend(grant.credit_unit.clone());
        ids.products.extend(&grant.applicable_product_ids);
        ids.subscriptions.extend(grant.subscription_id);
        ids.invoices.extend(grant.invoice_id);
        ids.credit_notes.extend(grant.credit_note_id);
    }
    Refs::load(state.store(), tenant_id, &ids).await
}

crate::v2_list!(CustomerCreditGrantList, CustomerCreditGrant);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct CustomerCreditGrantListQuery {
    /// Only the grants in this currency (ISO 4217).
    pub currency: Option<String>,
    /// Only the grants in this credit unit (its code); excludes `currency`.
    pub credit_unit: Option<String>,
    /// Only the grants in one of these statuses: repeat the parameter for several
    /// (`?status=pending&status=active`).
    #[param(style = Form, explode)]
    pub status: Option<Vec<GrantStatus>>,
    /// Only the grants that belong to this subscription, by id or external id.
    pub subscription: Option<String>,
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
}

fn currency_at(raw: Option<&str>, path: &'static str) -> Result<Option<Currency>, Problem> {
    raw.map(|raw| {
        Currency::from_str(&raw.trim().to_ascii_uppercase()).map_err(|_| {
            Problem::field(path, "invalid", format!("Not an ISO 4217 currency: {raw}"))
        })
    })
    .transpose()
}

const GRANTS_CURSOR: &str = "grants";
const LEDGER_CURSOR: &str = "ledger";

/// The grant a page of grants starts after: its `(created_at, id)`, the list's order.
fn grant_position(query: &PageQuery) -> Result<Option<GrantPosition>, Problem> {
    query
        .after()
        .map(|after| {
            let parts = KeyCursor::decode(after, GRANTS_CURSOR, 2)?;
            Ok(GrantPosition {
                created_at: KeyCursor::parse_moment(&parts[0])?,
                id: CreditGrantId::from(KeyCursor::parse_id(&parts[1])?),
            })
        })
        .transpose()
}

fn grant_cursor(position: GrantPosition) -> String {
    KeyCursor::new(
        GRANTS_CURSOR,
        vec![
            KeyCursor::moment(position.created_at),
            position.id.as_uuid().to_string(),
        ],
    )
    .encode()
}

/// The line a page of the ledger starts after: its place in the statement's order.
fn ledger_position(query: &PageQuery) -> Result<Option<LedgerPosition>, Problem> {
    query
        .after()
        .map(|after| {
            let parts = KeyCursor::decode(after, LEDGER_CURSOR, 4)?;
            let unrecorded = match parts[1].as_str() {
                "0" => false,
                "1" => true,
                _ => return Err(refused_cursor()),
            };
            Ok(LedgerPosition {
                effective_at: KeyCursor::parse_moment(&parts[0])?,
                unrecorded,
                recorded_at: KeyCursor::parse_moment(&parts[2])?,
                id: KeyCursor::parse_id(&parts[3])?,
            })
        })
        .transpose()
}

fn ledger_cursor(position: LedgerPosition) -> String {
    KeyCursor::new(
        LEDGER_CURSOR,
        vec![
            KeyCursor::moment(position.effective_at),
            if position.unrecorded { "1" } else { "0" }.to_owned(),
            KeyCursor::moment(position.recorded_at),
            position.id.to_string(),
        ],
    )
    .encode()
}

/// A page read by keyset (`limit + 1` rows): the rows shown, and the cursor after the last one
/// when the row past `limit` says more follow.
fn cut<T, K>(
    mut rows: Vec<T>,
    limit: usize,
    key: impl Fn(&T) -> K,
    cursor: impl Fn(K) -> String,
) -> (Vec<T>, Option<String>, bool) {
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = if has_more {
        rows.last().map(|row| cursor(key(row)))
    } else {
        None
    };
    (rows, next_cursor, has_more)
}

/// List a customer's credit grants
///
/// Newest first, in every currency and credit unit unless filtered. Paged by cursor: a grant
/// recorded meanwhile is on the first page, and shifts none of the others.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/credit-grants",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        CustomerCreditGrantListQuery,
    ),
    responses(
        (status = 200, description = "A page of credit grants", body = CustomerCreditGrantList),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A filter is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_credit_grants(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
    Query(query): Query<CustomerCreditGrantListQuery>,
) -> Result<Json<CustomerCreditGrantList>, Problem> {
    let tenant_id = auth.tenant_id;
    let page = PageQuery {
        limit: query.limit,
        after: query.after.clone(),
    };
    let limit = page.limit()?;
    let after = grant_position(&page)?;
    let customer = ids::customer(&state, tenant_id, &id_or_alias).await?;
    let currency = currency_at(query.currency.as_deref(), "currency")?;
    let code = match (&currency, &query.credit_unit) {
        (None, None) => None,
        (currency, credit_unit) => Some(requested_code(
            currency.as_ref(),
            credit_unit.as_deref(),
            &customer.currency,
        )?),
    };
    let subscription = match query.subscription.as_deref() {
        Some(raw) => Some(GrantSubscriptionFilter::Of(
            ids::subscription_id(&state, tenant_id, raw).await?,
        )),
        None => None,
    };
    let grants = state
        .store
        .customer_credit_grants_after(
            tenant_id,
            customer.id,
            code.as_deref(),
            subscription,
            &query
                .status
                .iter()
                .flatten()
                .copied()
                .map(Into::into)
                .collect::<Vec<_>>(),
            after,
            i64::try_from(limit + 1).unwrap_or(i64::MAX),
        )
        .await?;
    let (grants, next_cursor, has_more) = cut(
        grants,
        limit,
        |listed| GrantPosition::of(&listed.grant),
        grant_cursor,
    );
    let denominations = Denominations::load(
        &state.store,
        tenant_id,
        grants.iter().map(|listed| listed.currency.as_str()),
    )
    .await?;
    let plans = state
        .store
        .credit_grant_plans(tenant_id, grants.iter().map(|listed| &listed.grant))
        .await?;
    let shown = grants
        .iter()
        .map(|listed| {
            v1::CreditGrant::new(&listed.grant, &denominations.of(&listed.currency)?, &plans)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let refs = grant_refs(&state, tenant_id, &shown).await?;
    Ok(Json(CustomerCreditGrantList {
        data: shown
            .into_iter()
            .map(|grant| CustomerCreditGrant::new(grant, &refs))
            .collect(),
        next_cursor,
        has_more,
    }))
}

/// Get a customer's credit grant
///
/// As the customer's credit grants list it: what is left of it, its state now, what it pays.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/credit-grants/{grant_id}",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("grant_id" = String, Path, description = "Credit grant id"),
    ),
    responses(
        (status = 200, description = "The credit grant", body = CustomerCreditGrant),
        (status = 404, description = "The customer or the grant is not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_credit_grant(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_alias, grant_id)): Path<(String, String)>,
) -> Result<Json<CustomerCreditGrant>, Problem> {
    let tenant_id = auth.tenant_id;
    let grant_id: CreditGrantId = ids::plain_id(&grant_id, "credit grant")?;
    let customer = ids::customer(&state, tenant_id, &id_or_alias).await?;
    Ok(Json(
        credit_grant(&state, tenant_id, customer.id, grant_id).await?,
    ))
}

/// The customer's grant, as `GET /customers/{id}/credit-grants/{grant_id}` reads it.
pub(crate) async fn credit_grant(
    state: &impl Reader,
    tenant_id: TenantId,
    customer_id: CustomerId,
    grant_id: CreditGrantId,
) -> Result<CustomerCreditGrant, Problem> {
    let listed = state
        .store()
        .customer_credit_grant(tenant_id, customer_id, grant_id)
        .await?
        .ok_or_else(|| Problem::not_found(format!("credit grant {grant_id}")))?;
    let denomination = Denominations::load(state.store(), tenant_id, [listed.currency.as_str()])
        .await?
        .of(&listed.currency)?;
    let plans = state
        .store()
        .credit_grant_plans(tenant_id, [&listed.grant])
        .await?;
    let shown = v1::CreditGrant::new(&listed.grant, &denomination, &plans)?;
    let refs = grant_refs(state, tenant_id, std::slice::from_ref(&shown)).await?;
    Ok(CustomerCreditGrant::new(shown, &refs))
}

/// What a credit grant costs the customer, net of tax.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CostInput {
    /// In minor units of `currency`; zero or more.
    pub amount: i64,
    /// The currency it was paid in (ISO 4217).
    pub currency: String,
}

/// Credit granted without charging or invoicing.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateCreditGrant {
    /// Positive, in minor units of the currency, or of `credit_unit`.
    pub amount: i64,
    /// The customer's currency when left out (and no `credit_unit`).
    #[serde(default)]
    pub currency: Option<String>,
    /// A credit unit (its code) instead of a currency; its credit pays any charge.
    #[serde(default)]
    pub credit_unit: Option<String>,
    /// `promotional` gives credit; `paid` records credit the customer paid for outside of
    /// Meteroid (nothing is charged nor invoiced). Sell credit with a top-up.
    #[serde(rename = "type")]
    pub grant_type: GrantType,
    /// Required for paid credit in a credit unit. Paid currency credit costing less than its
    /// amount is granted as paid for the cost and promotional for the rest (`bonus_grant`).
    #[serde(default)]
    pub cost: Option<CostInput>,
    /// What the credit pays; `all` by default.
    #[serde(default)]
    pub applies_to: AppliesTo,
    /// The subscription it belongs to, by id or external id: it pays only that subscription's
    /// charges, and lapses when it ends.
    #[serde(default)]
    pub subscription: Option<String>,
    /// Only these products' charges, by code or id; every product when left out.
    #[serde(default)]
    pub products: Option<Vec<String>>,
    /// 100 by default; lower is drawn first.
    #[serde(default)]
    pub priority: Option<i32>,
    /// Unspent credit lapses then; never when left out.
    #[serde(default)]
    pub expires_at: Option<ApiDateTime>,
    /// Shown on the credit ledger.
    #[serde(default)]
    pub reason: Option<String>,
    /// Your own reference; required for paid credit costing more than zero when the customer's
    /// invoicing entity is French (the invoice or receipt of its payment).
    #[serde(default)]
    pub reference: Option<String>,
}

fn required_key(headers: &HeaderMap, what: &str) -> Result<String, Problem> {
    headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .ok_or_else(|| {
            Problem::key_required(format!(
                "{what} is sent with an Idempotency-Key: a retry must never add credit twice"
            ))
        })
}

/// Grant credit
///
/// Credit given (`promotional`), or paid for outside of Meteroid (`paid`), without charging or
/// invoicing. An `Idempotency-Key` is required.
#[utoipa::path(
    post,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/credit-grants",
    params(("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`")),
    request_body = CreateCreditGrant,
    responses(
        (status = 201, description = "Granted", body = CustomerCreditGrant),
        (status = 403, description = "Scoped credit is not in the organization's plan", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused, or no Idempotency-Key was sent", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_credit_grant(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id_or_alias): Path<String>,
    Body(body): Body<CreateCreditGrant>,
) -> Result<Response, Problem> {
    let key = required_key(&headers, "A credit grant")?;
    let tenant_id = auth.tenant_id;
    if body.amount <= 0 {
        return Err(Problem::field(
            "/amount",
            "invalid",
            "An amount is positive",
        ));
    }
    if body.products.as_ref().is_some_and(Vec::is_empty) {
        return Err(Problem::field(
            "/products",
            "invalid",
            "Leave products out for credit paying every product",
        ));
    }
    let customer = ids::customer(&state, tenant_id, &id_or_alias).await?;
    let currency = currency_at(body.currency.as_deref(), "/currency")?;
    let code = requested_code(
        currency.as_ref(),
        body.credit_unit.as_deref(),
        &customer.currency,
    )?;
    let cost = match &body.cost {
        Some(cost) => {
            let currency = currency_at(Some(cost.currency.as_str()), "/cost/currency")?
                .map(|currency| currency.to_string())
                .unwrap_or_default();
            Some(CreditCost::at_face(cost.amount, &currency))
        }
        None => None,
    };
    let subscription_id = match body.subscription.as_deref() {
        Some(raw) => Some(ids::subscription_id(&state, tenant_id, raw).await?),
        None => None,
    };
    let mut product_ids = Vec::new();
    for (index, raw) in body.products.iter().flatten().enumerate() {
        product_ids.push(
            ids::product_id(&state, tenant_id, raw)
                .await
                .map_err(|problem| {
                    if problem.status == 404 {
                        Problem::field(
                            format!("/products/{index}"),
                            "not_found",
                            format!("product {raw} not found"),
                        )
                    } else {
                        problem
                    }
                })?,
        );
    }
    let application = CreditApplication::from(body.applies_to);
    let scoped = subscription_id.is_some()
        || matches!(application, CreditApplication::Usage)
        || body.products.is_some();
    if scoped
        && !state
            .feature_resolver
            .resolve(auth.organization_id)
            .await
            .is_enabled(gates::SCOPED_CREDIT)
    {
        return Err(Problem::feature_not_available(SCOPED_CREDIT_NOT_IN_PLAN));
    }
    let identity = ApiIdempotency::new(
        "v2.credit_grant.create",
        &key,
        fingerprint(&(customer.id, &body))?,
    )
    .map_err(|error| Problem::at(IDEMPOTENCY_KEY_HEADER, error))?;
    // Refused before the wallet is opened, so a refused grant leaves no empty wallet behind.
    if let Some(subscription_id) = subscription_id {
        state
            .store
            .check_grant_subscription(
                tenant_id,
                customer.id,
                subscription_id,
                body.credit_unit.is_none().then_some(code.as_str()),
            )
            .await
            .map_err(|error| Problem::at("/subscription", error))?;
    }
    let reference = body
        .reference
        .clone()
        .map(FundingReference::new)
        .transpose()
        .map_err(|error| Problem::at("/reference", error))?;
    let wallet = state
        .store
        .create_customer_credit_wallet(tenant_id, customer.id, &code)
        .await?;
    let funded = state
        .services
        .grant_wallet_credit_once(
            &identity,
            FundCustomerCreditWallet {
                tenant_id,
                customer_id: customer.id,
                wallet_id: wallet.id,
                amount_minor: body.amount,
                classification: v1::CreditGrantType::from(body.grant_type).into(),
                priority: body.priority.unwrap_or(DEFAULT_PRIORITY),
                expires_at: body.expires_at.map(ApiDateTime::to_utc),
                idempotency_key: identity.derived_id(tenant_id),
                reason: body
                    .reason
                    .clone()
                    .filter(|reason| !reason.trim().is_empty())
                    .unwrap_or_else(|| "Credit granted through the API".into()),
                reference,
                actor_id: actor_user(&auth),
                cost,
            },
            GrantScope {
                subscription_id,
                application,
                product_ids: Vec::new(),
            }
            .with_products(product_ids),
        )
        .await?;
    // Granted: what it answers from here is what it did, whatever is read back.
    let read = async {
        let grant = funded
            .wallet
            .grants
            .iter()
            .find(|grant| grant.id == funded.grant_id)
            .ok_or_else(|| {
                log::error!("Funded credit grant {} is missing", funded.grant_id);
                Problem::internal()
            })?;
        let plans = state.store.credit_grant_plans(tenant_id, [grant]).await?;
        let mut shown = v1::CreditGrant::new(grant, &wallet_denomination(&funded.wallet)?, &plans)?;
        shown.bonus_grant_id = funded.bonus_grant_id;
        let refs = grant_refs(&state, tenant_id, std::slice::from_ref(&shown)).await?;
        Ok::<_, Problem>(CustomerCreditGrant::new(shown, &refs))
    }
    .await;
    Ok(committed_write(StatusCode::CREATED, read, || {
        serde_json::json!({
            "id": funded.grant_id.to_string(),
            "amount": body.amount,
        })
    }))
}

// ── Credit ledger ───────────────────────────────────────────────────

/// What moved the credit: `credit` (granted), `invoice_application` (applied to a finalized
/// invoice), `invoice_return` (given back by a voided invoice or a credit note),
/// `credit_reversal` (removed by a credit note, by hand or by a reversed payment),
/// `credit_reinstatement` (restored once a reversed payment's money was recovered),
/// `credit_expiry` (lapsed unspent) or `credit_rollover` (carried out of an expiring grant into
/// the grant it rolled over to).
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LedgerEntryType {
    /// Credit granted.
    Credit,
    /// Applied to a finalized invoice.
    InvoiceApplication,
    /// Given back by an invoice: voided, or returned by a credit note.
    InvoiceReturn,
    /// Removed by a credit note, by hand, or by a reversed payment.
    CreditReversal,
    /// Removed by a reversed payment, restored once its money was recovered.
    CreditReinstatement,
    /// Lapsed unspent.
    CreditExpiry,
    /// Carried out of an expiring grant into the grant it rolled over to.
    CreditRollover,
}

impl From<v1::LedgerEntryType> for LedgerEntryType {
    fn from(value: v1::LedgerEntryType) -> Self {
        match value {
            v1::LedgerEntryType::Credit => LedgerEntryType::Credit,
            v1::LedgerEntryType::InvoiceApplication => LedgerEntryType::InvoiceApplication,
            v1::LedgerEntryType::InvoiceReturn => LedgerEntryType::InvoiceReturn,
            v1::LedgerEntryType::CreditReversal => LedgerEntryType::CreditReversal,
            v1::LedgerEntryType::CreditReinstatement => LedgerEntryType::CreditReinstatement,
            v1::LedgerEntryType::CreditExpiry => LedgerEntryType::CreditExpiry,
            v1::LedgerEntryType::CreditRollover => LedgerEntryType::CreditRollover,
        }
    }
}

/// One movement of the customer's credit.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct LedgerEntry {
    /// The movement's id.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: LedgerEntryId,
    /// The grant the credit moved in or out of.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub grant: CreditGrantId,
    /// Position in the wallet's ledger, from 1, in the order movements took effect.
    pub sequence: i64,
    /// What moved the credit.
    #[serde(rename = "type")]
    pub entry_type: LedgerEntryType,
    /// The currency of its amounts (ISO 4217); absent for a credit unit ledger.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// The credit unit its amounts are in; absent for a currency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<CreditUnitRef>,
    /// Decimal places of `credit_unit`: amounts are integers of 10^-precision credit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_unit_precision: Option<u8>,
    /// What the grant's credit pays.
    pub applies_to: AppliesTo,
    /// The subscription the grant belongs to, when it belongs to one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionRef>,
    /// Signed change of the credit on account.
    pub amount: i64,
    /// The credit on account after it.
    pub balance_after: i64,
    /// The invoice the credit was applied to or given back by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice: Option<InvoiceRef>,
    /// The credit note that gave the credit back or removed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_note: Option<CreditNoteRef>,
    /// Why the credit moved, as the ledger shows it.
    pub reason: String,
    /// Your own reference, e.g. a contract or support ticket.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// The share of its grant's cost it moved, signed like `amount`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<CreditMoney>,
    /// When the movement took effect: the ledger's order.
    pub effective_at: ApiDateTime,
    /// When it was recorded.
    pub recorded_at: ApiDateTime,
}

impl LedgerEntry {
    fn new(entry: v1::LedgerEntry, refs: &Refs) -> Self {
        LedgerEntry {
            id: entry.id,
            grant: entry.credit_grant_id,
            sequence: entry.sequence,
            entry_type: entry.entry_type.into(),
            currency: entry.currency.map(|currency| currency.to_string()),
            credit_unit: entry
                .credit_unit
                .as_deref()
                .and_then(|code| refs.credit_unit_coded(code)),
            credit_unit_precision: entry.credit_unit_precision,
            applies_to: entry.applies_to.into(),
            subscription: entry.subscription_id.map(|id| refs.subscription(id)),
            amount: entry.amount,
            balance_after: entry.balance_after,
            invoice: entry.invoice_id.map(|id| refs.invoice(id)),
            credit_note: entry.credit_note_id.map(|id| refs.credit_note(id)),
            reason: entry.reason,
            reference: entry.reference,
            cost: entry.cost.map(|cost| CreditMoney {
                amount: cost.amount,
                currency: cost.currency.to_string(),
            }),
            effective_at: entry.effective_at,
            recorded_at: entry.recorded_at,
        }
    }
}

crate::v2_list!(LedgerEntryList, LedgerEntry);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct LedgerQuery {
    /// The ledger of this currency (ISO 4217); the customer's by default.
    pub currency: Option<String>,
    /// The ledger of this credit unit (its code) instead; excludes `currency`.
    pub credit_unit: Option<String>,
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
}

/// List a customer's credit ledger
///
/// Every movement of the customer's credit in one currency or credit unit, newest first (by when
/// it took effect, then when it was recorded), with the credit on account after it. Paged by
/// cursor: a movement recorded meanwhile with an earlier date shows on the page its date puts it
/// on, and no page skips or repeats one.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/credit-ledger",
    operation_id = "list_credit_ledger",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        LedgerQuery,
    ),
    responses(
        (status = 200, description = "A page of the ledger, newest first", body = LedgerEntryList),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A filter is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_ledger(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
    Query(query): Query<LedgerQuery>,
) -> Result<Json<LedgerEntryList>, Problem> {
    let tenant_id = auth.tenant_id;
    let page = PageQuery {
        limit: query.limit,
        after: query.after.clone(),
    };
    let limit = page.limit()?;
    let after = ledger_position(&page)?;
    let customer = ids::customer(&state, tenant_id, &id_or_alias).await?;
    let currency = currency_at(query.currency.as_deref(), "currency")?;
    let code = requested_code(
        currency.as_ref(),
        query.credit_unit.as_deref(),
        &customer.currency,
    )?;
    let denomination = Denominations::load(&state.store, tenant_id, [code.as_str()])
        .await?
        .of(&code)?;
    let entries = state
        .store
        .customer_credit_currency_ledger_after(
            tenant_id,
            customer.id,
            &code,
            after,
            i64::try_from(limit + 1).unwrap_or(i64::MAX),
        )
        .await?;
    let (entries, next_cursor, has_more) =
        cut(entries, limit, |(_, position)| *position, ledger_cursor);
    let shown = entries
        .iter()
        .map(|(entry, _)| v1::LedgerEntry::new(entry, &denomination, entry.application))
        .collect::<Result<Vec<_>, _>>()?;
    let refs = ledger_refs(&state, tenant_id, &shown).await?;
    Ok(Json(LedgerEntryList {
        data: shown
            .into_iter()
            .map(|entry| LedgerEntry::new(entry, &refs))
            .collect(),
        next_cursor,
        has_more,
    }))
}

/// What ledger lines name, read in one round.
async fn ledger_refs(
    state: &impl Reader,
    tenant_id: TenantId,
    entries: &[v1::LedgerEntry],
) -> Result<Refs, Problem> {
    let mut ids = ReferenceIds::default();
    for entry in entries {
        ids.credit_unit_codes.extend(entry.credit_unit.clone());
        ids.subscriptions.extend(entry.subscription_id);
        ids.invoices.extend(entry.invoice_id);
        ids.credit_notes.extend(entry.credit_note_id);
    }
    Refs::load(state.store(), tenant_id, &ids).await
}

/// A movement of the customer's wallet in `code` (a currency or a credit unit code), as `GET
/// /customers/{id}/credit-ledger` lists it.
pub(crate) async fn ledger_entry(
    state: &impl Reader,
    tenant_id: TenantId,
    customer_id: CustomerId,
    code: &str,
    entry_id: LedgerEntryId,
) -> Result<LedgerEntry, Problem> {
    let denomination = Denominations::load(state.store(), tenant_id, [code])
        .await?
        .of(code)?;
    let entry = state
        .store()
        .customer_credit_ledger_entry(tenant_id, customer_id, code, entry_id)
        .await?
        .ok_or_else(|| Problem::not_found(format!("ledger entry {entry_id}")))?;
    let shown = v1::LedgerEntry::new(&entry, &denomination, entry.application)?;
    let refs = ledger_refs(state, tenant_id, std::slice::from_ref(&shown)).await?;
    Ok(LedgerEntry::new(shown, &refs))
}

// ── Top-ups ─────────────────────────────────────────────────────────

/// How a top-up is paid: `charge` a saved payment method now, or `send_invoice` and grant the
/// credit once it is paid.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TopUpCollection {
    #[default]
    Charge,
    SendInvoice,
}

/// Credit sold to the customer, charged or invoiced. It is paid credit that pays any charge.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateTopUp {
    /// Credit bought, positive: minor units of the currency, sold one for one untaxed (the usage
    /// it pays is taxed when invoiced); or minor units of `credit_unit`, sold at its price per
    /// credit in the customer's currency, taxed under the unit's product.
    pub amount: i64,
    /// The customer's currency when left out; `send_invoice` sells only in it.
    #[serde(default)]
    pub currency: Option<String>,
    /// A credit unit (its code) to buy credit of, instead of a currency.
    #[serde(default)]
    pub credit_unit: Option<String>,
    /// How it is paid; `charge` by default.
    #[serde(default)]
    pub collection: TopUpCollection,
    /// `charge` only, a payment method id; the customer's default when left out. Without one,
    /// the answer carries a `checkout_url` where the customer pays.
    #[serde(default)]
    pub payment_method: Option<String>,
    /// The credit bought lapses then; never when left out.
    #[serde(default)]
    pub expires_at: Option<ApiDateTime>,
    /// With `credit_unit`: this purchase's price of one credit, a decimal string in major units
    /// of the customer's currency; the customer's agreed price, else the list price, by default.
    #[serde(default)]
    #[schema(format = "decimal")]
    pub unit_price: Option<String>,
}

/// Where a top-up stands: `succeeded` (paid, the credit granted), `pending` (the charge is
/// settling), `requires_customer_action` (the customer authenticates or pays at `checkout_url`)
/// or `awaiting_payment` (the invoice is issued; the credit is granted once it is paid).
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TopUpStatus {
    /// Paid; the credit is granted.
    Succeeded,
    /// The charge is settling; the credit is granted when it settles.
    Pending,
    /// The customer authenticates or pays at `checkout_url`.
    RequiresCustomerAction,
    /// The invoice is issued; the credit is granted when it is paid.
    AwaitingPayment,
}

/// Where a top-up stands; `wallet.funded` reports its credit granted.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct TopUp {
    /// Where it stands.
    pub status: TopUpStatus,
    /// Credit bought, in minor units of `currency`, or of `credit_unit` when set.
    pub amount: i64,
    /// Charged, tax included, in minor units of `currency`; absent until a charge is made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charged_amount: Option<i64>,
    /// The money it is paid in.
    pub currency: String,
    /// The credit unit bought; absent for currency credit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<CreditUnitRef>,
    /// The invoice documenting the purchase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice: Option<InvoiceRef>,
    /// The payment charged; absent until a charge is made.
    #[serde(
        serialize_with = "common_domain::ids::string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub payment_transaction: Option<PaymentTransactionId>,
    /// Where the customer pays, while it waits for them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkout_url: Option<String>,
}

/// A top-up charged, as its receipt tells it: without the checkout url a customer action needs.
fn receipt_top_up(receipt: &TopUpWalletReceipt, credit_unit: Option<&str>, refs: &Refs) -> TopUp {
    let status = if receipt.transaction.status == PaymentStatusEnum::Settled {
        TopUpStatus::Succeeded
    } else if receipt.next_action.is_some() {
        TopUpStatus::RequiresCustomerAction
    } else {
        TopUpStatus::Pending
    };
    TopUp {
        status,
        amount: receipt.credit_minor,
        charged_amount: Some(receipt.charge_minor),
        currency: receipt.currency.clone(),
        credit_unit: credit_unit.and_then(|code| refs.credit_unit_coded(code)),
        invoice: Some(refs.invoice(receipt.invoice_id)),
        payment_transaction: Some(receipt.transaction.id),
        checkout_url: None,
    }
}

/// The credit unit and the invoice a top-up names, read in one round.
async fn top_up_refs(
    state: &AppState,
    tenant_id: TenantId,
    credit_unit: Option<&str>,
    invoice: Option<InvoiceId>,
) -> Result<Refs, Problem> {
    let ids = ReferenceIds {
        credit_unit_codes: credit_unit.map(str::to_owned).into_iter().collect(),
        invoices: invoice.into_iter().collect(),
        ..ReferenceIds::default()
    };
    Refs::load(&state.store, tenant_id, &ids).await
}

async fn charged(
    state: &AppState,
    receipt: &TopUpWalletReceipt,
    credit_unit: Option<&str>,
) -> Result<TopUp, Problem> {
    let refs = top_up_refs(
        state,
        receipt.transaction.tenant_id,
        credit_unit,
        Some(receipt.invoice_id),
    )
    .await?;
    let mut top_up = receipt_top_up(receipt, credit_unit, &refs);
    if top_up.status == TopUpStatus::RequiresCustomerAction {
        let session = receipt
            .transaction
            .checkout_session_id
            .ok_or_else(Problem::internal)?;
        top_up.checkout_url = Some(
            state
                .services
                .top_up_checkout_url(receipt.transaction.tenant_id, session)
                .await?,
        );
    }
    Ok(top_up)
}

/// Top up a wallet
///
/// Sells credit: `charge` bills a saved payment method now (or answers a `checkout_url` without
/// one), `send_invoice` grants it once the invoice is paid. An `Idempotency-Key` is required.
#[utoipa::path(
    post,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/top-ups",
    params(("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`")),
    request_body = CreateTopUp,
    responses(
        (status = 201, description = "Paid, settling, or waiting for the customer", body = TopUp),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The charge was declined", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused, or no Idempotency-Key was sent", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many top-ups for this customer in a rolling day", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_top_up(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id_or_alias): Path<String>,
    Body(body): Body<CreateTopUp>,
) -> Result<Response, Problem> {
    let key = required_key(&headers, "A top-up")?;
    let tenant_id = auth.tenant_id;
    if body.amount <= 0 {
        return Err(Problem::field(
            "/amount",
            "invalid",
            "An amount is positive",
        ));
    }
    if body.collection == TopUpCollection::SendInvoice && body.payment_method.is_some() {
        return Err(Problem::field(
            "/payment_method",
            "invalid",
            "A payment method is charged with `charge` only",
        ));
    }
    if body
        .expires_at
        .is_some_and(|at| at.to_utc() <= common_clock::now())
    {
        return Err(Problem::field(
            "/expires_at",
            "invalid",
            "The credit expires in the future",
        ));
    }
    if body.unit_price.is_some() && body.credit_unit.is_none() {
        return Err(Problem::field(
            "/unit_price",
            "invalid",
            "A price per credit is for a credit unit top-up",
        ));
    }
    let unit_price = body
        .unit_price
        .as_deref()
        .map(|raw| {
            Decimal::from_str(raw.trim()).map_err(|_| {
                Problem::field(
                    "/unit_price",
                    "invalid",
                    "A decimal string, like \"0.0008\"",
                )
            })
        })
        .transpose()?;
    let payment_method = body
        .payment_method
        .as_deref()
        .map(|raw| {
            CustomerPaymentMethodId::from_str(raw.trim()).map_err(|_| {
                Problem::field(
                    "/payment_method",
                    "invalid",
                    format!("Not a payment method id: {raw}"),
                )
            })
        })
        .transpose()?;
    let customer = ids::customer(&state, tenant_id, &id_or_alias).await?;
    let identity = ApiIdempotency::new(
        "v2.top_up.create",
        &key,
        fingerprint(&(customer.id, &body))?,
    )
    .map_err(|error| Problem::at(IDEMPOTENCY_KEY_HEADER, error))?;
    let currency = currency_at(body.currency.as_deref(), "/currency")?;
    let code = requested_code(
        currency.as_ref(),
        body.credit_unit.as_deref(),
        &customer.currency,
    )?;
    let credit_unit = body.credit_unit.is_some().then(|| code.clone());
    // Read before anything is written: what is answered after names the unit by it.
    let unit_ref = top_up_refs(&state, tenant_id, credit_unit.as_deref(), None)
        .await?
        .credit_unit_coded(&code)
        .filter(|_| credit_unit.is_some());
    let wallet = state
        .store
        .create_customer_credit_wallet(tenant_id, customer.id, &code)
        .await?;
    // Each way of collecting writes first; what it answers from there is what it did, whatever is
    // read back: `known` when the read fails.
    let (read, known) = match body.collection {
        TopUpCollection::Charge => match payment_method.or(customer.current_payment_method_id) {
            Some(payment_method_id) => {
                let receipt = state
                    .services
                    .top_up_wallet_once(
                        &identity,
                        TopUpWallet {
                            tenant_id,
                            customer_id: customer.id,
                            wallet_id: wallet.id,
                            amount_minor: body.amount,
                            payment_method_id,
                            idempotency_key: key.clone(),
                            expires_at: body.expires_at.map(ApiDateTime::to_utc),
                            unit_price,
                            expected_unit_price: None,
                        },
                    )
                    .await?;
                let mut known = receipt_top_up(&receipt, None, &Refs::default());
                known.credit_unit = unit_ref.clone();
                (
                    charged(&state, &receipt, credit_unit.as_deref()).await,
                    serde_json::to_value(known).unwrap_or_default(),
                )
            }
            None => {
                let session = state
                    .services
                    .create_top_up_checkout_once(
                        &identity,
                        tenant_id,
                        customer.id,
                        wallet.id,
                        body.amount,
                        key.clone(),
                        body.expires_at.map(ApiDateTime::to_utc),
                        unit_price,
                    )
                    .await?;
                let known = serde_json::json!({
                    "status": TopUpStatus::RequiresCustomerAction,
                    "amount": body.amount,
                    "credit_unit": unit_ref.clone(),
                });
                let read = async {
                    Ok::<_, Problem>(
                        match state.services.top_up_checkout_receipt(&session).await? {
                            Some(receipt) => {
                                charged(&state, &receipt, credit_unit.as_deref()).await?
                            }
                            None => TopUp {
                                status: TopUpStatus::RequiresCustomerAction,
                                amount: body.amount,
                                charged_amount: None,
                                currency: session.top_up()?.currency,
                                credit_unit: unit_ref.clone(),
                                invoice: None,
                                payment_transaction: None,
                                checkout_url: Some(
                                    state
                                        .services
                                        .top_up_checkout_url(tenant_id, session.id)
                                        .await?,
                                ),
                            },
                        },
                    )
                }
                .await;
                (read, known)
            }
        },
        TopUpCollection::SendInvoice => {
            let sold = state
                .services
                .sell_wallet_credit_once(
                    &identity,
                    SellCustomerCredit {
                        expires_at: body.expires_at.map(ApiDateTime::to_utc),
                        tenant_id,
                        customer_id: customer.id,
                        wallet_id: wallet.id,
                        amount_minor: body.amount,
                        expires_in_months: None,
                        note: None,
                        actor: auth.as_actor(),
                        unit_price,
                        expected_unit_price: None,
                    },
                )
                .await?;
            let invoice = &sold.invoice.invoice;
            let known = TopUp {
                status: match sold.grant_id {
                    Some(_) => TopUpStatus::Succeeded,
                    None => TopUpStatus::AwaitingPayment,
                },
                amount: body.amount,
                charged_amount: None,
                currency: invoice.currency.clone(),
                credit_unit: unit_ref,
                invoice: Some(InvoiceRef {
                    id: invoice.id,
                    number: invoice.invoice_number.clone(),
                }),
                payment_transaction: None,
                checkout_url: None,
            };
            let read = match sold.grant_id {
                Some(_) => Ok(known.clone()),
                None => state
                    .store
                    .invoice_portal_token(tenant_id, invoice.id, false)
                    .await
                    .map_err(Problem::from)
                    .map(|token| TopUp {
                        checkout_url: Some(format!(
                            "{}/portal/invoice-payment?token={token}",
                            state.portal_url.trim_end_matches('/')
                        )),
                        ..known.clone()
                    }),
            };
            (read, serde_json::to_value(known).unwrap_or_default())
        }
    };
    Ok(committed_write(StatusCode::CREATED, read, || known))
}
