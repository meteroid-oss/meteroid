use super::model;
use crate::AppState;
use crate::currencies::mapping::from_str as currency_from_str;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{AliasOr, BaseId, CustomerId, SubscriptionId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_app::errors::RestApiError;
use meteroid_app::rest::denomination::{Denomination, Denominations, requested_code};
use meteroid_rest_common::QueryParams;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_rest_common::model::PaginationExt;
use meteroid_store::domain::Customer;
use meteroid_store::domain::credit::{
    AutomaticTopUp, CreditApplication, CreditGrant, CreditGrantState, CreditWallet,
};
use meteroid_store::repositories::{CustomerCreditWalletsInterface, CustomersInterface};
use serde::Deserialize;
use std::collections::BTreeMap;
use validator::Validate;

const DEFAULT_PER_PAGE: u32 = 10;

pub async fn resolve_customer(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    id_or_alias: AliasOr<CustomerId>,
) -> Result<Customer, RestApiError> {
    state
        .store
        .find_customer_by_id_or_alias(id_or_alias, auth.tenant_id)
        .await
        .map_err(RestApiError::from)
}

/// Path of `PATCH /customers/{id_or_alias}/wallets/{currency}`.
#[derive(Debug, Clone, Deserialize, Validate)]
pub struct WalletPath {
    #[validate(nested)]
    pub id_or_alias: AliasOr<CustomerId>,
    pub currency: String,
}

fn page_bounds(pagination: &meteroid_rest_common::model::PaginatedRequest) -> (u32, i64, i64) {
    let per_page = pagination.per_page.unwrap_or(DEFAULT_PER_PAGE);
    let page = pagination.page.unwrap_or(0);
    (
        per_page,
        i64::from(per_page),
        i64::from(page) * i64::from(per_page),
    )
}

fn total_pages(total_items: u64, per_page: u32) -> u32 {
    u32::try_from(total_items.div_ceil(u64::from(per_page))).unwrap_or(u32::MAX)
}

/// List wallets
///
/// A customer's credit per currency and credit unit: the credit on account, the usage not
/// invoiced yet and the balance left after it, with the wallet's alerts and automatic top-up. The
/// customer's own currency comes first, even when empty. With `subscription_id`, each wallet
/// counts only the credit that subscription can use.
#[utoipa::path(
    get,
    tag = "Credits",
    path = "/api/v1/customers/{id_or_alias}/wallets",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
        model::WalletListRequest,
    ),
    responses(
        (status = 200, description = "The customer's wallets", body = model::WalletListResponse),
        (status = 400, description = "Invalid request, or the subscription belongs to another customer", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer or subscription not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_wallets(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
    Valid(QueryParams(request)): Valid<QueryParams<model::WalletListRequest>>,
) -> Result<Json<model::WalletListResponse>, RestApiError> {
    let customer = resolve_customer(&state, &auth, id_or_alias).await?;
    let subscription_balances = match request.subscription_id {
        Some(subscription_id) => Some(
            state
                .store
                .hooks
                .credit
                .subscription_wallet_balances(
                    &state.services,
                    auth.tenant_id,
                    customer.id,
                    subscription_id,
                    None,
                )
                .await
                .map_err(RestApiError::from)?,
        ),
        None => None,
    };
    let wallets = state
        .services
        .customer_credit_wallets_with_auto_top_up(auth.tenant_id, customer.id)
        .await
        .map_err(RestApiError::from)?;
    let customer_currency = customer.currency.trim().to_ascii_uppercase();
    let mut by_currency: BTreeMap<String, model::Wallet> = BTreeMap::new();
    by_currency.insert(
        customer_currency.clone(),
        empty_wallet(Denomination::of_currency(currency_from_str(
            &customer_currency,
        )?)),
    );
    for (wallet, auto_top_up) in &wallets {
        let entry = match by_currency.entry(wallet.currency.clone()) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(empty_wallet(wallet_denomination(wallet)?))
            }
        };
        apply_wallet(entry, wallet, auto_top_up.as_ref(), request.subscription_id)?;
        if let Some(balance) = subscription_balances
            .iter()
            .flatten()
            .find(|balance| balance.wallet_id == wallet.id && balance.projected)
        {
            entry.apply_subscription_balance(
                balance.balance_minor,
                balance.shared_minor,
                balance.unbilled_minor,
                balance.as_of,
            );
        }
    }
    if subscription_balances.is_none() {
        let live = state
            .store
            .hooks
            .credit
            .live_credit_balances(&state.services, auth.tenant_id, customer.id)
            .await
            .map_err(RestApiError::from)?;
        for balance in live.iter().filter(|balance| balance.projected) {
            if let Some(entry) = by_currency.get_mut(&balance.currency) {
                entry.apply_live_balance(balance);
            }
        }
    }
    let mut data: Vec<model::Wallet> = Vec::with_capacity(by_currency.len());
    if let Some(own) = by_currency.remove(&customer_currency) {
        data.push(own);
    }
    data.extend(by_currency.into_values());
    Ok(Json(model::WalletListResponse { data }))
}

