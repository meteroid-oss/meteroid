//! A wallet's amounts are minor units of an ISO currency or of a credit unit. Requests name one
//! with `currency` or `credit_unit`; responses set exactly one of the two, with the unit's
//! precision.
use crate::errors::RestApiError;
use common_domain::ids::TenantId;
use meteroid_rest_common::currency::Currency;
use meteroid_store::Store;
use meteroid_store::domain::credit_units::{CreditUnitCode, DenominationCode};
use meteroid_store::repositories::CreditUnitsInterface;
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct Denomination {
    pub currency: Option<Currency>,
    pub credit_unit: Option<String>,
    pub credit_unit_precision: Option<u8>,
}

impl Denomination {
    pub fn of_currency(currency: Currency) -> Self {
        Self {
            currency: Some(currency),
            credit_unit: None,
            credit_unit_precision: None,
        }
    }

    pub fn of_unit(code: &str, precision: u8) -> Self {
        Self {
            currency: None,
            credit_unit: Some(code.to_owned()),
            credit_unit_precision: Some(precision),
        }
    }

    /// Minor-unit exponent of the amounts.
    pub fn precision(&self) -> Option<u8> {
        match (&self.currency, self.credit_unit_precision) {
            (Some(currency), _) => {
                meteroid_store::constants::Currencies::resolve_currency_precision(
                    &currency.to_string(),
                )
            }
            (None, precision) => precision,
        }
    }
}

/// The wallet key a request names: `credit_unit`, else `currency`, else the customer's currency.
pub fn requested_code(
    currency: Option<&Currency>,
    credit_unit: Option<&str>,
    customer_currency: &str,
) -> Result<String, RestApiError> {
    match (currency, credit_unit) {
        (Some(_), Some(_)) => Err(RestApiError::InvalidInput(
            "set either currency or credit_unit, not both".to_owned(),
        )),
        (None, Some(code)) => CreditUnitCode::new(code)
            .map(CreditUnitCode::into_inner)
            .map_err(|error| RestApiError::InvalidInput(error.current_context().to_string())),
        (Some(currency), None) => Ok(currency.to_string()),
        (None, None) => Ok(customer_currency.trim().to_ascii_uppercase()),
    }
}

/// The precisions of the tenant's credit units, read once per request and only when a unit is
/// named. Archived units keep their wallets, so they are included.
pub struct Denominations(HashMap<String, u8>);

impl Denominations {
    // Not `async fn`: the borrowed codes iterator must not live in the (Send) handler future.
    pub fn load<'a>(
        store: &'a Store,
        tenant_id: TenantId,
        codes: impl IntoIterator<Item = &'a str>,
    ) -> impl Future<Output = Result<Self, RestApiError>> + Send + 'a {
        let needs_units = codes.into_iter().any(|code| code.trim().len() != 3);
        async move {
            if !needs_units {
                return Ok(Self(HashMap::new()));
            }
            let units = store
                .list_credit_units(tenant_id, true)
                .await
                .map_err(RestApiError::from)?;
            Ok(Self(
                units
                    .into_iter()
                    .map(|unit| (unit.code.into_inner(), unit.precision))
                    .collect(),
            ))
        }
    }

    pub fn of(&self, code: &str) -> Result<Denomination, RestApiError> {
        match DenominationCode::parse(code).map_err(|_| unknown(code))? {
            DenominationCode::Currency(code) => Currency::from_str(&code)
                .map(Denomination::of_currency)
                .map_err(|_| RestApiError::StoreError),
            DenominationCode::Unit(code) => self
                .0
                .get(code.as_str())
                .map(|precision| Denomination::of_unit(code.as_str(), *precision))
                .ok_or_else(|| unknown(code.as_str())),
        }
    }
}

fn unknown(code: &str) -> RestApiError {
    RestApiError::InvalidInput(format!("unknown currency or credit unit '{code}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_names_a_currency_or_a_unit_never_both() {
        assert_eq!(requested_code(None, None, " eur").unwrap(), "EUR");
        assert_eq!(
            requested_code(Some(&Currency::USD), None, "EUR").unwrap(),
            "USD"
        );
        assert_eq!(
            requested_code(None, Some("credits"), "EUR").unwrap(),
            "CREDITS"
        );
        assert!(requested_code(Some(&Currency::USD), Some("CREDITS"), "EUR").is_err());
        assert!(requested_code(None, Some("EUR"), "EUR").is_err());
    }

    #[test]
    fn a_unit_reads_with_its_precision_and_a_currency_without() {
        let known = Denominations(HashMap::from([("CREDITS".to_owned(), 3)]));
        let credits = known.of("CREDITS").unwrap();
        assert!(credits.currency.is_none());
        assert_eq!(credits.credit_unit.as_deref(), Some("CREDITS"));
        assert_eq!(credits.precision(), Some(3));
        let eur = known.of("EUR").unwrap();
        assert_eq!(eur.credit_unit, None);
        assert_eq!(eur.precision(), Some(2));
        assert!(known.of("TOKENS").is_err());
    }
}
