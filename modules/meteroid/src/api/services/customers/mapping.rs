pub mod customer {
    use crate::api::services::errors;
    use crate::api::services::errors::DatabaseError;
    use meteroid_grpc::meteroid::api::customers::v1 as server;
    use meteroid_grpc::meteroid::api::customers::v1::{Customer, CustomerBillingConfig};
    use meteroid_repository::customers as db;
    use serde_json::Value;

    fn decode_billing_config(
        billing_config: Value,
    ) -> Result<CustomerBillingConfig, DatabaseError> {
        serde_json::from_value(billing_config).map_err(|_| {
            errors::DatabaseError::JsonParsingError(
                "Failed to deserialize billing config".to_owned(),
            )
        })
    }

    pub fn db_to_server(customer: db::Customer) -> Result<server::Customer, errors::DatabaseError> {
        let billing_config_decoded = decode_billing_config(customer.billing_config.into())?;

        Ok(Customer {
            id: customer.id.to_string(),
            name: customer.name,
            alias: customer.alias,
            billing_config: Some(billing_config_decoded),
        })
    }

    pub fn list_db_to_server(
        customer: db::ListCustomers,
    ) -> Result<server::Customer, errors::DatabaseError> {
        let billing_config_decoded = decode_billing_config(customer.billing_config.into())?;

        Ok(Customer {
            id: customer.id.to_string(),
            name: customer.name,
            alias: customer.alias,
            billing_config: Some(billing_config_decoded),
        })
    }
}
