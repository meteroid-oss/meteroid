use crate::adapters::payment_service_providers::initialize_payment_provider;
use crate::domain::connectors::Connector;
use crate::domain::customer_payment_methods::{CustomerPaymentMethod, SetupIntent};
use crate::domain::{CustomerConnection, CustomerPaymentMethodNew, PaymentMethodTypeEnum};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::{
    CustomerConnectionId, CustomerId, CustomerPaymentMethodId, PaymentTransactionId, TenantId,
};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::customer_payment_methods::{
    CustomerPaymentMethodRow, CustomerPaymentMethodRowNew,
};
use diesel_models::invoicing_entities::InvoicingEntityProvidersRow;
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
        transaction_id: &PaymentTransactionId,
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

        let customer_connection: CustomerConnection = CustomerConnection {
            id: connection.id,
            customer_id: connection.customer.id,
            connector_id: connection.connector.id,
            supported_payment_types: connection
                .supported_payment_types
                .as_ref()
                .map(|v| v.iter().flatten().map(|t| t.clone().into()).collect()),
            external_customer_id: connection.external_customer_id,
        };

        let connector = Connector::from_row(&self.settings.crypt_key, connection.connector)?;

        let provider = initialize_payment_provider(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        // payment methods for that connector are either retrieved from invoicing entity (default) or overridden through the connection
        let payment_methods = match connection.supported_payment_types {
            Some(types) => types
                .into_iter()
                .filter_map(|t| t.map(Into::<PaymentMethodTypeEnum>::into))
                .collect(),
            None => {
                let invoicing_entity_providers =
                    InvoicingEntityProvidersRow::resolve_providers_by_id(
                        &mut conn,
                        connection.customer.invoicing_entity_id,
                        *tenant_id,
                    )
                    .await
                    .map_err(|err| StoreError::DatabaseError(err.error))?;

                let mut payment_methods = Vec::new();
                if let Some(card_provider) = invoicing_entity_providers.card_provider {
                    if card_provider.id == connector.id {
                        payment_methods.push(PaymentMethodTypeEnum::Card);
                    }
                }
                if let Some(direct_debit_provider) =
                    invoicing_entity_providers.direct_debit_provider
                {
                    // TODO only one based on customer.country ? Or stripe / other do it by themselves ?
                    if direct_debit_provider.id == connector.id {
                        payment_methods = vec![
                            PaymentMethodTypeEnum::DirectDebitSepa,
                            PaymentMethodTypeEnum::DirectDebitAch,
                            PaymentMethodTypeEnum::DirectDebitBacs,
                        ];
                    }
                }

                payment_methods
            }
        };

        let setup_intent = provider
            .create_setup_intent_in_provider(&customer_connection, &connector, payment_methods)
            .await
            .change_context_lazy(|| StoreError::PaymentProviderError)?;

        Ok(setup_intent)
    }

    async fn create_payment_intent(
        &self,
        tenant_id: &TenantId,
        payment_method_id: &CustomerPaymentMethodId,
        transaction_id: &PaymentTransactionId,
        amount: u64,
        currency: String,
    ) -> StoreResult<PaymentIntent> {
        let mut conn = self.get_conn().await?;

        let method = CustomerPaymentMethodRow::get_by_id(&mut conn, tenant_id, payment_method_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connection =
            CustomerConnectionDetailsRow::get_by_id(&mut conn, tenant_id, &method.connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connector = Connector::from_row(&self.settings.crypt_key, connection.connector)?;

        let provider = initialize_payment_provider(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        let payment_intent = provider
            .create_payment_intent_in_provider(
                &connector,
                transaction_id,
                &connection.external_customer_id,
                &method.external_payment_method_id,
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
