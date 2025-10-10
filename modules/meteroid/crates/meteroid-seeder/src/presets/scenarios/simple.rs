use super::domain::{
    BillableMetric, Customer, FeeType, OrganizationDetails, Plan, PriceComponent, Scenario,
    Subscription,
};
use chrono::NaiveDate;
use meteroid_store::domain::{
    Address, BillingMetricAggregateEnum, BillingPeriodEnum, BillingType, PlanTypeEnum,
    ShippingAddress, TermRate, UsagePricingModel,
};

use common_domain::country::CountryCode;
use rust_decimal_macros::dec;

const ENTERPRISE_PLAN: &str = "Enterprise Platform Plan";
const STANDARD_20_PLAN: &str = "Standard 20 Plan";
const STANDARD_80_PLAN: &str = "Standard 80 Plan";

pub fn basic_scenario_1() -> Scenario {
    Scenario {
        name: "Basic 1".to_string(),
        metrics: vec![
            BillableMetric {
                code: "api_calls".to_string(),
                name: "API Calls".to_string(),
                description: Some("Number of API calls made".to_string()),
                aggregation_type: BillingMetricAggregateEnum::Sum,
                aggregation_key: None,
                unit_conversion_factor: None,
                unit_conversion_rounding: None,
                segmentation_matrix: None,
                usage_group_key: None,
            },
            BillableMetric {
                code: "storage_gb".to_string(),
                name: "Storage GB".to_string(),
                description: Some("Storage usage in gigabytes".to_string()),
                aggregation_type: BillingMetricAggregateEnum::Max,
                aggregation_key: None,
                unit_conversion_factor: None,
                unit_conversion_rounding: None,
                segmentation_matrix: None,
                usage_group_key: None,
            },
        ],
        plans: vec![
            Plan {
                name: ENTERPRISE_PLAN.to_string(),
                currency: "EUR".to_string(),
                plan_type: PlanTypeEnum::Standard,
                components: vec![
                    PriceComponent {
                        name: "Platform Integration Fee".to_string(),
                        fee: FeeType::OneTime {
                            unit_price: dec!(1500),
                            quantity: 1,
                        },
                    },
                    PriceComponent {
                        name: "Annual Platform License".to_string(),
                        fee: FeeType::ExtraRecurring {
                            unit_price: dec!(2400),
                            quantity: 1,
                            billing_type: BillingType::Advance,
                            cadence: BillingPeriodEnum::Annual,
                        },
                    },
                    PriceComponent {
                        name: "Monthly Subscription".to_string(),
                        fee: FeeType::Rate {
                            rates: vec![TermRate {
                                price: dec!(299),
                                term: BillingPeriodEnum::Monthly,
                            }],
                        },
                    },
                    PriceComponent {
                        name: "API Usage".to_string(),
                        fee: FeeType::Usage {
                            metric_code: "api_calls".to_string(),
                            pricing: UsagePricingModel::PerUnit {
                                rate: dec!(0.001),
                            },
                        },
                    },
                ],
            },
            Plan {
                name: STANDARD_20_PLAN.to_string(),
                currency: "EUR".to_string(),
                plan_type: PlanTypeEnum::Standard,
                components: vec![PriceComponent {
                    name: "Base Price".to_string(),
                    fee: FeeType::Rate {
                        rates: vec![TermRate {
                            price: dec!(20),
                            term: BillingPeriodEnum::Monthly,
                        }],
                    },
                }],
            },
            Plan {
                name: STANDARD_80_PLAN.to_string(),
                currency: "EUR".to_string(),
                plan_type: PlanTypeEnum::Standard,
                components: vec![PriceComponent {
                    name: "Base Price".to_string(),
                    fee: FeeType::Rate {
                        rates: vec![TermRate {
                            price: dec!(80),
                            term: BillingPeriodEnum::Monthly,
                        }],
                    },
                }],
            },
        ],
        customers: vec![
            Customer {
                name: "Cobalt Industries SAS".to_string(),
                email: "accounting@cobalt-industries.test".to_string(),
                currency: "EUR".to_string(),
                alias: Some("cobalt-industries".to_string()),
                phone: Some("+33 1 42 86 82 00".to_string()),
                vat_number: Some("FR32123456789".to_string()),
                billing_address: Some(Address {
                    line1: Some("42 Rue de la Innovation".to_string()),
                    line2: Some("Bâtiment C, 3ème étage".to_string()),
                    city: Some("Paris".to_string()),
                    country: CountryCode::parse_as_opt("FR"),
                    state: None,
                    zip_code: Some("75008".to_string()),
                }),
                shipping_address: None,
                invoicing_emails: vec![
                    "invoices@cobalt-industries.test".to_string(),
                    "finance@cobalt-industries.test".to_string(),
                ],
                subscription: Subscription {
                    plan_name: ENTERPRISE_PLAN.to_string(),
                    start_date: NaiveDate::from_ymd_opt(2024, 11, 3).unwrap(),
                },
            },
            Customer {
                name: "Pulse Analytics GmbH".to_string(),
                email: "billing@pulse-analytics.test".to_string(),
                currency: "EUR".to_string(),
                alias: Some("pulse-analytics".to_string()),
                phone: Some("+49 30 12345678".to_string()),
                vat_number: Some("DE123456789".to_string()),
                billing_address: Some(Address {
                    line1: Some("Friedrichstraße 123".to_string()),
                    line2: None,
                    city: Some("Berlin".to_string()),
                    country: CountryCode::parse_as_opt("DE"),
                    state: None,
                    zip_code: Some("10117".to_string()),
                }),
                shipping_address: Some(ShippingAddress {
                    address: Some(Address {
                        line1: Some("Warehouse District 45".to_string()),
                        line2: Some("Unit 12B".to_string()),
                        city: Some("Hamburg".to_string()),
                        country: CountryCode::parse_as_opt("DE"),
                        state: None,
                        zip_code: Some("20457".to_string()),
                    }),
                    same_as_billing: false,
                }),
                invoicing_emails: vec!["invoicing@pulse-analytics.test".to_string()],
                subscription: Subscription {
                    plan_name: STANDARD_80_PLAN.to_string(),
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 22).unwrap(),
                },
            },
            Customer {
                name: "Evergreen Dynamics Ltd".to_string(),
                email: "accounts@evergreen-dynamics.test".to_string(),
                currency: "EUR".to_string(),
                alias: Some("evergreen-dynamics".to_string()),
                phone: Some("+44 20 7946 0958".to_string()),
                vat_number: Some("GB123456789".to_string()),
                billing_address: Some(Address {
                    line1: Some("15 Canary Wharf".to_string()),
                    line2: Some("Tower 2, Floor 25".to_string()),
                    city: Some("London".to_string()),
                    country: CountryCode::parse_as_opt("GB"),
                    state: None,
                    zip_code: Some("E14 5AB".to_string()),
                }),
                shipping_address: None,
                invoicing_emails: vec![],
                subscription: Subscription {
                    plan_name: STANDARD_20_PLAN.to_string(),
                    start_date: NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
                },
            },
        ],
        organization: Some(OrganizationDetails {
            vat_number: Some("XX123456789".to_string()),
            address_line1: Some("Route de l'Innovation 10".to_string()),
            city: Some("Geneva".to_string()),
            zip_code: Some("1202".to_string()),
            invoice_footer_info: Some("Payment terms: Net 30 days. Late payment interest: 3% per annum.".to_string()),
            invoice_footer_legal: Some("Organization is registered in XX under company number XX-123.456.789. VAT ID: XX-123.456.789 TVA. Bank: XX National Bank, IBAN: XX93 00XX 2011 6238 52XX 7, BIC: SNXXCHZZXXX".to_string()),
        }),
    }
}
