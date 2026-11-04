use common_domain::identifiers::validate_code;
use meteroid_grpc::meteroid::api::eventinsights::v1 as grpc;
use meteroid_store::domain::MetricDefinition;
use meteroid_store::domain::enums::BillingMetricAggregateEnum;
use tonic::Status;

use crate::billablemetrics::mapping::{aggregation_type, metric};

pub fn draft_to_domain(draft: grpc::MetricDefinition) -> Result<MetricDefinition, Status> {
    validate_code(&draft.code).map_err(|e| Status::invalid_argument(e.to_string()))?;
    let aggregation = draft
        .aggregation
        .ok_or_else(|| Status::invalid_argument("a metric needs an aggregation"))?;
    let aggregation_type = aggregation_type::server_to_domain(aggregation.aggregation_type());
    let aggregation_key = aggregation.aggregation_key.filter(|key| !key.is_empty());
    if aggregation_type != BillingMetricAggregateEnum::Count && aggregation_key.is_none() {
        return Err(Status::invalid_argument(
            "this aggregation needs the property it aggregates",
        ));
    }

    Ok(MetricDefinition {
        code: draft.code,
        aggregation_type,
        aggregation_key,
        // Truncated like a saved metric's factor, so the preview matches what saving bills.
        unit_conversion_factor: aggregation.unit_conversion.map(|u| u.factor as i32),
        segmentation_matrix: metric::map_segmentation_matrix_from_server(
            draft.segmentation_matrix,
        )?,
        filters: metric::map_filters_from_server(draft.filters),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use meteroid_grpc::meteroid::api::billablemetrics::v1 as metrics;
    use meteroid_grpc::meteroid::api::billablemetrics::v1::segmentation_matrix::Matrix;

    fn draft(
        aggregation: metrics::aggregation::AggregationType,
        key: Option<&str>,
    ) -> grpc::MetricDefinition {
        grpc::MetricDefinition {
            code: "api_calls".to_string(),
            aggregation: Some(metrics::Aggregation {
                aggregation_type: aggregation.into(),
                aggregation_key: key.map(str::to_string),
                unit_conversion: Some(metrics::aggregation::UnitConversion {
                    factor: 1000.0,
                    rounding: 0,
                }),
            }),
            segmentation_matrix: None,
            filters: vec![],
        }
    }

    #[test]
    fn a_draft_maps_like_a_saved_metric() {
        let definition = draft_to_domain(draft(
            metrics::aggregation::AggregationType::Sum,
            Some("tokens"),
        ))
        .unwrap();
        assert_eq!(definition.aggregation_type, BillingMetricAggregateEnum::Sum);
        assert_eq!(definition.aggregation_key.as_deref(), Some("tokens"));
        assert_eq!(definition.unit_conversion_factor, Some(1000));
    }

    #[test]
    fn a_draft_needs_the_aggregated_property_unless_it_counts() {
        let missing =
            draft_to_domain(draft(metrics::aggregation::AggregationType::Max, None)).unwrap_err();
        assert_eq!(missing.code(), tonic::Code::InvalidArgument);
        assert!(draft_to_domain(draft(metrics::aggregation::AggregationType::Count, None)).is_ok());
    }

    #[test]
    fn a_matrix_missing_a_dimension_is_rejected_not_a_panic() {
        let mut d = draft(metrics::aggregation::AggregationType::Count, None);
        d.segmentation_matrix = Some(metrics::SegmentationMatrix {
            matrix: Some(Matrix::Single(
                metrics::segmentation_matrix::SegmentationMatrixSingle { dimension: None },
            )),
        });
        assert_eq!(
            draft_to_domain(d).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
    }
}
