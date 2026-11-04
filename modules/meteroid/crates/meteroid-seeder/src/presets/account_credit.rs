//! Credit of the customers of a scenario without a replayed history (`Customer::credit`), set up
//! once the subscriptions exist and before their catch-up bills them: each event runs at its
//! moment, so the invoices issued afterwards draw a grant from its date on. Wallet rules come last.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use common_domain::actor::Actor;
use common_domain::ids::{ConnectorId, CustomerId, InvoiceId, TenantId, WalletId};
use meteroid_store::domain::credit::GrantClassification;
use meteroid_store::domain::credit_units::CreditUnit;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
use meteroid_store::services::sandbox_replay::SandboxReplayInterface;
use meteroid_store::services::{InvoiceDelivery, SellCustomerCredit};
use meteroid_store::{Services, Store, StoreResult};
use uuid::Uuid;

use crate::presets::credits;
use crate::presets::replay::{
    attach_card, budget, configure_wallet, minor, set_default_payment_method,
};
use crate::presets::scenarios::domain::{Customer, Scenario};
use crate::presets::scenarios::timeline::{Action, Clock, Payable};

const BANK_TRANSFER_REFERENCE: &str = "Bank transfer";

/// Whether `action` can be part of `Customer::credit`: no subscription, invoice or usage of its own.
pub(super) fn is_account_credit(action: &Action) -> bool {
    matches!(
        action,
        Action::AddCard { .. }
            | Action::Purchase { .. }
            | Action::BankTransfer { .. }
            | Action::RedeemPromoCode { .. }
            | Action::AddCredit { .. }
            | Action::ConfigureWallet { .. }
            | Action::SetBudget(_)
    )
}

pub(super) struct AccountCredit<'a> {
    pub store: &'a Store,
    pub services: &'a Services,
    pub tenant_id: TenantId,
    pub scenario: &'a Scenario,
    pub credit_units: &'a HashMap<String, CreditUnit>,
    pub customers: &'a HashMap<String, CustomerId>,
    pub as_of: NaiveDateTime,
}

#[derive(Default)]
struct Account {
    wallets: HashMap<&'static str, WalletId>,
    payables: HashMap<Payable, InvoiceId>,
}

