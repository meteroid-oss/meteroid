//! Tax calculation implementation and scenario handling.
//!
//! This module provides the core tax calculation functionality, including
//! determination of applicable tax rates, calculation types, and final tax amounts
//! based on various scenarios and trade agreements.
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

use crate::types::TaxSystemType;

use super::{
    DatabaseError, ProcessingError, Region, TaxCalculationType, TaxDatabase, TaxRate, TaxScenario,
    TaxType, TradeAgreement, TradeAgreementOverride, TransactionType, VatRate,
};

impl TaxScenario {
    /// Creates a new tax calculation scenario with default settings.
    ///
    /// # Arguments
    ///
    /// * `source_region` - The region where the seller is located
    /// * `destination_region` - The region where the buyer is located
    /// * `transaction_type` - The type of transaction (B2B or B2C)
    ///
    /// # Examples
    ///
    /// ```
    /// use world_tax::types::{Region, TransactionType, TaxScenario};
    ///
    /// let scenario = TaxScenario::new(
    ///     Region::new("FR".to_string(), None).unwrap(),
    ///     Region::new("DE".to_string(), None).unwrap(),
    ///     TransactionType::B2B
    /// );
    /// ```
    pub fn new(
        source_region: Region,
        destination_region: Region,
        transaction_type: TransactionType,
    ) -> Self {
        Self {
            source_region,
            destination_region,
            transaction_type,
            trade_agreement_override: None,
            is_digital_product_or_service: false,
            has_resale_certificate: false,
            ignore_threshold: false,
            vat_rate: None,
        }
    }

    /// Sets a trade agreement override for the scenario.
    ///
    /// # Arguments
    ///
    /// * `override_type` - The trade agreement override to apply
    pub fn with_trade_agreement_override(mut self, override_type: TradeAgreementOverride) -> Self {
        self.trade_agreement_override = Some(override_type);
        self
    }

    /// Checks if the source and destination are in the same country.
    pub fn is_same_country(&self) -> bool {
        self.source_region.country == self.destination_region.country
    }

    /// Checks if the source and destination are in the same state/region.
    pub fn is_same_state(&self) -> bool {
        self.source_region.region == self.destination_region.region
    }

    /// Determines the calculation type based on the trade agreement and transaction details.
    ///
    /// # Arguments
    ///
    /// * `agreement` - The trade agreement to evaluate
    /// * `amount` - The transaction amount
    ///
    /// # Returns
    ///
    /// Returns the appropriate tax calculation type based on the agreement rules.
    fn get_calculation_type_from_agreement(
        &self,
        agreement: &TradeAgreement,
        amount: f64,
    ) -> Result<TaxCalculationType, ProcessingError> {
        if agreement.is_international() {
            // Custom union like EU
            match self.transaction_type {
                TransactionType::B2B => {
                    let rule = &agreement.tax_rules.internal_b2b;
                    if rule.is_some() {
                        // In the EU, likely to be reverse charge
                        return Ok(rule
                            .clone()
                            .unwrap()
                            .by_threshold(amount as u32, self.ignore_threshold)
                            .clone());
                    } else {
                        // Fallback to a safe option
                        Ok(TaxCalculationType::Destination)
                    }
                }
                TransactionType::B2C => {
                    let rule = &agreement.tax_rules.internal_b2c;
                    if rule.is_some() {
                        // In the EU, by threshold, likely to be origin or destination based
                        return Ok(rule
                            .clone()
                            .unwrap()
                            .by_threshold_or_digital_product_threshold(
                                amount as u32,
                                self.is_digital_product_or_service,
                                self.ignore_threshold,
                            )
                            .clone());
                    } else {
                        // Fallback to a safe option
                        Ok(TaxCalculationType::Destination)
                    }
                }
            }
        } else if agreement.is_federal() {
            // States like in the US, CA
            if self.destination_region.country == "CA" {
                if let Some(region) = &self.destination_region.region {
                    // HST provinces should always charge HST
                    if ["CA-NS", "CA-NB", "CA-NL", "CA-ON", "CA-PE"].contains(&region.as_str()) {
                        return Ok(TaxCalculationType::Destination);
                    }
                    // QC should always charge GST+QST
                    if region == "CA-QC" {
                        return Ok(TaxCalculationType::Destination);
                    }
                }
            }

            // States like in the US and other Canadian provinces
            match self.transaction_type {
                TransactionType::B2B => {
                    let rule = &agreement.tax_rules.internal_b2b;
                    if rule.is_some() {
                        let u_rule = rule.clone().unwrap();
                        if u_rule.is_reseller(self.has_resale_certificate) {
                            return Ok(TaxCalculationType::ZeroRated);
                        }
                        return Ok(rule
                            .clone()
                            .unwrap()
                            .by_threshold(amount as u32, self.ignore_threshold)
                            .clone());
                    } else {
                        Ok(TaxCalculationType::Destination)
                    }
                }
                TransactionType::B2C => {
                    let rule = &agreement.tax_rules.internal_b2c;
                    if rule.is_some() {
                        // Check threshold except for HST/QST provinces
                        if !self.ignore_threshold
                            && amount < rule.clone().unwrap().threshold.unwrap_or(u32::MAX) as f64
                        {
                            return Ok(TaxCalculationType::ZeroRated);
                        }
                        return Ok(rule
                            .clone()
                            .unwrap()
                            .by_threshold(amount as u32, self.ignore_threshold)
                            .clone());
                    } else {
                        Ok(TaxCalculationType::Destination)
                    }
                }
            }
        } else {
            return Ok(TaxCalculationType::Destination);
        }
    }

