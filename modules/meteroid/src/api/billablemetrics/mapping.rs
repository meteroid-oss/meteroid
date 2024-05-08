#[deprecated(note = "please use `aggregation_type` mod instead")]
pub mod aggregation_type_old {
    use meteroid_grpc::meteroid::api::billablemetrics::v1::aggregation as server;
    use meteroid_repository as db;

    pub fn db_to_server(e: db::BillingMetricAggregateEnum) -> server::AggregationType {
        match e {
            db::BillingMetricAggregateEnum::SUM => server::AggregationType::Sum,
            db::BillingMetricAggregateEnum::MIN => server::AggregationType::Min,
            db::BillingMetricAggregateEnum::MAX => server::AggregationType::Max,
            db::BillingMetricAggregateEnum::MEAN => server::AggregationType::Mean,
            db::BillingMetricAggregateEnum::COUNT => server::AggregationType::Count,
            db::BillingMetricAggregateEnum::COUNT_DISTINCT => {
                server::AggregationType::CountDistinct
            }
            db::BillingMetricAggregateEnum::LATEST => server::AggregationType::Latest,
        }
    }
}

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

    use metering_grpc::meteroid::metering::v1 as metering;
    use meteroid_grpc::meteroid::api::billablemetrics::v1::segmentation_matrix::Matrix;
    use meteroid_store::domain;
    use meteroid_store::errors::StoreError;

    use crate::api::shared::mapping::datetime::chrono_to_timestamp;

    pub struct ServerBillableMetricWrapper(pub server::BillableMetric);

    impl TryFrom<domain::BillableMetric> for ServerBillableMetricWrapper {
        type Error = Report<StoreError>;

        fn try_from(value: domain::BillableMetric) -> Result<Self, Self::Error> {
            Ok(ServerBillableMetricWrapper(server::BillableMetric {
                id: value.id.to_string(),
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
                segmentation_matrix: value
                    .segmentation_matrix
                    .map(|s| serde_json::from_value(s).unwrap()),
                archived_at: value.archived_at.map(chrono_to_timestamp),
                created_at: Some(chrono_to_timestamp(value.created_at)),
                usage_group_key: value.usage_group_key,
            }))
        }
    }

    pub struct ServerBillableMetricMetaWrapper(pub server::BillableMetricMeta);
    impl TryFrom<domain::BillableMetricMeta> for ServerBillableMetricMetaWrapper {
        type Error = Report<StoreError>;

        fn try_from(value: domain::BillableMetricMeta) -> Result<Self, Self::Error> {
            Ok(ServerBillableMetricMetaWrapper(
                server::BillableMetricMeta {
                    id: value.id.to_string(),
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

    pub fn domain_to_metering(metric: domain::BillableMetric) -> metering::Meter {
        let segmentation: Option<server::SegmentationMatrix> = metric
            .segmentation_matrix
            .map(|s| serde_json::from_value(s).unwrap()); // TODO not raw json

        let dimensions = segmentation
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
            .unwrap_or(vec![]);

        metering::Meter {
            meter_slug: metric.id.to_string(), // TODO slug would make it easier for external metering
            event_name: metric.code.to_string(),
            aggregation_key: metric.aggregation_key,
            aggregation: super::aggregation_type::domain_to_metering(metric.aggregation_type)
                .into(),
            dimensions,
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
