pub use meteroid_app::mapping::aggregation_type;

pub mod unit_conversion_rounding {
    use meteroid_grpc::meteroid::api::billablemetrics::v1::aggregation::unit_conversion as server;
    use meteroid_store::domain;

    pub fn server_to_domain(
        value: server::UnitConversionRounding,
    ) -> domain::enums::UnitConversionRoundingEnum {
        match value {
            server::UnitConversionRounding::None => domain::enums::UnitConversionRoundingEnum::None,
            server::UnitConversionRounding::Up => domain::enums::UnitConversionRoundingEnum::Up,
            server::UnitConversionRounding::Down => domain::enums::UnitConversionRoundingEnum::Down,
            server::UnitConversionRounding::Nearest => {
                domain::enums::UnitConversionRoundingEnum::Nearest
            }
        }
    }

    pub fn domain_to_server(
        value: domain::enums::UnitConversionRoundingEnum,
    ) -> server::UnitConversionRounding {
        match value {
            domain::enums::UnitConversionRoundingEnum::None => server::UnitConversionRounding::None,
            domain::enums::UnitConversionRoundingEnum::Up => server::UnitConversionRounding::Up,
            domain::enums::UnitConversionRoundingEnum::Down => server::UnitConversionRounding::Down,
            domain::enums::UnitConversionRoundingEnum::Nearest => {
                server::UnitConversionRounding::Nearest
            }
            _ => server::UnitConversionRounding::None, // TODO drop the extra types for now
        }
    }
}

pub mod metric {
    use error_stack::Report;
    use meteroid_grpc::meteroid::api::billablemetrics::v1 as server;
    use std::collections::HashMap;

    use meteroid_grpc::meteroid::api::billablemetrics::v1::segmentation_matrix::Matrix;
    use meteroid_grpc_mapping::shared::mapping::datetime::chrono_to_timestamp;
    use meteroid_store::domain;
    use meteroid_store::domain::billable_metrics::{
        Dimension, MetricFilter, MetricFilterOp, MetricFilters, MetricUnitLabel, SegmentationMatrix,
    };
    use meteroid_store::errors::StoreError;

    pub struct ServerBillableMetricWrapper(pub server::BillableMetric);

    impl TryFrom<domain::BillableMetric> for ServerBillableMetricWrapper {
        type Error = Report<StoreError>;

        fn try_from(value: domain::BillableMetric) -> Result<Self, Self::Error> {
            Ok(ServerBillableMetricWrapper(server::BillableMetric {
                id: value.id.as_proto(),
                local_id: value.id.as_proto(), //todo remove me
                name: value.name,
                code: value.code,
                description: value.description,
                aggregation: Some(server::Aggregation {
                    aggregation_type: super::aggregation_type::domain_to_server(
                        value.aggregation_type,
                    )
                    .into(),
                    aggregation_key: value.aggregation_key,
                    unit_conversion: value
                        .unit_conversion_factor
                        .zip(value.unit_conversion_rounding)
                        .map(|(factor, rounding)| server::aggregation::UnitConversion {
                            factor: f64::from(factor), // TODO
                            rounding: super::unit_conversion_rounding::domain_to_server(rounding)
                                .into(),
                        }),
                }),
                segmentation_matrix: map_segmentation_matrix(value.segmentation_matrix),
                archived_at: value.archived_at.map(chrono_to_timestamp),
                created_at: Some(chrono_to_timestamp(value.created_at)),
                usage_group_key: value.usage_group_key,
                product_id: value.product_id.map(|x| x.as_proto()),
                family_local_id: value.product_family_id.as_proto(),
                filters: map_filters_to_server(value.filters),
                unit_label: unit_label_to_server(value.unit_singular, value.unit_plural),
            }))
        }
    }

    pub struct ServerBillableMetricMetaWrapper(pub server::BillableMetricMeta);

    impl TryFrom<domain::BillableMetricMeta> for ServerBillableMetricMetaWrapper {
        type Error = Report<StoreError>;

        fn try_from(value: domain::BillableMetricMeta) -> Result<Self, Self::Error> {
            Ok(ServerBillableMetricMetaWrapper(
                server::BillableMetricMeta {
                    id: value.id.as_proto(),
                    name: value.name,
                    code: value.code,
                    description: value.description,
                    aggregation_type: super::aggregation_type::domain_to_server(
                        value.aggregation_type,
                    )
                    .into(),
                    aggregation_key: value.aggregation_key,
                    created_at: Some(chrono_to_timestamp(value.created_at)),
                    archived_at: value.archived_at.map(chrono_to_timestamp),
                    unit_conversion_factor: value.unit_conversion_factor,
                    unit_label: unit_label_to_server(value.unit_singular, value.unit_plural),
                },
            ))
        }
    }

