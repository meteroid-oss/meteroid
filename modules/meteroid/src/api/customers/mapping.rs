#[deprecated(note = "please use `customer` mod instead")]
pub mod customer_old {
    use serde_json::Value;

    use meteroid_grpc::meteroid::api::customers::v1 as server;
    use meteroid_repository::customers as db;

    use crate::api::errors;
    use crate::api::errors::DatabaseError;
    use crate::api::shared::mapping::datetime::datetime_to_timestamp;

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

    use meteroid_grpc::meteroid::api::customers::v1 as server;
    use meteroid_store::domain;
    use meteroid_store::errors::StoreError;

    use crate::api::customers::error::CustomerApiError;
    use crate::api::shared::mapping::datetime::chrono_to_timestamp;

    pub struct ServerBillingConfigWrapper(pub server::CustomerBillingConfig);

    impl TryFrom<domain::BillingConfig> for ServerBillingConfigWrapper {
        type Error = Report<StoreError>;

        fn try_from(value: domain::BillingConfig) -> Result<Self, Self::Error> {
            match value {
                domain::BillingConfig::Stripe(value) => {
                    Ok(ServerBillingConfigWrapper(server::CustomerBillingConfig {
                        billing_config_oneof: Some(
                            server::customer_billing_config::BillingConfigOneof::Stripe(
                                server::customer_billing_config::Stripe {
                                    customer_id: value.customer_id,
                                    collection_method: value.collection_method as i32,
                                },
                            ),
                        ),
                    }))
                }
            }
        }
    }

    pub struct DomainBillingConfigWrapper(pub domain::BillingConfig);

    impl TryFrom<server::CustomerBillingConfig> for DomainBillingConfigWrapper {
        type Error = CustomerApiError;

        fn try_from(value: server::CustomerBillingConfig) -> Result<Self, Self::Error> {
            match value.billing_config_oneof {
                Some(server::customer_billing_config::BillingConfigOneof::Stripe(value)) => {
                    Ok(DomainBillingConfigWrapper(domain::BillingConfig::Stripe(
                        domain::Stripe {
                            customer_id: value.customer_id,
                            collection_method: value.collection_method as i32, //todo fix this
                        },
                    )))
                }
                None => Err(CustomerApiError::MissingArgument(
                    "billing_config".to_string(),
                )),
            }
        }
    }

    pub struct ServerAddressWrapper(pub server::Address);

    impl TryFrom<domain::Address> for ServerAddressWrapper {
        type Error = Report<StoreError>;

        fn try_from(value: domain::Address) -> Result<Self, Self::Error> {
            Ok(ServerAddressWrapper(server::Address {
                line1: value.line1,
                line2: value.line2,
                city: value.city,
                country: value.country,
                state: value.state,
                zip_code: value.zip_code,
            }))
        }
    }

    pub struct DomainAddressWrapper(pub domain::Address);

    impl TryFrom<server::Address> for DomainAddressWrapper {
        type Error = CustomerApiError;

        fn try_from(value: server::Address) -> Result<Self, Self::Error> {
            Ok(DomainAddressWrapper(domain::Address {
                line1: value.line1,
                line2: value.line2,
                city: value.city,
                country: value.country,
                state: value.state,
                zip_code: value.zip_code,
            }))
        }
    }

    pub struct ServerShippingAddressWrapper(pub server::ShippingAddress);

    impl TryFrom<domain::ShippingAddress> for ServerShippingAddressWrapper {
        type Error = Report<StoreError>;

        fn try_from(value: domain::ShippingAddress) -> Result<Self, Self::Error> {
            Ok(ServerShippingAddressWrapper(server::ShippingAddress {
                address: value
                    .address
                    .map(ServerAddressWrapper::try_from)
                    .transpose()?
                    .map(|v| v.0),
                same_as_billing: value.same_as_billing,
            }))
        }
    }

    pub struct DomainShippingAddressWrapper(pub domain::ShippingAddress);

    impl TryFrom<server::ShippingAddress> for DomainShippingAddressWrapper {
        type Error = CustomerApiError;

        fn try_from(value: server::ShippingAddress) -> Result<Self, Self::Error> {
            Ok(DomainShippingAddressWrapper(domain::ShippingAddress {
                address: value
                    .address
                    .map(DomainAddressWrapper::try_from)
                    .transpose()?
                    .map(|v| v.0),
                same_as_billing: value.same_as_billing,
            }))
        }
    }

    pub struct ServerCustomerWrapper(pub server::Customer);
    impl TryFrom<domain::Customer> for ServerCustomerWrapper {
        type Error = Report<StoreError>;

        fn try_from(value: domain::Customer) -> Result<Self, Self::Error> {
            Ok(ServerCustomerWrapper(server::Customer {
                id: value.id.to_string(),
                billing_config: value
                    .billing_config
                    .map(ServerBillingConfigWrapper::try_from)
                    .transpose()?
                    .map(|v| v.0),
                name: value.name,
                alias: value.alias,
                email: value.email,
                invoicing_email: value.invoicing_email,
                phone: value.phone,
                balance_value_cents: value.balance_value_cents,
                balance_currency: value.balance_currency,
                archived_at: value.archived_at.map(chrono_to_timestamp),
                created_at: Some(chrono_to_timestamp(value.created_at)),
                billing_address: value
                    .billing_address
                    .map(ServerAddressWrapper::try_from)
                    .transpose()?
                    .map(|v| v.0),
                shipping_address: value
                    .shipping_address
                    .map(ServerShippingAddressWrapper::try_from)
                    .transpose()?
                    .map(|v| v.0),
            }))
        }
    }

    pub struct ServerCustomerBriefWrapper(pub server::CustomerBrief);
    impl TryFrom<domain::Customer> for ServerCustomerBriefWrapper {
        type Error = Report<StoreError>;

        fn try_from(value: domain::Customer) -> Result<Self, Self::Error> {
            Ok(ServerCustomerBriefWrapper(server::CustomerBrief {
                id: value.id.to_string(),
                name: value.name,
                alias: value.alias,
                email: value.email,
            }))
        }
    }
}
