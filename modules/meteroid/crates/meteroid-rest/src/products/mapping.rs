use meteroid_store::domain;
use meteroid_store::domain::prices::FeeStructure;

use super::model::*;

/// A product as REST v1 shows it: its fee type and structure are those of the charges using it
/// when they all bill it one way, absent otherwise.
pub fn product_to_rest(product: domain::Product, charge_structures: &[FeeStructure]) -> Product {
    let single = domain::products::single_structure(charge_structures);
    Product {
        id: product.id,
        name: product.name,
        description: product.description,
        fee_type: single.map(|fs| fs.fee_type().into()),
        fee_structure: single.map(fee_structure_to_rest),
        product_family_id: product.product_family_id,
        catalog: product.catalog,
        created_at: product.created_at.into(),
        archived_at: product.archived_at.map(Into::into),
        icon: product.icon,
        color: product.color,
    }
}

fn fee_structure_to_rest(fs: &FeeStructure) -> ProductFeeStructure {
    ProductFeeStructure::from(fs.clone())
}
