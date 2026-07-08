//! Tax rate provider and database functionality.
//!
//! This module provides the core functionality for accessing tax rates,
//! trade agreements, and calculating applicable tax rates for different
//! jurisdictions. It manages the loading and querying of tax-related data
//! from JSON sources.

use log::debug;
use std::collections::HashMap;

use super::types::{Country, TaxSystemType, TaxType, VatRate};
use crate::{
    errors::DatabaseError,
    types::{TaxRate, TradeAgreement},
};

/// Database containing tax rates and trade agreements for different jurisdictions.
///
/// The database is initialized from JSON files containing country-specific tax rates
/// and international trade agreements.
pub struct TaxDatabase {
    /// Map of country codes to their tax information
    countries: HashMap<String, Country>,
    /// Map of trade agreement identifiers to their details
    pub trade_agreements: HashMap<String, TradeAgreement>,
}

impl TaxDatabase {
    /// Creates a new TaxDatabase instance using embedded JSON data.
    ///
    /// # Examples
    ///
    /// ```
    /// use world_tax::provider::TaxDatabase;
    ///
    /// let db = TaxDatabase::new().expect("Failed to initialize tax database");
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the embedded JSON data cannot be parsed.
    pub fn new() -> Result<Self, serde_json::Error> {
        let countries = include_str!("../vat_rates.json");
        let trade_agreements = include_str!("../trade_agreements.json");

        Self::from_json(countries, trade_agreements)
    }

    /// Creates a new TaxDatabase instance from JSON strings.
    ///
    /// # Arguments
    ///
    /// * `countries_json` - JSON string containing country tax rates
    /// * `trade_agreements_json` - JSON string containing trade agreements
    ///
    /// # Errors
    ///
    /// Returns an error if either JSON string cannot be parsed.
    pub fn from_json(
        countries_json: &str,
        trade_agreements_json: &str,
    ) -> Result<Self, serde_json::Error> {
        let countries: HashMap<String, Country> = serde_json::from_str(countries_json)?;
        let trade_agreements: HashMap<String, TradeAgreement> =
            serde_json::from_str(trade_agreements_json)?;
        Ok(Self {
            countries,
            trade_agreements,
        })
    }

    /// Creates a new TaxDatabase instance from JSON files.
    ///
    /// # Arguments
    ///
    /// * `rates_path` - Path to the file containing country tax rates
    /// * `agreements_path` - Path to the file containing trade agreements
    ///
    /// # Errors
    ///
    /// Returns an error if either file cannot be read or parsed.
    pub fn from_files(
        rates_path: &str,
        agreements_path: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let rates_data = std::fs::read_to_string(rates_path)?;
        let agreements_data = std::fs::read_to_string(agreements_path)?;

        let countries = serde_json::from_str(&rates_data)?;
        let trade_agreements = serde_json::from_str(&agreements_data)?;

        Ok(Self {
            countries,
            trade_agreements,
        })
    }

