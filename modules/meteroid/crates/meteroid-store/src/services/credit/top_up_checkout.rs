use super::{invalid, precondition};
use crate::StoreResult;
use crate::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutSession, CheckoutType};
use crate::domain::credit::{TopUpWallet, TopUpWalletQuote};
use crate::services::{Services, ServicesEdge};
use chrono::{Duration, Utc};
use common_domain::ids::{
    BaseId, CheckoutSessionId, CreditUnitPriceId, CustomerId, CustomerPaymentMethodId, TenantId,
    WalletId,
};
use diesel_models::checkout_sessions::{CheckoutSessionRow, CheckoutSessionRowNew};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopUpCheckout {
    pub wallet_id: WalletId,
    pub amount_minor: i64,
    pub currency: String,
    pub idempotency_key: String,
    /// Absolute expiry of the purchased credit, chosen before the hosted page is opened.
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<Utc>>,
    /// A credit unit wallet: the price the page shows, pinned so paying charges exactly it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_sale: Option<CheckoutUnitSale>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckoutUnitSale {
    pub price_id: CreditUnitPriceId,
    /// Major units of `currency` per credit, as a decimal.
    pub unit_price: String,
    /// Sold at a price given for this purchase rather than the customer's or the list price.
    pub negotiated: bool,
    pub net_minor: i64,
    pub tax_minor: i64,
    pub charge_minor: i64,
}

impl TopUpCheckout {
    /// What the hosted page charges: the credit itself, or the price of a unit wallet's credits.
    pub fn charge_minor(&self) -> i64 {
        self.unit_sale
            .as_ref()
            .map_or(self.amount_minor, |sale| sale.charge_minor)
    }

    /// The charge before tax.
    pub fn net_minor(&self) -> i64 {
        self.unit_sale
            .as_ref()
            .map_or(self.amount_minor, |sale| sale.net_minor)
    }

    pub fn tax_minor(&self) -> i64 {
        self.unit_sale.as_ref().map_or(0, |sale| sale.tax_minor)
    }

    /// The price the page showed, which paying it must match.
    pub fn quote(&self) -> TopUpWalletQuote {
        TopUpWalletQuote {
            charge_minor: self.charge_minor(),
            currency: self.currency.clone(),
            net_minor: self.net_minor(),
            tax_minor: self.tax_minor(),
        }
    }

    fn negotiated_unit_price(&self) -> StoreResult<Option<rust_decimal::Decimal>> {
        self.unit_sale
            .as_ref()
            .filter(|sale| sale.negotiated)
            .map(|sale| {
                sale.unit_price
                    .parse()
                    .map_err(|_| precondition("the top-up checkout price is malformed"))
            })
            .transpose()
    }

    /// Whether a request under the same key asks for this very top-up.
    fn same_request(
        &self,
        wallet_id: WalletId,
        amount_minor: i64,
        expires_at: Option<chrono::DateTime<Utc>>,
        unit_price: Option<rust_decimal::Decimal>,
    ) -> StoreResult<bool> {
        Ok(self.wallet_id == wallet_id
            && self.amount_minor == amount_minor
            && self.expires_at == expires_at
            && self.negotiated_unit_price()?.map(|price| price.normalize())
                == unit_price.map(|price| price.normalize()))
    }
}

const MAX_RETURN_URL_BYTES: usize = 2048;

/// The page a hosted checkout returns to: only on the portal's own origin (no open redirect), and
/// stored without credentials, since a checkout session's URLs are read back by its viewers. The
/// portal re-attaches its own token on return.
fn portal_return_url(public_url: &str, raw: &str) -> StoreResult<String> {
    let refused = || invalid("the return URL must be a page of the customer portal");
    if raw.len() > MAX_RETURN_URL_BYTES {
        return Err(refused());
    }
    let portal = url::Url::parse(public_url).map_err(|_| refused())?;
    let mut url = url::Url::parse(raw).map_err(|_| refused())?;
    if url.origin() != portal.origin() || !matches!(url.scheme(), "http" | "https") {
        return Err(refused());
    }
    strip_credentials(&mut url).map_err(|_| refused())?;
    Ok(url.into())
}