    /// Determines which trade agreement rules apply to the scenario.
    ///
    /// # Arguments
    ///
    /// * `db` - The tax database containing trade agreements
    ///
    /// # Returns
    ///
    /// Returns the applicable trade agreement, if any.
    fn determine_rule(&self, db: &TaxDatabase) -> Result<Option<TradeAgreement>, DatabaseError> {
        if self.trade_agreement_override.is_some() {
            let overwrite = self.trade_agreement_override.clone().unwrap();
            match overwrite {
                TradeAgreementOverride::UseAgreement(agreement) => {
                    let rule = db.get_rule(agreement.as_str())?;
                    return Ok(Some(rule));
                }
                TradeAgreementOverride::NoAgreement => {
                    return Ok(None);
                }
            }
        }
        if self.is_same_country() {
            // Same country; Federal agreement (for ex. USA)
            Ok(db.get_federal_rule(self.source_region.country.as_str()))
        } else {
            // Different countries; Customs union agreement (for ex. EU)
            Ok(db.get_international_rule(
                self.source_region.country.as_str(),
                self.destination_region.country.as_str(),
            ))
        }
    }

    /// Determines the appropriate tax calculation type for the scenario.
    ///
    /// # Arguments
    ///
    /// * `db` - The tax database
    /// * `amount` - The transaction amount
    ///
    /// # Returns
    ///
    /// Returns the tax calculation type that should be applied.
    ///
    /// # Examples
    ///
    /// ```
    /// # use world_tax::types::{Region, TransactionType, TaxScenario};
    /// # use world_tax::provider::TaxDatabase;
    /// # let db = TaxDatabase::new().unwrap();
    /// # let scenario = TaxScenario::new(
    /// #     Region::new("FR".to_string(), None).unwrap(),
    /// #     Region::new("DE".to_string(), None).unwrap(),
    /// #     TransactionType::B2B
    /// # );
    /// let calc_type = scenario.determine_calculation_type(&db, 1000.0).unwrap();
    /// ```
    pub fn determine_calculation_type(
        &self,
        db: &TaxDatabase,
        amount: f64,
    ) -> Result<TaxCalculationType, ProcessingError> {
        // Check if there's a trade rule
        let agreement = self.determine_rule(db)?;

        if agreement.is_none() {
            // No agreement found, use default rules
            if self.is_same_country() {
                match self.transaction_type {
                    TransactionType::B2B => return Ok(TaxCalculationType::Origin),
                    TransactionType::B2C => return Ok(TaxCalculationType::Origin),
                }
            } else {
                return Ok(TaxCalculationType::ZeroRated);
            }
        }

        let calc_type = self.get_calculation_type_from_agreement(&agreement.unwrap(), amount)?;
        Ok(calc_type)
    }

