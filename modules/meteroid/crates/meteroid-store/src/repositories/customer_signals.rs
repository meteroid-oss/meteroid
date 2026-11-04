//! Signal reads core customer screens render; empty while no signal has fired, and without the
//! edition's alerting.

use std::collections::HashMap;

use common_domain::ids::{CustomerId, TenantId};

use crate::domain::customer_signals::OpenSignals;
use crate::domain::fx::FxTable;
use crate::{Store, StoreResult};

#[async_trait::async_trait]
pub trait CustomerSignalsInterface {
    /// Open signals of each of `customer_ids` that has some, keyed by customer.
    async fn open_signals_by_customer(
        &self,
        tenant_id: TenantId,
        customer_ids: &[CustomerId],
        fx: &FxTable,
    ) -> StoreResult<HashMap<CustomerId, OpenSignals>>;
}

#[async_trait::async_trait]
impl CustomerSignalsInterface for Store {
    async fn open_signals_by_customer(
        &self,
        tenant_id: TenantId,
        customer_ids: &[CustomerId],
        fx: &FxTable,
    ) -> StoreResult<HashMap<CustomerId, OpenSignals>> {
        let mut conn = self.get_conn().await?;
        self.hooks
            .signals
            .open_signals_by_customer(&mut conn, tenant_id, customer_ids, fx)
            .await
    }
}
