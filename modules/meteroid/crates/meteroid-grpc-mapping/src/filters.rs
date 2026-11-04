//! Mapping of the shared `meteroid.common.v1.Filter` wire type into the store filter domain.
//! Reused by the customers / subscriptions / invoices list handlers.

use common_grpc::meteroid::common::v1 as common;
use meteroid_store::domain::filters::{FieldRef, Filter, FilterOperator};
use tonic::Status;

fn map_operator(op: common::FilterOperator) -> Result<FilterOperator, Status> {
    use common::FilterOperator as P;
    Ok(match op {
        P::Unspecified => {
            return Err(Status::invalid_argument("filter operator is required"));
        }
        P::Equals => FilterOperator::Equals,
        P::NotEquals => FilterOperator::NotEquals,
        P::In => FilterOperator::In,
        P::Contains => FilterOperator::Contains,
        P::Gt => FilterOperator::Gt,
        P::Gte => FilterOperator::Gte,
        P::Lt => FilterOperator::Lt,
        P::Lte => FilterOperator::Lte,
        P::Between => FilterOperator::Between,
        P::IsSet => FilterOperator::IsSet,
        P::IsEmpty => FilterOperator::IsEmpty,
        P::IsTrue => FilterOperator::IsTrue,
        P::IsFalse => FilterOperator::IsFalse,
        P::Before => FilterOperator::Before,
        P::After => FilterOperator::After,
    })
}

/// Map a list of wire filters into domain filters, rejecting unknown/unspecified operators.
pub fn map_filters(filters: Vec<common::Filter>) -> Result<Vec<Filter>, Status> {
    filters
        .into_iter()
        .map(|f| {
            let op = common::FilterOperator::try_from(f.op)
                .map_err(|_| Status::invalid_argument("invalid filter operator"))?;
            Ok(Filter {
                field: FieldRef::parse(&f.field),
                op: map_operator(op)?,
                values: f.values,
            })
        })
        .collect()
}
