//! Wallet writes keyed by a caller's Idempotency-Key, the tenant-wide wallet roster and the portal
//! switches of an invoicing entity.
use crate::StoreResult;
use crate::domain::credit::{
    ApiIdempotency, CreditWalletSummary, GrantClassification, GrantScope, PortalCreditCapabilities,
    TopUpWallet, WalletLiveBalance, WalletStatus,
};
use crate::domain::credit_units::CreditUnit;
use crate::errors::StoreError;
use crate::repositories::credit::CustomerCreditWalletsInterface;
use crate::repositories::credit::customer_credit_wallets::{
    FundCustomerCreditWallet, FundedCustomerCreditWallet,
};
use crate::services::credit::purchase::{SellCustomerCredit, SoldCustomerCredit};
use crate::services::credit::wallet_purchase::TopUpWalletReceipt;
use crate::services::{Services, ServicesEdge};
use crate::store::PgConn;
use common_domain::ids::{BaseId, CreditUnitId, CustomerId, TenantId, WalletId};
use diesel_models::api_idempotency::{ApiIdempotencyKeyRow, ApiIdempotencyKeyRowNew};
use diesel_models::connectors::ConnectorRow;
use diesel_models::credit_units::CreditUnitRow;
use diesel_models::customer_credit::{CustomerCreditAccountRow, CustomerCreditRosterRow};
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::customers::CustomerRow;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::query::api_idempotency::ApiIdempotencyClaim;
use error_stack::Report;
use std::collections::HashMap;

impl ServicesEdge {
    /// The grant is keyed by the idempotency key, so a retry returns it instead of funding again.
    pub async fn grant_wallet_credit_once(
        &self,
        identity: &ApiIdempotency,
        command: FundCustomerCreditWallet,
        scope: GrantScope,
    ) -> StoreResult<FundedCustomerCreditWallet> {
        self.ensure_supporting_document(&command).await?;
        let command = FundCustomerCreditWallet {
            idempotency_key: identity.derived_id(command.tenant_id),
            ..command
        };
        self.once(
            command.tenant_id,
            identity,
            self.store
                .fund_scoped_customer_credit_wallet(command.clone(), scope),
        )
        .await
    }

