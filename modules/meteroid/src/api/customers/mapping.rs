pub mod customer_old {
    use crate::api::errors;
    use crate::api::errors::DatabaseError;
    use crate::api::shared::mapping::datetime::datetime_to_timestamp;

    use meteroid_grpc::meteroid::api::customers::v1 as server;
    use meteroid_repository::customers as db;
    use serde_json::Value;

    fn decode_billing_config(
        billing_config: Value,
    ) -> Result<server::CustomerBillingConfig, DatabaseError> {
        serde_json::from_value(billing_config).map_err(|_| {
            DatabaseError::JsonParsingError("Failed to deserialize billing config".to_owned())
        })
    }

    fn decode_billing_address(address: Value) -> Result<server::Address, DatabaseError> {
        serde_json::from_value(address).map_err(|_| {
            DatabaseError::JsonParsingError("Failed to deserialize billing address".to_owned())
        })
    }

    fn decode_shipping_address(address: Value) -> Result<server::ShippingAddress, DatabaseError> {
        serde_json::from_value(address).map_err(|_| {
            DatabaseError::JsonParsingError("Failed to deserialize shipping address".to_owned())
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
            billing_address: customer
                .billing_address
                .map(decode_billing_address)
                .transpose()?,
            shipping_address: customer
                .shipping_address
                .map(decode_shipping_address)
                .transpose()?,
        })
    }
}

pub mod customer {
    use error_stack::Report;

    use crate::api::customers::error::CustomerApiError;
    use meteroid_grpc::meteroid::api::customers::v1 as server;
    use meteroid_repository::customers as db;
    use meteroid_store::domain;
    use meteroid_store::errors::StoreError;

    use crate::api::errors;
    use crate::api::shared::mapping::datetime::chrono_to_timestamp;

    fn address_domain_to_server(
        value: domain::Address,
    ) -> Result<server::Address, Report<StoreError>> {
        Ok(server::Address {
            line1: value.line1,
            line2: value.line2,
            city: value.city,
            country: value.country,
            state: value.state,
            zip_code: value.zip_code,
        })
    }

    fn shipping_address_domain_to_server(
        value: domain::ShippingAddress,
    ) -> Result<server::ShippingAddress, Report<StoreError>> {
        Ok(server::ShippingAddress {
            address: value.address.map(address_domain_to_server).transpose()?,
            same_as_billing: value.same_as_billing,
        })
    }

    pub fn billing_config_server_to_domain(
        value: server::CustomerBillingConfig,
    ) -> Result<domain::BillingConfig, CustomerApiError> {
        match value.billing_config_oneof {
            Some(server::customer_billing_config::BillingConfigOneof::Stripe(value)) => {
                Ok(domain::BillingConfig::Stripe(domain::Stripe {
                    customer_id: value.customer_id,
                    collection_method: value.collection_method as i32, //todo fix this
                }))
            }
            None => Err(CustomerApiError::MissingArgument(
                "billing_config".to_string(),
            )),
        }
    }

    fn billing_config_domain_to_server(
        value: domain::BillingConfig,
    ) -> Result<server::CustomerBillingConfig, Report<StoreError>> {
        match value {
            domain::BillingConfig::Stripe(value) => Ok(server::CustomerBillingConfig {
                billing_config_oneof: Some(
                    server::customer_billing_config::BillingConfigOneof::Stripe(
                        server::customer_billing_config::Stripe {
                            customer_id: value.customer_id,
                            collection_method: value.collection_method as i32,
                        },
                    ),
                ),
            }),
        }
    }

    pub fn domain_to_server(
        customer: domain::Customer,
    ) -> Result<server::Customer, Report<StoreError>> {
        Ok(server::Customer {
            id: customer.id.to_string(),
            billing_config: customer
                .billing_config
                .map(billing_config_domain_to_server)
                .transpose()?,
            name: customer.name,
            alias: customer.alias,
            email: customer.email,
            invoicing_email: customer.invoicing_email,
            phone: customer.phone,
            balance_value_cents: customer.balance_value_cents,
            balance_currency: customer.balance_currency,
            archived_at: customer.archived_at.map(chrono_to_timestamp),
            created_at: Some(chrono_to_timestamp(customer.created_at)),
            billing_address: customer
                .billing_address
                .map(address_domain_to_server)
                .transpose()?,
            shipping_address: customer
                .shipping_address
                .map(shipping_address_domain_to_server)
                .transpose()?,
        })
    }

    pub fn list_db_to_server(
        customer: domain::Customer,
    ) -> Result<server::CustomerList, Report<StoreError>> {
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
