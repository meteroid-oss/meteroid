pub mod credit_units {
    use crate::creditunits::error::CreditUnitsApiError;
    use common_domain::ids::{CreditUnitId, TenantId};
    use meteroid_app::conversions::ProtoConv;
    use meteroid_grpc::meteroid::api::creditunits::v1 as server;
    use meteroid_store::domain::credit_units as domain;

    type Result<T> = std::result::Result<T, CreditUnitsApiError>;

    /// `prices` may hold other units' prices: only this unit's are kept.
    pub fn domain_to_proto(
        unit: domain::CreditUnit,
        prices: &[domain::CreditUnitPrice],
    ) -> server::CreditUnit {
        server::CreditUnit {
            prices: prices
                .iter()
                .filter(|price| price.credit_unit_id == unit.id && price.archived_at.is_none())
                .map(price_to_proto)
                .collect(),
            id: unit.id.as_proto(),
            code: unit.code.into_inner(),
            name: unit.name,
            plural_name: unit.plural_name,
            symbol: unit.symbol,
            precision: i32::from(unit.precision),
            archived: unit.archived_at.is_some(),
            created_at: Some(unit.created_at.as_proto()),
            product_id: unit.product_id.map(|id| id.as_proto()),
        }
    }

    pub fn price_to_proto(price: &domain::CreditUnitPrice) -> server::CreditUnitPrice {
        server::CreditUnitPrice {
            id: price.id.as_proto(),
            currency: price.currency.clone(),
            unit_price: price.unit_price.normalize().to_string(),
            created_at: price.created_at.as_proto(),
            archived_at: price.archived_at.map(|at| at.as_proto()),
        }
    }

    pub fn parse_unit_price(value: &str) -> Result<rust_decimal::Decimal> {
        value.trim().parse().map_err(|_| {
            CreditUnitsApiError::InvalidArgument("the unit price must be a decimal number".into())
        })
    }

    pub fn create_req_to_domain(
        tenant_id: TenantId,
        req: server::CreateCreditUnitRequest,
    ) -> Result<domain::CreditUnitNew> {
        let precision = u8::try_from(req.precision)
            .map_err(|_| CreditUnitsApiError::InvalidArgument("invalid precision".to_string()))?;
        domain::CreditUnitNew::new(
            tenant_id,
            req.code,
            req.name,
            req.plural_name,
            req.symbol.as_deref(),
            precision,
        )
        .map_err(Into::into)
    }

    pub fn update_req_to_domain(
        tenant_id: TenantId,
        req: server::UpdateCreditUnitRequest,
    ) -> Result<domain::CreditUnitPatch> {
        let id = CreditUnitId::from_proto(req.id)
            .map_err(|e| CreditUnitsApiError::InvalidArgument(e.to_string()))?;
        // An empty symbol clears it; a present non-empty one sets it; absent leaves it unchanged.
        let symbol = req
            .symbol
            .map(|value| if value.is_empty() { None } else { Some(value) });
        domain::CreditUnitPatch::new(tenant_id, id, req.name, req.plural_name, symbol, None)
            .map_err(Into::into)
    }
}