    /// A matrix missing a dimension is an invalid request, not a missing matrix.
    pub fn map_segmentation_matrix_from_server(
        segmentation_matrix: Option<server::SegmentationMatrix>,
    ) -> Result<Option<SegmentationMatrix>, tonic::Status> {
        let dimension = |dimension: Option<server::segmentation_matrix::Dimension>| {
            dimension
                .map(|d| Dimension {
                    key: d.key,
                    values: d.values,
                })
                .ok_or_else(|| {
                    tonic::Status::invalid_argument("a segmentation dimension is missing")
                })
        };
        Ok(match segmentation_matrix.and_then(|m| m.matrix) {
            None => None,
            Some(Matrix::Single(single)) => {
                Some(SegmentationMatrix::Single(dimension(single.dimension)?))
            }
            Some(Matrix::Double(double)) => Some(SegmentationMatrix::Double {
                dimension1: dimension(double.dimension1)?,
                dimension2: dimension(double.dimension2)?,
            }),
            Some(Matrix::Linked(linked)) => Some(SegmentationMatrix::Linked {
                dimension1_key: linked.dimension_key,
                dimension2_key: linked.linked_dimension_key,
                values: linked
                    .values
                    .into_iter()
                    .map(|(k, v)| (k, v.values))
                    .collect::<HashMap<String, Vec<String>>>(),
            }),
        })
    }

    pub fn map_segmentation_matrix(
        segmentation_matrix: Option<SegmentationMatrix>,
    ) -> Option<server::SegmentationMatrix> {
        segmentation_matrix
            .map(|sm| server::SegmentationMatrix {
                matrix: match sm {
                    SegmentationMatrix::Single(Dimension { key, values }) => Some(
                        server::segmentation_matrix::Matrix::Single(server::segmentation_matrix::SegmentationMatrixSingle {
                            dimension: Some(server::segmentation_matrix::Dimension {
                                key,
                                values,
                            })
                        })
                    ),
                    SegmentationMatrix::Double { dimension1, dimension2 } => {
                        Some(server::segmentation_matrix::Matrix::Double(server::segmentation_matrix::SegmentationMatrixDouble {
                            dimension1: Some(server::segmentation_matrix::Dimension {
                                key: dimension1.key,
                                values: dimension1.values,
                            }),
                            dimension2: Some(server::segmentation_matrix::Dimension {
                                key: dimension2.key,
                                values: dimension2.values,
                            }),
                        }))
                    }
                    SegmentationMatrix::Linked { dimension1_key, dimension2_key, values } => {
                        Some(server::segmentation_matrix::Matrix::Linked(server::segmentation_matrix::SegmentationMatrixLinked {
                            dimension_key: dimension1_key,
                            linked_dimension_key: dimension2_key,
                            values: values.iter()
                                .map(|(k, v)| (k.clone(), server::segmentation_matrix::segmentation_matrix_linked::DimensionValues { values: v.clone() }))
                                .collect::<HashMap<String, server::segmentation_matrix::segmentation_matrix_linked::DimensionValues>>(),
                        }))
                    }
                }
            })
    }

    pub fn list_db_to_server(
        metric: domain::billable_metrics::BillableMetricMeta,
    ) -> server::BillableMetricMeta {
        server::BillableMetricMeta {
            id: metric.id.to_string(),
            name: metric.name,
            code: metric.code,
            aggregation_type: super::aggregation_type::domain_to_server(metric.aggregation_type)
                .into(),
            aggregation_key: metric.aggregation_key,
            archived_at: metric.archived_at.map(chrono_to_timestamp),
            created_at: Some(chrono_to_timestamp(metric.created_at)),
            description: metric.description,
            unit_conversion_factor: metric.unit_conversion_factor,
            unit_label: unit_label_to_server(metric.unit_singular, metric.unit_plural),
        }
    }

    pub fn unit_label_to_server(
        singular: Option<String>,
        plural: Option<String>,
    ) -> Option<server::MetricUnitLabel> {
        singular
            .zip(plural)
            .map(|(singular, plural)| server::MetricUnitLabel { singular, plural })
    }

    pub fn unit_label_from_server(
        label: Option<&server::MetricUnitLabel>,
    ) -> Result<Option<MetricUnitLabel>, tonic::Status> {
        MetricUnitLabel::parse(
            label.map(|l| l.singular.as_str()),
            label.map(|l| l.plural.as_str()),
        )
        .map_err(tonic::Status::invalid_argument)
    }

