//! Credit grants, top-ups and wallet configuration. A retry with the same Idempotency-Key and
//! body returns the original outcome; the same key with another body is refused with 409.
use super::model::{
    self, CreateCreditGrantRequest, CreateTopUpRequest, PatchWalletRequest, TopUpCollection,
    TopUpResponse, TopUpStatus,
};
use super::router::{
    WalletPath, apply_wallet, empty_wallet, resolve_customer, wallet_denomination,
};
use crate::AppState;
use crate::currencies::mapping::from_str as currency_from_str;
use crate::currencies::model::Currency;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{AliasOr, CustomerId, InvoiceId, TenantId};
use common_grpc::middleware::server::auth::{AuthorizedAsTenant, TenantActor};
use meteroid_app::errors::RestApiError;
use meteroid_app::features::{SCOPED_CREDIT_NOT_IN_PLAN, ensure_change_allowed, gates};
use meteroid_app::rest::denomination::{Denominations, requested_code};
use meteroid_app::rest::writes::{fingerprint, idempotency_key, invalid, store_error};
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_store::domain::PaymentStatusEnum;
use meteroid_store::domain::credit::{
    ApiIdempotency, AutoTopUpChange, AutomaticTopUpConfig, ConfigureCreditWallet,
    CreditApplication, FundingReference, GrantScope, TopUpWallet, UnitPriceChange,
};
use meteroid_store::errors::StoreError;
use meteroid_store::hooks::USAGE_CONTROLS_UNAVAILABLE;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;
use meteroid_store::services::SellCustomerCredit;
use std::str::FromStr;

fn actor_user(auth: &AuthorizedAsTenant) -> Option<uuid::Uuid> {
    match auth.actor {
        TenantActor::User { .. } => Some(auth.actor.id()),
        TenantActor::ApiKey(_) | TenantActor::OAuthApp(_) => None,
    }
}

async fn invoice_payment_url(
    state: &AppState,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
) -> Result<String, RestApiError> {
    let token = state
        .store
        .invoice_portal_token(tenant_id, invoice_id, false)
        .await
        .map_err(|error| {
            log::error!("Error generating an invoice payment token: {error:?}");
            RestApiError::StoreError
        })?;
    Ok(format!(
        "{}/portal/invoice-payment?token={token}",
        state.portal_url.trim_end_matches('/')
    ))
}