    /// Retrieves the federal-level trade agreement for a country.
    ///
    /// # Arguments
    ///
    /// * `country` - The country code to look up
    ///
    /// # Returns
    ///
    /// Returns the trade agreement if one exists at the federal level for the country.
    pub fn get_federal_rule(&self, country: &str) -> Option<TradeAgreement> {
        let rule = self.trade_agreements.get(country);
        if let Some(rule) = rule {
            if rule.is_federal() {
                Some(rule.clone())
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Finds an international trade agreement between two countries.
    ///
    /// # Arguments
    ///
    /// * `source` - The source country code
    /// * `dest` - The destination country code
    ///
    /// # Returns
    ///
    /// Returns the trade agreement if one exists between the two countries.
    pub fn get_international_rule(&self, source: &str, dest: &str) -> Option<TradeAgreement> {
        for agreement in self.trade_agreements.values() {
            if agreement.members.contains(&source.to_string())
                && agreement.members.contains(&dest.to_string())
                && agreement.is_international()
            {
                return Some(agreement.clone());
            }
        }
        None
    }

    /// Retrieves tax information for a specific country.
    ///
    /// # Arguments
    ///
    /// * `code` - The country code to look up
    ///
    /// # Errors
    ///
    /// Returns `DatabaseError::CountryNotFound` if the country code is not found.
    pub fn get_country(&self, code: &str) -> Result<&Country, DatabaseError> {
        let country = self.countries.get(code);
        if let Some(country) = country {
            Ok(country)
        } else {
            Err(DatabaseError::CountryNotFound(code.to_string()))
        }
    }

    /// Retrieves a specific trade agreement by ID.
    ///
    /// # Arguments
    ///
    /// * `rule_id` - The identifier of the trade agreement
    ///
    /// # Errors
    ///
    /// Returns `DatabaseError::TradeAgreementNotFound` if the agreement is not found.
    pub fn get_rule(&self, rule_id: &str) -> Result<TradeAgreement, DatabaseError> {
        let rule = self.trade_agreements.get(rule_id);
        if let Some(rule) = rule {
            Ok(rule.clone())
        } else {
            Err(DatabaseError::TradeAgreementNotFound(rule_id.to_string()))
        }
    }

    /// Retrieves applicable tax rates for a jurisdiction.
    ///
    /// # Arguments
    ///
    /// * `country` - The country code
    /// * `region` - Optional region/state/province code  
    /// * `vat_rate` - Optional specific VAT rate to apply
    ///
    /// # Returns
    ///
    /// Returns a vector of applicable tax rates for the jurisdiction.
    ///
    /// # Errors
    ///
    /// Returns:
    /// - `DatabaseError::CountryNotFound` if the country is not found
    /// - `DatabaseError::VatRateNotFound` if a VAT rate was requested but not found
    /// - `DatabaseError::RegionNotFound` if a region was specified but not found
    ///
    /// # Examples
    ///
    /// ```
    /// # use world_tax::provider::TaxDatabase;
    /// # use world_tax::types::VatRate;
    /// # let db = TaxDatabase::new().unwrap();
    ///
    /// // Get standard VAT rate for France
    /// let rates = db.get_rate("FR", None, Some(&VatRate::Standard)).unwrap();
    ///
    /// // Get state sales tax for California, US
    /// let us_rates = db.get_rate("US", Some("US-CA"), None).unwrap();
    /// ```
    pub fn get_rate(
        &self,
        country: &str,
        region: Option<&str>,
        vat_rate: Option<&VatRate>,
    ) -> Result<Vec<TaxRate>, DatabaseError> {
        let country_data = self.get_country(country)?;
        let mut rates = Vec::new();

        // Special case for US which doesn't have a specific tax system type
        if country == "US" {
            if let Some(region_code) = region {
                if let Some(states) = &country_data.states {
                    if let Some(state) = states.get(region_code) {
                        // Only add the rate if it's non-zero
                        if state.standard_rate > 0.0 {
                            rates.push(TaxRate {
                                rate: state.standard_rate,
                                tax_type: TaxType::StateSalesTax,
                                compound: false,
                            });
                        }
                    }
                }
            }
            return Ok(rates);
        }

        match country_data.tax_type {
            TaxSystemType::Gst => {
                if let Some(region_code) = region {
                    if let Some(states) = &country_data.states {
                        if let Some(state) = states.get(region_code) {
                            match state.tax_type {
                                TaxSystemType::Hst => {
                                    rates.clear(); // Ensure no other rates exist
                                    rates.push(TaxRate {
                                        rate: state.standard_rate,
                                        tax_type: TaxType::HST,
                                        compound: false,
                                    });
                                }
                                TaxSystemType::Qst => {
                                    rates.push(TaxRate {
                                        rate: country_data.standard_rate,
                                        tax_type: TaxType::GST,
                                        compound: false,
                                    });
                                    rates.push(TaxRate {
                                        rate: state.standard_rate,
                                        tax_type: TaxType::QST,
                                        compound: true,
                                    });
                                }
                                TaxSystemType::Pst => {
                                    rates.push(TaxRate {
                                        rate: country_data.standard_rate,
                                        tax_type: TaxType::GST,
                                        compound: false,
                                    });
                                    rates.push(TaxRate {
                                        rate: state.standard_rate,
                                        tax_type: TaxType::PST,
                                        compound: true,
                                    });
                                }
                                _ => {
                                    debug!("Adding default GST rate");
                                    rates.push(TaxRate {
                                        rate: country_data.standard_rate,
                                        tax_type: TaxType::GST,
                                        compound: false,
                                    });
                                }
                            }
                        }
                    }
                } else {
                    rates.push(TaxRate {
                        rate: country_data.standard_rate,
                        tax_type: TaxType::GST,
                        compound: false,
                    });
                }
            }
            TaxSystemType::Vat => self.handle_vat_rates(country_data, vat_rate, &mut rates)?,
            TaxSystemType::Pst | TaxSystemType::Hst | TaxSystemType::Qst => {
                self.handle_gst_rates(country_data, region, &mut rates)?
            }
            TaxSystemType::None => {
                debug!("No tax system type");
            }
        }

        if rates.is_empty() {
            if matches!(country_data.tax_type, TaxSystemType::Vat) {
                Err(DatabaseError::VatRateNotFound(
                    vat_rate.unwrap_or(&VatRate::Standard).to_string(),
                ))
            } else {
                Ok(rates)
            }
        } else {
            Ok(rates)
        }
    }

    fn handle_vat_rates(
        &self,
        country: &Country,
        vat_rate: Option<&VatRate>,
        rates: &mut Vec<TaxRate>,
    ) -> Result<(), DatabaseError> {
        let rate_type = vat_rate.unwrap_or(&VatRate::Standard);
        let rate = match rate_type {
            VatRate::Standard => Some(country.standard_rate),
            VatRate::Reduced => country.reduced_rate,
            VatRate::ReducedAlt => country.reduced_rate_alt,
            VatRate::SuperReduced => country.super_reduced_rate,
            VatRate::Zero | VatRate::Exempt | VatRate::ReverseCharge => Some(0.0),
        };

        if let Some(rate_value) = rate {
            rates.push(TaxRate {
                rate: rate_value,
                tax_type: TaxType::VAT(rate_type.clone()),
                compound: false,
            });
        }
        Ok(())
    }

    fn handle_gst_rates(
        &self,
        country: &Country,
        region: Option<&str>,
        rates: &mut Vec<TaxRate>,
    ) -> Result<(), DatabaseError> {
        if let Some(region_code) = region {
            if let Some(states) = &country.states {
                if let Some(state) = states.get(region_code) {
                    debug!("############ Found state: {}", region_code);
                    match state.tax_type {
                        TaxSystemType::Hst => {
                            // HST replaces GST, single rate
                            rates.push(TaxRate {
                                rate: state.standard_rate,
                                tax_type: TaxType::HST,
                                compound: false,
                            });
                        }
                        TaxSystemType::Qst => {
                            // Add GST first
                            rates.push(TaxRate {
                                rate: country.standard_rate,
                                tax_type: TaxType::GST,
                                compound: false,
                            });
                            // Then QST
                            rates.push(TaxRate {
                                rate: state.standard_rate,
                                tax_type: TaxType::QST,
                                compound: true,
                            });
                        }
                        TaxSystemType::Pst => {
                            // Only add rates if not zero-rated
                            // Add GST first
                            rates.push(TaxRate {
                                rate: country.standard_rate,
                                tax_type: TaxType::GST,
                                compound: false,
                            });
                            // Then PST
                            rates.push(TaxRate {
                                rate: state.standard_rate,
                                tax_type: TaxType::PST,
                                compound: true,
                            });
                        }
                        _ => {
                            // Just GST for other cases
                            rates.push(TaxRate {
                                rate: country.standard_rate,
                                tax_type: TaxType::GST,
                                compound: false,
                            });
                        }
                    }
                    return Ok(());
                }
            }
        }

        // Default to just GST if no region or region not found
        rates.push(TaxRate {
            rate: country.standard_rate,
            tax_type: TaxType::GST,
            compound: false,
        });
        Ok(())
    }
}
