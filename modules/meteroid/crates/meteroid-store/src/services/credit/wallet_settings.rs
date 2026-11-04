//! Configuring an invoice-credit wallet: its unit price, and through the usage controls its live
//! settings; and the wallets with their automatic top-up.
use super::public_api::{ClaimOutcome, claim_tx};
use crate::StoreResult;
use crate::domain::credit::{
    ApiIdempotency, AutomaticTopUp, ConfigureCreditWallet, CreditWallet, UnitPriceChange,
};
use crate::errors::StoreError;
use crate::hooks::CreditControlHooks;
use crate::repositories::credit::CustomerCreditWalletsInterface;
use crate::repositories::credit::customer_credit::lock_customer_tx;
use crate::services::ServicesEdge;
use crate::store::PgConn;
use chrono::Utc;
use common_domain::ids::{CustomerId, TenantId, WalletId};
use diesel_models::customer_credit::CustomerCreditAccountRow;
use diesel_models::customers::CustomerRow;
use diesel_models::lock::CustomerLock;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// Lock order is always the idempotency key, then the wallet. Unset fields are resolved under
/// the lock, never from a read taken before it, which a concurrent write could have moved on.
/// The unit price is core's; the live settings are the usage controls'.
async fn apply_configure_credit_wallet_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    command: &ConfigureCreditWallet,
    identity: Option<&ApiIdempotency>,
) -> StoreResult<Option<AutomaticTopUp>> {
    if let Some(identity) = identity {
        match claim_tx(conn, command.tenant_id, identity).await? {
            ClaimOutcome::Fresh => {}
            ClaimOutcome::Existing => {
                return controls.automatic_top_up(conn, command.wallet_id).await;
            }
        }
    }
    let now = Utc::now();
    let lock = lock_customer_tx(conn, command.tenant_id, command.customer_id).await?;
    // Before the top-up's terms, so terms saved at the current price get the new one.
    let price_changed =
        set_unit_price_tx(conn, &lock, command.wallet_id, &command.unit_price).await?;
    let stored = controls
        .configure_wallet_controls(conn, &lock, command, price_changed, now)
        .await?;
    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
    Ok(stored)
}

/// Sets or clears a credit unit wallet's agreed price, in the customer's currency; true when it
/// changed. Quotes and top-ups pinned before keep their price.
async fn set_unit_price_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    wallet_id: WalletId,
    change: &UnitPriceChange,
) -> StoreResult<bool> {
    let price = match change {
        UnitPriceChange::Keep => return Ok(false),
        UnitPriceChange::Clear => None,
        UnitPriceChange::Set(price) => {
            Some(crate::domain::credit_units::validate_credit_unit_price(*price)?.normalize())
        }
    };
    let account = CustomerCreditAccountRow::lock_by_id(conn, lock, wallet_id)
        .await?
        .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
    let price = match price {
        Some(_) if account.credit_unit_id.is_none() => {
            return Err(super::invalid(
                "a unit price applies to credit unit wallets only: money credit is sold at its face value",
            ));
        }
        Some(price) => {
            let customer =
                CustomerRow::find_by_id(conn, &lock.customer_id(), &lock.tenant_id()).await?;
            Some((price, customer.currency.trim().to_ascii_uppercase()))
        }
        None => None,
    };
    let unchanged = match &price {
        Some((price, currency)) => {
            account.unit_price.map(|stored| stored.normalize()) == Some(*price)
                && account.unit_price_currency.as_deref() == Some(currency.as_str())
        }
        None => account.unit_price.is_none(),
    };
    if !unchanged {
        CustomerCreditAccountRow::set_unit_price(conn, lock, account.id, price).await?;
    }
    Ok(!unchanged)
}

impl ServicesEdge {
    /// An operator's configuration: each field the caller set is replaced as given, guardrails
    /// included for the top-up, and everything is applied in one wallet-locked transaction.
    pub async fn configure_credit_wallet(
        &self,
        command: ConfigureCreditWallet,
    ) -> StoreResult<(CreditWallet, Option<AutomaticTopUp>)> {
        let tenant_id = command.tenant_id;
        let wallet_id = command.wallet_id;
        let stored = self
            .store
            .transaction(|conn| {
                async move {
                    apply_configure_credit_wallet_tx(
                        conn,
                        &*self.store.hooks.credit,
                        &command,
                        None,
                    )
                    .await
                }
                .scope_boxed()
            })
            .await?;
        let wallet = self
            .store
            .customer_credit_wallet(tenant_id, wallet_id)
            .await?
            .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        Ok((wallet, stored))
    }

    /// `configure_credit_wallet`, keyed by the caller's idempotency key: the key is claimed in
    /// the same transaction as the write, so a retry returns the wallet as it now stands instead
    /// of re-applying the change; a different request under the same key is refused.
    pub async fn configure_credit_wallet_once(
        &self,
        identity: &ApiIdempotency,
        command: ConfigureCreditWallet,
    ) -> StoreResult<(CreditWallet, Option<AutomaticTopUp>)> {
        let tenant_id = command.tenant_id;
        let wallet_id = command.wallet_id;
        let stored = self
            .store
            .transaction(|conn| {
                async move {
                    apply_configure_credit_wallet_tx(
                        conn,
                        &*self.store.hooks.credit,
                        &command,
                        Some(identity),
                    )
                    .await
                }
                .scope_boxed()
            })
            .await?;
        let wallet = self
            .store
            .customer_credit_wallet(tenant_id, wallet_id)
            .await?
            .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        Ok((wallet, stored))
    }

    /// The customer's wallets, each with its automatic top-up.
    pub async fn customer_credit_wallets_with_auto_top_up(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Vec<(CreditWallet, Option<AutomaticTopUp>)>> {
        let wallets = self
            .store
            .customer_credit_wallets(tenant_id, customer_id)
            .await?;
        let mut conn = self.store.get_conn().await?;
        let mut configured = Vec::with_capacity(wallets.len());
        for wallet in wallets {
            let stored = self
                .store
                .hooks
                .credit
                .automatic_top_up(&mut conn, wallet.id)
                .await?;
            configured.push((wallet, stored));
        }
        Ok(configured)
    }

    pub async fn credit_wallet_auto_top_up(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
    ) -> StoreResult<Option<AutomaticTopUp>> {
        let mut conn = self.store.get_conn().await?;
        CustomerCreditAccountRow::find_by_id(&mut conn, tenant_id, wallet_id)
            .await?
            .filter(|account| account.customer_id == customer_id)
            .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        self.store
            .hooks
            .credit
            .automatic_top_up(&mut conn, wallet_id)
            .await
    }
}
