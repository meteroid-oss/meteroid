use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, ProductFamilyId, ProductId, TaxCategoryId, TenantId};
use diesel_models::products::{ProductRow, ProductRowNew};
use error_stack::Report;

use super::prices::Price;
use crate::errors::StoreError;

/// What is sold. How it is billed is its charges' (a plan's price components, add-ons): a
/// product has no fee structure.
#[derive(Clone, Debug)]
pub struct Product {
    pub id: ProductId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub product_family_id: ProductFamilyId,
    pub catalog: bool,
    pub tax_category_id: Option<TaxCategoryId>,
    pub icon: Option<String>,
    pub color: Option<String>,
}

/// The icons a product can wear: lucide names the dashboard ships.
pub const PRODUCT_ICONS: &[&str] = &[
    "box",
    "package",
    "cpu",
    "server",
    "database",
    "cloud",
    "zap",
    "bot",
    "sparkles",
    "brain",
    "message-square",
    "mail",
    "phone",
    "users",
    "user",
    "shield",
    "lock",
    "key",
    "globe",
    "bar-chart-3",
    "line-chart",
    "gauge",
    "hard-drive",
    "network",
    "code",
    "terminal",
    "layers",
    "puzzle",
    "rocket",
    "headphones",
    "life-buoy",
    "wrench",
];

/// The color tokens a product's icon is tinted with.
pub const PRODUCT_COLORS: &[&str] = &[
    "slate", "blue", "green", "amber", "red", "violet", "pink", "teal",
];

/// Rejects an icon or a color outside the dashboard's lists.
pub fn validate_product_appearance(
    icon: Option<&str>,
    color: Option<&str>,
) -> Result<(), Report<StoreError>> {
    if let Some(icon) = icon
        && !PRODUCT_ICONS.contains(&icon)
    {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "Unknown product icon: {icon}"
        ))));
    }
    if let Some(color) = color
        && !PRODUCT_COLORS.contains(&color)
    {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "Unknown product color: {color}"
        ))));
    }
    Ok(())
}

impl TryFrom<ProductRow> for Product {
    type Error = Report<StoreError>;

    fn try_from(row: ProductRow) -> Result<Self, Self::Error> {
        Ok(Product {
            id: row.id,
            name: row.name,
            description: row.description,
            created_at: row.created_at,
            updated_at: row.updated_at,
            archived_at: row.archived_at,
            tenant_id: row.tenant_id,
            product_family_id: row.product_family_id,
            catalog: row.catalog,
            tax_category_id: row.tax_category_id,
            icon: row.icon,
            color: row.color,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ProductNew {
    pub name: String,
    pub description: Option<String>,
    pub tenant_id: TenantId,
    pub family_id: ProductFamilyId,
    pub catalog: bool,
    pub tax_category_id: Option<TaxCategoryId>,
    pub icon: Option<String>,
    pub color: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProductWithLatestPrice {
    pub product: Product,
    pub latest_price: Option<Price>,
}

/// What the catalog list says of a product beyond itself.
#[derive(Clone, Debug, Default)]
pub struct ProductListFacts {
    /// Its latest price in each currency.
    pub latest_prices: Vec<Price>,
    pub feature_count: i64,
    pub plan_count: i64,
    /// Every distinct way its charges (plan charges, add-ons) bill it.
    pub charge_structures: Vec<super::prices::FeeStructure>,
}

/// The one way every structure of `structures` bills, if they agree: what a product's charges
/// say of it where a single fee type is shown (REST v1, the catalog).
pub fn single_structure(
    structures: &[super::prices::FeeStructure],
) -> Option<&super::prices::FeeStructure> {
    let first = structures.first()?;
    structures
        .iter()
        .all(|other| other.same_shape(first))
        .then_some(first)
}

/// Where a product is sold: the live plan versions with a charge of it, and its live add-ons.
#[derive(Clone, Debug, Default)]
pub struct ProductUsage {
    pub plans: Vec<ProductPlanUsage>,
    pub add_ons: Vec<super::add_ons::AddOn>,
}

/// A live version (a live plan's active or draft one) with a charge of the product.
#[derive(Clone, Debug)]
pub struct ProductPlanUsage {
    pub plan_id: common_domain::ids::PlanId,
    pub plan_name: String,
    pub plan_version_id: common_domain::ids::PlanVersionId,
    pub version: i32,
    pub is_draft: bool,
    /// The version's options, by position.
    pub options: Vec<super::plan_options::PlanOption>,
    /// Every currency the version is priced in: one the product's charge has no price in is a gap.
    pub currencies: Vec<String>,
    /// The version's charges of the product, with their prices and option links.
    pub components: Vec<super::price_components::PriceComponent>,
}

impl TryFrom<ProductNew> for ProductRowNew {
    type Error = Report<StoreError>;

    fn try_from(new: ProductNew) -> Result<Self, Self::Error> {
        Ok(ProductRowNew {
            id: ProductId::new(),
            name: new.name,
            description: new.description,
            tenant_id: new.tenant_id,
            product_family_id: new.family_id,
            catalog: new.catalog,
            tax_category_id: new.tax_category_id,
            icon: new.icon,
            color: new.color,
        })
    }
}