/// The currency, or the credit unit with its precision, a wallet holds.
pub fn wallet_denomination(wallet: &CreditWallet) -> Result<Denomination, RestApiError> {
    match &wallet.credit_unit {
        Some(unit) => Ok(Denomination::of_unit(unit.code.as_str(), unit.precision)),
        None => currency_from_str(&wallet.currency).map(Denomination::of_currency),
    }
}

pub fn empty_wallet(denomination: Denomination) -> model::Wallet {
    model::Wallet {
        currency: denomination.currency,
        credit_unit: denomination.credit_unit,
        credit_unit_precision: denomination.credit_unit_precision,
        credit_on_account: 0,
        credit_on_account_usage_only: 0,
        subscription_credit: Vec::new(),
        upcoming_credit: 0,
        low_balance_thresholds: Vec::new(),
        usage_at_zero: model::UsageAtZero::FollowPlan,
        pauses_at_zero: false,
        auto_top_up: None,
        unit_price: None,
        unit_price_currency: None,
        balance: None,
        shared_balance: None,
        unbilled_usage: None,
        as_of: None,
    }
}

/// The customer's one `CreditWallet` of a denomination as the REST API returns it: the credit
/// only usage can draw is the sum of its usage-only grants, and each subscription's own credit
/// the sum of the grants that belong to it. With `usable_by`, only the grants that subscription
/// can use count.
pub fn apply_wallet(
    entry: &mut model::Wallet,
    wallet: &CreditWallet,
    auto_top_up: Option<&AutomaticTopUp>,
    usable_by: Option<SubscriptionId>,
) -> Result<(), RestApiError> {
    let usable = |grant: &&CreditGrant| {
        usable_by.is_none_or(|subscription_id| {
            grant
                .scope
                .subscription_id
                .is_none_or(|own| own == subscription_id)
        })
    };
    match usable_by {
        None => {
            entry.credit_on_account = wallet.available_minor;
            entry.upcoming_credit = wallet.pending_minor;
        }
        Some(_) => {
            entry.credit_on_account = 0;
            entry.upcoming_credit = 0;
            for grant in wallet.grants.iter().filter(usable) {
                match grant.state {
                    CreditGrantState::Active => {
                        entry.credit_on_account =
                            checked_add(entry.credit_on_account, grant.remaining_minor)?;
                    }
                    CreditGrantState::Pending => {
                        entry.upcoming_credit =
                            checked_add(entry.upcoming_credit, grant.remaining_minor)?;
                    }
                    CreditGrantState::Depleted | CreditGrantState::Expired => {}
                }
            }
        }
    }
    entry.credit_on_account_usage_only = 0;
    entry.subscription_credit.clear();
    for grant in wallet
        .grants
        .iter()
        .filter(usable)
        .filter(|grant| grant.state == CreditGrantState::Active)
    {
        if grant.scope.application == CreditApplication::Usage {
            entry.credit_on_account_usage_only =
                checked_add(entry.credit_on_account_usage_only, grant.remaining_minor)?;
        }
        if let Some(subscription_id) = grant.scope.subscription_id {
            match entry
                .subscription_credit
                .iter_mut()
                .find(|credit| credit.subscription_id == subscription_id)
            {
                Some(credit) => {
                    credit.credit_on_account =
                        checked_add(credit.credit_on_account, grant.remaining_minor)?;
                }
                None => entry.subscription_credit.push(model::SubscriptionCredit {
                    subscription_id,
                    credit_on_account: grant.remaining_minor,
                }),
            }
        }
    }
    entry
        .subscription_credit
        .sort_by_key(|credit| credit.subscription_id.as_uuid());
    entry.low_balance_thresholds = wallet.low_balance_thresholds_minor.clone();
    entry.usage_at_zero = wallet.usage_at_zero.into();
    entry.pauses_at_zero = wallet.pauses_at_zero();
    entry.auto_top_up = auto_top_up.map(Into::into);
    entry.unit_price = wallet.unit_price.as_ref().map(|price| price.unit_price);
    entry.unit_price_currency = wallet
        .unit_price
        .as_ref()
        .map(|price| currency_from_str(&price.currency))
        .transpose()?;
    Ok(())
}

