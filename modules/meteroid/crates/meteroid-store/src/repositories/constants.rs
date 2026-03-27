use crate::StoreResult;
use crate::store::StoreInternal;
use common_domain::country::CountryCode;

impl StoreInternal {
    pub fn get_currency_from_country(&self, country: &CountryCode) -> StoreResult<String> {
        crate::utils::currency::get_currency_from_country(country)
    }
}
