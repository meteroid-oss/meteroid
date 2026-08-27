#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::model::*;
    use crate::{MeteroidTaxEngine, TaxEngine, shared};
    use std::str::FromStr;

    use common_domain::country::CountryCode;
    use rust_decimal_macros::dec;

    fn test_address(country: &str, region: Option<&str>) -> Address {
        Address {
            country: CountryCode::parse_as_opt(country),
            region: region.map(|s| s.to_string()),
            city: None,
            line1: None,
            postal_code: None,
        }
    }

    fn test_line_item(id: &str, amount: i64, custom_taxes: Vec<TaxRate>) -> LineItemForTax {
        LineItemForTax {
            line_id: id.to_string(),
            amount,
            custom_taxes,
            tax_category: None,
        }
    }

    /// A single customer-level tax rate — the sole production path for a
    /// customer-configured rate. Yields `TaxDetails::MultipleTaxes` with one entry.
    fn flat_rate(rate: rust_decimal::Decimal) -> CustomerTax {
        CustomerTax::TaxRates(vec![CustomerTaxRate {
            tax_code: String::new(),
            name: "Tax".to_string(),
            rate,
        }])
    }

    #[tokio::test]
    async fn test_nontaxable_category_overrides_taxable_customer() {
        // A line classified as non-taxable yields no tax even where the customer is taxable.
        let customer_tax = flat_rate(dec!(0.15));
        let invoicing_entity_address = test_address("US", Some("CA"));
        let line_items = vec![
            LineItemForTax {
                tax_category: Some(NONTAXABLE_CATEGORY_KEY.to_string()),
                ..test_line_item("nontaxable", 10000, vec![])
            },
            test_line_item("taxable", 10000, vec![]),
        ];

        let result = shared::compute_tax(
            customer_tax,
            invoicing_entity_address.clone(),
            invoicing_entity_address,
            line_items,
        )
        .await
        .unwrap();

        match &result[0].tax_details {
            TaxDetails::Exempt(VatExemptionReason::TaxExempt) => {}
            other => panic!("Expected non-taxable line to be exempt, got {other:?}"),
        }
        match &result[1].tax_details {
            TaxDetails::MultipleTaxes {
                total_tax_amount, ..
            } => assert_eq!(*total_tax_amount, 1500),
            other => panic!("Expected taxable line to be taxed, got {other:?}"),
        }

        let breakdown = shared::compute_breakdown_from_line_items(&result);
        assert_eq!(breakdown.tax_amount, 1500);
    }

    #[tokio::test]
    async fn test_tax_exempt_customer() {
        // Customer is tax exempt
        let customer_tax = CustomerTax::Exempt;
        let invoicing_entity_address = test_address("US", Some("CA"));
        let line_items = vec![
            test_line_item("item1", 10000, vec![]),
            test_line_item("item2", 5000, vec![]),
        ];

        let result = shared::compute_tax(
            customer_tax,
            invoicing_entity_address.clone(),
            invoicing_entity_address,
            line_items,
        )
        .await
        .unwrap();

        // All items should be exempt
        assert_eq!(result.len(), 2);
        for item in &result {
            match &item.tax_details {
                TaxDetails::Exempt(VatExemptionReason::TaxExempt) => {}
                _ => panic!("Expected tax exempt"),
            }
        }

        let breakdown = shared::compute_breakdown_from_line_items(&result);
        assert_eq!(breakdown.tax_amount, 0);
        assert_eq!(breakdown.total_amount_after_tax, 15000);
    }

    #[tokio::test]
    async fn test_custom_tax_rate_on_customer() {
        // Customer has custom tax rate of 15%
        let customer_tax = flat_rate(dec!(0.15));
        let invoicing_entity_address = test_address("US", Some("CA"));
        let line_items = vec![
            test_line_item("item1", 10000, vec![]),
            test_line_item("item2", 5000, vec![]),
        ];

        let result = shared::compute_tax(
            customer_tax,
            invoicing_entity_address.clone(),
            invoicing_entity_address,
            line_items,
        )
        .await
        .unwrap();

        assert_eq!(result.len(), 2);

        // Check first item: 10000 * 0.15 = 1500
        match &result[0].tax_details {
            TaxDetails::MultipleTaxes {
                taxes,
                total_tax_amount,
            } => {
                assert_eq!(*total_tax_amount, 1500);
                assert_eq!(taxes[0].tax_rate, dec!(0.15));
            }
            _ => panic!("Expected tax details"),
        }

        // Check second item: 5000 * 0.15 = 750
        match &result[1].tax_details {
            TaxDetails::MultipleTaxes {
                taxes,
                total_tax_amount,
            } => {
                assert_eq!(*total_tax_amount, 750);
                assert_eq!(taxes[0].tax_rate, dec!(0.15));
            }
            _ => panic!("Expected tax details"),
        }

        let breakdown = shared::compute_breakdown_from_line_items(&result);
        assert_eq!(breakdown.tax_amount, 2250); // 1500 + 750
        assert_eq!(breakdown.total_amount_after_tax, 17250); // 15000 + 2250
    }

    #[tokio::test]
    async fn test_line_item_custom_tax_overrides_customer_tax() {
        // Customer has 10% tax but line item has custom 20% tax
        let customer_tax = flat_rate(dec!(0.10));
        let invoicing_entity_address = test_address("FR", None);

        let custom_tax = TaxRate {
            reference: "custom_vat".to_string(),
            name: "French VAT".to_string(),
            tax_rules: vec![TaxRateRule {
                country: Some(CountryCode::from_str("FR").expect("failed to parse country code")),
                region: None,
                rate: dec!(0.20),
            }],
        };

        let line_items = vec![
            test_line_item("item1", 10000, vec![custom_tax.clone()]),
            test_line_item("item2", 5000, vec![]), // This will use customer tax
        ];

        let result = shared::compute_tax(
            customer_tax,
            invoicing_entity_address.clone(),
            invoicing_entity_address,
            line_items,
        )
        .await
        .unwrap();

        // First item should have 20% tax from custom tax
        match &result[0].tax_details {
            TaxDetails::Tax {
                tax_amount,
                tax_rate,
                tax_name,
                ..
            } => {
                assert_eq!(*tax_amount, 2000); // 10000 * 0.20
                assert_eq!(*tax_rate, dec!(0.20));
                assert_eq!(tax_name, "French VAT");
            }
            _ => panic!("Expected tax details"),
        }

        // Second item should have 10% tax from customer
        match &result[1].tax_details {
            TaxDetails::MultipleTaxes {
                taxes,
                total_tax_amount,
            } => {
                assert_eq!(*total_tax_amount, 500); // 5000 * 0.10
                assert_eq!(taxes[0].tax_rate, dec!(0.10));
            }
            _ => panic!("Expected tax details"),
        }

        let breakdown = shared::compute_breakdown_from_line_items(&result);
        assert_eq!(breakdown.tax_amount, 2500); // 2000 + 500
        assert_eq!(breakdown.total_amount_after_tax, 17500); // 15000 + 2500
    }

    #[tokio::test]
    async fn test_regional_tax_rules() {
        let customer_tax = CustomerTax::NoTax;
        let invoicing_entity_address = test_address("US", Some("CA"));

        let custom_tax = TaxRate {
            reference: "sales_tax".to_string(),
            name: "Sales Tax".to_string(),
            tax_rules: vec![
                // Generic US rate
                TaxRateRule {
                    country: Some(
                        CountryCode::from_str("US").expect("failed to parse country code"),
                    ),
                    region: None,
                    rate: dec!(0.05),
                },
                // Specific California rate (should be selected)
                TaxRateRule {
                    country: Some(
                        CountryCode::from_str("US").expect("failed to parse country code"),
                    ),
                    region: Some("CA".to_string()),
                    rate: dec!(0.0725),
                },
                // Specific New York rate (should not be selected)
                TaxRateRule {
                    country: Some(
                        CountryCode::from_str("US").expect("failed to parse country code"),
                    ),
                    region: Some("NY".to_string()),
                    rate: dec!(0.08),
                },
            ],
        };

        let line_items = vec![test_line_item("item1", 10000, vec![custom_tax])];

        let result = shared::compute_tax(
            customer_tax,
            invoicing_entity_address.clone(),
            invoicing_entity_address,
            line_items,
        )
        .await
        .unwrap();

        // Should use California rate (7.25%)
        match &result[0].tax_details {
            TaxDetails::Tax {
                tax_amount,
                tax_rate,
                ..
            } => {
                assert_eq!(*tax_amount, 725); // 10000 * 0.0725
                assert_eq!(*tax_rate, dec!(0.0725));
            }
            _ => panic!("Expected tax details"),
        }
    }

    #[tokio::test]
    async fn test_tax_breakdown_grouping() {
        let customer_tax = CustomerTax::NoTax;
        let invoicing_entity_address = test_address("US", Some("CA"));

        let custom_tax_1 = TaxRate {
            reference: "vat_standard".to_string(),
            name: "Standard VAT".to_string(),
            tax_rules: vec![TaxRateRule {
                country: Some(CountryCode::from_str("US").expect("failed to parse country code")),
                region: Some("CA".to_string()),
                rate: dec!(0.20),
            }],
        };

        let custom_tax_2 = TaxRate {
            reference: "vat_reduced".to_string(),
            name: "Reduced VAT".to_string(),
            tax_rules: vec![TaxRateRule {
                country: Some(CountryCode::from_str("US").expect("failed to parse country code")),
                region: Some("CA".to_string()),
                rate: dec!(0.05),
            }],
        };

        let line_items = vec![
            test_line_item("item1", 10000, vec![custom_tax_1.clone()]),
            test_line_item("item2", 5000, vec![custom_tax_1.clone()]),
            test_line_item("item3", 8000, vec![custom_tax_2]),
        ];

        let result = shared::compute_tax(
            customer_tax,
            invoicing_entity_address.clone(),
            invoicing_entity_address,
            line_items,
        )
        .await
        .unwrap();

        let breakdown = shared::compute_breakdown_from_line_items(&result);

        // Should have 2 groups (standard VAT and reduced VAT)
        assert_eq!(breakdown.breakdown.len(), 2);

        // Total tax: (10000 + 5000) * 0.20 + 8000 * 0.05 = 3000 + 400 = 3400
        assert_eq!(breakdown.tax_amount, 3400);
        assert_eq!(breakdown.total_amount_after_tax, 26400); // 23000 + 3400
    }

    #[tokio::test]
    async fn test_rounding_behavior() {
        // Test that tax amounts are rounded correctly
        let customer_tax = flat_rate(dec!(0.21)); // 21% tax
        let invoicing_entity_address = test_address("US", None);

        // 999 * 0.21 = 209.79, should round to 210
        let line_items = vec![test_line_item("item1", 999, vec![])];

        let result = shared::compute_tax(
            customer_tax.clone(),
            invoicing_entity_address.clone(),
            invoicing_entity_address.clone(),
            line_items,
        )
        .await
        .unwrap();

        match &result[0].tax_details {
            TaxDetails::MultipleTaxes {
                total_tax_amount, ..
            } => {
                assert_eq!(*total_tax_amount, 210); // Rounded up from 209.79
            }
            _ => panic!("Expected tax details"),
        }

        // Test rounding down: 997 * 0.21 = 209.37, should round to 209
        let line_items = vec![test_line_item("item2", 997, vec![])];

        let result = shared::compute_tax(
            customer_tax,
            invoicing_entity_address.clone(),
            invoicing_entity_address,
            line_items,
        )
        .await
        .unwrap();

        match &result[0].tax_details {
            TaxDetails::MultipleTaxes {
                total_tax_amount, ..
            } => {
                assert_eq!(*total_tax_amount, 209); // Rounded down from 209.37
            }
            _ => panic!("Expected tax details"),
        }
    }

    #[tokio::test]
    async fn test_zero_amount_line_items() {
        let customer_tax = flat_rate(dec!(0.20));
        let invoicing_entity_address = test_address("US", None);
        let line_items = vec![
            test_line_item("item1", 0, vec![]),
            test_line_item("item2", 1000, vec![]),
        ];

        let result = shared::compute_tax(
            customer_tax,
            invoicing_entity_address.clone(),
            invoicing_entity_address,
            line_items,
        )
        .await
        .unwrap();

        // Zero amount should result in zero tax
        match &result[0].tax_details {
            TaxDetails::MultipleTaxes {
                total_tax_amount, ..
            } => {
                assert_eq!(*total_tax_amount, 0);
            }
            _ => panic!("Expected tax details"),
        }

        // Non-zero amount should have tax
        match &result[1].tax_details {
            TaxDetails::MultipleTaxes {
                total_tax_amount, ..
            } => {
                assert_eq!(*total_tax_amount, 200); // 1000 * 0.20
            }
            _ => panic!("Expected tax details"),
        }
    }

    #[tokio::test]
    async fn test_reverse_charge_vat() {
        use world_tax::{TaxRate, TaxType, VatRate};

        // Simulate B2B reverse charge scenario
        let customer_tax = CustomerTax::ResolvedTaxRate(TaxRate {
            rate: 0.0,
            tax_type: TaxType::VAT(VatRate::ReverseCharge),
            compound: false,
        });

        let invoicing_entity_address = test_address("FR", None);
        let line_items = vec![test_line_item("item1", 10000, vec![])];

        let result = shared::compute_tax(
            customer_tax,
            invoicing_entity_address.clone(),
            invoicing_entity_address,
            line_items,
        )
        .await
        .unwrap();

        // Should be exempt due to reverse charge
        match &result[0].tax_details {
            TaxDetails::Exempt(VatExemptionReason::ReverseCharge) => {}
            _ => panic!("Expected reverse charge exemption"),
        }

        let breakdown = shared::compute_breakdown_from_line_items(&result);
        assert_eq!(breakdown.tax_amount, 0);
        assert_eq!(breakdown.total_amount_after_tax, 10000);
    }

    #[tokio::test]
    async fn test_compound_multiple_tax_rates() {
        use world_tax::{TaxRate, TaxType, VatRate};

        // GST (5%, non-compound) + QST (9.975%, compounds on GST), Canadian style.
        let customer_tax = CustomerTax::ResolvedMultipleTaxRates(vec![
            TaxRate {
                rate: 0.05,
                tax_type: TaxType::GST,
                compound: false,
            },
            TaxRate {
                rate: 0.09975,
                tax_type: TaxType::QST,
                compound: true,
            },
        ]);

        let entity_address = test_address("CA", Some("QC"));
        let line_items = vec![test_line_item("item1", 10000, vec![])];

        let result = shared::compute_tax(
            customer_tax,
            entity_address.clone(),
            entity_address,
            line_items,
        )
        .await
        .unwrap();

        match &result[0].tax_details {
            TaxDetails::MultipleTaxes {
                taxes,
                total_tax_amount,
            } => {
                // GST on base: 10000 * 0.05 = 500
                assert_eq!(taxes[0].tax_amount, 500);
                // QST on base + GST: (10000 + 500) * 0.09975 = 1047.375 -> 1047
                assert_eq!(taxes[1].tax_amount, 1047);
                assert_eq!(*total_tax_amount, 1547);
            }
            _ => panic!("Expected multiple taxes"),
        }
    }

    fn test_customer(
        vat_number: Option<String>,
        tax_exempt: bool,
        custom_tax_rates: Vec<CustomerTaxRate>,
        country: &str,
    ) -> CustomerForTax {
        CustomerForTax {
            vat_number: vat_number.clone(),
            vat_number_format_valid: vat_number.is_some(),
            vat_number_vies_valid: None,
            require_vies_valid_for_reverse_charge: false,
            tax_status: if tax_exempt {
                CustomerTaxStatus::Exempt
            } else {
                CustomerTaxStatus::Taxable
            },
            exemption_reason: None,
            custom_tax_rates,
            billing_address: test_address(country, None),
            shipping_address: None,
        }
    }

    mod tax_engines {
        use super::*;
        use crate::{ManualTaxEngine, MeteroidTaxEngine, TaxEngine};

        #[tokio::test]
        async fn test_meteroid_tax_engine_eu_vat() {
            let engine = MeteroidTaxEngine;

            // Test B2C transaction within EU - should apply VAT
            let customer = test_customer(None, false, vec![], "DE"); // German B2C customer
            let invoicing_entity_address = test_address("FR", None); // French company
            let line_items = vec![test_line_item("item1", 10000, vec![])];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    invoicing_entity_address,
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            // Should have tax applied (German VAT rate)
            assert!(result.tax_amount > 0);
            assert_eq!(result.total_amount_after_tax, 10000 + result.tax_amount);
        }

        #[tokio::test]
        async fn test_meteroid_tax_engine_strict_vies_mode() {
            let engine = MeteroidTaxEngine;
            let invoicing_entity_address = test_address("FR", None);
            let invoice_date = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

            let mut customer = test_customer(Some("DE123456789".to_string()), false, vec![], "DE");
            customer.require_vies_valid_for_reverse_charge = true;

            // Strict mode + unverified number: no reverse charge, standard rate applies
            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer.clone(),
                    invoicing_entity_address.clone(),
                    vec![test_line_item("item1", 10000, vec![])],
                    invoice_date,
                )
                .await
                .unwrap();
            assert!(result.tax_amount > 0);

            // Strict mode + VIES-verified: reverse charge applies
            customer.vat_number_vies_valid = Some(true);
            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    invoicing_entity_address,
                    vec![test_line_item("item1", 10000, vec![])],
                    invoice_date,
                )
                .await
                .unwrap();
            assert_eq!(result.tax_amount, 0);
        }

        #[tokio::test]
        async fn test_meteroid_tax_engine_b2b_reverse_charge() {
            let engine = MeteroidTaxEngine;

            // Test B2B transaction between different EU countries - should be reverse charge
            let customer = test_customer(Some("DE123456789".to_string()), false, vec![], "DE");
            let invoicing_entity_address = test_address("FR", None);
            let line_items = vec![test_line_item("item1", 10000, vec![])];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    invoicing_entity_address,
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            // Should be reverse charge (0% tax but with reverse charge exemption)
            assert_eq!(result.tax_amount, 0);
            assert_eq!(result.total_amount_after_tax, 10000);
        }

        #[tokio::test]
        async fn test_manual_tax_engine_respects_customer_settings() {
            let engine = ManualTaxEngine;

            // Test customer with custom tax rate
            let customer = test_customer(
                None,
                false,
                vec![CustomerTaxRate {
                    tax_code: "CUSTOM".to_string(),
                    name: "Custom Tax".to_string(),
                    rate: rust_decimal_macros::dec!(0.18),
                }],
                "US",
            );
            let invoicing_entity_address = test_address("US", None);
            let line_items = vec![test_line_item("item1", 10000, vec![])];

            let result = engine
                .calculate_line_items_tax(
                    "USD".to_string(),
                    customer,
                    invoicing_entity_address,
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            // Should apply the custom 18% rate
            assert_eq!(result.tax_amount, 1800); // 10000 * 0.18
            assert_eq!(result.total_amount_after_tax, 11800);
        }

        #[tokio::test]
        async fn test_manual_tax_engine_tax_exempt_customer() {
            let engine = ManualTaxEngine;

            // Test tax-exempt customer
            let customer = test_customer(None, true, vec![], "US");
            let invoicing_entity_address = test_address("US", None);
            let line_items = vec![test_line_item("item1", 10000, vec![])];

            let result = engine
                .calculate_line_items_tax(
                    "USD".to_string(),
                    customer,
                    invoicing_entity_address,
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            // Should be fully exempt
            assert_eq!(result.tax_amount, 0);
            assert_eq!(result.total_amount_after_tax, 10000);
        }

        #[tokio::test]
        async fn test_manual_tax_engine_defaults_to_no_tax() {
            let engine = ManualTaxEngine;

            // Test regular customer with no special settings
            let customer = test_customer(None, false, vec![], "US");
            let invoicing_entity_address = test_address("US", None);
            let line_items = vec![test_line_item("item1", 10000, vec![])];

            let result = engine
                .calculate_line_items_tax(
                    "USD".to_string(),
                    customer,
                    invoicing_entity_address,
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            // Manual engine should default to no tax
            assert_eq!(result.tax_amount, 0);
            assert_eq!(result.total_amount_after_tax, 10000);
        }

        #[tokio::test]
        async fn test_meteroid_engine_invalid_vat_number_format() {
            let engine = MeteroidTaxEngine;

            // Customer has VAT number but format is invalid
            let mut customer = test_customer(Some("INVALID_VAT".to_string()), false, vec![], "DE");
            customer.vat_number_format_valid = false;

            let invoicing_entity_address = test_address("FR", None);
            let line_items = vec![test_line_item("item1", 10000, vec![])];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    invoicing_entity_address,
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            // Should be treated as B2C since vat_number_format_valid is false
            // This means it should apply German VAT rate instead of reverse charge
            assert!(result.tax_amount > 0);
        }

        #[tokio::test]
        async fn test_meteroid_engine_empty_vat_number() {
            let engine = MeteroidTaxEngine;

            // Customer has empty VAT number string
            let customer = test_customer(Some("".to_string()), false, vec![], "DE");
            let invoicing_entity_address = test_address("FR", None);
            let line_items = vec![test_line_item("item1", 10000, vec![])];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    invoicing_entity_address,
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            // Empty VAT number should be treated as B2C, not B2B
            // So it should apply German VAT rate, not reverse charge
            assert!(result.tax_amount > 0);
        }

        #[tokio::test]
        async fn test_meteroid_engine_missing_customer_country() {
            let engine = MeteroidTaxEngine;

            // Customer with no billing country
            let mut customer = test_customer(None, false, vec![], "");
            customer.billing_address.country = None;

            let invoicing_entity_address = test_address("FR", None);
            let line_items = vec![test_line_item("item1", 10000, vec![])];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    invoicing_entity_address,
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            // Should default to no tax when customer country is missing
            assert_eq!(result.tax_amount, 0);
            assert_eq!(result.total_amount_after_tax, 10000);
        }

        #[tokio::test]
        async fn test_meteroid_engine_missing_invoicing_country() {
            let engine = MeteroidTaxEngine;

            let customer = test_customer(None, false, vec![], "DE");
            let mut invoicing_entity_address = test_address("", None);
            invoicing_entity_address.country = None;

            let line_items = vec![test_line_item("item1", 10000, vec![])];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    invoicing_entity_address,
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            // Should default to no tax when invoicing country is missing
            assert_eq!(result.tax_amount, 0);
            assert_eq!(result.total_amount_after_tax, 10000);
        }
    }

    #[tokio::test]
    async fn test_reverse_charge_preserved_in_result() {
        let engine = MeteroidTaxEngine;

        // B2B transaction between different EU countries
        let customer = CustomerForTax {
            vat_number: Some("DE123456789".to_string()),
            vat_number_format_valid: true,
            vat_number_vies_valid: None,
            require_vies_valid_for_reverse_charge: false,
            tax_status: CustomerTaxStatus::Taxable,
            exemption_reason: None,
            custom_tax_rates: vec![],
            billing_address: Address {
                country: Some(CountryCode::from_str("DE").expect("failed to parse country code")),
                region: None,
                city: None,
                line1: None,
                postal_code: None,
            },
            shipping_address: None,
        };

        let invoicing_entity_address = Address {
            country: Some(CountryCode::from_str("FR").expect("failed to parse country code")),
            region: None,
            city: None,
            line1: None,
            postal_code: None,
        };

        let line_items = vec![LineItemForTax {
            line_id: "item1".to_string(),
            amount: 10000,
            custom_taxes: vec![],
            tax_category: None,
        }];

        let result = engine
            .calculate_line_items_tax(
                "EUR".to_string(),
                customer,
                invoicing_entity_address,
                line_items,
                chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            )
            .await
            .unwrap();

        // Should be reverse charge
        assert_eq!(result.tax_amount, 0);
        assert_eq!(result.total_amount_after_tax, 10000);

        // Check that breakdown contains reverse charge exemption
        assert_eq!(result.breakdown.len(), 1);
        match &result.breakdown[0].details {
            TaxDetails::Exempt(VatExemptionReason::ReverseCharge) => {
                // Success - reverse charge is preserved
            }
            _ => panic!("Expected reverse charge exemption in breakdown"),
        }
    }

    #[tokio::test]
    async fn test_tax_exempt_preserved_in_result() {
        let engine = MeteroidTaxEngine;

        // Tax-exempt customer
        let customer = CustomerForTax {
            vat_number: None,
            vat_number_format_valid: false,
            vat_number_vies_valid: None,
            require_vies_valid_for_reverse_charge: false,
            tax_status: CustomerTaxStatus::Exempt,
            exemption_reason: None,
            custom_tax_rates: vec![],
            billing_address: Address {
                country: Some(CountryCode::from_str("FR").expect("failed to parse country code")),
                region: None,
                city: None,
                line1: None,
                postal_code: None,
            },
            shipping_address: None,
        };

        let invoicing_entity_address = Address {
            country: Some(CountryCode::from_str("FR").expect("failed to parse country code")),
            region: None,
            city: None,
            line1: None,
            postal_code: None,
        };

        let line_items = vec![LineItemForTax {
            line_id: "item1".to_string(),
            amount: 10000,
            custom_taxes: vec![],
            tax_category: None,
        }];

        let result = engine
            .calculate_line_items_tax(
                "EUR".to_string(),
                customer,
                invoicing_entity_address,
                line_items,
                chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            )
            .await
            .unwrap();

        // Should be tax exempt
        assert_eq!(result.tax_amount, 0);

        // Check that breakdown contains tax exempt reason
        assert_eq!(result.breakdown.len(), 1);
        match &result.breakdown[0].details {
            TaxDetails::Exempt(VatExemptionReason::TaxExempt) => {
                // Success - tax exempt is preserved
            }
            _ => panic!("Expected tax exempt in breakdown"),
        }
    }

    #[tokio::test]
    async fn test_reverse_charge_status_surfaces_reason() {
        let engine = MeteroidTaxEngine;

        // Explicit ReverseCharge status with a free-text legal mention.
        let customer = CustomerForTax {
            vat_number: None,
            vat_number_format_valid: false,
            vat_number_vies_valid: None,
            require_vies_valid_for_reverse_charge: false,
            tax_status: CustomerTaxStatus::ReverseCharge,
            exemption_reason: Some("Article 196 - reverse charge".to_string()),
            custom_tax_rates: vec![],
            billing_address: test_address("FR", None),
            shipping_address: None,
        };

        let result = engine
            .calculate_line_items_tax(
                "EUR".to_string(),
                customer,
                test_address("FR", None),
                vec![LineItemForTax {
                    line_id: "item1".to_string(),
                    amount: 10000,
                    custom_taxes: vec![],
                    tax_category: None,
                }],
                chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(result.tax_amount, 0);
        assert_eq!(
            result.exemption_reason.as_deref(),
            Some("Article 196 - reverse charge")
        );
        assert_eq!(result.breakdown.len(), 1);
        match &result.breakdown[0].details {
            TaxDetails::Exempt(VatExemptionReason::ReverseCharge) => {}
            _ => panic!("Expected reverse charge exemption in breakdown"),
        }
    }

    mod destination_matching {
        use super::*;
        use crate::ManualTaxEngine;

        fn customer_with_addresses(billing: Address, shipping: Option<Address>) -> CustomerForTax {
            CustomerForTax {
                vat_number: None,
                vat_number_format_valid: false,
                vat_number_vies_valid: None,
                require_vies_valid_for_reverse_charge: false,
                tax_status: CustomerTaxStatus::Taxable,
                exemption_reason: None,
                custom_tax_rates: vec![],
                billing_address: billing,
                shipping_address: shipping,
            }
        }

        // Override rule table keyed by destination country: DE=19%, FR=20%.
        fn country_rate_override() -> TaxRate {
            TaxRate {
                reference: "sales_tax".to_string(),
                name: "Sales Tax".to_string(),
                tax_rules: vec![
                    TaxRateRule {
                        country: CountryCode::parse_as_opt("DE"),
                        region: None,
                        rate: dec!(0.19),
                    },
                    TaxRateRule {
                        country: CountryCode::parse_as_opt("FR"),
                        region: None,
                        rate: dec!(0.20),
                    },
                ],
            }
        }

        #[tokio::test]
        async fn override_matches_customer_country_not_seller() {
            // Seller in FR, customer billing in DE: the DE rule (19%) must win,
            // proving the rule table keys on the customer's destination, not the
            // seller's own FR address (which would have selected 20%).
            let engine = ManualTaxEngine;
            let customer = customer_with_addresses(test_address("DE", None), None);
            let line_items = vec![test_line_item(
                "item1",
                10000,
                vec![country_rate_override()],
            )];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    test_address("FR", None),
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(result.tax_amount, 1900); // 10000 * 0.19 (DE), not 0.20 (FR)
        }

        #[tokio::test]
        async fn override_uses_shipping_over_billing() {
            // Ship-to (FR) is the place of supply and overrides the billing
            // country (DE) for destination matching.
            let engine = ManualTaxEngine;
            let customer =
                customer_with_addresses(test_address("DE", None), Some(test_address("FR", None)));
            let line_items = vec![test_line_item(
                "item1",
                10000,
                vec![country_rate_override()],
            )];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    test_address("FR", None),
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(result.tax_amount, 2000); // shipping FR -> 0.20, not billing DE 0.19
        }

        #[tokio::test]
        async fn override_region_matches_customer_destination() {
            // Most-specific-wins on the customer's destination region.
            let engine = ManualTaxEngine;
            let override_tax = TaxRate {
                reference: "us_sales_tax".to_string(),
                name: "US Sales Tax".to_string(),
                tax_rules: vec![
                    TaxRateRule {
                        country: CountryCode::parse_as_opt("US"),
                        region: None,
                        rate: dec!(0.05),
                    },
                    TaxRateRule {
                        country: CountryCode::parse_as_opt("US"),
                        region: Some("CA".to_string()),
                        rate: dec!(0.0725),
                    },
                ],
            };
            // Seller with a different country/region entirely.
            let customer = customer_with_addresses(test_address("US", Some("CA")), None);
            let line_items = vec![test_line_item("item1", 10000, vec![override_tax])];

            let result = engine
                .calculate_line_items_tax(
                    "USD".to_string(),
                    customer,
                    test_address("FR", None),
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(result.tax_amount, 725); // US/CA destination -> 7.25%
        }

        #[tokio::test]
        async fn override_everywhere_fallback_applies_off_table() {
            // A rule with no country is an "everywhere" fallback: a customer whose
            // destination is not in the explicit table still gets the fallback rate.
            let engine = ManualTaxEngine;
            let override_tax = TaxRate {
                reference: "levy".to_string(),
                name: "Levy".to_string(),
                tax_rules: vec![
                    TaxRateRule {
                        country: None,
                        region: None,
                        rate: dec!(0.05),
                    },
                    TaxRateRule {
                        country: CountryCode::parse_as_opt("DE"),
                        region: None,
                        rate: dec!(0.19),
                    },
                ],
            };
            let customer = customer_with_addresses(test_address("FR", None), None);
            let line_items = vec![test_line_item("item1", 10000, vec![override_tax])];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    test_address("FR", None),
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(result.tax_amount, 500); // FR not in table -> 5% everywhere fallback
        }

        #[tokio::test]
        async fn regeneration_is_deterministic() {
            // Re-generating an invoice with identical inputs yields identical tax:
            // the destination-matched result must be stable across runs.
            let engine = ManualTaxEngine;
            let make_customer = || {
                customer_with_addresses(
                    test_address("DE", None),
                    Some(test_address("US", Some("CA"))),
                )
            };
            let override_tax = TaxRate {
                reference: "sales_tax".to_string(),
                name: "Sales Tax".to_string(),
                tax_rules: vec![
                    TaxRateRule {
                        country: CountryCode::parse_as_opt("US"),
                        region: None,
                        rate: dec!(0.05),
                    },
                    TaxRateRule {
                        country: CountryCode::parse_as_opt("US"),
                        region: Some("CA".to_string()),
                        rate: dec!(0.0725),
                    },
                    TaxRateRule {
                        country: CountryCode::parse_as_opt("DE"),
                        region: None,
                        rate: dec!(0.19),
                    },
                ],
            };

            let run = || {
                let engine = ManualTaxEngine;
                let items = vec![test_line_item("item1", 33333, vec![override_tax.clone()])];
                async move {
                    engine
                        .calculate_line_items_tax(
                            "USD".to_string(),
                            make_customer(),
                            test_address("FR", None),
                            items,
                            chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                        )
                        .await
                        .unwrap()
                }
            };

            let first = run().await;
            let second = run().await;

            // Ship-to US/CA -> 7.25% on 33333 = 2416.6 -> 2417 (half-away-from-zero).
            assert_eq!(first.tax_amount, 2417);
            assert_eq!(first.tax_amount, second.tax_amount);
            assert_eq!(first.total_amount_after_tax, second.total_amount_after_tax);
            assert_eq!(first.breakdown.len(), second.breakdown.len());
        }
    }

    mod precedence_ladder {
        // Finding C2: exemption > reverse charge > override (replace) > engine rate.
        use super::*;

        fn override_20pct() -> TaxRate {
            TaxRate {
                reference: "override".to_string(),
                name: "Override VAT".to_string(),
                tax_rules: vec![TaxRateRule {
                    country: None, // everywhere fallback, always matches
                    region: None,
                    rate: dec!(0.20),
                }],
            }
        }

        #[tokio::test]
        async fn exemption_outranks_override() {
            // An exempt customer stays untaxed even when the line carries an override.
            let line_items = vec![test_line_item("item1", 10000, vec![override_20pct()])];
            let seller = test_address("FR", None);

            let result =
                shared::compute_tax(CustomerTax::Exempt, seller.clone(), seller, line_items)
                    .await
                    .unwrap();

            match &result[0].tax_details {
                TaxDetails::Exempt(VatExemptionReason::TaxExempt) => {}
                other => panic!("expected exemption to beat override, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn reverse_charge_outranks_override() {
            // Reverse charge shifts liability to the buyer; an override cannot
            // re-impose seller-side tax.
            let line_items = vec![test_line_item("item1", 10000, vec![override_20pct()])];
            let seller = test_address("FR", None);

            let result = shared::compute_tax(
                CustomerTax::ReverseCharge,
                seller.clone(),
                seller,
                line_items,
            )
            .await
            .unwrap();

            match &result[0].tax_details {
                TaxDetails::Exempt(VatExemptionReason::ReverseCharge) => {}
                other => panic!("expected reverse charge to beat override, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn override_replaces_engine_rate() {
            // With no exemption/reverse charge, the override (20%) fully replaces
            // the engine rate (10%) rather than stacking on top of it.
            let line_items = vec![test_line_item("item1", 10000, vec![override_20pct()])];
            let seller = test_address("FR", None);

            let result =
                shared::compute_tax(flat_rate(dec!(0.10)), seller.clone(), seller, line_items)
                    .await
                    .unwrap();

            match &result[0].tax_details {
                TaxDetails::Tax {
                    tax_amount,
                    tax_rate,
                    tax_name,
                    ..
                } => {
                    assert_eq!(*tax_amount, 2000); // 10000 * 0.20 only, not 0.30
                    assert_eq!(*tax_rate, dec!(0.20));
                    assert_eq!(tax_name, "Override VAT");
                }
                other => panic!("expected override to replace engine rate, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn engine_rate_applies_without_override() {
            // Bottom rung: no exemption, no override -> engine rate applies.
            let line_items = vec![test_line_item("item1", 10000, vec![])];
            let seller = test_address("FR", None);

            let result =
                shared::compute_tax(flat_rate(dec!(0.10)), seller.clone(), seller, line_items)
                    .await
                    .unwrap();

            match &result[0].tax_details {
                TaxDetails::MultipleTaxes {
                    total_tax_amount, ..
                } => assert_eq!(*total_tax_amount, 1000),
                other => panic!("expected engine rate, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn test_exempt_status_surfaces_reason() {
        let engine = MeteroidTaxEngine;

        let customer = CustomerForTax {
            vat_number: None,
            vat_number_format_valid: false,
            vat_number_vies_valid: None,
            require_vies_valid_for_reverse_charge: false,
            tax_status: CustomerTaxStatus::Exempt,
            exemption_reason: Some("Registered charity".to_string()),
            custom_tax_rates: vec![],
            billing_address: test_address("FR", None),
            shipping_address: None,
        };

        let result = engine
            .calculate_line_items_tax(
                "EUR".to_string(),
                customer,
                test_address("FR", None),
                vec![LineItemForTax {
                    line_id: "item1".to_string(),
                    amount: 10000,
                    custom_taxes: vec![],
                    tax_category: None,
                }],
                chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(result.tax_amount, 0);
        assert_eq!(
            result.exemption_reason.as_deref(),
            Some("Registered charity")
        );
        match &result.breakdown[0].details {
            TaxDetails::Exempt(VatExemptionReason::TaxExempt) => {}
            _ => panic!("Expected tax exempt in breakdown"),
        }
    }

    // Rate class is engine-internal: every category resolves to the destination's
    // STANDARD rate. Overrides (custom rates) are the only way to replace it.
    mod standard_rate_resolution {
        use super::*;

        #[tokio::test]
        async fn every_line_gets_the_destination_standard_rate() {
            // FR domestic B2C sale: every line takes the FR standard rate (20%),
            // regardless of what is being sold.
            let engine = MeteroidTaxEngine;
            let customer = test_customer(None, false, vec![], "FR");
            let line_items = vec![
                test_line_item("saas", 10000, vec![]),
                test_line_item("ebook", 10000, vec![]),
            ];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    test_address("FR", None),
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            // Both lines at 20% -> 4000 total, aggregated into one breakdown line.
            assert_eq!(result.tax_amount, 4000);
            assert_eq!(result.total_amount_after_tax, 24000);
            assert_eq!(result.breakdown.len(), 1);
            match &result.breakdown[0].details {
                TaxDetails::Tax { tax_rate, .. } => {
                    assert_eq!(*tax_rate, rust_decimal::Decimal::from_str("0.2").unwrap())
                }
                other => panic!("expected standard VAT, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn override_replaces_the_standard_rate() {
            // A custom rate targeting the customer's destination country replaces
            // the engine's computed standard rate for that line.
            let engine = MeteroidTaxEngine;
            let customer = test_customer(None, false, vec![], "FR");
            let override_tax = TaxRate {
                reference: "reduced_ebook".to_string(),
                name: "FR reduced".to_string(),
                tax_rules: vec![TaxRateRule {
                    country: CountryCode::parse_as_opt("FR"),
                    region: None,
                    rate: dec!(0.055),
                }],
            };
            let line_items = vec![
                test_line_item("saas", 10000, vec![]),
                test_line_item("ebook", 10000, vec![override_tax]),
            ];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    test_address("FR", None),
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            // SaaS at 20% (2000) + e-book override at 5.5% (550).
            assert_eq!(result.tax_amount, 2550);
            assert_eq!(result.total_amount_after_tax, 22550);
        }

        #[tokio::test]
        async fn nontaxable_category_stays_exempt() {
            // The `nontaxable` category is special-cased to exempt even in a plain
            // taxable VAT scenario.
            let engine = MeteroidTaxEngine;
            let customer = test_customer(None, false, vec![], "FR");
            let line_items = vec![
                LineItemForTax {
                    tax_category: Some(NONTAXABLE_CATEGORY_KEY.to_string()),
                    ..test_line_item("exempt", 10000, vec![])
                },
                test_line_item("taxable", 10000, vec![]),
            ];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    test_address("FR", None),
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            // Only the taxable line is taxed (20% of 10000).
            assert_eq!(result.tax_amount, 2000);
        }

        #[tokio::test]
        async fn reverse_charge_ignores_category() {
            // Cross-border B2B is reverse charge regardless of what is sold.
            let engine = MeteroidTaxEngine;
            let customer = test_customer(Some("DE123456789".to_string()), false, vec![], "DE");
            let line_items = vec![test_line_item("ebook", 10000, vec![])];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    test_address("FR", None),
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(result.tax_amount, 0);
            match &result.breakdown[0].details {
                TaxDetails::Exempt(VatExemptionReason::ReverseCharge) => {}
                other => panic!("expected reverse charge, got {other:?}"),
            }
        }
    }

    // Finding W4: credit/negative lines must reduce tax symmetrically.
    mod signed_credit_lines {
        use super::*;
        use crate::ManualTaxEngine;

        #[tokio::test]
        async fn downgrade_credit_line_nets_tax() {
            // FR domestic B2C (standard 20%): a +10000 charge and a -4000 downgrade
            // proration credit at the same rate net to 6000 taxable and 1200 tax
            // (2000 charge tax minus 800 credit tax).
            let engine = MeteroidTaxEngine;
            let customer = test_customer(None, false, vec![], "FR");
            let line_items = vec![
                test_line_item("charge", 10000, vec![]),
                test_line_item("credit", -4000, vec![]),
            ];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    test_address("FR", None),
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(result.tax_amount, 1200);
            assert_eq!(result.total_amount_after_tax, 7200);

            let charge = result
                .line_items
                .iter()
                .find(|l| l.line_id == "charge")
                .unwrap();
            let credit = result
                .line_items
                .iter()
                .find(|l| l.line_id == "credit")
                .unwrap();
            match &charge.tax_details {
                TaxDetails::Tax { tax_amount, .. } => assert_eq!(*tax_amount, 2000),
                other => panic!("expected tax on charge, got {other:?}"),
            }
            match &credit.tax_details {
                TaxDetails::Tax { tax_amount, .. } => assert_eq!(*tax_amount, -800),
                other => panic!("expected negative tax on credit line, got {other:?}"),
            }

            // Same rate class aggregates into one netted breakdown line.
            assert_eq!(result.breakdown.len(), 1);
            match &result.breakdown[0].details {
                TaxDetails::Tax { tax_amount, .. } => assert_eq!(*tax_amount, 1200),
                other => panic!("expected netted breakdown, got {other:?}"),
            }
            assert_eq!(result.breakdown[0].taxable_amount, 6000);
        }

        #[tokio::test]
        async fn override_credit_line_nets_tax() {
            // A merchant override (destination-matched) applied to both a charge and
            // a credit line: the credit reduces the override tax symmetrically.
            let engine = ManualTaxEngine;
            let customer = test_customer(None, false, vec![], "FR");
            let override_tax = TaxRate {
                reference: "sales_tax".to_string(),
                name: "Sales Tax".to_string(),
                tax_rules: vec![TaxRateRule {
                    country: CountryCode::parse_as_opt("FR"),
                    region: None,
                    rate: dec!(0.20),
                }],
            };
            let line_items = vec![
                test_line_item("charge", 10000, vec![override_tax.clone()]),
                test_line_item("credit", -4000, vec![override_tax]),
            ];

            let result = engine
                .calculate_line_items_tax(
                    "EUR".to_string(),
                    customer,
                    test_address("FR", None),
                    line_items,
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(result.tax_amount, 1200); // 2000 - 800
            assert_eq!(result.total_amount_after_tax, 7200);
        }
    }
}
