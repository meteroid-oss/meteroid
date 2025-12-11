use o2o::o2o;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(o2o, Serialize, Deserialize, Debug, Clone, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::enums::BillingMetricAggregateEnum)]
pub enum BillingMetricAggregateEnum {
    Count,
    Latest,
    Max,
    Min,
    Mean,
    Sum,
    CountDistinct,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::enums::UnitConversionRoundingEnum)]
pub enum UnitConversionRoundingEnum {
    Up,
    Down,
    Nearest,
    NearestHalf,
    NearestDecile,
    None,
}

#[derive(Clone, Debug, Serialize, o2o, utoipa::ToSchema)]
#[from_owned(meteroid_store::domain::billable_metrics::Dimension)]
pub struct MetricDimension {
    pub key: String,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Serialize, o2o, utoipa::ToSchema)]
#[serde(tag = "discriminator", rename_all = "SCREAMING_SNAKE_CASE")]
#[from_owned(meteroid_store::domain::billable_metrics::SegmentationMatrix)]
pub enum MetricSegmentationMatrix {
    Single(#[from(~.into())] MetricDimension),
    Double {
        #[from(~.into())]
        dimension1: MetricDimension,
        #[from(~.into())]
        dimension2: MetricDimension,
    },
    Linked {
        dimension1_key: String,
        dimension2_key: String,
        values: HashMap<String, Vec<String>>,
    },
}
