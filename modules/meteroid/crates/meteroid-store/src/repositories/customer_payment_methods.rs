use crate::domain::customer_payment_methods::CustomerPaymentMethod;
use crate::domain::subscriptions::PaymentMethodsConfig;
use crate::domain::{CustomerPaymentMethodNew, ResolvedPaymentMethod};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::{
    ConnectorId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId, SubscriptionId,
    TenantId,
};
use diesel_models::customer_payment_methods::{
    CustomerPaymentMethodRow, CustomerPaymentMethodRowNew,
};
use diesel_models::enums::PaymentMethodTypeEnum as DieselPaymentMethodTypeEnum;

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

        let context =
            CustomerPaymentMethodRow::get_subscription_payment_context(&mut conn, tenant_id, id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let config: Option<PaymentMethodsConfig> = context
            .payment_methods_config
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| {
                StoreError::SerdeError(format!("Failed to parse payment_methods_config: {}", e), e)
            })?;

        let config = config.unwrap_or_else(PaymentMethodsConfig::online);

        match config {
            // External = NEVER auto-charge
            PaymentMethodsConfig::External => Ok(ResolvedPaymentMethod::NotConfigured),

            PaymentMethodsConfig::BankTransfer { account_id } => {
                let bank_account_id = account_id.or(context.invoicing_entity_bank_account_id);
                match bank_account_id {
                    Some(id) => Ok(ResolvedPaymentMethod::BankTransfer(id)),
                    None => Ok(ResolvedPaymentMethod::NotConfigured),
                }
            }

            PaymentMethodsConfig::Online {
                config: online_config,
            } => {
                self.resolve_online_payment_method(
                    &mut conn,
                    &tenant_id,
                    &context.customer_id,
                    context.card_provider_id,
                    context.direct_debit_provider_id,
                    online_config.as_ref(),
                )
                .await
            }
        }
    }
}

impl Store {
    /// Prefers card over direct debit.
    async fn resolve_online_payment_method(
        &self,
        conn: &mut crate::store::PgConn,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        card_provider_id: Option<ConnectorId>,
        direct_debit_provider_id: Option<ConnectorId>,
        online_config: Option<&crate::domain::subscriptions::OnlineMethodsConfig>,
    ) -> StoreResult<ResolvedPaymentMethod> {
        let card_enabled = online_config
            .and_then(|c| c.card.as_ref())
            .map(|m| m.enabled)
            .unwrap_or(true);

        let direct_debit_enabled = online_config
            .and_then(|c| c.direct_debit.as_ref())
            .map(|m| m.enabled)
            .unwrap_or(true);

        let mut valid_provider_ids: Vec<ConnectorId> = Vec::new();

        if card_enabled && let Some(provider_id) = card_provider_id {
            valid_provider_ids.push(provider_id);
        }

        if direct_debit_enabled
            && let Some(provider_id) = direct_debit_provider_id
            && !valid_provider_ids.contains(&provider_id)
        {
            valid_provider_ids.push(provider_id);
        }

        if valid_provider_ids.is_empty() {
            return Ok(ResolvedPaymentMethod::NotConfigured);
        }

        let matching_methods =
            CustomerPaymentMethodRow::list_customer_payment_methods_by_providers(
                conn,
                tenant_id,
                customer_id,
                &valid_provider_ids,
            )
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let mut best_method = None;

        for method in &matching_methods {
            let is_card = method.payment_method_type == DieselPaymentMethodTypeEnum::Card;
            let is_direct_debit = matches!(
                method.payment_method_type,
                DieselPaymentMethodTypeEnum::DirectDebitSepa
                    | DieselPaymentMethodTypeEnum::DirectDebitAch
                    | DieselPaymentMethodTypeEnum::DirectDebitBacs
            );

            let provider_matches_card =
                card_enabled && card_provider_id.is_some_and(|p| p == method.connector_id);
            let provider_matches_dd = direct_debit_enabled
                && direct_debit_provider_id.is_some_and(|p| p == method.connector_id);

            if is_card && provider_matches_card {
                best_method = Some(method.id);
                break;
            } else if is_direct_debit && provider_matches_dd && best_method.is_none() {
                best_method = Some(method.id);
            }
        }

        match best_method {
            Some(payment_method_id) => Ok(ResolvedPaymentMethod::CustomerPaymentMethod(
                payment_method_id,
            )),
            None => Ok(ResolvedPaymentMethod::NotConfigured),
        }
    }
}