/// Create a credit grant
///
/// Grants credit without charging or invoicing: PROMOTIONAL credit for free, or PAID credit the
/// customer paid for outside Meteroid ("Paid outside Meteroid"). A PAID currency grant whose
/// `cost` is below its `amount` is split into a PAID grant for the cost and a PROMOTIONAL bonus
/// grant for the rest, returned as `bonus_grant_id`; a `cost` above the `amount` is refused.
/// For a customer of a French invoicing entity, PAID credit that cost more than zero needs a
/// `reference` to the invoice or receipt of its payment. Sell credit through Meteroid with a
/// top-up.
#[utoipa::path(
    post,
    tag = "Credits",
    path = "/api/v1/customers/{id_or_alias}/credit-grants",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse it only to retry the same request"),
    ),
    request_body = CreateCreditGrantRequest,
    responses(
        (status = 201, description = "Credit granted", body = model::CreditGrant),
        (status = 400, description = "Invalid request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
        (status = 409, description = "Idempotency-Key reused with another request", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_credit_grant(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
    headers: HeaderMap,
    Json(request): Json<CreateCreditGrantRequest>,
) -> Result<(StatusCode, Json<model::CreditGrant>), RestApiError> {
    let key = idempotency_key(&headers)?;
    if request.amount <= 0 {
        return Err(invalid("amount must be positive"));
    }
    if request
        .applicable_product_ids
        .as_ref()
        .is_some_and(Vec::is_empty)
    {
        return Err(invalid(
            "applicable_product_ids must not be empty; omit it to apply to every product",
        ));
    }
    let customer = resolve_customer(&state, &auth, id_or_alias).await?;
    let identity = ApiIdempotency::new(
        "credit_grant.create",
        key,
        fingerprint(&(customer.id, &request))?,
    )
    .map_err(RestApiError::from)?;
    let currency = requested_code(
        request.currency.as_ref(),
        request.credit_unit.as_deref(),
        &customer.currency,
    )?;
    let application = CreditApplication::from(request.applies_to);
    ensure_change_allowed(
        state.feature_resolver.as_ref(),
        auth.organization_id,
        gates::SCOPED_CREDIT,
        request.subscription_id.is_some()
            || matches!(application, CreditApplication::Usage)
            || request.applicable_product_ids.is_some(),
        SCOPED_CREDIT_NOT_IN_PLAN,
    )
    .await?;
    // Refused before the wallet is opened, so a refused grant leaves no empty wallet behind.
    if let Some(subscription_id) = request.subscription_id {
        state
            .store
            .check_grant_subscription(
                auth.tenant_id,
                customer.id,
                subscription_id,
                request.credit_unit.is_none().then_some(currency.as_str()),
            )
            .await
            .map_err(store_error)?;
    }
    let wallet = state
        .store
        .create_customer_credit_wallet(auth.tenant_id, customer.id, &currency)
        .await
        .map_err(store_error)?;
    let funded = state
        .services
        .grant_wallet_credit_once(
            &identity,
            FundCustomerCreditWallet {
                tenant_id: auth.tenant_id,
                customer_id: customer.id,
                wallet_id: wallet.id,
                amount_minor: request.amount,
                classification: request.grant_type.into(),
                priority: request.priority.unwrap_or(DEFAULT_PRIORITY),
                expires_at: request.expires_at.map(Into::into),
                idempotency_key: identity.derived_id(auth.tenant_id),
                reason: request
                    .reason
                    .clone()
                    .filter(|reason| !reason.trim().is_empty())
                    .unwrap_or_else(|| "Credit granted through the API".into()),
                reference: request
                    .reference
                    .clone()
                    .map(FundingReference::new)
                    .transpose()
                    .map_err(RestApiError::from)?,
                actor_id: actor_user(&auth),
                cost: request.cost.as_ref().map(Into::into),
            },
            GrantScope {
                subscription_id: request.subscription_id,
                application,
                product_ids: Vec::new(),
            }
            .with_products(request.applicable_product_ids.clone().unwrap_or_default()),
        )
        .await
        .map_err(store_error)?;
    let grant = funded
        .wallet
        .grants
        .iter()
        .find(|grant| grant.id == funded.grant_id)
        .ok_or_else(|| {
            log::error!("Funded credit grant {} is missing", funded.grant_id);
            RestApiError::StoreError
        })?;
    let plans = state
        .store
        .credit_grant_plans(auth.tenant_id, [grant])
        .await
        .map_err(RestApiError::from)?;
    let mut created =
        model::CreditGrant::new(grant, &wallet_denomination(&funded.wallet)?, &plans)?;
    created.bonus_grant_id = funded.bonus_grant_id;
    Ok((StatusCode::CREATED, Json(created)))
}

/// Create a top-up
///
/// Sells credit: CHARGE bills a saved payment method now, SEND_INVOICE grants it once paid. The
/// credit is PAID and applies to ALL charges. Currency credit is sold one for one, untaxed, as
/// its usage is taxed when invoiced. With `credit_unit`, sells credits of that unit at its
/// selling price in the customer's currency, taxed at purchase under the unit's product; refused
/// when the unit has no price in that currency or no product. Without a saved payment method,
/// CHARGE returns a hosted `checkout_url` where the customer pays.
#[utoipa::path(
    post,
    tag = "Credits",
    path = "/api/v1/customers/{id_or_alias}/top-ups",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse it only to retry the same request"),
    ),
    request_body = CreateTopUpRequest,
    responses(
        (status = 201, description = "Top-up paid, settling, or awaiting the customer", body = TopUpResponse),
        (status = 400, description = "Invalid request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer or payment method not found", body = RestErrorResponse),
        (status = 409, description = "Idempotency-Key reused with another request or the charge was declined", body = RestErrorResponse),
        (status = 429, description = "Too many top-ups for this customer in a rolling day", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_top_up(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
    headers: HeaderMap,
    Json(request): Json<CreateTopUpRequest>,
) -> Result<(StatusCode, Json<TopUpResponse>), RestApiError> {
    let key = idempotency_key(&headers)?;
    if request.amount <= 0 {
        return Err(invalid("amount must be positive"));
    }
    if request.collection == TopUpCollection::SendInvoice && request.payment_method_id.is_some() {
        return Err(invalid("payment_method_id applies to CHARGE collection"));
    }
    if request
        .expires_at
        .is_some_and(|at| at <= chrono::Utc::now().into())
    {
        return Err(invalid("expires_at must be in the future"));
    }
    if request.unit_price.is_some() && request.credit_unit.is_none() {
        return Err(invalid("unit_price applies to a credit_unit top-up"));
    }
    let customer = resolve_customer(&state, &auth, id_or_alias).await?;
    let identity =
        ApiIdempotency::new("top_up.create", key, fingerprint(&(customer.id, &request))?)
            .map_err(RestApiError::from)?;
    let wallet_code = requested_code(
        request.currency.as_ref(),
        request.credit_unit.as_deref(),
        &customer.currency,
    )?;
    let credit_unit = request.credit_unit.is_some().then(|| wallet_code.clone());
    let wallet = state
        .store
        .create_customer_credit_wallet(auth.tenant_id, customer.id, &wallet_code)
        .await
        .map_err(store_error)?;
    let response = match request.collection {
        TopUpCollection::Charge => {
            let Some(payment_method_id) = request
                .payment_method_id
                .or(customer.current_payment_method_id)
            else {
                let session = state
                    .services
                    .create_top_up_checkout_once(
                        &identity,
                        auth.tenant_id,
                        customer.id,
                        wallet.id,
                        request.amount,
                        key.to_owned(),
                        request.expires_at.map(Into::into),
                        request.unit_price,
                    )
                    .await
                    .map_err(store_error)?;
                if let Some(receipt) = state
                    .services
                    .top_up_checkout_receipt(&session)
                    .await
                    .map_err(store_error)?
                {
                    return Ok((
                        StatusCode::CREATED,
                        Json(charge_response(&state, receipt, credit_unit).await?),
                    ));
                }
                let charge_currency = session.top_up().map_err(store_error)?.currency;
                return Ok((
                    StatusCode::CREATED,
                    Json(TopUpResponse {
                        status: TopUpStatus::RequiresCustomerAction,
                        amount: request.amount,
                        charged_amount: None,
                        currency: currency_from_str(&charge_currency)?,
                        credit_unit,
                        invoice_id: None,
                        payment_transaction_id: None,
                        checkout_url: Some(
                            state
                                .services
                                .top_up_checkout_url(auth.tenant_id, session.id)
                                .await
                                .map_err(store_error)?,
                        ),
                    }),
                ));
            };
            let receipt = state
                .services
                .top_up_wallet_once(
                    &identity,
                    TopUpWallet {
                        tenant_id: auth.tenant_id,
                        customer_id: customer.id,
                        wallet_id: wallet.id,
                        amount_minor: request.amount,
                        payment_method_id,
                        idempotency_key: key.to_owned(),
                        expires_at: request.expires_at.map(Into::into),
                        unit_price: request.unit_price,
                        expected_unit_price: None,
                    },
                )
                .await
                .map_err(store_error)?;
            charge_response(&state, receipt, credit_unit).await?
        }
        TopUpCollection::SendInvoice => {
            let sold = state
                .services
                .sell_wallet_credit_once(
                    &identity,
                    SellCustomerCredit {
                        expires_at: request.expires_at.map(Into::into),
                        tenant_id: auth.tenant_id,
                        customer_id: customer.id,
                        wallet_id: wallet.id,
                        amount_minor: request.amount,
                        expires_in_months: None,
                        note: None,
                        actor: auth.as_actor(),
                        unit_price: request.unit_price,
                        expected_unit_price: None,
                    },
                )
                .await
                .map_err(store_error)?;
            let invoice = &sold.invoice.invoice;
            let (status, checkout_url) = match sold.grant_id {
                Some(_) => (TopUpStatus::Succeeded, None),
                None => (
                    TopUpStatus::AwaitingPayment,
                    Some(invoice_payment_url(&state, auth.tenant_id, invoice.id).await?),
                ),
            };
            TopUpResponse {
                status,
                amount: request.amount,
                charged_amount: None,
                currency: currency_from_str(&invoice.currency)?,
                credit_unit,
                invoice_id: Some(invoice.id),
                payment_transaction_id: None,
                checkout_url,
            }
        }
    };
    Ok((StatusCode::CREATED, Json(response)))
}

/// Configure a wallet
///
/// Settings of a customer's wallet in one currency or credit unit: whether usage is refused
/// once its credit is used up, the balances that fire `wallet.balance_low`, and its automatic
/// top-up. Opens the wallet if it does not exist yet. Partial update: omit a field to leave it
/// unchanged.
#[utoipa::path(
    patch,
    tag = "Credits",
    path = "/api/v1/customers/{id_or_alias}/wallets/{currency}",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
        ("currency" = String, Path, description = "ISO 4217 currency code, or a credit unit code"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse it only to retry the same request"),
    ),
    request_body = PatchWalletRequest,
    responses(
        (status = 200, description = "The wallet's configuration after the change", body = model::Wallet),
        (status = 400, description = "Invalid request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 403, description = "Automatic top-ups are not enabled for this organization: turning one on or changing its terms is refused", body = RestErrorResponse),
        (status = 404, description = "Customer or payment method not found", body = RestErrorResponse),
        (status = 409, description = "Idempotency-Key reused with another request", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn patch_wallet(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Valid(Path(path)): Valid<Path<WalletPath>>,
    headers: HeaderMap,
    Valid(Json(request)): Valid<Json<PatchWalletRequest>>,
) -> Result<Json<model::Wallet>, RestApiError> {
    let key = idempotency_key(&headers)?;
    let customer = resolve_customer(&state, &auth, path.id_or_alias).await?;
    let currency = if path.currency.trim().len() == 3 {
        Currency::from_str(path.currency.trim())
            .map_err(|_| invalid(format!("unknown currency code '{}'", path.currency)))?
            .to_string()
    } else {
        requested_code(None, Some(path.currency.as_str()), &customer.currency)?
    };
    let denomination = Denominations::load(&state.store, auth.tenant_id, [currency.as_str()])
        .await?
        .of(&currency)?;
    let identity = ApiIdempotency::new(
        "wallet.configure",
        key,
        fingerprint(&(customer.id, &currency, &request))?,
    )
    .map_err(RestApiError::from)?;
    if !state.store.hooks.credit.available()
        && (request.auto_top_up.is_some()
            || request.low_balance_thresholds.is_some()
            || request.usage_at_zero.is_some())
    {
        return Err(invalid(USAGE_CONTROLS_UNAVAILABLE));
    }
    let wallet = state
        .store
        .create_customer_credit_wallet(auth.tenant_id, customer.id, &currency)
        .await
        .map_err(store_error)?;
    // A partial patch: each field omitted here is resolved by the store, under the wallet's
    // lock, against whatever is stored right now (parked state included), never against a value
    // read before that lock.
    let auto_top_up = match request.auto_top_up.clone() {
        None => AutoTopUpChange::Keep,
        Some(None) => AutoTopUpChange::Remove,
        Some(Some(write)) if write.at_current_price => {
            AutoTopUpChange::SetAtCurrentPrice(AutomaticTopUpConfig::from(write))
        }
        Some(Some(write)) => AutoTopUpChange::Set(AutomaticTopUpConfig::from(write)),
    };
    // Without the gate, resending the stored terms (a full-object PATCH) still passes.
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
    state
        .services
        .configure_credit_wallet_once(
            &identity,
            ConfigureCreditWallet {
                tenant_id: auth.tenant_id,
                customer_id: customer.id,
                wallet_id: wallet.id,
                auto_top_up,
                low_balance_thresholds_minor: request.low_balance_thresholds.clone(),
                usage_at_zero: request.usage_at_zero.map(Into::into),
                unit_price: match request.unit_price {
                    None => UnitPriceChange::Keep,
                    Some(None) => UnitPriceChange::Clear,
                    Some(Some(price)) => UnitPriceChange::Set(price),
                },
                actor_id: actor_user(&auth),
                consent_attested_by: match &request.auto_top_up {
                    Some(Some(write)) if write.customer_consent_attested => Some(auth.as_actor()),
                    _ => None,
                },
            },
        )
        .await
        .map_err(|error| match error.current_context() {
            StoreError::Forbidden(_) => RestApiError::Forbidden,
            _ => store_error(error),
        })?;

    // Re-read the view `GET /wallets` builds, so a PATCH response and the next GET agree.
    let wallets = state
        .services
        .customer_credit_wallets_with_auto_top_up(auth.tenant_id, customer.id)
        .await
        .map_err(store_error)?;
    let mut response = empty_wallet(denomination);
    for (account, stored) in &wallets {
        if account.currency == currency {
            apply_wallet(&mut response, account, stored.as_ref(), None)?;
        }
    }
    if let Some(balance) = state
        .store
        .hooks
        .credit
        .live_credit_balances(&state.services, auth.tenant_id, customer.id)
        .await
        .map_err(store_error)?
        .into_iter()
        .find(|balance| balance.currency == currency && balance.projected)
    {
        response.apply_live_balance(&balance);
    }
    Ok(Json(response))
}

async fn charge_response(
    state: &AppState,
    receipt: meteroid_store::services::TopUpWalletReceipt,
    credit_unit: Option<String>,
) -> Result<TopUpResponse, RestApiError> {
    let (status, checkout_url) = if receipt.transaction.status == PaymentStatusEnum::Settled {
        (TopUpStatus::Succeeded, None)
    } else if receipt.next_action.is_some() {
        (
            TopUpStatus::RequiresCustomerAction,
            Some(
                state
                    .services
                    .top_up_checkout_url(
                        receipt.transaction.tenant_id,
                        receipt
                            .transaction
                            .checkout_session_id
                            .ok_or(RestApiError::StoreError)?,
                    )
                    .await
                    .map_err(store_error)?,
            ),
        )
    } else {
        (TopUpStatus::Pending, None)
    };
    Ok(TopUpResponse {
        status,
        amount: receipt.credit_minor,
        charged_amount: Some(receipt.charge_minor),
        currency: currency_from_str(&receipt.currency)?,
        credit_unit,
        invoice_id: Some(receipt.invoice_id),
        payment_transaction_id: Some(receipt.transaction.id),
        checkout_url,
    })
}

#[cfg(test)]
mod tests {
    use super::fingerprint;
    use crate::credits::model::PatchWalletRequest;

    fn request(auto_top_up_json: &str) -> PatchWalletRequest {
        let body = format!(r#"{{"auto_top_up":{auto_top_up_json}}}"#);
        serde_json::from_str(&body).expect("valid PatchWalletRequest body")
    }

    /// Old code: `auto_top_up` serialized as `"auto_top_up":null` whether it was omitted or sent
    /// as an explicit `null`, so an omitted-then-null replay was wrongly accepted as the same
    /// request (and, pre-finding-2-fix, would delete a rule the first call never touched).
    #[test]
    fn omitted_and_null_auto_top_up_fingerprint_differently() {
        let omitted: PatchWalletRequest =
            serde_json::from_str("{}").expect("valid empty PatchWalletRequest body");
        let null = request("null");
        let set = request(
            r#"{"method":"FIXED","threshold":100,"amount":500,"target":null,"minimum":null,"maximum":null,"daily_cap":null,"monthly_cap":null,"payment_method_id":null}"#,
        );

        let omitted_fp = fingerprint(&omitted).unwrap();
        let null_fp = fingerprint(&null).unwrap();
        let set_fp = fingerprint(&set).unwrap();

        assert_ne!(
            omitted_fp, null_fp,
            "omitted must not fingerprint like null"
        );
        assert_ne!(omitted_fp, set_fp);
        assert_ne!(null_fp, set_fp);
    }

    #[test]
    fn two_parses_of_the_same_omitted_body_fingerprint_the_same() {
        let a: PatchWalletRequest = serde_json::from_str("{}").unwrap();
        let b: PatchWalletRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(fingerprint(&a).unwrap(), fingerprint(&b).unwrap());
    }
}
