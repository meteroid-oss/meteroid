#[cfg(test)]
mod tests {
    use crate::{
        Region, TaxDatabase, TaxScenario, TaxType, TradeAgreementOverride, TransactionType, VatRate,
    };
    use rust_decimal_macros::dec;

    fn setup() -> TaxDatabase {
        // Use the compiled-in data (`include_str!`) so the tests don't depend on
        // the process working directory.
        TaxDatabase::new().expect("Tax database should load")
    }

    #[test]
    fn test_german_vat_calculation() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("DE".to_string(), None).expect("Valid German region"),
            Region::new("DE".to_string(), None).expect("Valid German region"),
            TransactionType::B2C,
        );
        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 19.0); // Germany's actual VAT rate

        let rates = scenario
            .get_rates(100.0, &db)
            .expect("Rates should be found");
        assert_eq!(rates.len(), 1);
        assert_eq!(rates[0].rate, 0.19);
        assert_eq!(rates[0].tax_type, TaxType::VAT(VatRate::Standard));
        assert!(!rates[0].compound);
    }

    #[test]
    fn test_canadian_gst_bc_pst_below_threshold() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("CA".to_string(), Some("CA-BC".to_string()))
                .expect("Valid Canadian BC region"),
            Region::new("CA".to_string(), Some("CA-BC".to_string()))
                .expect("Valid Canadian BC region"),
            TransactionType::B2C,
        );

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 0.0); // Combined GST (5%) + PST (7%) for British Columbia
    }

    #[test]
    fn test_canadian_gst_bc_pst_ignore_threshold() {
        let db = setup();
        let mut scenario = TaxScenario::new(
            Region::new("CA".to_string(), Some("CA-BC".to_string()))
                .expect("Valid Canadian BC region"),
            Region::new("CA".to_string(), Some("CA-BC".to_string()))
                .expect("Valid Canadian BC region"),
            TransactionType::B2C,
        );
        scenario.ignore_threshold = true;

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 12.35); // Combined GST (5%) + PST (7%) for British Columbia
    }

    #[test]
    fn test_canadian_gst_bc_pst_above_threshold() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("CA".to_string(), Some("CA-BC".to_string()))
                .expect("Valid Canadian BC region"),
            Region::new("CA".to_string(), Some("CA-BC".to_string()))
                .expect("Valid Canadian BC region"),
            TransactionType::B2C,
        );

        let tax = scenario
            .calculate_tax(100000.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 12350.0); // Combined GST (5%) + PST (7%) for British Columbia
    }

    #[test]
    fn test_eu_cross_border_b2b() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("DE".to_string(), None).expect("Valid German region"),
            Region::new("FR".to_string(), None).expect("Valid French region"),
            TransactionType::B2B,
        );

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 0.0); // EU reverse charge mechanism
    }

    #[test]
    fn test_eu_cross_border_b2c_digital() {
        let db = setup();
        let mut scenario = TaxScenario::new(
            Region::new("DE".to_string(), None).expect("Valid German region"),
            Region::new("FR".to_string(), None).expect("Valid French region"),
            TransactionType::B2B,
        );
        scenario.is_digital_product_or_service = true;

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 0.0); // EU reverse charge mechanism
    }

    #[test]
    fn test_eu_cross_border_b2c() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("DE".to_string(), None).expect("Valid German region"),
            Region::new("FR".to_string(), None).expect("Valid French region"),
            TransactionType::B2C,
        );

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 19.0); // EU reverse charge mechanism
    }

    #[test]
    fn test_french_reduced_vat() {
        let db = setup();
        let mut scenario = TaxScenario::new(
            Region::new("FR".to_string(), None).expect("Valid French region"),
            Region::new("FR".to_string(), None).expect("Valid French region"),
            TransactionType::B2C,
        );
        scenario.vat_rate = Some(VatRate::ReducedAlt);

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 5.5); // France's actual reduced VAT rate
    }

    #[test]
    fn test_german_domestic_b2b() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("DE".to_string(), None).expect("Valid German region"),
            Region::new("DE".to_string(), None).expect("Valid German region"),
            TransactionType::B2B,
        );

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 19.0); // German domestic B2B VAT rate
    }

    #[test]
    fn test_germany_thailand_b2b() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("DE".to_string(), None).expect("Valid German region"),
            Region::new("TH".to_string(), None).expect("Valid Thai region"),
            TransactionType::B2B,
        );

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 0.0); // Export from EU is zero-rated
    }

    #[test]
    fn test_germany_thailand_b2c() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("DE".to_string(), None).expect("Valid German region"),
            Region::new("TH".to_string(), None).expect("Valid Thai region"),
            TransactionType::B2C,
        );

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 0.0); // Export from EU to non-EU country is zero-rated for B2C too
    }

    #[test]
    fn test_us_interstate_b2c_below_threshold() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("US".to_string(), Some("US-CA".to_string())).expect("Valid US-CA region"),
            Region::new("US".to_string(), Some("US-WA".to_string())).expect("Valid US-WA region"),
            TransactionType::B2C,
        );

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 0.0); // Washington state sales tax rate for remote sellers
    }

    #[test]
    fn test_us_interstate_b2c_ignore_threshold() {
        let db = setup();
        let mut scenario = TaxScenario::new(
            Region::new("US".to_string(), Some("US-CA".to_string())).expect("Valid US-CA region"),
            Region::new("US".to_string(), Some("US-WA".to_string())).expect("Valid US-WA region"),
            TransactionType::B2C,
        );
        scenario.ignore_threshold = true;

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 6.5); // Washington state sales tax rate for remote sellers
    }

    #[test]
    fn test_us_interstate_b2c_above_threshold() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("US".to_string(), Some("US-CA".to_string())).expect("Valid US-CA region"),
            Region::new("US".to_string(), Some("US-WA".to_string())).expect("Valid US-WA region"),
            TransactionType::B2C,
        );

        let tax = scenario
            .calculate_tax(100000.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 6500.0); // Washington state sales tax rate for remote sellers
    }

    #[test]
    fn test_us_interstate_b2b() {
        let db = setup();
        let mut scenario = TaxScenario::new(
            Region::new("US".to_string(), Some("US-TX".to_string())).expect("Valid US-TX region"),
            Region::new("US".to_string(), Some("US-WA".to_string())).expect("Valid US-WA region"),
            TransactionType::B2B,
        );
        scenario.has_resale_certificate = true;

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 0.0);
    }

    #[test]
    fn test_us_interstate_b2b_reseller() {
        let db = setup();
        let mut scenario = TaxScenario::new(
            Region::new("US".to_string(), Some("US-WA".to_string())).expect("Valid US-WA region"),
            Region::new("US".to_string(), Some("US-TX".to_string())).expect("Valid US-TX region"),
            TransactionType::B2B,
        );
        scenario.has_resale_certificate = true;

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 0.0);
    }

    #[test]
    fn test_gcc_cross_border_b2b() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("AE".to_string(), None).expect("Valid UAE region"),
            Region::new("QA".to_string(), None).expect("Valid Qatar region"),
            TransactionType::B2B,
        );

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 0.0); // GCC countries have no VAT
    }

    #[test]
    fn test_gcc_cross_border_b2c() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("AE".to_string(), None).expect("Valid UAE region"),
            Region::new("QA".to_string(), None).expect("Valid Qatar region"),
            TransactionType::B2C,
        );

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 5.0); // GCC countries have no VAT
    }

    #[test]
    fn test_gcc_cross_border_b2c_manual() {
        let db = setup();
        let scenario = TaxScenario {
            source_region: Region::new("AE".to_string(), None).expect("Valid UAE region"),
            destination_region: Region::new("QA".to_string(), None).expect("Valid Qatar region"),
            transaction_type: TransactionType::B2C,
            trade_agreement_override: None,
            is_digital_product_or_service: false,
            has_resale_certificate: false,
            ignore_threshold: false,
            vat_rate: None,
        };

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 5.0); // GCC countries have no VAT
    }

    #[test]
    fn test_gcc_cross_border_b2c_manual_no_agreement() {
        let db = setup();
        let scenario = TaxScenario {
            source_region: Region::new("AE".to_string(), None).expect("Valid UAE region"),
            destination_region: Region::new("QA".to_string(), None).expect("Valid Qatar region"),
            transaction_type: TransactionType::B2C,
            trade_agreement_override: Some(TradeAgreementOverride::NoAgreement),
            is_digital_product_or_service: false,
            has_resale_certificate: false,
            ignore_threshold: false,
            vat_rate: None,
        };

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 0.0); // GCC countries have no VAT
    }

    #[test]
    fn test_canadian_quebec_gst_qst() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("CA".to_string(), Some("CA-QC".to_string()))
                .expect("Valid Canadian QC region"),
            Region::new("CA".to_string(), Some("CA-QC".to_string()))
                .expect("Valid Canadian QC region"),
            TransactionType::B2C,
        );

        let rates = scenario
            .get_rates(100.0, &db)
            .expect("Rates should be found");
        assert_eq!(rates.len(), 2); // Should have both GST and QST

        let gst_rate = rates
            .iter()
            .find(|r| matches!(r.tax_type, TaxType::GST))
            .expect("Should have GST");
        assert_eq!(gst_rate.rate, 0.05); // 5% GST

        let qst_rate = rates
            .iter()
            .find(|r| matches!(r.tax_type, TaxType::QST))
            .expect("Should have QST");
        assert_eq!(qst_rate.rate, 0.09975); // 9.975% QST
        assert!(qst_rate.compound); // QST should compound on GST
    }

    #[test]
    fn test_canadian_nova_scotia_hst() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("CA".to_string(), Some("CA-NS".to_string()))
                .expect("Valid Canadian NS region"),
            Region::new("CA".to_string(), Some("CA-NS".to_string()))
                .expect("Valid Canadian NS region"),
            TransactionType::B2C,
        );

        let rates = scenario
            .get_rates(100.0, &db)
            .expect("Rates should be found");
        assert_eq!(rates.len(), 1); // Should only have HST
        assert_eq!(rates[0].tax_type, TaxType::HST);
        assert_eq!(rates[0].rate, 0.10); // Nova Scotia HST rate 10%
        assert!(!rates[0].compound); // HST should not compound
    }

    #[test]
    fn test_eu_zero_rate() {
        let db = setup();
        let mut scenario = TaxScenario::new(
            Region::new("IE".to_string(), None).expect("Valid Irish region"),
            Region::new("IE".to_string(), None).expect("Valid Irish region"),
            TransactionType::B2C,
        );
        scenario.vat_rate = Some(VatRate::Zero);

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 0.0); // Zero-rated goods in Ireland
    }

    #[test]
    fn test_multiple_tax_rates() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("CA".to_string(), Some("CA-BC".to_string()))
                .expect("Valid Canadian BC region"),
            Region::new("CA".to_string(), Some("CA-BC".to_string()))
                .expect("Valid Canadian BC region"),
            TransactionType::B2C,
        );

        let rates = scenario
            .get_rates(100000.0, &db)
            .expect("Rates should be found");
        assert_eq!(rates.len(), 2); // Should have both GST and PST
        assert!(rates.iter().any(|r| matches!(r.tax_type, TaxType::GST)));
        assert!(rates.iter().any(|r| matches!(r.tax_type, TaxType::PST)));
    }

    #[test]
    fn test_reverse_charge_vat() {
        let db = setup();
        let mut scenario = TaxScenario::new(
            Region::new("DE".to_string(), None).expect("Valid German region"),
            Region::new("FR".to_string(), None).expect("Valid French region"),
            TransactionType::B2B,
        );
        scenario.vat_rate = Some(VatRate::ReverseCharge);

        let rates = scenario
            .get_rates(100.0, &db)
            .expect("Rates should be found");
        assert_eq!(rates.len(), 1);
        assert_eq!(rates[0].rate, 0.0);
        assert!(matches!(
            rates[0].tax_type,
            TaxType::VAT(VatRate::ReverseCharge)
        ));
    }

    #[test]
    fn test_us_state_no_sales_tax() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("US".to_string(), Some("US-OR".to_string())).expect("Valid US-OR region"),
            Region::new("US".to_string(), Some("US-OR".to_string())).expect("Valid US-OR region"),
            TransactionType::B2C,
        );

        let tax = scenario
            .calculate_tax(100.0, &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, 0.0); // Oregon has no sales tax
    }

    #[test]
    fn test_us_states_get_rates() {
        let db = setup();
        let mut scenario = TaxScenario::new(
            Region::new("US".to_string(), Some("US-AS".to_string())).expect("Valid US-AK region"),
            Region::new("US".to_string(), Some("US-CA".to_string())).expect("Valid US-CA region"),
            TransactionType::B2C,
        );
        scenario.ignore_threshold = true;

        let rates = scenario.get_rates(1.0, &db).expect("Rates should be found");
        assert_eq!(rates.len(), 1);
        assert_eq!(rates[0].rate, 0.0825); // California sales tax rate
    }

    #[test]
    fn test_specific_trade_agreement() {
        // let db = setup();
        let mut scenario = TaxScenario::new(
            Region::new("DE".to_string(), None).expect("Valid German region"),
            Region::new("FR".to_string(), None).expect("Valid French region"),
            TransactionType::B2C,
        );
        scenario.trade_agreement_override =
            Some(TradeAgreementOverride::UseAgreement("EU".to_string()));

        // let tax = scenario.calculate_tax(100.0, &db).expect("Tax calculation should succeed");
        // Assert based on EU agreement rules
    }

    #[test]
    fn test_exempt_vat_rate() {
        let db = setup();
        let mut scenario = TaxScenario::new(
            Region::new("GB".to_string(), None).expect("Valid UK region"),
            Region::new("GB".to_string(), None).expect("Valid UK region"),
            TransactionType::B2C,
        );
        scenario.vat_rate = Some(VatRate::Exempt);

        let rates = scenario
            .get_rates(100.0, &db)
            .expect("Rates should be found");
        assert_eq!(rates.len(), 1);
        assert_eq!(rates[0].rate, 0.0);
        assert!(matches!(rates[0].tax_type, TaxType::VAT(VatRate::Exempt)));
    }

    #[test]
    fn test_decimal_german_vat_calculation() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("DE".to_string(), None).expect("Valid German region"),
            Region::new("DE".to_string(), None).expect("Valid German region"),
            TransactionType::B2C,
        );

        let tax = scenario
            .calculate_tax_decimal(dec!(100.00), &db)
            .expect("Tax calculation should succeed");
        assert_eq!(tax, dec!(19.00)); // Germany's VAT rate with precise decimal calculation
    }

    #[test]
    fn test_decimal_multiple_compound_calculations() {
        let db = setup();
        let scenario = TaxScenario::new(
            Region::new("CA".to_string(), Some("CA-QC".to_string()))
                .expect("Valid Canadian QC region"),
            Region::new("CA".to_string(), Some("CA-QC".to_string()))
                .expect("Valid Canadian QC region"),
            TransactionType::B2C,
        );

        let amount = dec!(7999999.99);
        let decimal_tax = scenario
            .calculate_tax_decimal(amount, &db)
            .expect("Decimal tax calculation should succeed");
        let float_tax = scenario
            .calculate_tax(7999999.99, &db)
            .expect("Float tax calculation should succeed");

        assert_eq!(decimal_tax, dec!(1237899.998452625));
        assert_eq!(float_tax, 1237900.0); // Should show difference from float calculation
    }

    #[test]
    fn load_included_db() {
        let _ = TaxDatabase::new();
    }
}
