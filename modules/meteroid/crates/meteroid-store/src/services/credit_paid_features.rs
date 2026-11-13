//! Which of a customer's wallets pay each metered feature's usage, read from the denominations
//! the subscriptions charge its metric in, as the access check reads them.
use crate::StoreResult;
use crate::domain::entitlements::ResolvedEntitlementValue;
use crate::errors::StoreError;
use crate::repositories::credit::allocation::{CreditDemand, GrantCandidate};
use crate::repositories::credit::customer_credit::{candidate, normalized_currency};
use crate::repositories::{EntitlementsInterface, SubscriptionInterface};
use crate::services::Services;
use crate::services::credit::billing_windows::{
    Charge, charged_denominations, subscription_windows,
};
use chrono::{DateTime, Utc};
use common_domain::ids::{
    BaseId, BillableMetricId, CustomerId, FeatureId, ProductId, SubscriptionId, TenantId, WalletId,
};
use diesel_models::customer_credit::{CustomerCreditAccountRow, CustomerCreditGrantRow};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditPayer {
    pub wallet_id: WalletId,
    pub subscription_id: SubscriptionId,
    pub product_id: Option<ProductId>,
}

#[derive(Debug, Clone)]
pub struct CreditPaidFeature {
    pub feature_id: FeatureId,
    pub metric_id: BillableMetricId,
    pub payers: Vec<CreditPayer>,
}

#[derive(Debug, Clone)]
pub(crate) struct PayerWallet {
    pub id: WalletId,
    /// Normalized, as charges name it: a credit unit's code or a currency.
    pub currency: String,
    pub credit_unit: bool,
    pub grants: Vec<GrantCandidate>,
}

/// The wallets paying `charged`: usage priced in a credit unit is owed to that unit's wallet
/// whatever it holds; money usage is paid by the currency's wallet while it holds credit that can
/// pay it now.
pub(crate) fn credit_payers(
    charged: &HashSet<Charge>,
    wallets: &[PayerWallet],
    now: DateTime<Utc>,
) -> Vec<CreditPayer> {
    let mut payers = charged
        .iter()
        .filter_map(|(subscription_id, currency, product_id)| {
            let wallet = wallets.iter().find(|wallet| wallet.currency == *currency)?;
            let demand = CreditDemand {
                subscription_id: Some(*subscription_id),
                product_id: *product_id,
                usage: true,
                amount_minor: 1,
                dated_before: None,
            };
            let pays = wallet.credit_unit
                || wallet
                    .grants
                    .iter()
                    .any(|grant| grant.spendable_at(now) && grant.pays(&demand));
            pays.then_some(CreditPayer {
                wallet_id: wallet.id,
                subscription_id: *subscription_id,
                product_id: *product_id,
            })
        })
        .collect::<Vec<_>>();
    payers.sort_by_key(|payer| {
        (
            payer.wallet_id.as_uuid(),
            payer.subscription_id.as_uuid(),
            payer.product_id.map(|id| id.as_uuid()),
        )
    });
    payers
}

