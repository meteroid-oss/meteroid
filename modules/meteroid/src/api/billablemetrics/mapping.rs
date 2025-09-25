pub mod aggregation_type {
    use metering_grpc::meteroid::metering::v1 as metering;
    use meteroid_grpc::meteroid::api::billablemetrics::v1::aggregation as server;
    use meteroid_store::domain;

    pub fn server_to_domain(
        value: server::AggregationType,
    ) -> domain::enums::BillingMetricAggregateEnum {
        match value {
            server::AggregationType::Sum => domain::enums::BillingMetricAggregateEnum::Sum,
            server::AggregationType::Min => domain::enums::BillingMetricAggregateEnum::Min,
            server::AggregationType::Max => domain::enums::BillingMetricAggregateEnum::Max,
            server::AggregationType::Mean => domain::enums::BillingMetricAggregateEnum::Mean,
            server::AggregationType::Count => domain::enums::BillingMetricAggregateEnum::Count,
            server::AggregationType::CountDistinct => {
                domain::enums::BillingMetricAggregateEnum::CountDistinct
            }
            server::AggregationType::Latest => domain::enums::BillingMetricAggregateEnum::Latest,
        }
    }

    pub fn domain_to_server(
        value: domain::enums::BillingMetricAggregateEnum,
    ) -> server::AggregationType {
        match value {
            domain::enums::BillingMetricAggregateEnum::Sum => server::AggregationType::Sum,
            domain::enums::BillingMetricAggregateEnum::Min => server::AggregationType::Min,
            domain::enums::BillingMetricAggregateEnum::Max => server::AggregationType::Max,
            domain::enums::BillingMetricAggregateEnum::Mean => server::AggregationType::Mean,
            domain::enums::BillingMetricAggregateEnum::Count => server::AggregationType::Count,
            domain::enums::BillingMetricAggregateEnum::CountDistinct => {
                server::AggregationType::CountDistinct
            }
            domain::enums::BillingMetricAggregateEnum::Latest => server::AggregationType::Latest,
        }
    }

    pub fn domain_to_metering(
        value: domain::enums::BillingMetricAggregateEnum,
    ) -> metering::meter::AggregationType {
        match value {
            domain::enums::BillingMetricAggregateEnum::Sum => metering::meter::AggregationType::Sum,
            domain::enums::BillingMetricAggregateEnum::Min => metering::meter::AggregationType::Min,
            domain::enums::BillingMetricAggregateEnum::Max => metering::meter::AggregationType::Max,
            domain::enums::BillingMetricAggregateEnum::Mean => {
                metering::meter::AggregationType::Mean
            }
            domain::enums::BillingMetricAggregateEnum::Count => {
                metering::meter::AggregationType::Count
            }
            domain::enums::BillingMetricAggregateEnum::CountDistinct => {
                metering::meter::AggregationType::CountDistinct
            }
            domain::enums::BillingMetricAggregateEnum::Latest => {
                metering::meter::AggregationType::Latest
            }
        }
    }
}

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

    use crate::api::shared::mapping::datetime::chrono_to_timestamp;
    use metering_grpc::meteroid::metering::v1 as metering;
    use meteroid_grpc::meteroid::api::billablemetrics::v1::segmentation_matrix::Matrix;
    use meteroid_store::domain;
    use meteroid_store::domain::billable_metrics::{Dimension, SegmentationMatrix};
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
                            factor: factor as f64, // TODO
                            rounding: super::unit_conversion_rounding::domain_to_server(rounding)
                                .into(),
                        }),
                }),
                segmentation_matrix: map_segmentation_matrix(value.segmentation_matrix),
                archived_at: value.archived_at.map(chrono_to_timestamp),
                created_at: Some(chrono_to_timestamp(value.created_at)),
                usage_group_key: value.usage_group_key,
                product_id: value.product_id.map(|x| x.as_proto()),
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
                    aggregation_type: super::aggregation_type::domain_to_server(
                        value.aggregation_type,
                    )
                    .into(),
                    aggregation_key: value.aggregation_key,
                    created_at: Some(chrono_to_timestamp(value.created_at)),
                    archived_at: value.archived_at.map(chrono_to_timestamp),
                },
            ))
        }
    }

    pub fn map_segmentation_matrix_from_server(
        segmentation_matrix: Option<server::SegmentationMatrix>,
    ) -> Option<SegmentationMatrix> {
        segmentation_matrix.and_then(|sm| match sm.matrix {
            Some(Matrix::Single(s)) => Some(SegmentationMatrix::Single(Dimension {
                key: s.dimension.as_ref().unwrap().key.clone(),
                values: s.dimension.as_ref().unwrap().values.clone(),
            })),
            Some(Matrix::Double(d)) => Some(SegmentationMatrix::Double {
                dimension1: Dimension {
                    key: d.dimension1.as_ref().unwrap().key.clone(),
                    values: d.dimension1.as_ref().unwrap().values.clone(),
                },
                dimension2: Dimension {
                    key: d.dimension2.as_ref().unwrap().key.clone(),
                    values: d.dimension2.as_ref().unwrap().values.clone(),
                },
            }),
            Some(Matrix::Linked(l)) => Some(SegmentationMatrix::Linked {
                dimension1_key: l.dimension_key.clone(),
                dimension2_key: l.linked_dimension_key.clone(),
                values: l
                    .values
                    .iter()
                    .map(|(k, v)| (k.clone(), v.values.clone()))
                    .collect::<HashMap<String, Vec<String>>>(),
            }),
            _ => None,
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

    pub fn domain_to_metering(metric: domain::BillableMetric) -> metering::Meter {
        let segmentation: Option<server::SegmentationMatrix> =
            map_segmentation_matrix(metric.segmentation_matrix);

        let mut group_by = segmentation
            .and_then(|s| s.matrix)
            .map(|matrix| match matrix {
                // TODO improve the asref & cloning
                Matrix::Single(s) => s
                    .dimension
                    .as_ref()
                    .iter()
                    .map(|d| d.key.clone())
                    .collect::<Vec<String>>(),
                Matrix::Double(d) => {
                    let mut vec = d
                        .dimension1
                        .as_ref()
                        .iter()
                        .map(|d| d.key.clone())
                        .collect::<Vec<String>>();
                    vec.extend(
                        d.dimension2
                            .as_ref()
                            .iter()
                            .map(|d| d.key.clone())
                            .collect::<Vec<String>>(),
                    );
                    vec
                }
                Matrix::Linked(l) => {
                    vec![l.dimension_key, l.linked_dimension_key]
                }
            })
            .unwrap_or_default();

        if let Some(usage_key) = metric.usage_group_key
            && !usage_key.is_empty()
        {
            group_by.push(usage_key);
        }

        metering::Meter {
            id: metric.id.as_proto(), // we could allow optional external_id if the metric is defined externally
            code: metric.code.to_string(),
            aggregation_key: metric.aggregation_key,
            aggregation: super::aggregation_type::domain_to_metering(metric.aggregation_type)
                .into(),
            dimensions: group_by,
        }
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
        }
    }
}