    /// French entities need a supporting document for every accounting entry (CGI art. 289,
    /// Code de commerce L123-22): paid credit recorded without a Meteroid invoice names its own.
    pub async fn ensure_supporting_document(
        &self,
        command: &FundCustomerCreditWallet,
    ) -> StoreResult<()> {
        let paid = command.classification == GrantClassification::Paid
            && command
                .cost
                .as_ref()
                .is_none_or(|cost| cost.amount_minor > 0);
        if !paid || command.reference.is_some() {
            return Ok(());
        }
        let mut conn = self.store.get_conn().await?;
        let customer =
            CustomerRow::find_by_id(&mut conn, &command.customer_id, &command.tenant_id).await?;
        let entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
            &mut conn,
            customer.invoicing_entity_id,
            command.tenant_id,
        )
        .await?;
        if entity.country.code == "FR" {
            return Err(super::invalid(
                "A reference to the invoice or receipt for this payment is required for French \
                 entities.",
            ));
        }
        Ok(())
    }

    /// The purchase is keyed by the idempotency key: a retry finishes issuing the same invoice.
    pub async fn sell_wallet_credit_once(
        &self,
        identity: &ApiIdempotency,
        command: SellCustomerCredit,
    ) -> StoreResult<SoldCustomerCredit> {
        let tenant_id = command.tenant_id;
        let purchase_id = identity.derived_id(tenant_id);
        self.once(
            tenant_id,
            identity,
            self.sell_customer_credit(purchase_id, command),
        )
        .await
    }

    pub async fn portal_credit_capabilities(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<PortalCreditCapabilities> {
        self.services
            .portal_credit_capabilities(tenant_id, customer_id)
            .await
    }

    /// The charge is keyed by the idempotency key: a retry resumes the same payment and never
    /// charges twice.
    pub async fn top_up_wallet_once(
        &self,
        identity: &ApiIdempotency,
        command: TopUpWallet,
    ) -> StoreResult<TopUpWalletReceipt> {
        self.once(
            command.tenant_id,
            identity,
            self.top_up_wallet(command, None),
        )
        .await
    }

    /// The customer can top this wallet up from the portal, adding a payment method there if
    /// needed.
    pub async fn ensure_self_service_top_up(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
    ) -> StoreResult<()> {
        let capabilities = self
            .portal_credit_capabilities(tenant_id, customer_id)
            .await?;
        let mut conn = self.store.get_conn().await?;
        let account = CustomerCreditAccountRow::find_by_id(&mut conn, tenant_id, wallet_id)
            .await?
            .filter(|account| account.customer_id == customer_id)
            .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        if !capabilities.top_up_enabled {
            return Err(super::precondition(
                "Credit can't be bought from this portal. Contact the merchant to add credit.",
            ));
        }
        let customer = CustomerRow::find_by_id(&mut conn, &customer_id, &tenant_id).await?;
        super::unit_wallets::resolve_unit_sale(&mut conn, &account, &customer.currency, None)
            .await
            .map(|_| ())
    }

    /// Wallets of every customer, by customer name; `search` matches the name or alias.
    /// `auto_top_up_failing` keeps the wallets whose automatic top-up failed or was stopped.
    pub async fn list_credit_wallet_summaries(
        &self,
        tenant_id: TenantId,
        search: Option<&str>,
        status: Option<WalletStatus>,
        auto_top_up_failing: bool,
        limit: i64,
        offset: i64,
    ) -> StoreResult<(Vec<CreditWalletSummary>, u64)> {
        let search = search.map(str::trim).filter(|term| !term.is_empty());
        let status = status.map(WalletStatus::as_db_str);
        let now = common_domain::clock::now();
        let live = self.store.hooks.credit.available();
        let mut conn = self.store.get_conn().await?;
        let rows = CustomerCreditRosterRow::list(
            &mut conn,
            tenant_id,
            search,
            status,
            auto_top_up_failing,
            now,
            limit.clamp(1, 100),
            offset.max(0),
            live,
        )
        .await?;
        let total = CustomerCreditRosterRow::count(
            &mut conn,
            tenant_id,
            search,
            status,
            auto_top_up_failing,
            now,
            live,
        )
        .await?;
        let mut unit_ids = rows
            .iter()
            .filter_map(|row| row.credit_unit_id)
            .collect::<Vec<_>>();
        unit_ids.sort_by_key(|id| id.as_uuid());
        unit_ids.dedup();
        let units = CreditUnitRow::list_by_ids(&mut conn, tenant_id, &unit_ids)
            .await?
            .into_iter()
            .map(|row| CreditUnit::try_from(row).map(|unit| (unit.id, unit)))
            .collect::<StoreResult<HashMap<CreditUnitId, CreditUnit>>>()?;
        let wallets = rows
            .into_iter()
            .map(|row| {
                let credit_unit = row
                    .credit_unit_id
                    .map(|id| {
                        units.get(&id).cloned().ok_or_else(|| {
                            Report::new(StoreError::ValueNotFound(format!(
                                "credit unit {id} of wallet {}",
                                row.id
                            )))
                        })
                    })
                    .transpose()?;
                let live = match (
                    row.live_balance_minor,
                    row.pending_usage_minor,
                    row.live_as_of,
                ) {
                    (Some(live_balance_minor), Some(pending_usage_minor), Some(as_of)) => {
                        Some(WalletLiveBalance {
                            live_balance_minor,
                            pending_usage_minor,
                            uncovered_usage_minor: row.uncovered_usage_minor.unwrap_or(0),
                            as_of,
                        })
                    }
                    _ => None,
                };
                Ok(CreditWalletSummary {
                    id: row.id,
                    customer_id: row.customer_id,
                    customer_name: row.customer_name,
                    customer_alias: row.customer_alias,
                    currency: row.currency,
                    credit_unit,
                    available_minor: row.available_minor,
                    pending_minor: row.pending_minor,
                    low_balance_thresholds_minor: row.low_balance_thresholds_minor,
                    auto_top_up_enabled: row.auto_top_up_enabled,
                    auto_top_up_suspended: row.auto_top_up_suspended,
                    auto_top_up_failing: row.auto_top_up_failing,
                    auto_top_up_threshold_minor: row.auto_top_up_threshold_minor,
                    usage_at_zero: row.usage_at_zero.into(),
                    pauses_at_zero: row.pauses_at_zero,
                    live_stale: live.is_some() && row.live_stale,
                    live,
                    status: WalletStatus::from_db_str(&row.status)?,
                    created_at: row.created_at,
                })
            })
            .collect::<StoreResult<Vec<_>>>()?;
        Ok((wallets, u64::try_from(total).unwrap_or_default()))
    }

    /// Runs `write` under the claimed key. A request refused before it changed anything releases
    /// the key, so the caller can correct it and retry under the same key.
    pub(crate) async fn once<T>(
        &self,
        tenant_id: TenantId,
        identity: &ApiIdempotency,
        write: impl Future<Output = StoreResult<T>>,
    ) -> StoreResult<T> {
        let claim = ApiIdempotencyKeyRowNew {
            tenant_id,
            operation: identity.operation.to_owned(),
            idempotency_key: identity.key.clone(),
            request_fingerprint: identity.fingerprint.to_vec(),
        };
        let mut conn = self.store.get_conn().await?;
        let stored = claim.claim(&mut conn).await?;
        drop(conn);
        if stored.request_fingerprint != claim.request_fingerprint {
            return Err(Report::new(StoreError::Conflict(
                "this Idempotency-Key was already used with a different request".to_owned(),
            )));
        }
        let outcome = write.await;
        if let Err(error) = &outcome
            && matches!(
                error.current_context(),
                StoreError::InvalidArgument(_) | StoreError::ValueNotFound(_)
            )
        {
            let mut conn = self.store.get_conn().await?;
            ApiIdempotencyKeyRow::release(
                &mut conn,
                tenant_id,
                &claim.operation,
                &claim.idempotency_key,
                &claim.request_fingerprint,
            )
            .await?;
        }
        outcome
    }
}