impl Services {
    /// Every metered feature of the customer whose usage a wallet pays, with its payers.
    pub(crate) async fn list_credit_paid_features(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Vec<CreditPaidFeature>> {
        let now = common_domain::clock::now();
        let mut conn = self.store.get_conn().await?;
        let accounts =
            CustomerCreditAccountRow::list_for_customer(&mut conn, tenant_id, customer_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        if accounts.is_empty() {
            return Ok(Vec::new());
        }
        let metered = self
            .store
            .resolve_entitlements_for_customer(&mut conn, customer_id, tenant_id)
            .await?
            .into_iter()
            .filter_map(|entitlement| match entitlement.value {
                ResolvedEntitlementValue::Metered { metric_id, .. } => {
                    Some((entitlement.feature.id, metric_id))
                }
                ResolvedEntitlementValue::Boolean { .. }
                | ResolvedEntitlementValue::Config { .. } => None,
            })
            .collect::<Vec<_>>();
        if metered.is_empty() {
            return Ok(Vec::new());
        }
        let grants = CustomerCreditGrantRow::list_available_for_customers(
            &mut conn,
            tenant_id,
            &[customer_id],
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let wallets = accounts
            .iter()
            .map(|account| {
                Ok(PayerWallet {
                    id: account.id,
                    currency: normalized_currency(&account.currency),
                    credit_unit: account.credit_unit_id.is_some(),
                    grants: grants
                        .iter()
                        .filter(|(scoped, _)| scoped.grant.account_id == account.id)
                        .map(|(scoped, _)| candidate(scoped))
                        .collect::<StoreResult<_>>()?,
                })
            })
            .collect::<StoreResult<Vec<_>>>()?;

        let subscriptions =
            SubscriptionRow::find_active_by_customer(&mut conn, customer_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        let mut windows = Vec::new();
        for subscription in &subscriptions {
            let details = self
                .store
                .get_subscription_details_with_conn(&mut conn, tenant_id, subscription.id)
                .await?;
            windows.extend(subscription_windows(&mut conn, &details).await?);
        }

        Ok(metered
            .into_iter()
            .filter_map(|(feature_id, metric_id)| {
                let charged = charged_denominations(&windows, metric_id);
                let payers = credit_payers(&charged, &wallets, now);
                (!payers.is_empty()).then_some(CreditPaidFeature {
                    feature_id,
                    metric_id,
                    payers,
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::credit::{CreditApplication, GrantClassification, GrantScope};
    use chrono::Duration;
    use common_domain::ids::CreditGrantId;
    use uuid::Uuid;

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap() + Duration::minutes(minutes)
    }

    fn grant(remaining_minor: i64, scope: GrantScope, lapses_at: Option<i64>) -> GrantCandidate {
        GrantCandidate {
            id: CreditGrantId::new(),
            scope,
            remaining_minor,
            priority: 100,
            classification: GrantClassification::Paid,
            effective_at: at(0),
            pays_usage_from: at(0),
            lapses_at: lapses_at.map(at),
            created_at: at(0),
        }
    }

    fn scope(subscription_id: Option<SubscriptionId>, product_ids: Vec<ProductId>) -> GrantScope {
        GrantScope {
            subscription_id,
            application: CreditApplication::Usage,
            product_ids,
        }
    }

    fn wallet(currency: &str, credit_unit: bool, grants: Vec<GrantCandidate>) -> PayerWallet {
        PayerWallet {
            id: WalletId::new(),
            currency: currency.to_owned(),
            credit_unit,
            grants,
        }
    }

    fn subscription(n: u128) -> SubscriptionId {
        SubscriptionId::from(Uuid::from_u128(n))
    }

    #[test]
    fn usage_priced_in_a_credit_unit_is_paid_by_its_wallet_even_when_empty() {
        let credits = wallet("HALYARD_CREDITS", true, Vec::new());
        let usd = wallet("USD", false, vec![grant(2_500, scope(None, vec![]), None)]);
        let charged = HashSet::from([(subscription(1), "HALYARD_CREDITS".to_owned(), None)]);

        let payers = credit_payers(&charged, &[usd, credits.clone()], at(1));

        assert_eq!(
            payers,
            vec![CreditPayer {
                wallet_id: credits.id,
                subscription_id: subscription(1),
                product_id: None,
            }]
        );
    }

    #[test]
    fn money_usage_is_paid_by_the_currency_wallet_only_with_credit_that_can_pay_it_now() {
        let product = ProductId::from(Uuid::from_u128(9));
        let other_product = ProductId::from(Uuid::from_u128(10));
        let charged = HashSet::from([(subscription(1), "USD".to_owned(), Some(product))]);
        let paying = |grants| credit_payers(&charged, &[wallet("USD", false, grants)], at(1));

        assert_eq!(paying(vec![grant(100, scope(None, vec![]), None)]).len(), 1);
        assert_eq!(
            paying(vec![grant(
                100,
                scope(Some(subscription(1)), vec![product]),
                None
            )])
            .len(),
            1
        );
        assert!(paying(vec![]).is_empty(), "an empty wallet pays nothing");
        assert!(
            paying(vec![grant(0, scope(None, vec![]), None)]).is_empty(),
            "a spent grant pays nothing"
        );
        assert!(
            paying(vec![grant(100, scope(None, vec![]), Some(0))]).is_empty(),
            "a lapsed grant pays nothing"
        );
        assert!(
            paying(vec![grant(100, scope(Some(subscription(2)), vec![]), None)]).is_empty(),
            "another subscription's credit pays nothing"
        );
        assert!(
            paying(vec![grant(100, scope(None, vec![other_product]), None)]).is_empty(),
            "credit of another product pays nothing"
        );
    }

    #[test]
    fn a_denomination_without_a_wallet_has_no_payer() {
        let charged = HashSet::from([(subscription(1), "EUR".to_owned(), None)]);
        let usd = wallet("USD", false, vec![grant(100, scope(None, vec![]), None)]);

        assert!(credit_payers(&charged, &[usd], at(1)).is_empty());
    }
}
