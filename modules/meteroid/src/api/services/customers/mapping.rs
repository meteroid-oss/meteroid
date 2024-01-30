pub mod customer {
    use crate::api::services::errors;
    use crate::api::services::errors::DatabaseError;
    use crate::api::services::shared::mapping::datetime::datetime_to_timestamp;

    use meteroid_grpc::meteroid::api::customers::v1 as server;
    use meteroid_repository::customers as db;
    use serde_json::Value;

    fn decode_billing_config(
        billing_config: Value,
    ) -> Result<server::CustomerBillingConfig, DatabaseError> {
        serde_json::from_value(billing_config).map_err(|_| {
            errors::DatabaseError::JsonParsingError(
                "Failed to deserialize billing config".to_owned(),
            )
        })
    }

    fn decode_address(address: Value) -> Result<server::Address, DatabaseError> {
        serde_json::from_value(address).map_err(|_| {
            errors::DatabaseError::JsonParsingError("Failed to deserialize address".to_owned())
        })
    }

    pub fn db_to_server(customer: db::Customer) -> Result<server::Customer, errors::DatabaseError> {
        let billing_config_decoded = customer
            .billing_config
            .map(decode_billing_config)
            .transpose()?;

        Ok(server::Customer {
            id: customer.id.to_string(),
            billing_config: billing_config_decoded,
            name: customer.name,
            alias: customer.alias,
            email: customer.email,
            invoicing_email: customer.invoicing_email,
            phone: customer.phone,
            balance_value_cents: customer.balance_value_cents,
            balance_currency: customer.balance_currency,
            archived_at: customer.archived_at.map(datetime_to_timestamp),
            created_at: customer.created_at.map(datetime_to_timestamp),
            billing_address: customer.billing_address.map(decode_address).transpose()?,
            shipping_address: customer.shipping_address.map(decode_address).transpose()?,
        })
    }

    pub fn list_db_to_server(
        customer: db::CustomerList,
    ) -> Result<server::CustomerList, errors::DatabaseError> {
        Ok(server::CustomerList {
            id: customer.id.to_string(),
            name: customer.name,
            alias: customer.alias,
            email: customer.email,
        })
    }

    pub fn create_db_to_server(
        customer: db::CreateCustomer,
    ) -> Result<server::CustomerList, errors::DatabaseError> {
        Ok(server::CustomerList {
            id: customer.id.to_string(),
            name: customer.name,
            alias: customer.alias,
            email: customer.email,
        })
    }
}
