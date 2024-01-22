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
            billing_config: decode_billing_config(customer.billing_config.into()).ok(),
            name: customer.name,
            alias: customer.alias,
            email: customer.email,
            invoicing_email: customer.invoicing_email,
            phone: customer.phone,
            balance_value: customer.balance_value,
            balance_currency: customer.balance_currency,
            archived_at: customer.archived_at,
            created_at: customer.created_at,
            billing_address_line1: customer.billing_address_line1,
            billing_address_line2: customer.billing_address_line2,
            billing_address_city: customer.billing_address_city,
            billing_address_country: customer.billing_address_country,
            billing_address_state: customer.billing_address_state,
            billing_address_zipcode: customer.billing_address_zipcode,
            shipping_address_same: customer.shipping_address_same,
            shipping_address_line1: customer.shipping_address_line1,
            shipping_address_line2: customer.shipping_address_line2,
            shipping_address_city: customer.shipping_address_city,
            shipping_address_country: customer.shipping_address_country,
            shipping_address_state: customer.shipping_address_state,
            shipping_address_zipcode: customer.shipping_address_zipcode,
        })
    }

    pub fn list_db_to_server(
        customer: db::CustomerList,
    ) -> Result<server::CustomerList, errors::DatabaseError> {
        Ok(CustomerList {
            id: customer.id.to_string(),
            name: customer.name,
            alias: customer.alias,
            email: customer.email,
        })
    }
}
