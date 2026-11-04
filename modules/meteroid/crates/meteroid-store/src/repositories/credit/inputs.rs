//! Whose live-credit inputs a catalogue or entitlement write changes, and the matching bump.

use crate::store::PgConn;
use crate::{Store, StoreResult};
use chrono::{Days, NaiveDate};
use common_domain::ids::{CustomerId, SubscriptionId, TenantId};
use diesel_models::customers::CustomerRow;
use diesel_models::enums::EntitlementEntityTypeEnum;
use diesel_models::lock::CustomerLock;
use diesel_models::query::credit_inputs::{self, PartitionCreation};
use diesel_models::subscriptions::SubscriptionRow;
use std::collections::BTreeSet;
use uuid::Uuid;

/// Taken before the write, so the customer lock stays the transaction's first lock.
pub(crate) enum InputsOwner {
    Customer(CustomerLock),
    Tenant(TenantId),
    Nothing,
}

impl InputsOwner {
    pub(crate) async fn of_entitlement(
        conn: &mut PgConn,
        tenant_id: TenantId,
        entity_type: &EntitlementEntityTypeEnum,
        entity_id: Uuid,
    ) -> StoreResult<Self> {
        match entity_type {
            EntitlementEntityTypeEnum::Customer => Ok(Self::Customer(
                CustomerRow::lock(conn, tenant_id, CustomerId::from(entity_id)).await?,
            )),
            EntitlementEntityTypeEnum::Subscription => Ok(Self::Customer(
                SubscriptionRow::lock_customer_of_tenant(
                    conn,
                    tenant_id,
                    SubscriptionId::from(entity_id),
                )
                .await?,
            )),
            EntitlementEntityTypeEnum::Quote => Ok(Self::Nothing),
            EntitlementEntityTypeEnum::Plan
            | EntitlementEntityTypeEnum::PlanVersion
            | EntitlementEntityTypeEnum::AddOn
            | EntitlementEntityTypeEnum::Feature => Ok(Self::Tenant(tenant_id)),
        }
    }

    /// Bumps the customer now; returns the tenants `transaction_bumping_tenants` bumps last.
    pub(crate) async fn bump(&self, conn: &mut PgConn) -> StoreResult<Vec<TenantId>> {
        match self {
            Self::Customer(lock) => {
                credit_inputs::bump_customer(conn, lock).await?;
                Ok(vec![])
            }
            Self::Tenant(tenant_id) => Ok(vec![*tenant_id]),
            Self::Nothing => Ok(vec![]),
        }
    }
}

/// Partitions kept ahead of today, and days of rows kept behind it.
pub const FEED_DAYS_AHEAD: u64 = 14;
pub const FEED_RETENTION_DAYS: u64 = 7;
const DROP_ATTEMPTS: u32 = 5;

/// A maintenance step that failed; the next pass retries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedMaintenanceStep {
    Connection,
    Guards,
    List,
    Create,
    Detach,
    Finish,
    Purge,
    Measure,
}

impl FeedMaintenanceStep {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connection => "connection",
            Self::Guards => "guards",
            Self::List => "list",
            Self::Create => "create",
            Self::Detach => "detach",
            Self::Finish => "finish",
            Self::Purge => "purge",
            Self::Measure => "measure",
        }
    }
}

/// What one maintenance pass of the change feed did and found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreditFeedMaintenance {
    pub created: Vec<NaiveDate>,
    /// Days left in the DEFAULT partition because it already holds rows of that day.
    pub skipped: Vec<NaiveDate>,
    pub dropped: Vec<NaiveDate>,
    pub failures: Vec<(FeedMaintenanceStep, Option<NaiveDate>)>,
    /// Consecutive days after today with their own partition; 0 when it could not be read.
    pub days_ahead: i64,
    pub default_rows: Option<i64>,
}

