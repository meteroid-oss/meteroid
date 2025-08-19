mod model;

pub use model::*;
mod shared;
pub mod validation;

use error_stack::{Report, ResultExt};
use rust_decimal::prelude::ToPrimitive;
use world_tax::{Region, TaxRate, TaxScenario};

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
    ) -> error_stack::Result<VatNumberExternalValidationResult, TaxEngineError>;

    async fn calculate_line_items_tax(
        &self,
        currency: String,
        customer: CustomerForTax,
        invoicing_entity_address: Address,
        line_items: Vec<LineItemForTax>,
        invoice_date: chrono::NaiveDate,
    ) -> error_stack::Result<CalculationResult, TaxEngineError>;

    async fn calculate_customer_tax(
        &self,
        customer: CustomerForTax,
        invoicing_entity_address: Address,
        amount: u64,
        currency: &str,
    ) -> error_stack::Result<CustomerTax, TaxEngineError>;
}

pub struct MeteroidTaxEngine;

lazy_static::lazy_static! {
    static ref TAX_DATABASE: world_tax::TaxDatabase = world_tax::TaxDatabase::new()
            .expect("Failed to initialize world tax database");
}

#[async_trait::async_trait]
impl TaxEngine for MeteroidTaxEngine {
    async fn validate_vat_number(
        &self,
        _vat_number: String,
        _address: Address,
    ) -> error_stack::Result<VatNumberExternalValidationResult, TaxEngineError> {
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
    ) -> error_stack::Result<CalculationResult, TaxEngineError> {
        let amount = line_items.iter().map(|item| item.amount).sum::<u64>();

        let customer_tax = self
            .calculate_customer_tax(
                customer,
                invoicing_entity_address.clone(),
                amount,
                &currency,
            )
            .await
            .change_context(TaxEngineError::TaxCalculationError)?;

        let line_items =
            shared::compute_tax(customer_tax, invoicing_entity_address, line_items).await?;

        let breakdown = shared::compute_breakdown_from_line_items(&line_items);

        Ok(breakdown)
    }

    async fn calculate_customer_tax(
        &self,
        customer: CustomerForTax,
        invoicing_entity_address: Address,
        amount: u64,
        currency: &str,
    ) -> error_stack::Result<CustomerTax, TaxEngineError> {
        if customer.tax_exempt {
            return Ok(CustomerTax::Exempt);
        }
        if let Some(rate) = customer.custom_tax_rate {
            return Ok(CustomerTax::CustomTaxRate(rate));
        }

        let is_b2b = customer.vat_number.is_some() && customer.vat_number_format_valid;

        let invoicing_entity_country = match &invoicing_entity_address.country {
            Some(country) => country,
            None => return Ok(CustomerTax::NoTax),
        };

        let customer_billing_country = match &customer.billing_address.country {
            Some(country) => country,
            None => return Ok(CustomerTax::NoTax),
        };

        let scenario = TaxScenario::new(
            Region::new(invoicing_entity_country.clone(), None)
                .change_context(TaxEngineError::InvalidCountryOrRegion)?,
            Region::new(customer_billing_country.clone(), None)
                .change_context(TaxEngineError::InvalidCountryOrRegion)?,
            match is_b2b {
                true => world_tax::TransactionType::B2B,
                false => world_tax::TransactionType::B2C,
            },
        );

        let cur =
            rusty_money::iso::find(currency).ok_or(Report::new(TaxEngineError::InvalidCurrency))?;
        let amount_f64 = rusty_money::Money::from_minor(amount as i64, cur)
            .amount()
            .to_f64()
            .ok_or(Report::new(TaxEngineError::TaxCalculationError))?;

        let rates = scenario
            .get_rates(amount_f64, &TAX_DATABASE)
            .change_context(TaxEngineError::TaxCalculationError)?;

        match rates.first() {
            Some(rate) => Ok(CustomerTax::ResolvedTaxRate(TaxRate {
                rate: rate.rate,
                tax_type: rate.tax_type.clone(),
                compound: rate.compound,
            })),
            None => Ok(CustomerTax::NoTax),
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
    ) -> error_stack::Result<VatNumberExternalValidationResult, TaxEngineError> {
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
    ) -> error_stack::Result<CalculationResult, TaxEngineError> {
        let amount = line_items.iter().map(|item| item.amount).sum::<u64>();

        let customer_tax = self
            .calculate_customer_tax(
                customer,
                invoicing_entity_address.clone(),
                amount,
                &currency,
            )
            .await
            .change_context(TaxEngineError::TaxCalculationError)?;

        let line_items =
            shared::compute_tax(customer_tax, invoicing_entity_address, line_items).await?;

        let breakdown = shared::compute_breakdown_from_line_items(&line_items);

        Ok(breakdown)
    }

    async fn calculate_customer_tax(
        &self,
        customer: CustomerForTax,
        _invoicing_entity_address: Address,
        _amount: u64,
        _currency: &str,
    ) -> error_stack::Result<CustomerTax, TaxEngineError> {
        if customer.tax_exempt {
            return Ok(CustomerTax::Exempt);
        }
        if let Some(rate) = customer.custom_tax_rate {
            return Ok(CustomerTax::CustomTaxRate(rate));
        }
        Ok(CustomerTax::NoTax)
    }
}
