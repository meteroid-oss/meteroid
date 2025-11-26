use crate::domain::customer_payment_methods::CustomerPaymentMethod;
use crate::domain::{CustomerPaymentMethodNew, ResolvedPaymentMethod};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::{
    CustomerConnectionId, CustomerId, CustomerPaymentMethodId, SubscriptionId, TenantId,
};
use diesel_models::customer_payment_methods::{
    CustomerPaymentMethodRow, CustomerPaymentMethodRowNew,
};
use diesel_models::enums::PaymentMethodTypeEnum;

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

    async fn resolve_payment_method_for_subscription(
        &self,
        tenant_id: TenantId,
        id: SubscriptionId,
    ) -> StoreResult<ResolvedPaymentMethod>;
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

    async fn resolve_payment_method_for_subscription(
        &self,
        tenant_id: TenantId,
        id: SubscriptionId,
    ) -> StoreResult<ResolvedPaymentMethod> {
        let mut conn = self.get_conn().await?;

        let resolved =
            CustomerPaymentMethodRow::resolve_subscription_payment_method(&mut conn, tenant_id, id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let resolved = match resolved.subscription_payment_method {
            Some(PaymentMethodTypeEnum::Transfer) => resolved
                .subscription_bank_account_id
                .or(resolved.customer_bank_account_id)
                .or(resolved.invoicing_entity_bank_account_id)
                .map_or(
                    ResolvedPaymentMethod::NotConfigured,
                    ResolvedPaymentMethod::BankTransfer,
                ),
            Some(PaymentMethodTypeEnum::Other) => ResolvedPaymentMethod::NotConfigured,
            None | Some(_) => resolved
                .subscription_payment_method_id
                .or(resolved.customer_payment_method_id)
                .map_or(
                    ResolvedPaymentMethod::NotConfigured,
                    ResolvedPaymentMethod::CustomerPaymentMethod,
                ),
        };

        Ok(resolved)
    }
}
