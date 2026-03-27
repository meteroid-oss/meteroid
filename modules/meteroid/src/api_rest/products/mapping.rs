use meteroid_store::domain;
use meteroid_store::domain::prices::FeeStructure;

use super::model::*;
use crate::errors::RestApiError;

pub fn product_to_rest(product: domain::Product) -> Product {
    Product {
        id: product.id,
        name: product.name,
        description: product.description,
        fee_type: product.fee_type.into(),
        fee_structure: fee_structure_to_rest(&product.fee_structure),
        product_family_id: product.product_family_id,
        catalog: product.catalog,
        created_at: product.created_at,
        archived_at: product.archived_at,
    }
}

fn fee_structure_to_rest(fs: &FeeStructure) -> ProductFeeStructure {
    ProductFeeStructure::from(fs.clone())
}

pub fn rest_fee_structure_to_domain(
    fs: &ProductFeeStructure,
) -> Result<(domain::enums::FeeTypeEnum, FeeStructure), RestApiError> {
    let fee_type = fs.fee_type_enum();
    let fee_structure = FeeStructure::from(fs.clone());
    Ok((fee_type, fee_structure))
}