    /// Gets the applicable tax rates for the scenario.
    ///
    /// # Arguments
    ///
    /// * `amount` - The transaction amount
    /// * `db` - The tax database
    ///
    /// # Returns
    ///
    /// Returns a vector of applicable tax rates.
    ///
    /// # Examples
    ///
    /// ```
    /// # use world_tax::types::{Region, TransactionType, TaxScenario};
    /// # use world_tax::provider::TaxDatabase;
    /// # let db = TaxDatabase::new().unwrap();
    /// # let scenario = TaxScenario::new(
    /// #     Region::new("FR".to_string(), None).unwrap(),
    /// #     Region::new("DE".to_string(), None).unwrap(),
    /// #     TransactionType::B2B
    /// # );
    /// let rates = scenario.get_rates(1000.0, &db).unwrap();
    /// ```
    pub fn get_rates(
        &self,
        amount: f64,
        db: &TaxDatabase,
    ) -> Result<Vec<TaxRate>, ProcessingError> {
        let calculation_type = self.determine_calculation_type(db, amount)?;

        // Special handling for US B2B with resale certificate
        if self.source_region.country == "US"
            && self.transaction_type == TransactionType::B2B
            && self.has_resale_certificate
        {
            return Ok(vec![]);
        }

        // Get the country's tax system type
        let country = db.get_country(&self.destination_region.country)?;

        match calculation_type {
            TaxCalculationType::ReverseCharge => {
                match country.tax_type {
                    TaxSystemType::Vat => Ok(vec![TaxRate {
                        tax_type: TaxType::VAT(VatRate::ReverseCharge),
                        compound: false,
                        rate: 0.0,
                    }]),
                    _ => {
                        // For non-VAT systems, proceed with normal rate lookup
                        self.get_regional_rates(calculation_type, db)
                    }
                }
            }
            TaxCalculationType::ZeroRated => {
                match country.tax_type {
                    TaxSystemType::Vat => Ok(vec![TaxRate {
                        tax_type: TaxType::VAT(VatRate::Zero),
                        compound: false,
                        rate: 0.0,
                    }]),
                    _ => Ok(vec![]), // For non-VAT systems, no tax
                }
            }
            TaxCalculationType::Exempt => {
                // Only apply Exempt for VAT systems
                match country.tax_type {
                    TaxSystemType::Vat => Ok(vec![TaxRate {
                        tax_type: TaxType::VAT(VatRate::Exempt),
                        compound: false,
                        rate: 0.0,
                    }]),
                    _ => self.get_regional_rates(calculation_type, db), // For non-VAT systems, proceed with normal lookup
                }
            }
            _ => self.get_regional_rates(calculation_type, db),
        }
    }

    // Helper method to get regional rates
    fn get_regional_rates(
        &self,
        calculation_type: TaxCalculationType,
        db: &TaxDatabase,
    ) -> Result<Vec<TaxRate>, ProcessingError> {
        let region = match calculation_type {
            TaxCalculationType::Origin => &self.source_region,
            TaxCalculationType::ZeroRated => return Ok(vec![]),
            _ => &self.destination_region,
        };

        // For US interstate commerce and Canadian provinces, handle thresholds
        if (region.country == "US" || region.country == "CA") && !self.is_same_state() {
            match calculation_type {
                TaxCalculationType::Origin => Ok(vec![]),
                TaxCalculationType::ZeroRated => Ok(vec![]),
                TaxCalculationType::Destination => db
                    .get_rate(
                        &region.country,
                        region.region.as_deref(),
                        self.vat_rate.as_ref(),
                    )
                    .map_err(ProcessingError::from),
                _ => Ok(vec![]),
            }
        } else {
            // Normal rate lookup for other cases
            db.get_rate(
                &region.country,
                region.region.as_deref(),
                self.vat_rate.as_ref(),
            )
            .map_err(ProcessingError::from)
        }
    }

    /// Calculates the total tax amount for the scenario.
    ///
    /// # Arguments
    ///
    /// * `amount` - The transaction amount
    /// * `db` - The tax database
    ///
    /// # Returns
    ///
    /// Returns the calculated tax amount, rounded to 2 decimal places.
    ///
    /// # Examples
    ///
    /// ```
    /// # use world_tax::types::{Region, TransactionType, TaxScenario};
    /// # use world_tax::provider::TaxDatabase;
    /// # let db = TaxDatabase::new().unwrap();
    /// # let scenario = TaxScenario::new(
    /// #     Region::new("FR".to_string(), None).unwrap(),
    /// #     Region::new("DE".to_string(), None).unwrap(),
    /// #     TransactionType::B2B
    /// # );
    /// let tax_amount = scenario.calculate_tax(1000.0, &db).unwrap();
    /// ```
    pub fn calculate_tax(&self, amount: f64, db: &TaxDatabase) -> Result<f64, ProcessingError> {
        let rates = self.get_rates(amount, db)?;

        let mut total_tax = 0.0;
        let base_amount = amount;

        for rate in rates {
            let tax_amount = if rate.compound {
                (base_amount + total_tax) * rate.rate
            } else {
                base_amount * rate.rate
            };
            total_tax += tax_amount;
        }

        Ok((total_tax * 100.0).round() / 100.0)
    }

    pub fn calculate_tax_decimal(
        &self,
        amount: Decimal,
        db: &TaxDatabase,
    ) -> Result<Decimal, ProcessingError> {
        // Accuracy doesn't matter as much here, because we're looking for the treshold only
        let amount_f64 = amount.to_f64().ok_or(ProcessingError::InvalidAmount)?;
        let rates = self.get_rates(amount_f64, db)?;

        let mut total_tax = Decimal::from(0);
        let base_amount = amount;

        for rate in rates {
            let tax_amount = if rate.compound {
                (base_amount + total_tax) * Decimal::from_f64(rate.rate).unwrap()
            } else {
                base_amount * Decimal::from_f64(rate.rate).unwrap()
            };
            total_tax += tax_amount;
        }

        Ok(total_tax)
    }
}
