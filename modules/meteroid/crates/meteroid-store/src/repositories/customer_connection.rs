use crate::domain::CustomerConnection;
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::{CustomerConnectionId, CustomerId, TenantId};
use diesel_models::customer_connection::CustomerConnectionRow;

#[async_trait::async_trait]
pub trait CustomerConnectionInterface {
    async fn get_connection_by_id(
        &self,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
    ) -> StoreResult<CustomerConnection>;

    async fn list_connections_by_customer_id(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
    ) -> StoreResult<Vec<CustomerConnection>>;

    async fn upsert_customer_connection(
        &self,
        tenant_id: &TenantId,
        connection: CustomerConnection,
    ) -> StoreResult<CustomerConnection>;

    async fn delete_customer_connection(
        &self,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
    ) -> StoreResult<()>;
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

    async fn list_connections_by_customer_id(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
    ) -> StoreResult<Vec<CustomerConnection>> {
        let mut conn = self.get_conn().await?;

        let connections =
            CustomerConnectionRow::list_connections_by_customer_id(&mut conn, tenant_id, customer_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(connections.into_iter().map(|c| c.into()).collect())
    }

    async fn upsert_customer_connection(
        &self,
        tenant_id: &TenantId,
        connection: CustomerConnection,
    ) -> StoreResult<CustomerConnection> {
        let mut conn = self.get_conn().await?;

        let row: CustomerConnectionRow = connection.into();

        let result = CustomerConnectionRow::upsert(&mut conn, tenant_id, row)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(result.into())
    }

    async fn delete_customer_connection(
        &self,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        CustomerConnectionRow::delete(&mut conn, *customer_connection_id, *tenant_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(())
    }
}
