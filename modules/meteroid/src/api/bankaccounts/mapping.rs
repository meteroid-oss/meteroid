pub mod bank_accounts {
    use crate::api::bankaccounts::error::BankAccountsApiError;
    use meteroid_grpc::meteroid::api::bankaccounts::v1 as server;

    use meteroid_store::domain::bank_accounts as domain;

    use common_domain::ids::{BankAccountId, BaseId, TenantId};
    use uuid::Uuid;

    mod format {
        use meteroid_grpc::meteroid::api::bankaccounts::v1 as server;

        use meteroid_store::domain::enums::BankAccountFormat;

        // Keep only alphanumeric characters
        fn normalize(input: &str) -> String {
            input
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_uppercase()
        }

        fn format_parts(part1: &str, part2: Option<&str>) -> String {
            let part1 = normalize(part1);
            let part2 = part2.map(normalize).unwrap_or_default();
            if part2.is_empty() {
                part1
            } else {
                format!("{} {}", part1, part2)
            }
        }

        pub(crate) fn proto_to_domain(
            format: server::bank_account_data::Format,
        ) -> (BankAccountFormat, String) {
            match format {
                server::bank_account_data::Format::IbanBicSwift(server::IbanBicSwift {
                    iban,
                    bic_swift,
                }) => (
                    BankAccountFormat::IbanBicSwift,
                    format_parts(&iban, bic_swift.as_deref()),
                ),
                server::bank_account_data::Format::AccountNumberBicSwift(
                    server::AccountNumberBicSwift {
                        account_number,
                        bic_swift,
                    },
                ) => (
                    BankAccountFormat::AccountBicSwift,
                    format_parts(&account_number, Some(&bic_swift)),
                ),
                server::bank_account_data::Format::AccountNumberRoutingNumber(
                    server::AccountNumberRoutingNumber {
                        account_number,
                        routing_number,
                    },
                ) => (
                    BankAccountFormat::AccountRouting,
                    format_parts(&account_number, Some(&routing_number)),
                ),
                server::bank_account_data::Format::SortCodeAccountNumber(
                    server::SortCodeAccountNumber {
                        sort_code,
                        account_number,
                    },
                ) => (
                    BankAccountFormat::SortCodeAccount,
                    format_parts(&sort_code, Some(&account_number)),
                ),
            }
        }

        fn parse_parts(input: &str) -> (String, Option<String>) {
            let mut parts = input.split_whitespace(); // Split on whitespace
            let part1 = parts.next().unwrap_or("").to_string(); // First part is mandatory
            let part2 = parts.next().map(String::from); // Second part is optional
            (part1, part2)
        }

        pub fn domain_to_proto(
            format: BankAccountFormat,
            account_numbers: String,
        ) -> server::bank_account_data::Format {
            match format {
                BankAccountFormat::IbanBicSwift => {
                    let (iban, bic_swift) = parse_parts(&account_numbers);
                    server::bank_account_data::Format::IbanBicSwift(server::IbanBicSwift {
                        iban,
                        bic_swift: bic_swift.into(),
                    })
                }
                BankAccountFormat::AccountBicSwift => {
                    let (account_number, bic_swift_opt) = parse_parts(&account_numbers);

                    // soft failure
                    let bic_swift = bic_swift_opt.unwrap_or_else(|| {
                        log::error!("Bic/Swift is missing for AccountBicSwift format");
                        String::new()
                    });

                    server::bank_account_data::Format::AccountNumberBicSwift(
                        server::AccountNumberBicSwift {
                            account_number,
                            bic_swift,
                        },
                    )
                }
                BankAccountFormat::AccountRouting => {
                    let (account_number, routing_number_opt) = parse_parts(&account_numbers);

                    let routing_number = routing_number_opt.unwrap_or_else(|| {
                        log::error!("Routing number is missing for AccountRouting format");
                        String::new()
                    });

                    server::bank_account_data::Format::AccountNumberRoutingNumber(
                        server::AccountNumberRoutingNumber {
                            account_number,
                            routing_number,
                        },
                    )
                }
                BankAccountFormat::SortCodeAccount => {
                    let (sort_code, account_number_opt) = parse_parts(&account_numbers);

                    let account_number = account_number_opt.unwrap_or_else(|| {
                        log::error!("Account number is missing for SortCodeAccount format");
                        String::new()
                    });
                    server::bank_account_data::Format::SortCodeAccountNumber(
                        server::SortCodeAccountNumber {
                            sort_code,
                            account_number,
                        },
                    )
                }
            }
        }
    }

    pub fn proto_to_domain(
        proto: server::BankAccountData,
        tenant_id: TenantId,
        actor: Uuid,
    ) -> Result<domain::BankAccountNew, BankAccountsApiError> {
        // clean the account numbers from any additional characters

        let (format, account_numbers) = format::proto_to_domain(
            proto
                .format
                .ok_or(BankAccountsApiError::MissingArgument("format".to_string()))?,
        );

        Ok(domain::BankAccountNew {
            id: BankAccountId::new(),
            created_by: actor,
            tenant_id,
            country: proto.country,
            bank_name: proto.bank_name,
            format,
            currency: proto.currency,
            account_numbers,
        })
    }

    pub fn domain_to_proto(domain: domain::BankAccount) -> server::BankAccount {
        server::BankAccount {
            id: domain.id.as_proto(),
            local_id: domain.id.as_proto(), //todo remove me
            data: Some(server::BankAccountData {
                country: domain.country,
                bank_name: domain.bank_name,
                format: Some(format::domain_to_proto(
                    domain.format,
                    domain.account_numbers,
                )),
                currency: domain.currency,
            }),
        }
    }

    pub(crate) fn proto_to_patch_domain(
        proto: server::UpdateBankAccountRequest,
        tenant_id: TenantId,
    ) -> Result<domain::BankAccountPatch, BankAccountsApiError> {
        let data = proto.data.ok_or_else(|| {
            BankAccountsApiError::MissingArgument("Missing patch data".to_string())
        })?;

        let (format, account_numbers) = format::proto_to_domain(
            data.format
                .ok_or(BankAccountsApiError::MissingArgument("format".to_string()))?,
        );

        Ok(domain::BankAccountPatch {
            id: BankAccountId::from_proto(proto.id).unwrap(),
            tenant_id,
            country: Some(data.country),
            bank_name: Some(data.bank_name),
            format: Some(format),
            currency: Some(data.currency),
            account_numbers: Some(account_numbers),
        })
    }
}
