//! A subscription's credit ends with it. The terminal transition sets `ends_at` on its grants; the
//! expiry sweep then settles its usage up to that instant from them and lapses the rest. A missed
//! hook (an older release, a new terminal path) is healed by the sweep.
use crate::Store;
use crate::StoreResult;
use crate::errors::StoreError;
use crate::hooks::CreditControlHooks;
use crate::repositories::credit::customer_credit::{
    balance_changed_tx, lock_customer_tx, lock_grants_tx, normalized_currency,
};
use crate::services::credit::metrics;
use crate::services::credit::scope::credit_ended_at;
use crate::store::PgConn;
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, SubscriptionId, TenantId, WalletId};
use diesel_models::customer_credit::{CustomerCreditAccountRow, CustomerCreditGrantRow};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// Sets `ends_at` on every grant of `subscription_id` that had not expired by `ended_at`, under
/// the customer and grant locks. Idempotent; returns the wallets whose credit ended.
pub(crate) async fn end_subscription_credit_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    ended_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> StoreResult<Vec<WalletId>> {
    let customer_id = SubscriptionRow::get_customer_id(conn, &tenant_id, subscription_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let mut denominations: Vec<String> =
        CustomerCreditGrantRow::unended_denominations_of_subscription(
            conn,
            tenant_id,
            subscription_id,
        )
        .await?
        .iter()
        .map(|currency| normalized_currency(currency))
        .collect();
    denominations.sort();
    denominations.dedup();
    let lock = lock_customer_tx(conn, tenant_id, customer_id).await?;
    for denomination in &denominations {
        lock_grants_tx(conn, &lock, denomination).await?;
    }
    // An abandoned subscription's period may start later: its credit ends when it was abandoned.
    let ended_at = ended_at.min(now);
    let ended =
        CustomerCreditGrantRow::end_for_subscription(conn, tenant_id, subscription_id, ended_at)
            .await?;
    if !ended.is_empty() {
        // Ending credit moves no value but changes what it can pay: a movement for readers.
        CustomerCreditAccountRow::bump_ledger_version(conn, &lock).await?;
    }
    let mut wallets: Vec<WalletId> = ended.iter().map(|grant| grant.account_id).collect();
    wallets.sort_by_key(|wallet| wallet.as_uuid());
    wallets.dedup();
    balance_changed_tx(conn, controls, tenant_id, &wallets, now).await?;
    Ok(wallets)
}

/// Ends the credit of terminal subscriptions whose transition did not, `page` at a time; returns
/// how many subscriptions it repaired. Each runs in its own transaction; one failing is logged.
pub(crate) async fn end_credit_of_ended_subscriptions(
    store: &Store,
    page: i64,
) -> StoreResult<usize> {
    let page = page.max(1);
    let mut repaired = 0_usize;
    let mut after = None;
    loop {
        let mut conn = store.get_conn().await?;
        let unended =
            CustomerCreditGrantRow::list_unended_of_ended_subscriptions(&mut conn, after, page)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        drop(conn);
        let full_page = i64::try_from(unended.len()).is_ok_and(|len| len == page);
        after = unended
            .last()
            .map(|row| (row.tenant_id, row.subscription_id));
        for row in unended {
            let outcome = store
                .transaction(|conn| {
                    async move {
                        SubscriptionRow::lock_customer_of(conn, row.subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        let subscription = SubscriptionRow::get_subscription_by_id(
                            conn,
                            &row.tenant_id,
                            row.subscription_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .subscription;
                        let Some(ended_at) = credit_ended_at(&subscription) else {
                            return Ok(Vec::new());
                        };
                        end_subscription_credit_tx(
                            conn,
                            &*store.hooks.credit,
                            row.tenant_id,
                            row.subscription_id,
                            ended_at,
                            common_domain::clock::now(),
                        )
                        .await
                    }
                    .scope_boxed()
                })
                .await;
            match outcome {
                Ok(wallets) if !wallets.is_empty() => {
                    log::warn!(
                        "credit of ended subscription {} was not ended with it; repaired",
                        row.subscription_id
                    );
                    repaired += 1;
                }
                Ok(_) => {}
                Err(error) => log::error!(
                    "credit of ended subscription {} was not ended: {error:?}",
                    row.subscription_id
                ),
            }
        }
        if !full_page {
            break;
        }
    }
    metrics::subscription_end_repaired(repaired);
    Ok(repaired)
}
