pub mod customer {
    use crate::connectors::mapping::connectors::connection_metadata_to_server;
    use crate::customers::error::CustomerApiError;
    use crate::shared::mapping::datetime::chrono_to_timestamp;
    use common_domain::country::CountryCode;
    use error_stack::Report;
    use meteroid_app::conversions::ProtoConv;
    use meteroid_grpc::meteroid::api::customers::v1 as server;
    use meteroid_store::domain;
    use meteroid_store::domain::enums::ConnectorProviderEnum;
    use meteroid_store::errors::StoreError;

    pub fn customer_type_to_server(t: domain::CustomerType) -> server::CustomerType {
        match t {
            domain::CustomerType::Company => server::CustomerType::Company,
            domain::CustomerType::Individual => server::CustomerType::Individual,
        }
    }

    pub fn customer_type_from_server(value: i32) -> Result<domain::CustomerType, tonic::Status> {
        match server::CustomerType::try_from(value) {
            Ok(server::CustomerType::Company) => Ok(domain::CustomerType::Company),
            Ok(server::CustomerType::Individual) => Ok(domain::CustomerType::Individual),
            Err(_) => Err(tonic::Status::invalid_argument(format!(
                "unknown customer_type value: {value}"
            ))),
        }
    }

    pub fn vat_validation_status_to_server(
        status: Option<domain::VatNumberValidationStatus>,
    ) -> server::VatNumberValidationStatus {
        match status {
            None => server::VatNumberValidationStatus::Unspecified,
            Some(domain::VatNumberValidationStatus::Pending) => {
                server::VatNumberValidationStatus::Pending
            }
            Some(domain::VatNumberValidationStatus::Valid) => {
                server::VatNumberValidationStatus::Valid
            }
            Some(domain::VatNumberValidationStatus::Invalid) => {
                server::VatNumberValidationStatus::Invalid
            }
            Some(domain::VatNumberValidationStatus::Unavailable) => {
                server::VatNumberValidationStatus::Unavailable
            }
        }
    }

    pub fn tax_status_to_server(status: domain::CustomerTaxStatus) -> server::CustomerTaxStatus {
        match status {
            domain::CustomerTaxStatus::Taxable => server::CustomerTaxStatus::Taxable,
            domain::CustomerTaxStatus::Exempt => server::CustomerTaxStatus::Exempt,
            domain::CustomerTaxStatus::ReverseCharge => server::CustomerTaxStatus::ReverseCharge,
        }
    }