pub(super) async fn seed_account_credit(seed: &AccountCredit<'_>) -> StoreResult<()> {
    let clock = Clock::new(seed.as_of, 0);
    let mut timeline: Vec<(NaiveDateTime, usize, usize, &Customer, Action)> = Vec::new();
    for (index, customer) in seed.scenario.customers.iter().enumerate() {
        for (position, event) in customer.credit.iter().enumerate() {
            timeline.push((
                clock.resolve(event.at),
                index,
                position,
                customer,
                event.action.clone(),
            ));
        }
    }
    timeline.sort_by_key(|(at, index, position, _, _)| (*at, *index, *position));
    let Some((first, ..)) = timeline.first() else {
        return Ok(());
    };
    credits::create_promo_codes(
        seed.services,
        seed.tenant_id,
        &seed.scenario.promo_codes,
        *first,
    )
    .await?;

    let mut connector: Option<ConnectorId> = None;
    let mut accounts: HashMap<usize, Account> = HashMap::new();
    let mut rules = Vec::new();
    for (at, index, _, customer, action) in timeline {
        let customer_id = *seed.customers.get(&customer.name).ok_or_else(|| {
            StoreError::ValueNotFound(format!("Customer was not found {}", customer.name))
        })?;
        let account = accounts.entry(index).or_default();
        let business_time = at.and_utc();
        match action {
            Action::AddCard {
                spec, make_default, ..
            } => {
                let connector_id = match connector {
                    Some(id) => id,
                    None => {
                        let id = credits::connect_mock_provider(seed.store, seed.tenant_id).await?;
                        connector = Some(id);
                        id
                    }
                };
                let method = attach_card(
                    seed.store,
                    seed.tenant_id,
                    connector_id,
                    customer_id,
                    &spec,
                    seed.as_of,
                )
                .await?;
                if make_default {
                    set_default_payment_method(seed.store, seed.tenant_id, customer_id, method.id)
                        .await?;
                }
            }
            Action::Purchase {
                purchase,
                denomination,
                amount,
                expires_in_months,
            } => {
                let wallet_id = wallet(seed, account, customer_id, denomination).await?;
                let sold = seed
                    .services
                    .sell_customer_credit_at(
                        Uuid::now_v7(),
                        SellCustomerCredit {
                            expires_at: None,
                            tenant_id: seed.tenant_id,
                            customer_id,
                            wallet_id,
                            amount_minor: minor(amount, denomination, seed.credit_units)?,
                            expires_in_months,
                            note: None,
                            actor: Actor::System,
                            unit_price: None,
                            expected_unit_price: None,
                        },
                        business_time,
                        InvoiceDelivery::Suppressed,
                    )
                    .await?;
                account.payables.insert(purchase, sold.invoice.invoice.id);
            }
            Action::BankTransfer { payable } => {
                let invoice_id = *account.payables.get(payable).ok_or_else(|| {
                    StoreError::InvalidArgument(format!(
                        "{} pays an unknown payable {payable}",
                        customer.name
                    ))
                })?;
                seed.services
                    .settle_invoice_payment_at(
                        Actor::System,
                        seed.tenant_id,
                        invoice_id,
                        business_time,
                        Some(BANK_TRANSFER_REFERENCE.to_string()),
                        InvoiceDelivery::Suppressed,
                    )
                    .await?;
            }
            Action::RedeemPromoCode { code } => {
                seed.services
                    .redeem_coupon_credit_code_at(seed.tenant_id, customer_id, code, business_time)
                    .await?;
            }
            Action::AddCredit {
                denomination,
                amount,
                reason,
                promotional,
                expires,
            } => {
                let wallet_id = wallet(seed, account, customer_id, denomination).await?;
                seed.store
                    .fund_customer_credit_wallet_at(
                        FundCustomerCreditWallet {
                            tenant_id: seed.tenant_id,
                            customer_id,
                            wallet_id,
                            amount_minor: minor(amount, denomination, seed.credit_units)?,
                            classification: if promotional {
                                GrantClassification::Promotional
                            } else {
                                GrantClassification::Paid
                            },
                            priority: DEFAULT_PRIORITY,
                            expires_at: expires.map(|when| clock.resolve(when).and_utc()),
                            idempotency_key: Uuid::now_v7(),
                            reason: reason.to_string(),
                            reference: None,
                            actor_id: None,
                            cost: None,
                        },
                        business_time,
                    )
                    .await?;
            }
            rule @ Action::ConfigureWallet { .. } => rules.push((index, customer_id, rule)),
            Action::SetBudget(spec) => {
                seed.store
                    .hooks
                    .credit
                    .seed_budget(
                        seed.services,
                        budget(
                            seed.tenant_id,
                            customer_id,
                            &customer.currency,
                            &spec,
                            seed.credit_units,
                        )?,
                    )
                    .await?;
            }
            other => {
                return Err(StoreError::InvalidArgument(format!(
                    "{}'s credit cannot {other:?}",
                    customer.name
                ))
                .into());
            }
        }
    }

    // Wallet rules are usage controls: an edition without them seeds none.
    let rules = if seed.store.hooks.credit.available() {
        rules
    } else {
        Vec::new()
    };
    for (index, customer_id, rule) in rules {
        let Action::ConfigureWallet {
            denomination,
            low_balance_thresholds,
            usage_at_zero,
            auto_top_up,
        } = rule
        else {
            continue;
        };
        let account = accounts.entry(index).or_default();
        let wallet_id = wallet(seed, account, customer_id, denomination).await?;
        seed.services
            .configure_credit_wallet(configure_wallet(
                seed.tenant_id,
                customer_id,
                wallet_id,
                denomination,
                &low_balance_thresholds,
                usage_at_zero,
                auto_top_up.as_ref(),
                seed.credit_units,
            )?)
            .await?;
    }
    Ok(())
}

async fn wallet(
    seed: &AccountCredit<'_>,
    account: &mut Account,
    customer_id: CustomerId,
    denomination: &'static str,
) -> StoreResult<WalletId> {
    if let Some(wallet) = account.wallets.get(denomination) {
        return Ok(*wallet);
    }
    let wallet = seed
        .store
        .create_customer_credit_wallet(seed.tenant_id, customer_id, denomination)
        .await?
        .id;
    account.wallets.insert(denomination, wallet);
    Ok(wallet)
}
