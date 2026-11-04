mod model;

pub use model::*;
pub mod apply;
mod shared;
pub use apply::{
    LineToTax, TaxToApply, apply_taxes, effective_rate, largest_remainder, round_to_minor,
};
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
        /// A VAT-registered seller invoicing outside its own VAT territory: a
        /// zero/empty rate set must surface as `Export` (carrying the export mention)
        /// rather than a plain `NoTax`/zero rate.
        is_export: bool,
        /// Whether the supply was priced as one to a taxable person. Travels with the
        /// exemption so a document cites the place-of-supply article matching the
        /// basis the tax was computed on, instead of re-deriving one at render time.
        basis: SupplyBasis,
        /// Whether the *seller's* own country runs a VAT-type system. Carried
        /// explicitly rather than inferred from `is_export`: a seller registered for
        /// VAT nowhere makes no taxable supply in any VAT system, so world-tax
        /// answering "the destination's VAT, at zero" must not become a rate row.
        source_is_vat: bool,
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

        // Export: a VAT-registered seller invoicing outside its own VAT territory charges
        // no VAT (mention text is resolved per jurisdiction at render time). World-tax
        // returns an empty rate set (non-VAT destination, e.g. US) or a single
        // VatRate::Zero (e.g. CH/GB).
        //
        // The comparison is between VAT *territories*, not country codes: a supply from
        // Vaduz to Zurich, or from Paris to Monaco, never leaves the seller's territory
        // and is domestic however different the two codes look.
        let source_is_vat = TAX_DATABASE
            .get_country(&invoicing_entity_country.code)
            .map(|c| matches!(c.tax_type, world_tax::types::TaxSystemType::Vat))
            .unwrap_or(false);
        let intra_territory = common_domain::country::same_vat_territory(
            &invoicing_entity_country.code,
            &customer_billing_country.code,
        );
        let is_export = source_is_vat && !intra_territory;

        // Guyane and Mayotte sit inside France's VAT territory yet levy no VAT at all
        // (CGI art. 294-1). A supply taxable there from inside the territory is not an
        // export, and it is not undetermined either: the law states the tax does not
        // apply, and the document must say so. Recognised from the rate data — a
        // `none`-type entry for a code belonging to another code's VAT territory —
        // rather than from a list of codes. A seller outside the territory never gets
        // here: for a VAT seller it is an export, for any other it stays undetermined.
        let destination = &customer_billing_country.code;
        let destination_levies_no_vat = intra_territory
            && common_domain::country::vat_territory(destination) != destination.as_str()
            && TAX_DATABASE
                .get_country(destination)
                .is_ok_and(|c| matches!(c.tax_type, world_tax::types::TaxSystemType::None));
        if destination_levies_no_vat {
            return Ok(ScenarioResolution::Immediate(CustomerTax::NoVatTerritory));
        }

        // Resolve against the country whose rate schedule actually applies. Monaco bills
        // French rates, the Isle of Man UK rates and Liechtenstein Swiss ones, so a supply
        // between such a territory and its host is domestic and bears the host's rate —
        // not the 0% a "different country code" reading produces. This also puts Monaco
        // inside the EU for the trade-agreement lookup, which it is.
        let source_schedule =
            common_domain::country::vat_rate_territory(&invoicing_entity_country.code);
        let destination_schedule =
            common_domain::country::vat_rate_territory(&customer_billing_country.code);

        // One VAT territory normally means one rate schedule, and the fold above
        // suffices. The exception is métropole ↔ the French overseas departments:
        // GP/MQ/RE bill their own rates (CGI art. 296: 8.5% standard, 2.1% reduced)
        // and GF/YT no VAT at all (CGI art. 294-1), yet a service between them and
        // the mainland is neither an export nor reverse-charged — BOFiP
        // BOI-TVA-GEO-20-40 §230 keeps the supplier liable in both directions, at
        // the rate in force where the supply is taxable (§100), which for the
        // services billed here is the destination side (CGI art. 259 1°, art. 259 D).
        // Folding the scenario to domestic *in the destination* makes world-tax
        // produce exactly that; left as two distinct codes, it would find no
        // agreement between them and zero-rate a supply that owes real tax.
        let (source_schedule, destination_schedule) =
            if intra_territory && source_schedule != destination_schedule {
                (destination_schedule, destination_schedule)
            } else {
                (source_schedule, destination_schedule)
            };

        let scenario = TaxScenario::new(
            Region::new(source_schedule.to_string(), None)
                .change_context(TaxEngineError::InvalidCountryOrRegion)?,
            Region::new(destination_schedule.to_string(), None)
                .change_context(TaxEngineError::InvalidCountryOrRegion)?,
            if is_b2b {
                world_tax::TransactionType::B2B
            } else {
                world_tax::TransactionType::B2C
            },
        );

        let cur = meteroid_money::iso::find(currency)
            .ok_or(Report::new(TaxEngineError::InvalidCurrency))?;
        let amount_f64 = meteroid_money::Money::from_minor(amount as i64, cur)
            .amount()
            .to_f64()
            .ok_or(Report::new(TaxEngineError::TaxCalculationError))?;

        Ok(ScenarioResolution::Scenario {
            scenario,
            amount_f64,
            is_export,
            source_is_vat,
            basis: if is_b2b {
                SupplyBasis::Business
            } else {
                SupplyBasis::Consumer
            },
        })
    }

    /// Resolves the destination country's standard rate(s) for the scenario
    /// (domestic / intra-EU reverse charge / intra-EU B2C destination / export zero).
    ///
    /// There is deliberately no unrated-domestic guard here any more. `resolve_scenario`
    /// folds every supply that stays inside one VAT territory onto a single country
    /// code, so a domestic scenario resolves against exactly one rate entry — and when
    /// that entry is missing, `get_rates` itself fails loudly (CountryNotFound) instead
    /// of returning zero. The zero or empty rate sets that do come back are therefore
    /// all legitimate: an export, or a non-VAT federal country. (The no-VAT parts of
    /// the territory — Guyane/Mayotte, CGI art. 294-1 — never reach here: they resolve
    /// to `NoVatTerritory` in `resolve_scenario`.) The guard that used to sit here had
    /// exactly one true positive — the then-unrated French overseas departments, rated
    /// since (CGI art. 296) — and one false one: a non-VAT seller's supply to a country
    /// we hold no rate data for, which owes nothing of ours regardless of that data.
    fn resolve_scenario_rates(
        scenario: &TaxScenario,
        amount_f64: f64,
        is_export: bool,
        basis: SupplyBasis,
        source_is_vat: bool,
    ) -> Result<CustomerTax, Report<TaxEngineError>> {
        let mut sc = scenario.clone();
        sc.vat_rate = Some(world_tax::VatRate::Standard);

        let rates = sc
            .get_rates(amount_f64, &TAX_DATABASE)
            .change_context(TaxEngineError::TaxCalculationError)?;

        Ok(rates_to_customer_tax(
            rates,
            is_export,
            basis,
            source_is_vat,
        ))
    }
}

