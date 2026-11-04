//! What the usage-controls module's engine reads of these services, re-exported by
//! `crate::usage_controls_support`: internal to that module.

use std::future::Future;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, TenantId};
use diesel_models::customer_credit::CustomerCreditAccountRow;
use rust_decimal::Decimal;

use crate::domain::SubscriptionDetails;
use crate::domain::credit::{ApiIdempotency, TopUpWalletQuote};
use crate::services::clients::usage::UsageClient;
use crate::services::credit::forecast::{CustomerCreditForecast, ForecastUntil};
use crate::services::credit::unit_wallets::UnitSale;
use crate::services::{Services, ServicesEdge};
use crate::store::PgConn;
use crate::{Store, StoreResult};

/// The services the live engine prices and forecasts through.
#[derive(Clone)]
pub struct EngineServices(Services);

impl EngineServices {
    pub fn of(services: &ServicesEdge) -> Self {
        Self(services.services.clone())
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.0.store
    }

    pub fn usage_client(&self) -> &Arc<dyn UsageClient> {
        &self.0.usage_client
    }

    /// These services reading metering through `usage_client`.
    pub fn reading_usage(&self, usage_client: Arc<dyn UsageClient>) -> Self {
        Self(self.0.reading_usage(usage_client))
    }

    pub async fn forecast_customer_credit(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        until: ForecastUntil,
        subject: Option<&SubscriptionDetails>,
        at: DateTime<Utc>,
    ) -> StoreResult<CustomerCreditForecast> {
        self.0
            .forecast_customer_credit(conn, tenant_id, customer_id, until, subject, at)
            .await
    }

    pub async fn price_top_up_unchecked_tx(
        &self,
        conn: &mut PgConn,
        account: &CustomerCreditAccountRow,
        amount_minor: i64,
        pinned: Option<&UnitSale>,
        unit_price: Option<Decimal>,
    ) -> StoreResult<(TopUpWalletQuote, Option<UnitSale>)> {
        self.0
            .price_top_up_unchecked_tx(conn, account, amount_minor, pinned, unit_price)
            .await
    }
}

/// Runs `write` under the claimed API idempotency key (`ServicesEdge::once`).
pub async fn once<T>(
    services: &ServicesEdge,
    tenant_id: TenantId,
    identity: &ApiIdempotency,
    write: impl Future<Output = StoreResult<T>>,
) -> StoreResult<T> {
    services.once(tenant_id, identity, write).await
}

/// Arms and charges the automatic top-ups the usage controls queued, each armed in its own
/// transaction; attempts left in flight are resumed by the wallet purchase executor.
pub async fn charge_queued_automatic_top_ups(services: &ServicesEdge) -> StoreResult<()> {
    services.services.charge_queued_automatic_top_ups().await
}
