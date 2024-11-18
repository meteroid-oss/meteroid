pub mod customer {
    use error_stack::Report;

    use meteroid_grpc::meteroid::api::customers::v1 as server;
    use meteroid_store::domain;
    use meteroid_store::errors::StoreError;

    use crate::api::customers::error::CustomerApiError;
    use crate::api::shared::conversions::ProtoConv;
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
                                    collection_method: value.collection_method,
                                },
                            ),
                        ),
                    }))
                }
                domain::BillingConfig::Manual => {
                    Ok(ServerBillingConfigWrapper(server::CustomerBillingConfig {
                        billing_config_oneof: Some(
                            server::customer_billing_config::BillingConfigOneof::Manual(
                                server::customer_billing_config::Manual {},
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
                            collection_method: value.collection_method, //todo fix this
                        },
                    )))
                }
                Some(server::customer_billing_config::BillingConfigOneof::Manual(_)) => {
                    Ok(DomainBillingConfigWrapper(domain::BillingConfig::Manual))
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
                id: value.id.as_proto(),
                local_id: value.local_id,
                billing_config: Some(ServerBillingConfigWrapper::try_from(value.billing_config)?.0),
                invoicing_entity_id: value.invoicing_entity_id.as_proto(),
                name: value.name,
                alias: value.alias,
                email: value.email,
                invoicing_email: value.invoicing_email,
                phone: value.phone,
                balance_value_cents: value.balance_value_cents,
                currency: value.currency,
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
                local_id: value.local_id,
                name: value.name,
                alias: value.alias,
                country: value
                    .billing_address
                    .as_ref()
                    .and_then(|v| v.country.clone()),
                email: value.email,
                created_at: value.created_at.as_proto(),
            }))
        }
    }
}
