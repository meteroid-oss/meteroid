//! Shared builders for archetype scenarios.

use super::domain::{
    AddOn, BillableMetric, Customer, FeeType, OrganizationDetails, PriceComponent, Subscription,
    UsageProfile,
};
use crate::utils::slugify;
use common_domain::country::CountryCode;
use meteroid_store::domain::{
    Address, BillingMetricAggregateEnum, BillingPeriodEnum, DowngradePolicy, TermRate,
    UpgradePolicy, UsagePricingModel,
};
use rust_decimal::Decimal;

pub(crate) fn customer(
    name: &str,
    currency: &str,
    country: &str,
    city: &str,
    subscriptions: Vec<Subscription>,
) -> Customer {
    let slug = slugify(name);
    Customer {
        name: name.to_string(),
        email: format!("billing@{slug}.test"),
        currency: currency.to_string(),
        alias: Some(slug.clone()),
        phone: None,
        vat_number: None,
        billing_address: Some(Address {
            line1: Some("1 Demo Street".to_string()),
            line2: None,
            city: Some(city.to_string()),
            country: CountryCode::parse_as_opt(country),
            state: None,
            zip_code: Some("00000".to_string()),
        }),
        shipping_address: None,
        invoicing_emails: vec![format!("invoices@{slug}.test")],
        subscriptions,
        usage: UsageProfile::Generated,
        history: None,
        credit: vec![],
        tax_rates: vec![],
    }
}

pub(crate) fn metric(
    code: &str,
    name: &str,
    agg: BillingMetricAggregateEnum,
    key: &str,
) -> BillableMetric {
    BillableMetric {
        code: code.to_string(),
        name: name.to_string(),
        description: None,
        aggregation_type: agg,
        aggregation_key: Some(key.to_string()),
        ..Default::default()
    }
}

/// A flat monthly recurring charge.
pub(crate) fn rate(name: &str, price: Decimal) -> PriceComponent {
    PriceComponent {
        name: name.to_string(),
        fee: FeeType::Rate {
            rates: vec![TermRate {
                price,
                term: BillingPeriodEnum::Monthly,
            }],
        },
    }
}

/// A per-unit metered usage charge over `metric_code`.
pub(crate) fn usage(name: &str, metric_code: &str, unit_rate: Decimal) -> PriceComponent {
    PriceComponent {
        name: name.to_string(),
        fee: FeeType::Usage {
            metric_code: metric_code.to_string(),
            pricing: UsagePricingModel::PerUnit { rate: unit_rate },
            cadence: BillingPeriodEnum::Monthly,
            credit_unit: None,
        },
    }
}

/// A per-seat slot charge, named "Seats" so the runner can set each
/// subscription's initial seat count.
pub(crate) fn seats(unit_price: Decimal) -> PriceComponent {
    PriceComponent {
        name: "Seats".to_string(),
        fee: FeeType::Slot {
            rates: vec![TermRate {
                price: unit_price,
                term: BillingPeriodEnum::Monthly,
            }],
            slot_unit_name: "seat".to_string(),
            upgrade_policy: UpgradePolicy::Prorated,
            downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
            minimum_count: Some(1),
            quota: None,
        },
    }
}

/// A flat monthly add-on catalog entry.
pub(crate) fn add_on(name: &str, price: Decimal) -> AddOn {
    AddOn {
        name: name.to_string(),
        price,
        cadence: BillingPeriodEnum::Monthly,
    }
}

pub(crate) fn org(vat: &str, city: &str, zip: &str) -> OrganizationDetails {
    OrganizationDetails {
        vat_number: Some(vat.to_string()),
        address_line1: Some("1 Demo Street".to_string()),
        city: Some(city.to_string()),
        zip_code: Some(zip.to_string()),
        invoice_footer_info: Some("Payment terms: Net 30 days.".to_string()),
        invoice_footer_legal: Some("Demo organization for sandbox data.".to_string()),
        invoicing_language: Some("en-US".to_string()),
        country: None,
        state: None,
        tax_resolver: None,
    }
}
