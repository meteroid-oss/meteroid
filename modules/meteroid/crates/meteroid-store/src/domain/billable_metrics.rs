use super::enums::{BillingMetricAggregateEnum, UnitConversionRoundingEnum};
use crate::errors::StoreError;
use chrono::NaiveDateTime;
use error_stack::Report;
use std::collections::HashMap;

use diesel_models::billable_metrics::{BillableMetricMetaRow, BillableMetricRow};
use o2o::o2o;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, o2o)]
#[try_map_owned(BillableMetricRow, Report<StoreError>)]
pub struct BillableMetric {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub code: String,
    #[map(~.into())]
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    #[map(~.map(| x | x.into()))]
    pub unit_conversion_rounding: Option<UnitConversionRoundingEnum>,
    #[into(
        ~.map(| x | serde_json::to_value(& x).map_err(| e | {
        StoreError::SerdeError("Failed to serialize segmentation_matrix".to_string(), e)
        }))
        .transpose() ?
    )]
    #[from(
        ~.map(| x | serde_json::from_value(x).map_err(| e | {
        StoreError::SerdeError("Failed to deserialize segmentation_matrix".to_string(), e)
        }))
        .transpose() ?
    )]
    pub segmentation_matrix: Option<SegmentationMatrix>,
    pub usage_group_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
    pub product_family_id: Uuid,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dimension {
    pub key: String,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SegmentationMatrix {
    Single(Dimension),
    Double {
        dimension1: Dimension,
        dimension2: Dimension,
    },
    Linked {
        dimension1_key: String,
        dimension2_key: String,
        values: HashMap<String, Vec<String>>,
    },
}

#[derive(Clone, Debug)]
pub struct BillableMetricNew {
    pub name: String,
    pub description: Option<String>,
    pub code: String,
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    pub unit_conversion_rounding: Option<UnitConversionRoundingEnum>,
    pub segmentation_matrix: Option<SegmentationMatrix>,
    pub usage_group_key: Option<String>,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub family_external_id: String,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(BillableMetricMetaRow)]
#[owned_into(BillableMetricMetaRow)]
pub struct BillableMetricMeta {
    pub id: Uuid,
    pub name: String,
    pub code: String,
    #[map(~.into())]
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
}
