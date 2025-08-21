use tax_ids::TaxId;

pub fn validate_vat_number_format(vat_number: &str) -> bool {
    // TODO   assert_eq!(tax_id.country_code(), customer_country_code);
    TaxId::new(vat_number).is_ok()
}