impl Store {
    /// Creates the missing partitions up to `FEED_DAYS_AHEAD` days after `today`, drops those
    /// older than `FEED_RETENTION_DAYS` (recording their last row), and purges the DEFAULT
    /// partition's rows as old. Each day is its own step: a failure is recorded and the pass
    /// goes on. Run by a single scheduler leader.
    pub async fn maintain_credit_input_feed(&self, today: NaiveDate) -> CreditFeedMaintenance {
        let mut outcome = CreditFeedMaintenance::default();
        let mut conn = match self.get_conn().await {
            Ok(conn) => conn,
            Err(error) => {
                log::error!("credit input feed: no connection: {error:?}");
                outcome
                    .failures
                    .push((FeedMaintenanceStep::Connection, None));
                return outcome;
            }
        };
        let fail = |outcome: &mut CreditFeedMaintenance,
                    step: FeedMaintenanceStep,
                    day: Option<NaiveDate>,
                    error: &dyn std::fmt::Debug| {
            log::error!(
                "credit input feed: {} {day:?} failed: {error:?}",
                step.as_str()
            );
            outcome.failures.push((step, day));
        };
        if let Err(error) = credit_inputs::drop_leftover_default_guards(&mut conn).await {
            fail(&mut outcome, FeedMaintenanceStep::Guards, None, &error);
        }
        match credit_inputs::partition_days(&mut conn).await {
            Ok(days) => {
                let existing: BTreeSet<NaiveDate> = days.into_iter().collect();
                for day in today.iter_days().take(FEED_DAYS_AHEAD as usize + 1) {
                    if existing.contains(&day) {
                        continue;
                    }
                    match credit_inputs::create_partition(&mut conn, day).await {
                        Ok(PartitionCreation::Created) => outcome.created.push(day),
                        Ok(PartitionCreation::Skipped) => outcome.skipped.push(day),
                        Err(error) => {
                            fail(&mut outcome, FeedMaintenanceStep::Create, Some(day), &error)
                        }
                    }
                }
                let cutoff = today - Days::new(FEED_RETENTION_DAYS);
                for day in existing.iter().copied().filter(|day| *day < cutoff) {
                    let mut attempt = 1;
                    let detached = loop {
                        match credit_inputs::detach_partition(&mut conn, day).await {
                            Ok(()) => break true,
                            Err(error) if attempt < DROP_ATTEMPTS => {
                                log::warn!("credit input feed: detaching {day} failed: {error:?}");
                                tokio::time::sleep(std::time::Duration::from_millis(
                                    500 * u64::from(attempt),
                                ))
                                .await;
                                attempt += 1;
                            }
                            Err(error) => {
                                fail(&mut outcome, FeedMaintenanceStep::Detach, Some(day), &error);
                                break false;
                            }
                        }
                    };
                    if detached {
                        match credit_inputs::finish_dropped_partition(&mut conn, day).await {
                            Ok(()) => outcome.dropped.push(day),
                            Err(error) => {
                                fail(&mut outcome, FeedMaintenanceStep::Finish, Some(day), &error)
                            }
                        }
                    }
                }
            }
            Err(error) => fail(&mut outcome, FeedMaintenanceStep::List, None, &error),
        }
        // Detached by an earlier pass that stopped before recording and dropping them.
        match credit_inputs::detached_partition_days(&mut conn).await {
            Ok(days) => {
                for day in days {
                    match credit_inputs::finish_dropped_partition(&mut conn, day).await {
                        Ok(()) => outcome.dropped.push(day),
                        Err(error) => {
                            fail(&mut outcome, FeedMaintenanceStep::Finish, Some(day), &error)
                        }
                    }
                }
            }
            Err(error) => fail(&mut outcome, FeedMaintenanceStep::List, None, &error),
        }
        let cutoff = today - Days::new(FEED_RETENTION_DAYS);
        if let Err(error) = credit_inputs::purge_default_before(&mut conn, cutoff).await {
            fail(&mut outcome, FeedMaintenanceStep::Purge, None, &error);
        }
        match credit_inputs::partition_days(&mut conn).await {
            Ok(days) => {
                let now: BTreeSet<NaiveDate> = days.into_iter().collect();
                if now.contains(&today) {
                    outcome.days_ahead = today
                        .iter_days()
                        .skip(1)
                        .take_while(|day| now.contains(day))
                        .count() as i64;
                }
            }
            Err(error) => fail(&mut outcome, FeedMaintenanceStep::Measure, None, &error),
        }
        match credit_inputs::default_partition_rows(&mut conn).await {
            Ok(rows) => outcome.default_rows = Some(rows),
            Err(error) => fail(&mut outcome, FeedMaintenanceStep::Measure, None, &error),
        }
        outcome
    }
}
