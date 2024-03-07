pub mod aggregation_type {
    use metering_grpc::meteroid::metering::v1 as metering;
    use meteroid_grpc::meteroid::api::billablemetrics::v1::aggregation as server;
    use meteroid_repository as db;

    pub fn server_to_db(e: server::AggregationType) -> db::BillingMetricAggregateEnum {
        match e {
            server::AggregationType::Sum => db::BillingMetricAggregateEnum::SUM,
            server::AggregationType::Min => db::BillingMetricAggregateEnum::MIN,
            server::AggregationType::Max => db::BillingMetricAggregateEnum::MAX,
            server::AggregationType::Mean => db::BillingMetricAggregateEnum::MEAN,
            server::AggregationType::Count => db::BillingMetricAggregateEnum::COUNT,
            server::AggregationType::CountDistinct => {
                db::BillingMetricAggregateEnum::COUNT_DISTINCT
            }
            server::AggregationType::Latest => db::BillingMetricAggregateEnum::LATEST,
        }
    }

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

    pub fn db_to_metering(e: db::BillingMetricAggregateEnum) -> metering::meter::AggregationType {
        match e {
            db::BillingMetricAggregateEnum::SUM => metering::meter::AggregationType::Sum,
            db::BillingMetricAggregateEnum::MIN => metering::meter::AggregationType::Min,
            db::BillingMetricAggregateEnum::MAX => metering::meter::AggregationType::Max,
            db::BillingMetricAggregateEnum::MEAN => metering::meter::AggregationType::Mean,
            db::BillingMetricAggregateEnum::COUNT => metering::meter::AggregationType::Count,
            db::BillingMetricAggregateEnum::COUNT_DISTINCT => {
                metering::meter::AggregationType::CountDistinct
            }
            db::BillingMetricAggregateEnum::LATEST => metering::meter::AggregationType::Latest,
        }
    }
}

pub mod unit_conversion_rounding {

    use meteroid_grpc::meteroid::api::billablemetrics::v1::aggregation::unit_conversion as server;
    use meteroid_repository as db;

    pub fn server_to_db(e: server::UnitConversionRounding) -> db::UnitConversionRoundingEnum {
        match e {
            server::UnitConversionRounding::None => db::UnitConversionRoundingEnum::NONE,
            server::UnitConversionRounding::Up => db::UnitConversionRoundingEnum::UP,
            server::UnitConversionRounding::Down => db::UnitConversionRoundingEnum::DOWN,
            server::UnitConversionRounding::Nearest => db::UnitConversionRoundingEnum::NEAREST,
        }
    }

    pub fn db_to_server(e: db::UnitConversionRoundingEnum) -> server::UnitConversionRounding {
        match e {
            db::UnitConversionRoundingEnum::NONE => server::UnitConversionRounding::None,
            db::UnitConversionRoundingEnum::UP => server::UnitConversionRounding::Up,
            db::UnitConversionRoundingEnum::DOWN => server::UnitConversionRounding::Down,
            db::UnitConversionRoundingEnum::NEAREST => server::UnitConversionRounding::Nearest,
            _ => server::UnitConversionRounding::None, // TODO drop the extra types for now
        }
    }
}

pub mod metric {

    use meteroid_grpc::meteroid::api::billablemetrics::v1 as server;
    use meteroid_repository::billable_metrics as db;

    use metering_grpc::meteroid::metering::v1 as metering;
    use meteroid_grpc::meteroid::api::billablemetrics::v1::segmentation_matrix::Matrix;
    use meteroid_grpc::meteroid::api::billablemetrics::v1::SegmentationMatrix;

    use crate::api::shared::mapping::datetime::datetime_to_timestamp;
    pub fn db_to_server(metric: db::BillableMetric) -> server::BillableMetric {
        server::BillableMetric {
            id: metric.id.to_string(),
            name: metric.name,
            code: metric.code,
            description: metric.description,
            aggregation: Some(server::Aggregation {
                aggregation_type: super::aggregation_type::db_to_server(metric.aggregation_type)
                    .into(),
                aggregation_key: metric.aggregation_key,
                unit_conversion: metric
                    .unit_conversion_factor
                    .zip(metric.unit_conversion_rounding)
                    .map(|(factor, rounding)| server::aggregation::UnitConversion {
                        factor: factor as f64, // TODO
                        rounding: super::unit_conversion_rounding::db_to_server(rounding).into(),
                    }),
            }),
            segmentation_matrix: metric
                .segmentation_matrix
                .map(|s| serde_json::from_value(s).unwrap()),
            archived_at: metric.archived_at.map(datetime_to_timestamp),
            created_at: Some(datetime_to_timestamp(metric.created_at)),
            usage_group_key: metric.usage_group_key,
        }
    }

    pub fn db_to_metering(metric: db::BillableMetric) -> metering::Meter {
        let segmentation: Option<SegmentationMatrix> = metric
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
            aggregation: super::aggregation_type::db_to_metering(metric.aggregation_type).into(),
            dimensions,
        }
    }

    pub fn list_db_to_server(metric: db::ListBillableMetrics) -> server::BillableMetricMeta {
        server::BillableMetricMeta {
            id: metric.id.to_string(),
            name: metric.name,
            code: metric.code,
            aggregation_type: super::aggregation_type::db_to_server(metric.aggregation_type).into(),
            aggregation_key: metric.aggregation_key,
            archived_at: metric.archived_at.map(datetime_to_timestamp),
            created_at: Some(datetime_to_timestamp(metric.created_at)),
        }
    }
}
