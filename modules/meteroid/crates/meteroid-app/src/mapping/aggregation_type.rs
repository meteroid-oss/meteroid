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
        domain::enums::BillingMetricAggregateEnum::Mean => metering::meter::AggregationType::Mean,
        domain::enums::BillingMetricAggregateEnum::Count => metering::meter::AggregationType::Count,
        domain::enums::BillingMetricAggregateEnum::CountDistinct => {
            metering::meter::AggregationType::CountDistinct
        }
        domain::enums::BillingMetricAggregateEnum::Latest => {
            metering::meter::AggregationType::Latest
        }
    }
}