fn checked_add(left: i64, right: i64) -> Result<i64, RestApiError> {
    left.checked_add(right).ok_or_else(|| {
        log::error!("Customer credit balance overflow");
        RestApiError::StoreError
    })
}

/// List credit grants
///
/// A customer's credit grants, newest first, in every currency and credit unit unless filtered.
#[utoipa::path(
    get,
    tag = "Credits",
    path = "/api/v1/customers/{id_or_alias}/credit-grants",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
        model::CreditGrantListRequest,
    ),
    responses(
        (status = 200, description = "Credit grants, newest first", body = model::CreditGrantListResponse),
        (status = 400, description = "Invalid request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_credit_grants(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
    Valid(QueryParams(request)): Valid<QueryParams<model::CreditGrantListRequest>>,
) -> Result<Json<model::CreditGrantListResponse>, RestApiError> {
    let customer = resolve_customer(&state, &auth, id_or_alias).await?;
    let (per_page, limit, offset) = page_bounds(&request.pagination);
    let currency = match (&request.currency, &request.credit_unit) {
        (None, None) => None,
        (currency, credit_unit) => Some(requested_code(
            currency.as_ref(),
            credit_unit.as_deref(),
            &customer.currency,
        )?),
    };
    let (grants, total_items) = state
        .store
        .customer_credit_grants(
            auth.tenant_id,
            customer.id,
            currency.as_deref(),
            request.subscription_filter()?,
            request.status.map(Into::into),
            limit,
            offset,
        )
        .await
        .map_err(RestApiError::from)?;
    let denominations = Denominations::load(
        &state.store,
        auth.tenant_id,
        grants.iter().map(|listed| listed.currency.as_str()),
    )
    .await?;
    let plans = state
        .store
        .credit_grant_plans(auth.tenant_id, grants.iter().map(|listed| &listed.grant))
        .await
        .map_err(RestApiError::from)?;
    Ok(Json(model::CreditGrantListResponse {
        data: grants
            .iter()
            .map(|listed| {
                model::CreditGrant::new(&listed.grant, &denominations.of(&listed.currency)?, &plans)
            })
            .collect::<Result<_, _>>()?,
        pagination_meta: request
            .pagination
            .into_response(total_pages(total_items, per_page), total_items),
    }))
}

/// List credit ledger entries
///
/// Every movement of a customer's credit in one currency or credit unit, newest first, with the
/// credit on account after it: credit granted, applied to invoices, given back, expired or
/// rolled over.
#[utoipa::path(
    get,
    tag = "Credits",
    path = "/api/v1/customers/{id_or_alias}/credit-ledger",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
        model::CreditLedgerListRequest,
    ),
    responses(
        (status = 200, description = "Ledger entries, newest first", body = model::LedgerListResponse),
        (status = 400, description = "Invalid request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_ledger_entries(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
    Valid(QueryParams(request)): Valid<QueryParams<model::CreditLedgerListRequest>>,
) -> Result<Json<model::LedgerListResponse>, RestApiError> {
    let customer = resolve_customer(&state, &auth, id_or_alias).await?;
    let (per_page, limit, offset) = page_bounds(&request.pagination);
    let code = requested_code(
        request.currency.as_ref(),
        request.credit_unit.as_deref(),
        &customer.currency,
    )?;
    let denomination = Denominations::load(&state.store, auth.tenant_id, [code.as_str()])
        .await?
        .of(&code)?;
    let (entries, total_items) = state
        .store
        .customer_credit_currency_ledger(auth.tenant_id, customer.id, &code, limit, offset)
        .await
        .map_err(RestApiError::from)?;
    Ok(Json(model::LedgerListResponse {
        data: entries
            .iter()
            .map(|(entry, application)| model::LedgerEntry::new(entry, &denomination, *application))
            .collect::<Result<_, _>>()?,
        pagination_meta: request
            .pagination
            .into_response(total_pages(total_items, per_page), total_items),
    }))
}
