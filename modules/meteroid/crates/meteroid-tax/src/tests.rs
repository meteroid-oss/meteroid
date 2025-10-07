#[cfg(test)]
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

    fn test_line_item(id: &str, amount: u64, custom_tax: Option<CustomTax>) -> LineItemForTax {
        LineItemForTax {
            line_id: id.to_string(),
            amount,
            custom_tax,
        }
    }

    #[tokio::test]
    async fn test_tax_exempt_customer() {
        // Customer is tax exempt
        let customer_tax = CustomerTax::Exempt;
        let invoicing_entity_address = test_address("US", Some("CA"));
        let line_items = vec![
            test_line_item("item1", 10000, None),
            test_line_item("item2", 5000, None),
        ];

        let result = shared::compute_tax(customer_tax, invoicing_entity_address, line_items)
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
        let customer_tax = CustomerTax::CustomTaxRate(dec!(0.15));
        let invoicing_entity_address = test_address("US", Some("CA"));
        let line_items = vec![
            test_line_item("item1", 10000, None),
            test_line_item("item2", 5000, None),
        ];

        let result = shared::compute_tax(customer_tax, invoicing_entity_address, line_items)
            .await
            .unwrap();

        assert_eq!(result.len(), 2);

        // Check first item: 10000 * 0.15 = 1500
        match &result[0].tax_details {
            TaxDetails::Tax {
                tax_amount,
                tax_rate,
                ..
            } => {
                assert_eq!(*tax_amount, 1500);
                assert_eq!(*tax_rate, dec!(0.15));
            }
            _ => panic!("Expected tax details"),
        }

        // Check second item: 5000 * 0.15 = 750
        match &result[1].tax_details {
            TaxDetails::Tax {
                tax_amount,
                tax_rate,
                ..
            } => {
                assert_eq!(*tax_amount, 750);
                assert_eq!(*tax_rate, dec!(0.15));
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
        let customer_tax = CustomerTax::CustomTaxRate(dec!(0.10));
        let invoicing_entity_address = test_address("FR", None);

        let custom_tax = CustomTax {
            reference: "custom_vat".to_string(),
            name: "French VAT".to_string(),
            tax_rules: vec![TaxRule {
                country: Some(CountryCode::from_str("FR").expect("failed to parse country code")),
                region: None,
                rate: dec!(0.20),
            }],
        };

        let line_items = vec![
            test_line_item("item1", 10000, Some(custom_tax.clone())),
            test_line_item("item2", 5000, None), // This will use customer tax
        ];

        let result = shared::compute_tax(customer_tax, invoicing_entity_address, line_items)
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
            TaxDetails::Tax {
                tax_amount,
                tax_rate,
                ..
            } => {
                assert_eq!(*tax_amount, 500); // 5000 * 0.10
                assert_eq!(*tax_rate, dec!(0.10));
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

        let custom_tax = CustomTax {
            reference: "sales_tax".to_string(),
            name: "Sales Tax".to_string(),
            tax_rules: vec![
                // Generic US rate
                TaxRule {
                    country: Some(
                        CountryCode::from_str("US").expect("failed to parse country code"),
                    ),
                    region: None,
                    rate: dec!(0.05),
                },
                // Specific California rate (should be selected)
                TaxRule {
                    country: Some(
                        CountryCode::from_str("US").expect("failed to parse country code"),
                    ),
                    region: Some("CA".to_string()),
                    rate: dec!(0.0725),
                },
                // Specific New York rate (should not be selected)
                TaxRule {
                    country: Some(
                        CountryCode::from_str("US").expect("failed to parse country code"),
                    ),
                    region: Some("NY".to_string()),
                    rate: dec!(0.08),
                },
            ],
        };

        let line_items = vec![test_line_item("item1", 10000, Some(custom_tax))];

        let result = shared::compute_tax(customer_tax, invoicing_entity_address, line_items)
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

        let custom_tax_1 = CustomTax {
            reference: "vat_standard".to_string(),
            name: "Standard VAT".to_string(),
            tax_rules: vec![TaxRule {
                country: Some(CountryCode::from_str("US").expect("failed to parse country code")),
                region: Some("CA".to_string()),
                rate: dec!(0.20),
            }],
        };

        let custom_tax_2 = CustomTax {
            reference: "vat_reduced".to_string(),
            name: "Reduced VAT".to_string(),
            tax_rules: vec![TaxRule {
                country: Some(CountryCode::from_str("US").expect("failed to parse country code")),
                region: Some("CA".to_string()),
                rate: dec!(0.05),
            }],
        };

        let line_items = vec![
            test_line_item("item1", 10000, Some(custom_tax_1.clone())),
            test_line_item("item2", 5000, Some(custom_tax_1.clone())),
            test_line_item("item3", 8000, Some(custom_tax_2)),
        ];

        let result = shared::compute_tax(customer_tax, invoicing_entity_address, line_items)
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
        let customer_tax = CustomerTax::CustomTaxRate(dec!(0.21)); // 21% tax
        let invoicing_entity_address = test_address("US", None);

        // 999 * 0.21 = 209.79, should round to 210
        let line_items = vec![test_line_item("item1", 999, None)];

        let result = shared::compute_tax(
            customer_tax.clone(),
            invoicing_entity_address.clone(),
            line_items,
        )
        .await
        .unwrap();

        match &result[0].tax_details {
            TaxDetails::Tax { tax_amount, .. } => {
                assert_eq!(*tax_amount, 210); // Rounded up from 209.79
            }
            _ => panic!("Expected tax details"),
        }

        // Test rounding down: 997 * 0.21 = 209.37, should round to 209
        let line_items = vec![test_line_item("item2", 997, None)];

        let result = shared::compute_tax(customer_tax, invoicing_entity_address, line_items)
            .await
            .unwrap();

        match &result[0].tax_details {
            TaxDetails::Tax { tax_amount, .. } => {
                assert_eq!(*tax_amount, 209); // Rounded down from 209.37
            }
            _ => panic!("Expected tax details"),
        }
    }

    #[tokio::test]
    async fn test_zero_amount_line_items() {
        let customer_tax = CustomerTax::CustomTaxRate(dec!(0.20));
        let invoicing_entity_address = test_address("US", None);
        let line_items = vec![
            test_line_item("item1", 0, None),
            test_line_item("item2", 1000, None),
        ];

        let result = shared::compute_tax(customer_tax, invoicing_entity_address, line_items)
            .await
            .unwrap();

        // Zero amount should result in zero tax
        match &result[0].tax_details {
            TaxDetails::Tax { tax_amount, .. } => {
                assert_eq!(*tax_amount, 0);
            }
            _ => panic!("Expected tax details"),
        }

        // Non-zero amount should have tax
        match &result[1].tax_details {
            TaxDetails::Tax { tax_amount, .. } => {
                assert_eq!(*tax_amount, 200); // 1000 * 0.20
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
        let line_items = vec![test_line_item("item1", 10000, None)];

        let result = shared::compute_tax(customer_tax, invoicing_entity_address, line_items)
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

    fn test_customer(
        vat_number: Option<String>,
        tax_exempt: bool,
        custom_tax_rate: Option<rust_decimal::Decimal>,
        country: &str,
    ) -> CustomerForTax {
        CustomerForTax {
            vat_number: vat_number.clone(),
            vat_number_format_valid: vat_number.is_some(),
            tax_exempt,
            custom_tax_rate,
            billing_address: test_address(country, None),
        }
    }

    mod tax_engines {
        use super::*;
        use crate::{ManualTaxEngine, MeteroidTaxEngine, TaxEngine};

        #[tokio::test]
        async fn test_meteroid_tax_engine_eu_vat() {
            let engine = MeteroidTaxEngine;

            // Test B2C transaction within EU - should apply VAT
            let customer = test_customer(None, false, None, "DE"); // German B2C customer
            let invoicing_entity_address = test_address("FR", None); // French company
            let line_items = vec![test_line_item("item1", 10000, None)];

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
        async fn test_meteroid_tax_engine_b2b_reverse_charge() {
            let engine = MeteroidTaxEngine;

            // Test B2B transaction between different EU countries - should be reverse charge
            let customer = test_customer(Some("DE123456789".to_string()), false, None, "DE");
            let invoicing_entity_address = test_address("FR", None);
            let line_items = vec![test_line_item("item1", 10000, None)];

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
            let customer = test_customer(None, false, Some(rust_decimal_macros::dec!(0.18)), "US");
            let invoicing_entity_address = test_address("US", None);
            let line_items = vec![test_line_item("item1", 10000, None)];

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
            let customer = test_customer(None, true, None, "US");
            let invoicing_entity_address = test_address("US", None);
            let line_items = vec![test_line_item("item1", 10000, None)];

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
            let customer = test_customer(None, false, None, "US");
            let invoicing_entity_address = test_address("US", None);
            let line_items = vec![test_line_item("item1", 10000, None)];

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
            let mut customer = test_customer(Some("INVALID_VAT".to_string()), false, None, "DE");
            customer.vat_number_format_valid = false;

            let invoicing_entity_address = test_address("FR", None);
            let line_items = vec![test_line_item("item1", 10000, None)];

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
            let customer = test_customer(Some("".to_string()), false, None, "DE");
            let invoicing_entity_address = test_address("FR", None);
            let line_items = vec![test_line_item("item1", 10000, None)];

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
            let mut customer = test_customer(None, false, None, "");
            customer.billing_address.country = None;

            let invoicing_entity_address = test_address("FR", None);
            let line_items = vec![test_line_item("item1", 10000, None)];

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

            let customer = test_customer(None, false, None, "DE");
            let mut invoicing_entity_address = test_address("", None);
            invoicing_entity_address.country = None;

            let line_items = vec![test_line_item("item1", 10000, None)];

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
            tax_exempt: false,
            custom_tax_rate: None,
            billing_address: Address {
                country: Some(CountryCode::from_str("DE").expect("failed to parse country code")),
                region: None,
                city: None,
                line1: None,
                postal_code: None,
            },
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
            custom_tax: None,
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
            tax_exempt: true,
            custom_tax_rate: None,
            billing_address: Address {
                country: Some(CountryCode::from_str("FR").expect("failed to parse country code")),
                region: None,
                city: None,
                line1: None,
                postal_code: None,
            },
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
            custom_tax: None,
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
}