fn rates_to_customer_tax(
    rates: Vec<world_tax::TaxRate>,
    is_export: bool,
    basis: SupplyBasis,
    source_is_vat: bool,
) -> CustomerTax {
    match rates.len() {
        0 => {
            if is_export {
                CustomerTax::Export(basis)
            } else {
                CustomerTax::NoTax
            }
        }
        1 => {
            let is_zero_rated = matches!(
                rates[0].tax_type,
                world_tax::TaxType::VAT(world_tax::VatRate::Zero)
            );
            if is_export && is_zero_rated {
                CustomerTax::Export(basis)
            } else if is_zero_rated && !source_is_vat {
                // A seller whose own country runs no VAT-type system is registered for
                // VAT nowhere this engine knows of. World-tax still answers "the
                // destination's VAT, at zero" when the destination reads as VAT — but
                // this seller makes no taxable supply in any VAT system, so a 0% VAT row
                // (UNCL 5305 `Z`) would assert a zero-rated *taxable* supply that does
                // not exist, and `Export` would assert "not subject to tax", which may
                // equally be false: the supply can be subject to destination VAT through
                // the non-Union One-Stop-Shop scheme, or to a customer-side reverse
                // charge. Record no tax and leave the position undetermined, exactly as
                // this seller's domestic sales already do. A merchant who is OSS-
                // registered, or whose business customer self-accounts, says so through
                // custom rates or the customer's reverse-charge status.
                CustomerTax::NoTax
            } else {
                CustomerTax::ResolvedTaxRate(world_tax::TaxRate {
                    rate: rates[0].rate,
                    tax_type: rates[0].tax_type.clone(),
                    compound: rates[0].compound,
                })
            }
        }
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
                is_export,
                basis,
                source_is_vat,
            } => Self::resolve_scenario_rates(
                &scenario,
                amount_f64,
                is_export,
                basis,
                source_is_vat,
            )?,
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
                is_export,
                basis,
                source_is_vat,
            } => {
                Self::resolve_scenario_rates(&scenario, amount_f64, is_export, basis, source_is_vat)
            }
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
