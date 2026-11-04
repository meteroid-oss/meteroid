use crate::credit_units::model as rest;
use common_domain::ids::{CreditUnitId, TenantId};
use meteroid_app::errors::RestApiError;
use meteroid_store::domain::credit_units as domain;

/// `prices` may hold other units' prices: only this unit's active ones are kept.
pub fn domain_to_rest(
    unit: domain::CreditUnit,
    prices: &[domain::CreditUnitPrice],
) -> rest::CreditUnit {
    rest::CreditUnit {
        prices: prices
            .iter()
            .filter(|price| price.credit_unit_id == unit.id && price.archived_at.is_none())
            .map(price_to_rest)
            .collect(),
        product_id: unit.product_id,
        id: unit.id,
        code: unit.code.into_inner(),
        name: unit.name,
        plural_name: unit.plural_name,
        symbol: unit.symbol,
        precision: unit.precision,
        archived: unit.archived_at.is_some(),
    }
}

pub fn price_to_rest(price: &domain::CreditUnitPrice) -> rest::CreditUnitPrice {
    rest::CreditUnitPrice {
        id: price.id,
        currency: price.currency.clone(),
        unit_price: price.unit_price.normalize(),
        created_at: price.created_at.into(),
        archived_at: price.archived_at.map(Into::into),
    }
}

pub fn create_req_to_domain(
    tenant_id: TenantId,
    req: rest::CreditUnitCreateRequest,
) -> Result<domain::CreditUnitNew, RestApiError> {
    domain::CreditUnitNew::new(
        tenant_id,
        req.code,
        req.name,
        req.plural_name,
        req.symbol.as_deref(),
        req.precision,
    )
    .map_err(RestApiError::from)
}

pub fn update_req_to_domain(
    tenant_id: TenantId,
    id: CreditUnitId,
    req: rest::CreditUnitUpdateRequest,
) -> Result<domain::CreditUnitPatch, RestApiError> {
    let patch = domain::CreditUnitPatch::new(
        tenant_id,
        id,
        req.name,
        req.plural_name,
        req.symbol,
        req.archived,
    )
    .map_err(RestApiError::from)?;
    Ok(match req.product_id {
        Some(product_id) => patch.with_product_id(product_id),
        None => patch,
    })
}
