use super::enums::{BillingMetricAggregateEnum, UnitConversionRoundingEnum};
use crate::errors::StoreErrorReport;
use chrono::NaiveDateTime;
use std::collections::HashMap;

use crate::json_value_serde;
use common_domain::ids::{BillableMetricId, ProductFamilyId, ProductId, TenantId};
use diesel_models::billable_metrics::{BillableMetricMetaRow, BillableMetricRow};
use o2o::o2o;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, o2o)]
#[try_map_owned(BillableMetricRow, StoreErrorReport)]
pub struct BillableMetric {
    pub id: BillableMetricId,
    pub name: String,
    pub description: Option<String>,
    pub code: String,
    #[map(~.into())]
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    #[map(~.map(| x | x.into()))]
    pub unit_conversion_rounding: Option<UnitConversionRoundingEnum>,
    #[map(~.map(| x | x.try_into()).transpose()?)]
    pub segmentation_matrix: Option<SegmentationMatrix>,
    #[map(~.map(| x | x.try_into()).transpose()?)]
    pub filters: Option<MetricFilters>,
    pub usage_group_key: Option<String>,
    pub unit_singular: Option<String>,
    pub unit_plural: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub product_family_id: ProductFamilyId,
    pub product_id: Option<ProductId>,
    /// The events of the customer the metric counts when read for a subscription.
    #[ghost({crate::domain::usage_attribution::UsageScope::default()})]
    pub usage_scope: crate::domain::usage_attribution::UsageScope,
}

/// What metering aggregates for a metric: a saved billable metric, or a draft from the editor.
#[derive(Clone, Debug)]
pub struct MetricDefinition {
    pub code: String,
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    pub segmentation_matrix: Option<SegmentationMatrix>,
    pub filters: Option<MetricFilters>,
}

impl From<&BillableMetric> for MetricDefinition {
    fn from(metric: &BillableMetric) -> Self {
        Self {
            code: metric.code.clone(),
            aggregation_type: metric.aggregation_type,
            aggregation_key: metric.aggregation_key.clone(),
            unit_conversion_factor: metric.unit_conversion_factor,
            segmentation_matrix: metric.segmentation_matrix.clone(),
            filters: metric.filters.clone(),
        }
    }
}

impl MetricDefinition {
    /// Raw usage in the metric's billed unit, before the rounding that applies to a period total.
    pub fn unrounded_units(&self, units: rust_decimal::Decimal) -> rust_decimal::Decimal {
        crate::services::invoice_lines::component::unrounded_units(
            self.unit_conversion_factor,
            units,
        )
    }

    /// The matrix as a filter, once every dimension lists values: an editor draft with a
    /// dimension still empty restricts nothing yet.
    pub fn complete_segmentation_matrix(&self) -> Option<&SegmentationMatrix> {
        self.segmentation_matrix
            .as_ref()
            .filter(|matrix| match matrix {
                SegmentationMatrix::Single(dimension) => !dimension.values.is_empty(),
                SegmentationMatrix::Double {
                    dimension1,
                    dimension2,
                } => !dimension1.values.is_empty() && !dimension2.values.is_empty(),
                SegmentationMatrix::Linked { values, .. } => !values.is_empty(),
            })
    }
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

json_value_serde!(SegmentationMatrix);

/// UI-facing operator of a [`MetricFilter`]. `Equal`/`NotEqual` are the single-value
/// forms of `In`/`NotIn`; all four collapse to In/NotIn at the query-engine boundary.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetricFilterOp {
    Equal,
    NotEqual,
    In,
    NotIn,
}

impl MetricFilterOp {
    /// True for the negation operators (`NotEqual`/`NotIn`) — those whose engine semantics
    /// are presence-required (an event missing the property is excluded).
    pub fn is_negation(self) -> bool {
        matches!(self, MetricFilterOp::NotEqual | MetricFilterOp::NotIn)
    }
}