/// Whether this call is the one that claimed the key inside its own transaction, or a replay of
/// an earlier call whose transaction already committed.
pub(crate) enum ClaimOutcome {
    Fresh,
    Existing,
}

/// Claimed in the caller's transaction before any other lock: a committed claim proves the write
/// ran, so a replay skips it; a concurrent duplicate waits on the key's row.
pub(crate) async fn claim_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    identity: &ApiIdempotency,
) -> StoreResult<ClaimOutcome> {
    let claim = ApiIdempotencyKeyRowNew {
        tenant_id,
        operation: identity.operation.to_owned(),
        idempotency_key: identity.key.clone(),
        request_fingerprint: identity.fingerprint.to_vec(),
    };
    match claim.claim_in_tx(conn).await? {
        ApiIdempotencyClaim::Fresh => Ok(ClaimOutcome::Fresh),
        ApiIdempotencyClaim::Existing(existing) => {
            if existing.request_fingerprint != claim.request_fingerprint {
                return Err(Report::new(StoreError::Conflict(
                    "this Idempotency-Key was already used with a different request".to_owned(),
                )));
            }
            Ok(ClaimOutcome::Existing)
        }
    }
}

impl Services {
    /// A portal switch offers nothing without a way to pay by card.
    pub(crate) async fn portal_credit_capabilities(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<PortalCreditCapabilities> {
        let mut conn = self.store.get_conn().await?;
        let customer = CustomerRow::find_by_id(&mut conn, &customer_id, &tenant_id).await?;
        let entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
            &mut conn,
            customer.invoicing_entity_id,
            tenant_id,
        )
        .await?;
        // Top-ups are paid by card: an active card provider to add one, or a chargeable saved card.
        let card_provider_active = match entity.card_provider_id {
            Some(id) => !ConnectorRow::filter_active_ids(&mut conn, tenant_id, &[id])
                .await?
                .is_empty(),
            None => false,
        };
        let can_pay = card_provider_active
            || CustomerPaymentMethodRow::has_chargeable_card(&mut conn, &tenant_id, &customer_id)
                .await?;
        Ok(portal_capabilities(
            entity.portal_credit_top_up_enabled,
            entity.portal_auto_top_up_enabled,
            can_pay,
        ))
    }
}

fn portal_capabilities(
    top_up_switch: bool,
    auto_top_up_switch: bool,
    can_pay: bool,
) -> PortalCreditCapabilities {
    PortalCreditCapabilities {
        top_up_enabled: top_up_switch && can_pay,
        auto_top_up_enabled: auto_top_up_switch && can_pay,
    }
}

#[cfg(test)]
mod portal_capabilities_tests {
    use super::portal_capabilities;

    #[test]
    fn a_switch_without_a_way_to_pay_offers_nothing() {
        let both = portal_capabilities(true, true, true);
        assert!(both.top_up_enabled && both.auto_top_up_enabled);
        let unpayable = portal_capabilities(true, true, false);
        assert!(!unpayable.top_up_enabled && !unpayable.auto_top_up_enabled);
        let off = portal_capabilities(false, false, true);
        assert!(!off.top_up_enabled && !off.auto_top_up_enabled);
    }
}
