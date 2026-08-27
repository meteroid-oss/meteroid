mod model;

pub use model::*;
mod shared;
pub mod validation;
pub mod vies;

#[cfg(test)]
mod tests;

use error_stack::{Report, ResultExt};
use rust_decimal::prelude::ToPrimitive;
use world_tax::{Region, TaxScenario};

#[derive(thiserror::Error, Debug, Clone)]
pub enum TaxEngineError {
    #[error("Failed to compute tax")]
    TaxCalculationError,
    #[error("Invalid country or region provided")]
    InvalidCountryOrRegion,
    #[error("Invalid currency provided")]
    InvalidCurrency,
}

#[async_trait::async_trait]
pub trait TaxEngine: Send + Sync {
    async fn validate_vat_number(
        &self,
        vat_number: String,
        address: Address,
    ) -> Result<VatNumberExternalValidationResult, Report<TaxEngineError>>;

    async fn calculate_line_items_tax(
        &self,
        currency: String,
        customer: CustomerForTax,
        invoicing_entity_address: Address,
        line_items: Vec<LineItemForTax>,
        invoice_date: chrono::NaiveDate,
    ) -> Result<CalculationResult, Report<TaxEngineError>>;

    async fn calculate_customer_tax(
        &self,
        customer: CustomerForTax,
        invoicing_entity_address: Address,
        amount: u64,
        currency: &str,
    ) -> Result<CustomerTax, Report<TaxEngineError>>;
}

/// Destination (place-of-supply) address for override matching: the customer's
/// ship-to when set, otherwise their billing address.
fn destination_address(customer: &CustomerForTax) -> Address {
    customer
        .shipping_address
        .clone()
        .unwrap_or_else(|| customer.billing_address.clone())
}

pub struct MeteroidTaxEngine;

lazy_static::lazy_static! {
    static ref TAX_DATABASE: world_tax::TaxDatabase = world_tax::TaxDatabase::new()
            .expect("Failed to initialize world tax database");
}

/// Outcome of resolving the customer + jurisdiction against the built-in engine.
enum ScenarioResolution {
    /// A jurisdiction-independent result (exempt / reverse charge / customer custom
    /// rates / no-tax / missing country), identical for every line.
    Immediate(CustomerTax),
    /// A statutory VAT/GST scenario resolved at the destination's standard rate.
    /// `amount_f64` is the invoice-wide amount used only for threshold selection.
    Scenario {
        scenario: TaxScenario,
        amount_f64: f64,
    },
}

impl MeteroidTaxEngine {
    /// Builds the tax scenario for the customer, or returns a class-independent
    /// `CustomerTax` when no statutory scenario applies.
    fn resolve_scenario(
        customer: &CustomerForTax,
        invoicing_entity_address: &Address,
        amount: u64,
        currency: &str,
    ) -> Result<ScenarioResolution, Report<TaxEngineError>> {
        match customer.tax_status {
            CustomerTaxStatus::Exempt => {
                return Ok(ScenarioResolution::Immediate(CustomerTax::Exempt));
            }
            CustomerTaxStatus::ReverseCharge => {
                return Ok(ScenarioResolution::Immediate(CustomerTax::ReverseCharge));
            }
            CustomerTaxStatus::Taxable => {}
        }
        if !customer.custom_tax_rates.is_empty() {
            return Ok(ScenarioResolution::Immediate(CustomerTax::TaxRates(
                customer.custom_tax_rates.clone(),
            )));
        }

        // Strict mode (per invoicing entity, Hyperline-style): reverse charge only
        // once VIES confirmed the number; otherwise the customer's country rate
        // applies. Default is fail-open on format validity alone.
        let vies_ok_or_not_required = !customer.require_vies_valid_for_reverse_charge
            || customer.vat_number_vies_valid == Some(true);
        let is_b2b = customer
            .vat_number
            .as_ref()
            .is_some_and(|vat| !vat.trim().is_empty())
            && customer.vat_number_format_valid
            && vies_ok_or_not_required;

        let invoicing_entity_country = match &invoicing_entity_address.country {
            Some(country) => country,
            None => return Ok(ScenarioResolution::Immediate(CustomerTax::NoTax)),
        };

        let customer_billing_country = match &customer.billing_address.country {
            Some(country) => country,
            None => return Ok(ScenarioResolution::Immediate(CustomerTax::NoTax)),
        };

        let scenario = TaxScenario::new(
            Region::new(invoicing_entity_country.code.clone(), None)
                .change_context(TaxEngineError::InvalidCountryOrRegion)?,
            Region::new(customer_billing_country.code.clone(), None)
                .change_context(TaxEngineError::InvalidCountryOrRegion)?,
            if is_b2b {
                world_tax::TransactionType::B2B
            } else {
                world_tax::TransactionType::B2C
            },
        );

        let cur =
            rusty_money::iso::find(currency).ok_or(Report::new(TaxEngineError::InvalidCurrency))?;
        let amount_f64 = rusty_money::Money::from_minor(amount as i64, cur)
            .amount()
            .to_f64()
            .ok_or(Report::new(TaxEngineError::TaxCalculationError))?;

        Ok(ScenarioResolution::Scenario {
            scenario,
            amount_f64,
        })
    }

    /// Resolves the destination country's standard rate(s) for the scenario
    /// (domestic / intra-EU reverse charge / intra-EU B2C destination / export zero).
    fn resolve_scenario_rates(
        scenario: &TaxScenario,
        amount_f64: f64,
    ) -> Result<CustomerTax, Report<TaxEngineError>> {
        let mut sc = scenario.clone();
        sc.vat_rate = Some(world_tax::VatRate::Standard);

        let rates = sc
            .get_rates(amount_f64, &TAX_DATABASE)
            .change_context(TaxEngineError::TaxCalculationError)?;

        Ok(rates_to_customer_tax(rates))
    }
}

