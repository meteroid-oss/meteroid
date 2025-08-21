use common_domain::ids::{CustomTaxId, InvoicingEntityId, ProductId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Clone, Debug, Identifiable, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::custom_tax)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomTaxRow {
    pub id: CustomTaxId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub name: String,
    pub tax_code: String,
    pub rules: serde_json::Value,
}

#[derive(Clone, Debug, Identifiable, Queryable, Selectable, Insertable)]
#[diesel(primary_key(product_id, invoicing_entity_id))]
#[diesel(table_name = crate::schema::product_accounting)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProductAccountingRow {
    pub product_id: ProductId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub custom_tax_id: Option<CustomTaxId>,
    pub product_code: Option<String>,
    pub ledger_account_code: Option<String>,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProductAccountingWithTaxRow {
    #[diesel(embed)]
    pub product_accounting: ProductAccountingRow,
    #[diesel(embed)]
    pub custom_tax: Option<CustomTaxRow>,
}
