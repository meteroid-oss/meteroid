use crate::domain::CustomerConnection;
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::{CustomerConnectionId, TenantId};
use diesel_models::customer_connection::CustomerConnectionRow;

#[async_trait::async_trait]
pub trait CustomerConnectionInterface {
    async fn get_connection_by_id(
        &self,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
    ) -> StoreResult<CustomerConnection>;
}

#[async_trait::async_trait]
impl CustomerConnectionInterface for Store {
    async fn get_connection_by_id(
        &self,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
    ) -> StoreResult<CustomerConnection> {
        let mut conn = self.get_conn().await?;

        let connection =
            CustomerConnectionRow::get_by_id(&mut conn, tenant_id, customer_connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(connection.into())
    }
}
