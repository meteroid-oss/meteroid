use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, ProductFamilyId, ProductId, TenantId};
use diesel_models::products::{ProductRow, ProductRowNew};
use error_stack::Report;
use uuid::Uuid;

use super::enums::FeeTypeEnum;
use super::prices::Price;
use crate::domain::prices::FeeStructure;
use crate::errors::StoreError;

#[derive(Clone, Debug)]
pub struct Product {
    pub id: ProductId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub product_family_id: ProductFamilyId,
    pub fee_type: FeeTypeEnum,
    pub fee_structure: FeeStructure,
    pub catalog: bool,
}

impl TryFrom<ProductRow> for Product {
    type Error = Report<StoreError>;

    fn try_from(row: ProductRow) -> Result<Self, Self::Error> {
        let fee_structure =
            serde_json::from_value::<FeeStructure>(row.fee_structure).map_err(|e| {
                Report::new(StoreError::SerdeError(
                    "Failed to deserialize FeeStructure".to_string(),
                    e,
                ))
            })?;

        Ok(Product {
            id: row.id,
            name: row.name,
            description: row.description,
            created_at: row.created_at,
            created_by: row.created_by,
            updated_at: row.updated_at,
            archived_at: row.archived_at,
            tenant_id: row.tenant_id,
            product_family_id: row.product_family_id,
            fee_type: row.fee_type.into(),
            fee_structure,
            catalog: row.catalog,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ProductNew {
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub tenant_id: TenantId,
    pub family_id: ProductFamilyId,
    pub fee_type: FeeTypeEnum,
    pub fee_structure: FeeStructure,
    pub catalog: bool,
}

#[derive(Clone, Debug)]
pub struct ProductWithLatestPrice {
    pub product: Product,
    pub latest_price: Option<Price>,
}

impl TryFrom<ProductNew> for ProductRowNew {
    type Error = Report<StoreError>;

    fn try_from(new: ProductNew) -> Result<Self, Self::Error> {
        let fee_structure = serde_json::to_value(&new.fee_structure).map_err(|e| {
            Report::new(StoreError::SerdeError(
                "Failed to serialize FeeStructure".to_string(),
                e,
            ))
        })?;

        Ok(ProductRowNew {
            id: ProductId::new(),
            name: new.name,
            description: new.description,
            created_by: new.created_by,
            tenant_id: new.tenant_id,
            product_family_id: new.family_id,
            fee_type: new.fee_type.into(),
            fee_structure,
            catalog: new.catalog,
        })
    }
}
