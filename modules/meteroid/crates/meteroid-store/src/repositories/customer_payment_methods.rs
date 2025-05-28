use crate::domain::CustomerPaymentMethodNew;
use crate::domain::customer_payment_methods::CustomerPaymentMethod;
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::{CustomerConnectionId, CustomerId, CustomerPaymentMethodId, TenantId};
use diesel_models::customer_payment_methods::{
    CustomerPaymentMethodRow, CustomerPaymentMethodRowNew,
};

#[async_trait::async_trait]
pub trait CustomerPaymentMethodsInterface {
    async fn list_payment_methods_by_connection(
        &self,
        tenant_id: &TenantId,
        connection_id: &CustomerConnectionId,
    ) -> StoreResult<Vec<CustomerPaymentMethod>>;

    async fn list_payment_methods_by_customer(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
    ) -> StoreResult<Vec<CustomerPaymentMethod>>;

    async fn get_payment_method_by_id(
        &self,
        tenant_id: &TenantId,
        id: &CustomerPaymentMethodId,
    ) -> StoreResult<CustomerPaymentMethod>;

    async fn upsert_payment_method(
        &self,
        method: CustomerPaymentMethodNew,
    ) -> StoreResult<CustomerPaymentMethod>;

    async fn insert_payment_method_if_not_exist(
        &self,
        method: CustomerPaymentMethodNew,
    ) -> StoreResult<CustomerPaymentMethod>;
}

#[async_trait::async_trait]
impl CustomerPaymentMethodsInterface for Store {
    async fn list_payment_methods_by_connection(
        &self,
        tenant_id: &TenantId,
        connection_id: &CustomerConnectionId,
    ) -> StoreResult<Vec<CustomerPaymentMethod>> {
        let mut conn = self.get_conn().await?;

        let customer_payment_methods =
            CustomerPaymentMethodRow::list_by_connection_id(&mut conn, tenant_id, connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?
                .into_iter()
                .map(Into::into)
                .collect();

        Ok(customer_payment_methods)
    }

    async fn list_payment_methods_by_customer(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
    ) -> StoreResult<Vec<CustomerPaymentMethod>> {
        let mut conn = self.get_conn().await?;

        let customer_payment_methods =
            CustomerPaymentMethodRow::list_by_customer_id(&mut conn, tenant_id, customer_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?
                .into_iter()
                .map(Into::into)
                .collect();

        Ok(customer_payment_methods)
    }

    async fn get_payment_method_by_id(
        &self,
        tenant_id: &TenantId,
        id: &CustomerPaymentMethodId,
    ) -> StoreResult<CustomerPaymentMethod> {
        let mut conn = self.get_conn().await?;

        let customer_payment_method = CustomerPaymentMethodRow::get_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?
            .into();

        Ok(customer_payment_method)
    }

    async fn upsert_payment_method(
        &self,
        method: CustomerPaymentMethodNew,
    ) -> StoreResult<CustomerPaymentMethod> {
        let mut conn = self.get_conn().await?;
        let row: CustomerPaymentMethodRowNew = method.into();

        let customer_payment_method = row
            .upsert(&mut conn)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?
            .into();

        Ok(customer_payment_method)
    }

    async fn insert_payment_method_if_not_exist(
        &self,
        method: CustomerPaymentMethodNew,
    ) -> StoreResult<CustomerPaymentMethod> {
        let mut conn = self.get_conn().await?;
        let row: CustomerPaymentMethodRowNew = method.into();

        let customer_payment_method = row
            .insert_if_not_exist(&mut conn)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?
            .into();

        Ok(customer_payment_method)
    }
}