fn rates_to_customer_tax(rates: Vec<world_tax::TaxRate>) -> CustomerTax {
    match rates.len() {
        0 => CustomerTax::NoTax,
        1 => CustomerTax::ResolvedTaxRate(world_tax::TaxRate {
            rate: rates[0].rate,
            tax_type: rates[0].tax_type.clone(),
            compound: rates[0].compound,
        }),
        _ => CustomerTax::ResolvedMultipleTaxRates(
            rates
                .into_iter()
                .map(|rate| world_tax::TaxRate {
                    rate: rate.rate,
                    tax_type: rate.tax_type.clone(),
                    compound: rate.compound,
                })
                .collect(),
        ),
    }
}

#[async_trait::async_trait]
impl TaxEngine for MeteroidTaxEngine {
    async fn validate_vat_number(
        &self,
        vat_number: String,
        _address: Address,
    ) -> Result<VatNumberExternalValidationResult, Report<TaxEngineError>> {
        Ok(vies::validate(&vat_number).await.result)
    }
    async fn calculate_line_items_tax(
        &self,
        currency: String,
        customer: CustomerForTax,
        invoicing_entity_address: Address,
        line_items: Vec<LineItemForTax>,
        _invoice_date: chrono::NaiveDate,
    ) -> Result<CalculationResult, Report<TaxEngineError>> {
        // Net invoice amount, only used for threshold tier selection; a net-credit
        // invoice clamps to 0 (never crosses an upward threshold).
        let amount = line_items
            .iter()
            .map(|item| item.amount)
            .sum::<i64>()
            .max(0) as u64;

        let exemption_reason = customer.exemption_reason.clone();
        let destination_address = destination_address(&customer);

        let resolution =
            Self::resolve_scenario(&customer, &invoicing_entity_address, amount, &currency)?;

        let customer_tax = match resolution {
            ScenarioResolution::Immediate(customer_tax) => customer_tax,
            ScenarioResolution::Scenario {
                scenario,
                amount_f64,
            } => Self::resolve_scenario_rates(&scenario, amount_f64)?,
        };

        let computed = shared::compute_tax(
            customer_tax,
            invoicing_entity_address,
            destination_address,
            line_items,
        )
        .await?;

        let mut breakdown = shared::compute_breakdown_from_line_items(&computed);
        breakdown.exemption_reason = exemption_reason;

        Ok(breakdown)
    }

    async fn calculate_customer_tax(
        &self,
        customer: CustomerForTax,
        invoicing_entity_address: Address,
        amount: u64,
        currency: &str,
    ) -> Result<CustomerTax, Report<TaxEngineError>> {
        // Single-amount, customer-level rate: resolved at the standard rate.
        match Self::resolve_scenario(&customer, &invoicing_entity_address, amount, currency)? {
            ScenarioResolution::Immediate(customer_tax) => Ok(customer_tax),
            ScenarioResolution::Scenario {
                scenario,
                amount_f64,
            } => Self::resolve_scenario_rates(&scenario, amount_f64),
        }
    }
}

pub struct ManualTaxEngine;

#[async_trait::async_trait]
impl TaxEngine for ManualTaxEngine {
    async fn validate_vat_number(
        &self,
        _vat_number: String,
        _address: Address,
    ) -> Result<VatNumberExternalValidationResult, Report<TaxEngineError>> {
        // TODO Implement the VIES validation
        Ok(VatNumberExternalValidationResult::ServiceUnavailable)
    }
    async fn calculate_line_items_tax(
        &self,
        currency: String,
        customer: CustomerForTax,
        invoicing_entity_address: Address,
        line_items: Vec<LineItemForTax>,
        _invoice_date: chrono::NaiveDate,
    ) -> Result<CalculationResult, Report<TaxEngineError>> {
        let amount = line_items
            .iter()
            .map(|item| item.amount)
            .sum::<i64>()
            .max(0) as u64;

        let exemption_reason = customer.exemption_reason.clone();
        let destination_address = destination_address(&customer);

        let customer_tax = self
            .calculate_customer_tax(
                customer,
                invoicing_entity_address.clone(),
                amount,
                &currency,
            )
            .await
            .change_context(TaxEngineError::TaxCalculationError)?;

        let line_items = shared::compute_tax(
            customer_tax,
            invoicing_entity_address,
            destination_address,
            line_items,
        )
        .await?;

        let mut breakdown = shared::compute_breakdown_from_line_items(&line_items);
        breakdown.exemption_reason = exemption_reason;

        Ok(breakdown)
    }

    async fn calculate_customer_tax(
        &self,
        customer: CustomerForTax,
        _invoicing_entity_address: Address,
        _amount: u64,
        _currency: &str,
    ) -> Result<CustomerTax, Report<TaxEngineError>> {
        match customer.tax_status {
            CustomerTaxStatus::Exempt => return Ok(CustomerTax::Exempt),
            CustomerTaxStatus::ReverseCharge => return Ok(CustomerTax::ReverseCharge),
            CustomerTaxStatus::Taxable => {}
        }
        if !customer.custom_tax_rates.is_empty() {
            return Ok(CustomerTax::TaxRates(customer.custom_tax_rates));
        }
        Ok(CustomerTax::NoTax)
    }
}