/// Drops user info, the fragment and every credential-like query parameter: a return URL handed
/// to a provider is stored and shown by it.
pub(crate) fn strip_credentials(url: &mut url::Url) -> Result<(), ()> {
    url.set_username("")?;
    url.set_password(None)?;
    url.set_fragment(None);
    let kept = url
        .query_pairs()
        .filter(|(key, _)| !is_credential(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    if kept.is_empty() {
        url.set_query(None);
    } else {
        url.query_pairs_mut().clear().extend_pairs(kept);
    }
    Ok(())
}

fn is_credential(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    ["token", "secret", "password", "jwt", "session"]
        .iter()
        .any(|word| key.contains(word))
        || matches!(key.as_str(), "auth" | "key" | "code" | "signature" | "sig")
}

pub(crate) fn session_id(tenant: TenantId, customer: CustomerId, key: &str) -> CheckoutSessionId {
    Uuid::new_v5(
        &tenant.as_uuid(),
        format!("top-up-checkout:{customer}:{}", key.trim()).as_bytes(),
    )
    .into()
}

impl CheckoutSession {
    pub fn top_up(&self) -> StoreResult<TopUpCheckout> {
        if self.checkout_type != CheckoutType::TopUp {
            return Err(invalid("expected a top-up checkout"));
        }
        serde_json::from_value(self.metadata.clone().unwrap_or_default()).map_err(|error| {
            crate::errors::StoreError::SerdeError("top-up checkout".into(), error).into()
        })
    }
}

impl ServicesEdge {
    pub async fn create_top_up_checkout_once(
        &self,
        identity: &crate::domain::credit::ApiIdempotency,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        amount_minor: i64,
        key: String,
        expires_at: Option<chrono::DateTime<Utc>>,
        unit_price: Option<rust_decimal::Decimal>,
    ) -> StoreResult<CheckoutSession> {
        self.once(
            tenant_id,
            identity,
            self.create_top_up_checkout(
                tenant_id,
                customer_id,
                wallet_id,
                amount_minor,
                key,
                expires_at,
                unit_price,
                None,
            ),
        )
        .await
    }

    pub async fn create_top_up_checkout(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        amount_minor: i64,
        key: String,
        expires_at: Option<chrono::DateTime<Utc>>,
        unit_price: Option<rust_decimal::Decimal>,
        expected_quote: Option<TopUpWalletQuote>,
    ) -> StoreResult<CheckoutSession> {
        self.services
            .create_top_up_checkout(
                tenant_id,
                customer_id,
                wallet_id,
                amount_minor,
                key,
                expires_at,
                unit_price,
                expected_quote,
                None,
            )
            .await
    }

    /// A customer's own top-up from the portal: the hosted page returns to `return_url` once
    /// paid or abandoned.
    pub async fn create_portal_top_up_checkout(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        amount_minor: i64,
        key: String,
        expected_quote: Option<TopUpWalletQuote>,
        return_url: Option<String>,
    ) -> StoreResult<CheckoutSession> {
        self.services
            .create_top_up_checkout(
                tenant_id,
                customer_id,
                wallet_id,
                amount_minor,
                key,
                // The portal never lets a customer pick their own credit expiry.
                None,
                // The portal sells at the customer's agreed price, else the list price.
                None,
                expected_quote,
                return_url,
            )
            .await
    }

    pub async fn top_up_checkout_url(
        &self,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> StoreResult<String> {
        self.services.top_up_checkout_url(tenant_id, id).await
    }

    pub(crate) async fn complete_top_up_checkout(
        &self,
        session: CheckoutSession,
        payment_method_id: CustomerPaymentMethodId,
        amount: u64,
        currency: String,
    ) -> StoreResult<CheckoutCompletionResult> {
        if !session.can_complete() {
            return Err(precondition(
                "checkout session is expired or already completed",
            ));
        }
        let purchase = session.top_up()?;
        if u64::try_from(purchase.charge_minor()).ok() != Some(amount)
            || purchase.currency != currency
        {
            return Err(precondition(super::wallet_purchase::PRICE_CHANGED_REFRESH));
        }
        let mut conn = self.store.get_conn().await?;
        let payment_method_id =
            diesel_models::payments::PaymentTransactionRow::get_active_by_checkout_session_id(
                &mut conn,
                session.id,
                session.tenant_id,
            )
            .await?
            .and_then(|payment| payment.payment_method_id)
            .unwrap_or(payment_method_id);
        drop(conn);
        let receipt = self
            .services
            .top_up_wallet(
                TopUpWallet {
                    tenant_id: session.tenant_id,
                    customer_id: session.customer_id,
                    wallet_id: purchase.wallet_id,
                    amount_minor: purchase.amount_minor,
                    payment_method_id,
                    unit_price: purchase.negotiated_unit_price()?,
                    expected_unit_price: None,
                    idempotency_key: purchase.idempotency_key.clone(),
                    expires_at: purchase.expires_at,
                },
                // The page's price: a fresh pin at another price is refused before any charge.
                Some(purchase.quote()),
            )
            .await?;
        if receipt.transaction.status == crate::domain::PaymentStatusEnum::Settled {
            Ok(CheckoutCompletionResult::TopUpCompleted {
                transaction: receipt.transaction,
            })
        } else {
            Ok(CheckoutCompletionResult::AwaitingPayment {
                transaction: receipt.transaction,
                next_action: receipt.next_action,
            })
        }
    }
}

impl Services {
    pub async fn top_up_checkout_url(
        &self,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> StoreResult<String> {
        use crate::repositories::portal_tokens::PortalTokensInterface;

        let token = self
            .store
            .checkout_portal_token(tenant_id, id, false)
            .await?;
        Ok(format!(
            "{}/checkout?token={token}",
            self.store.settings.public_url
        ))
    }

    pub async fn create_top_up_checkout(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        amount_minor: i64,
        key: String,
        credit_expires_at: Option<chrono::DateTime<Utc>>,
        unit_price: Option<rust_decimal::Decimal>,
        expected_quote: Option<TopUpWalletQuote>,
        return_url: Option<String>,
    ) -> StoreResult<CheckoutSession> {
        if key.trim().is_empty() || key.trim().len() > 255 {
            return Err(invalid("a top-up idempotency key is 1 to 255 bytes"));
        }
        let return_url = return_url
            .map(|url| portal_return_url(&self.store.settings.public_url, url.trim()))
            .transpose()?;
        let id = session_id(tenant_id, customer_id, &key);
        // An open session keeps the price its page shows; only a new one is priced now.
        let mut conn = self.store.get_conn().await?;
        if let Some(existing) = CheckoutSessionRow::find_by_id(&mut conn, tenant_id, id).await? {
            let session: CheckoutSession = existing.into();
            let purchase = session.top_up()?;
            if !purchase.same_request(wallet_id, amount_minor, credit_expires_at, unit_price)? {
                return Err(crate::errors::StoreError::Conflict(
                    "this key already identifies a different top-up".into(),
                )
                .into());
            }
            if expected_quote.is_some_and(|quote| quote != purchase.quote()) {
                return Err(precondition(super::wallet_purchase::PRICE_CHANGED_REFRESH));
            }
            return Ok(session);
        }
        drop(conn);
        let (priced, sale) = self
            .price_wallet_top_up(tenant_id, customer_id, wallet_id, amount_minor, unit_price)
            .await?;
        if expected_quote.is_some_and(|quote| quote != priced) {
            return Err(precondition(super::wallet_purchase::PRICE_CHANGED_REFRESH));
        }
        let metadata = serde_json::to_value(TopUpCheckout {
            wallet_id,
            amount_minor,
            currency: priced.currency.clone(),
            idempotency_key: key.trim().to_owned(),
            expires_at: credit_expires_at,
            unit_sale: sale.map(|sale| CheckoutUnitSale {
                price_id: sale.price_id,
                unit_price: sale.unit_price.normalize().to_string(),
                negotiated: unit_price.is_some(),
                net_minor: priced.net_minor,
                tax_minor: priced.tax_minor,
                charge_minor: priced.charge_minor,
            }),
        })
        .map_err(|error| crate::errors::StoreError::SerdeError("top-up checkout".into(), error))?;
        let mut conn = self.store.get_conn().await?;
        let row = CheckoutSessionRowNew {
            id,
            tenant_id,
            customer_id,
            plan_version_id: None,
            billing_start_date: None,
            billing_day_anchor: None,
            net_terms: None,
            trial_duration_days: None,
            end_date: None,
            auto_advance_invoices: false,
            charge_automatically: false,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            payment_methods_config: None,
            components: None,
            add_ons: None,
            coupon_code: None,
            coupon_ids: vec![],
            expires_at: Some(Utc::now() + Duration::hours(24)),
            metadata: Some(metadata.clone()),
            checkout_type: diesel_models::enums::CheckoutTypeEnum::TopUp,
            subscription_id: None,
            change_date: None,
            success_url: return_url.clone(),
            cancel_url: return_url,
        }
        .insert_or_get(&mut conn)
        .await?;
        if row.metadata != Some(metadata) {
            return Err(crate::errors::StoreError::Conflict(
                "this key already identifies a different top-up".into(),
            )
            .into());
        }
        Ok(row.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_return_url_stays_on_the_portal_and_carries_no_credentials() {
        let portal = "https://billing.example.com";
        assert_eq!(
            portal_return_url(
                portal,
                "https://user:pw@billing.example.com/portal?token=abc&page=balance&topUpReturn=w1#t"
            )
            .unwrap(),
            "https://billing.example.com/portal?page=balance&topUpReturn=w1"
        );
        assert_eq!(
            portal_return_url(portal, "https://billing.example.com/portal?access_token=x").unwrap(),
            "https://billing.example.com/portal"
        );
        for refused in [
            "https://evil.example.com/portal",
            "http://billing.example.com/portal",
            "https://billing.example.com:8443/portal",
            "javascript:alert(1)",
            "/portal",
        ] {
            assert!(portal_return_url(portal, refused).is_err(), "{refused}");
        }
        let long = format!("{portal}/portal?page={}", "a".repeat(MAX_RETURN_URL_BYTES));
        assert!(portal_return_url(portal, &long).is_err());
        assert!(portal_return_url("http://localhost:5173", "http://localhost:5173/portal").is_ok());
    }
}
