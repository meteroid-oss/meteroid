use crate::StoreResult;
use crate::adapters::payment_service_providers::initialize_payment_provider;
use crate::domain::connectors::Connector;
use crate::domain::{CustomerConnection, PaymentMethodTypeEnum, SetupIntent};
use crate::errors::StoreError;
use crate::services::Services;
use crate::store::PgConn;
use common_domain::ids::{CustomerConnectionId, TenantId};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::invoicing_entities::InvoicingEntityProvidersRow;
use error_stack::ResultExt;

impl Services {
    pub(in crate::services) async fn create_setup_intent(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
    ) -> StoreResult<SetupIntent> {
        let connection =
            CustomerConnectionDetailsRow::get_by_id(conn, tenant_id, customer_connection_id)
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

        let connector = Connector::from_row(&self.store.settings.crypt_key, connection.connector)?;

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
                        conn,
                        connection.customer.invoicing_entity_id,
                        *tenant_id,
                    )
                    .await
                    .map_err(|err| StoreError::DatabaseError(err.error))?;

                let mut payment_methods = Vec::new();
                if let Some(card_provider) = invoicing_entity_providers.card_provider
                    && card_provider.id == connector.id
                {
                    payment_methods.push(PaymentMethodTypeEnum::Card);
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
}
