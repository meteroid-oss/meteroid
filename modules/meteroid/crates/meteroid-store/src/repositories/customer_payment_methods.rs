use crate::adapters::payment_service_providers::initialize_payment_provider;
use crate::domain::connectors::Connector;
use crate::domain::customer_payment_methods::{CustomerPaymentMethod, SetupIntent};
use crate::domain::CustomerPaymentMethodNew;
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::{CustomerConnectionId, CustomerId, CustomerPaymentMethodId, TenantId};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::customer_payment_methods::{
    CustomerPaymentMethodRow, CustomerPaymentMethodRowNew,
};
use error_stack::ResultExt;
use stripe_client::payment_intents::PaymentIntent;

#[async_trait::async_trait]
pub trait CustomerPaymentMethodsInterface {
    async fn create_setup_intent(
        &self,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
    ) -> StoreResult<SetupIntent>;

    async fn create_payment_intent(
        &self,
        tenant_id: &TenantId,
        payment_method_id: &CustomerPaymentMethodId,
        amount: u64,
        currency: String,
    ) -> StoreResult<PaymentIntent>;
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
    async fn create_setup_intent(
        &self,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
    ) -> StoreResult<SetupIntent> {
        let mut conn = self.get_conn().await?;

        let connection =
            CustomerConnectionDetailsRow::get_by_id(&mut conn, tenant_id, customer_connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connector = Connector::from_row(&self.settings.crypt_key, connection.connector)?;

        let provider = initialize_payment_provider(&connector);

        let setup_intent = provider
            .create_setup_intent_in_provider(&connector, &connection.external_customer_id)
            .await
            .change_context_lazy(|| StoreError::PaymentProviderError)?;

        Ok(setup_intent)
    }

    async fn create_payment_intent(
        &self,
        tenant_id: &TenantId,
        payment_method_id: &CustomerPaymentMethodId,
        amount: u64,
        currency: String,
    ) -> StoreResult<PaymentIntent> {
        let mut conn = self.get_conn().await?;

        let method = CustomerPaymentMethodRow::get_by_id(&mut conn, tenant_id, payment_method_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connection = CustomerConnectionDetailsRow::get_by_id(
            &mut conn,
            tenant_id,
            &method.connection_id.ok_or(StoreError::InvalidArgument(
                "Payment method is not connected and cannot be used for payment intent".to_string(),
            ))?,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        let external_payment_method_id =
            method
                .external_payment_method_id
                .ok_or(StoreError::InvalidArgument(
                    "Payment method has no external id and cannot be used for payment intent"
                        .to_string(),
                ))?;

        let connector = Connector::from_row(&self.settings.crypt_key, connection.connector)?;

        let provider = initialize_payment_provider(&connector);

        let payment_intent = provider
            .create_payment_intent_in_provider(
                &connector,
                &connection.external_customer_id,
                &external_payment_method_id,
                amount as i64,
                &currency,
            )
            .await
            .change_context_lazy(|| StoreError::PaymentProviderError)?;

        Ok(payment_intent)
    }

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
