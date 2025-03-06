pub mod customer {
    use error_stack::Report;

    use meteroid_grpc::meteroid::api::customers::v1 as server;
    use meteroid_store::domain;
    use meteroid_store::errors::StoreError;

    use crate::api::customers::error::CustomerApiError;
    use crate::api::shared::conversions::ProtoConv;
    use crate::api::shared::mapping::datetime::chrono_to_timestamp;

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
                local_id: value.id.as_proto(), // todo remove me
                invoicing_entity_id: value.invoicing_entity_id.as_proto(),
                name: value.name,
                alias: value.alias,
                billing_email: value.billing_email,
                invoicing_emails: value.invoicing_emails,
                phone: value.phone,
                balance_value_cents: value.balance_value_cents,
                currency: value.currency,
                archived_at: value.archived_at.map(chrono_to_timestamp),
                created_at: Some(chrono_to_timestamp(value.created_at)),
                vat_number: value.vat_number,
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
                id: value.id.as_proto(),
                local_id: value.id.as_proto(), // todo remove me
                name: value.name,
                alias: value.alias,
                country: value
                    .billing_address
                    .as_ref()
                    .and_then(|v| v.country.clone()),
                billing_email: value.billing_email,
                created_at: value.created_at.as_proto(),
            }))
        }
    }
}
pub mod customer_payment_method {

    use meteroid_grpc::meteroid::api::customers::v1 as server;
    use meteroid_store::domain;

    pub fn domain_to_server(
        method: domain::CustomerPaymentMethod,
    ) -> server::CustomerPaymentMethod {
        server::CustomerPaymentMethod {
            id: method.id.as_proto(),
            customer_id: method.customer_id.as_proto(),

            connection_id: method.connection_id.map(|v| v.as_proto()),
            payment_method_type: match method.payment_method_type {
                domain::PaymentMethodTypeEnum::Card => {
                    server::customer_payment_method::PaymentMethodTypeEnum::Card as i32
                }
                domain::PaymentMethodTypeEnum::DirectDebitAch => {
                    server::customer_payment_method::PaymentMethodTypeEnum::DirectDebitAch as i32
                }
                domain::PaymentMethodTypeEnum::Transfer => {
                    server::customer_payment_method::PaymentMethodTypeEnum::Transfer as i32
                }
                domain::PaymentMethodTypeEnum::DirectDebitSepa => {
                    server::customer_payment_method::PaymentMethodTypeEnum::DirectDebitSepa as i32
                }
                domain::PaymentMethodTypeEnum::DirectDebitBacs => {
                    server::customer_payment_method::PaymentMethodTypeEnum::DirectDebitBacs as i32
                }
                domain::PaymentMethodTypeEnum::Other => {
                    server::customer_payment_method::PaymentMethodTypeEnum::Other as i32
                }
            },
            card_brand: method.card_brand,
            card_last4: method.card_last4,
            card_exp_month: method.card_exp_month,
            card_exp_year: method.card_exp_year,
            account_number_hint: method.account_number_hint,
        }
    }
}