    fn filter_op_from_server(op: server::metric_filter::Operator) -> MetricFilterOp {
        match op {
            server::metric_filter::Operator::Equal => MetricFilterOp::Equal,
            server::metric_filter::Operator::NotEqual => MetricFilterOp::NotEqual,
            server::metric_filter::Operator::In => MetricFilterOp::In,
            server::metric_filter::Operator::NotIn => MetricFilterOp::NotIn,
        }
    }

    fn filter_op_to_server(op: MetricFilterOp) -> server::metric_filter::Operator {
        match op {
            MetricFilterOp::Equal => server::metric_filter::Operator::Equal,
            MetricFilterOp::NotEqual => server::metric_filter::Operator::NotEqual,
            MetricFilterOp::In => server::metric_filter::Operator::In,
            MetricFilterOp::NotIn => server::metric_filter::Operator::NotIn,
        }
    }

    /// Server filter list → domain. An empty list maps to `None` so nothing is persisted.
    pub fn map_filters_from_server(filters: Vec<server::MetricFilter>) -> Option<MetricFilters> {
        if filters.is_empty() {
            return None;
        }
        Some(MetricFilters(
            filters
                .into_iter()
                .map(|f| {
                    let op = filter_op_from_server(f.op());
                    MetricFilter {
                        property: f.property,
                        op,
                        values: f.values,
                    }
                })
                .collect(),
        ))
    }

    pub fn map_filters_to_server(filters: Option<MetricFilters>) -> Vec<server::MetricFilter> {
        filters
            .map(|MetricFilters(list)| {
                list.into_iter()
                    .map(|f| server::MetricFilter {
                        property: f.property,
                        op: filter_op_to_server(f.op).into(),
                        values: f.values,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Merges the values-only update with the existing segmentation matrix structure (keeping keys immutable)
    pub fn merge_segmentation_matrix_values(
        existing: Option<SegmentationMatrix>,
        values_update: server::SegmentationMatrixValuesUpdate,
    ) -> Option<SegmentationMatrix> {
        use server::segmentation_matrix_values_update::Values;

        let existing = existing?;

        match (existing, values_update.values) {
            (SegmentationMatrix::Single(dim), Some(Values::Single(new_values))) => {
                Some(SegmentationMatrix::Single(Dimension {
                    key: dim.key,
                    values: new_values.values,
                }))
            }
            (
                SegmentationMatrix::Double {
                    dimension1,
                    dimension2,
                },
                Some(Values::Double(new_values)),
            ) => Some(SegmentationMatrix::Double {
                dimension1: Dimension {
                    key: dimension1.key,
                    values: new_values.dimension1_values,
                },
                dimension2: Dimension {
                    key: dimension2.key,
                    values: new_values.dimension2_values,
                },
            }),
            (
                SegmentationMatrix::Linked {
                    dimension1_key,
                    dimension2_key,
                    ..
                },
                Some(Values::Linked(new_values)),
            ) => Some(SegmentationMatrix::Linked {
                dimension1_key,
                dimension2_key,
                values: new_values
                    .values
                    .iter()
                    .map(|(k, v)| (k.clone(), v.values.clone()))
                    .collect(),
            }),
            // Mismatch between existing matrix type and update type - ignore update
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::metric::map_segmentation_matrix_from_server;
    use meteroid_grpc::meteroid::api::billablemetrics::v1::SegmentationMatrix;
    use meteroid_grpc::meteroid::api::billablemetrics::v1::segmentation_matrix::{
        Dimension, Matrix, SegmentationMatrixDouble,
    };
    use meteroid_store::domain::billable_metrics::SegmentationMatrix as Domain;

    fn double(dimension2: Option<Dimension>) -> Option<SegmentationMatrix> {
        Some(SegmentationMatrix {
            matrix: Some(Matrix::Double(SegmentationMatrixDouble {
                dimension1: Some(Dimension {
                    key: "region".to_string(),
                    values: vec!["eu".to_string()],
                }),
                dimension2,
            })),
        })
    }

    #[test]
    fn a_matrix_missing_a_dimension_is_an_invalid_argument() {
        let refused = map_segmentation_matrix_from_server(double(None)).unwrap_err();
        assert_eq!(refused.code(), tonic::Code::InvalidArgument);

        let mapped = map_segmentation_matrix_from_server(double(Some(Dimension {
            key: "tier".to_string(),
            values: vec!["pro".to_string()],
        })))
        .unwrap();
        assert!(matches!(
            mapped,
            Some(Domain::Double { dimension1, dimension2 })
                if dimension1.key == "region" && dimension2.values == ["pro"]
        ));
        assert!(map_segmentation_matrix_from_server(None).unwrap().is_none());
    }
}