/// A pre-aggregation filter on an event property. Restricts which raw events feed the
/// metric's aggregation (a WHERE-only condition), it does NOT split the result the way a
/// segmentation dimension does. Filters are ANDed together.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetricFilter {
    pub property: String,
    pub op: MetricFilterOp,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MetricFilters(pub Vec<MetricFilter>);

json_value_serde!(MetricFilters);

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
    pub filters: Option<MetricFilters>,
    pub usage_group_key: Option<String>,
    pub unit_label: Option<MetricUnitLabel>,
    pub tenant_id: TenantId,
    pub product_family_id: ProductFamilyId,
    pub product_id: Option<ProductId>,
}

#[derive(Clone, Debug)]
pub struct BillableMetricUpdate {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub unit_conversion_factor: Option<Option<i32>>,
    pub unit_conversion_rounding: Option<Option<UnitConversionRoundingEnum>>,
    pub segmentation_matrix: Option<Option<SegmentationMatrix>>,
    pub filters: Option<Option<MetricFilters>>,
    pub unit_label: Option<Option<MetricUnitLabel>>,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(BillableMetricMetaRow)]
#[owned_into(BillableMetricMetaRow)]
pub struct BillableMetricMeta {
    pub id: BillableMetricId,
    pub name: String,
    pub code: String,
    pub description: Option<String>,
    #[map(~.into())]
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    /// Raw segmentation matrix JSON; its dimension keys are the properties this
    /// usage metric can be sliced by (surfaced as analytics breakdowns).
    pub segmentation_matrix: Option<serde_json::Value>,
    /// Raw filters JSON (array of `MetricFilter`); echoed into the analytics
    /// custom-metric so a saved metric explored in Reports is filtered like billing.
    pub filters: Option<serde_json::Value>,
    pub unit_singular: Option<String>,
    pub unit_plural: Option<String>,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
}

pub const MAX_UNIT_LABEL_LEN: usize = 32;

/// What one unit of a metric is called, e.g. "token" / "tokens".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricUnitLabel {
    singular: String,
    plural: String,
}

impl MetricUnitLabel {
    /// Both or neither; a blank label counts as absent.
    pub fn parse(singular: Option<&str>, plural: Option<&str>) -> Result<Option<Self>, String> {
        fn present(label: Option<&str>) -> Option<&str> {
            label.map(str::trim).filter(|l| !l.is_empty())
        }
        match (present(singular), present(plural)) {
            (None, None) => Ok(None),
            (Some(singular), Some(plural)) => {
                for (field, label) in [("unit_singular", singular), ("unit_plural", plural)] {
                    if label.chars().count() > MAX_UNIT_LABEL_LEN {
                        return Err(format!(
                            "{field} cannot exceed {MAX_UNIT_LABEL_LEN} characters"
                        ));
                    }
                    if label.chars().any(char::is_control) {
                        return Err(format!("{field} cannot contain control characters"));
                    }
                }
                Ok(Some(Self {
                    singular: singular.to_owned(),
                    plural: plural.to_owned(),
                }))
            }
            _ => Err("unit_singular and unit_plural must be set together".to_owned()),
        }
    }

    pub fn singular(&self) -> &str {
        &self.singular
    }

    pub fn plural(&self) -> &str {
        &self.plural
    }

    pub fn into_parts(self) -> (String, String) {
        (self.singular, self.plural)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_label_is_both_or_neither_and_trimmed() {
        assert_eq!(MetricUnitLabel::parse(None, None), Ok(None));
        assert_eq!(MetricUnitLabel::parse(Some("  "), Some("")), Ok(None));
        let label = MetricUnitLabel::parse(Some(" token "), Some("tokens\t"))
            .unwrap()
            .unwrap();
        assert_eq!((label.singular(), label.plural()), ("token", "tokens"));
        assert!(MetricUnitLabel::parse(Some("token"), None).is_err());
        assert!(MetricUnitLabel::parse(Some(""), Some("tokens")).is_err());
        assert!(MetricUnitLabel::parse(Some(&"t".repeat(33)), Some("tokens")).is_err());
        assert!(MetricUnitLabel::parse(Some("tok\u{0}en"), Some("tokens")).is_err());
    }
}