    pub fn tax_status_from_server(status: server::CustomerTaxStatus) -> domain::CustomerTaxStatus {
        match status {
            server::CustomerTaxStatus::Taxable => domain::CustomerTaxStatus::Taxable,
            server::CustomerTaxStatus::Exempt => domain::CustomerTaxStatus::Exempt,
            server::CustomerTaxStatus::ReverseCharge => domain::CustomerTaxStatus::ReverseCharge,
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
                country: value.country.map(|c| c.as_proto()),
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
                country: CountryCode::from_proto_opt(value.country).map_err(|_| {
                    CustomerApiError::InvalidArgument("Invalid country code".into())
                })?,
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
                customer_type: customer_type_to_server(value.customer_type) as i32,
                first_name: value.first_name,
                last_name: value.last_name,
                alias: value.alias,
                billing_email: value.billing_email,
                invoicing_emails: value.invoicing_emails,
                phone: value.phone,
                current_payment_method_id: value.current_payment_method_id.map(|v| v.as_proto()),
                currency: value.currency,
                archived_at: value.archived_at.map(chrono_to_timestamp),
                created_at: Some(chrono_to_timestamp(value.created_at)),
                vat_number: value.vat_number,
                legal_number: value.legal_number,
                buyer_reference: value.buyer_reference,
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
                connection_metadata: value.conn_meta.as_ref().map(connection_metadata_to_server),
                custom_taxes: value
                    .custom_taxes
                    .into_iter()
                    .map(|t| server::CustomerTaxRate {
                        tax_code: t.tax_code,
                        name: t.name,
                        rate: t.rate.to_string(),
                    })
                    .collect(),
                tax_status: tax_status_to_server(value.tax_status) as i32,
                exemption_reason: value.exemption_reason,
                is_vat_number_valid: value.vat_number_format_valid,
                vat_number_validation_status: vat_validation_status_to_server(
                    value.vat_number_validation_status,
                ) as i32,
                vat_number_checked_at: value.vat_number_checked_at.map(chrono_to_timestamp),
                vat_number_registered_name: value
                    .vat_number_vies_check
                    .as_ref()
                    .and_then(|c| c.name.clone()),
                vat_number_consultation_number: value
                    .vat_number_vies_check
                    .as_ref()
                    .and_then(|c| c.request_identifier.clone()),
                customer_connections: Vec::new(), // Will be populated in the service layer
                payment_methods: Vec::new(),      // Will be populated in the service layer
                connected_account_id: value.connected_account_id.map(|id| id.to_string()),
                lifecycle_state: String::new(), // Derived on read in the detail endpoint
                custom_properties: (!value.custom_properties.is_null())
                    .then(|| value.custom_properties.to_string()),
                preferred_locales: value.preferred_locales,
                // display_name is enriched by the detail endpoint (needs a user lookup).
                owner: value.owner_user_id.map(|id| server::CustomerOwner {
                    user_id: id.as_proto(),
                    display_name: String::new(),
                }),
            }))
        }
    }

    pub struct ServerCustomerBriefWrapper(pub server::CustomerBrief);

    impl TryFrom<domain::Customer> for ServerCustomerBriefWrapper {
        type Error = Report<StoreError>;

        fn try_from(value: domain::Customer) -> Result<Self, Self::Error> {
            let currency = value.currency.clone();
            Ok(ServerCustomerBriefWrapper(customer_brief_from(
                value,
                0,
                String::new(),
                currency,
            )))
        }
    }

    impl TryFrom<domain::CustomerWithMetrics> for ServerCustomerBriefWrapper {
        type Error = Report<StoreError>;

        fn try_from(value: domain::CustomerWithMetrics) -> Result<Self, Self::Error> {
            let currency = value.customer.currency.clone();
            Ok(ServerCustomerBriefWrapper(customer_brief_from(
                value.customer,
                value.mrr_cents,
                value.lifecycle_state,
                currency,
            )))
        }
    }

    fn customer_brief_from(
        value: domain::Customer,
        mrr_cents: i64,
        lifecycle_state: String,
        currency: String,
    ) -> server::CustomerBrief {
        server::CustomerBrief {
            id: value.id.as_proto(),
            local_id: value.id.as_proto(), // todo remove me
            name: value.name,
            alias: value.alias,
            country: value
                .billing_address
                .as_ref()
                .and_then(|v| v.country.clone())
                .map(|c| c.as_proto()),
            billing_email: value.billing_email,
            created_at: value.created_at.as_proto(),
            mrr_cents,
            currency,
            lifecycle_state,
            custom_properties: (!value.custom_properties.is_null())
                .then(|| value.custom_properties.to_string()),
            // display_name is batch-resolved by the caller; the id ships the filter/roll-up key.
            owner: value.owner_user_id.map(|id| server::CustomerOwner {
                user_id: id.as_proto(),
                display_name: String::new(),
            }),
            // Filled by ListCustomers for callers who may see signals.
            open_signals: None,
        }
    }

    pub fn custom_taxes_from_grpc(
        custom_taxes: Option<server::update_customer::CustomTaxesList>,
    ) -> Result<Option<Vec<domain::CustomerTaxRate>>, CustomerApiError> {
        custom_taxes
            .map(|list| {
                list.taxes
                    .into_iter()
                    .map(|t| {
                        Ok(domain::CustomerTaxRate {
                            tax_code: t.tax_code,
                            name: t.name,
                            rate: rust_decimal::Decimal::from_proto(t.rate).map_err(|_| {
                                CustomerApiError::InvalidArgument("Invalid tax rate".to_string())
                            })?,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
    }

    /// Converts a customer connection to proto representation.
    /// Returns None for Mock connectors, which should not be exposed via API.
    pub fn map_customer_connection_to_proto(
        connection: domain::CustomerConnection,
        provider: ConnectorProviderEnum,
        external_company_id: Option<String>,
    ) -> Option<server::CustomerConnection> {
        let connector_provider =
            crate::connectors::mapping::connectors::connector_provider_to_server(&provider)?;

        Some(server::CustomerConnection {
            id: connection.id.as_proto(),
            connector_id: connection.connector_id.as_proto(),
            external_customer_id: connection.external_customer_id,
            connector_provider: connector_provider.into(),
            external_company_id,
        })
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
            connection_id: method.connection_id.as_proto(),
            external_payment_method: method.external_payment_method_id,
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

pub mod summary {
    use meteroid_app::conversions::{AsProtoOpt, ProtoConv};
    use meteroid_grpc::meteroid::api::customers::v1 as server;
    use meteroid_store::domain::customer_summary as domain;
    use meteroid_store::domain::customer_summary::RenewalMode;

    fn totals(values: &[domain::MoneyTotal]) -> Vec<server::CustomerMoneyTotal> {
        values
            .iter()
            .map(|t| server::CustomerMoneyTotal {
                currency: t.currency.clone(),
                amount_cents: t.amount_cents,
                invoice_count: t.invoice_count,
            })
            .collect()
    }

    pub fn summary_to_server(
        value: &domain::CustomerSummary,
    ) -> server::GetCustomerSummaryResponse {
        server::GetCustomerSummaryResponse {
            mrr_cents: value.mrr_cents,
            reporting_currency: value.reporting_currency.clone(),
            receivables: value
                .receivables
                .as_ref()
                .map(|r| server::CustomerReceivables {
                    open: totals(&r.open),
                    overdue: totals(&r.overdue),
                    last_failure_at: r.last_failure_at.as_proto(),
                    last_failure_reason: r.last_failure_reason.clone(),
                    last_failure_invoice_id: r.last_failure_invoice_id.map(|id| id.as_proto()),
                    next_retry_at: r.next_retry_at.as_proto(),
                }),
            term: value.term.as_ref().map(|t| server::CustomerTerm {
                subscription_id: t.subscription_id.as_proto(),
                plan_name: t.plan_name.clone(),
                date: t.date.as_proto(),
                mode: match t.mode {
                    RenewalMode::Ending => "ending",
                    RenewalMode::Renewing => "renewing",
                }
                .to_string(),
                cancellation_event_id: t.cancellation_event_id.map(|id| id.as_proto()),
            }),
        }
    }
}
