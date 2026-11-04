//! How a metric's usage is broken down and added up: the dimensions it is always read by, the
//! canonical key of a group of dimension values, and which aggregations add up across windows.
use crate::domain::BillableMetric;
use crate::domain::billable_metrics::SegmentationMatrix;
use crate::domain::enums::BillingMetricAggregateEnum;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const MAX_GROUP_KEY_BYTES: usize = 1024;

/// Only aggregations that add up across windows can be folded.
pub const fn foldable(aggregation: &BillingMetricAggregateEnum) -> bool {
    matches!(
        aggregation,
        BillingMetricAggregateEnum::Sum | BillingMetricAggregateEnum::Count
    )
}

/// Empty without dimensions, else the dimensions as JSON with sorted keys.
pub fn group_key(dimensions: &HashMap<String, String>) -> String {
    if dimensions.is_empty() {
        return String::new();
    }
    let sorted = dimensions.iter().collect::<BTreeMap<_, _>>();
    serde_json::to_string(&sorted).unwrap_or_default()
}

pub fn dimensions_of(key: &str) -> HashMap<String, String> {
    if key.is_empty() {
        return HashMap::new();
    }
    serde_json::from_str(key).unwrap_or_default()
}

/// The dimensions a metric's usage is always broken down by: its matrix and its group key.
pub fn metric_dimensions(metric: &BillableMetric) -> BTreeSet<String> {
    dimension_set(
        metric.segmentation_matrix.as_ref(),
        metric.usage_group_key.as_deref(),
    )
}

fn dimension_set(
    matrix: Option<&SegmentationMatrix>,
    usage_group_key: Option<&str>,
) -> BTreeSet<String> {
    let matrix_keys: Vec<&str> = match matrix {
        None => Vec::new(),
        Some(SegmentationMatrix::Single(dimension)) => vec![dimension.key.as_str()],
        Some(SegmentationMatrix::Double {
            dimension1,
            dimension2,
        }) => vec![dimension1.key.as_str(), dimension2.key.as_str()],
        Some(SegmentationMatrix::Linked {
            dimension1_key,
            dimension2_key,
            ..
        }) => vec![dimension1_key.as_str(), dimension2_key.as_str()],
    };
    matrix_keys
        .into_iter()
        .chain(usage_group_key)
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grouping_of(matrix: Option<&SegmentationMatrix>, usage_group_key: Option<&str>) -> String {
        dimension_set(matrix, usage_group_key)
            .into_iter()
            .collect::<Vec<_>>()
            .join(",")
    }

    fn dims(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn grouping_names_the_dimensions_sorted_and_deduplicated() {
        use crate::domain::billable_metrics::Dimension;
        let dimension = |key: &str| Dimension {
            key: key.to_owned(),
            values: vec![],
        };
        assert_eq!(grouping_of(None, None), "");
        assert_eq!(grouping_of(None, Some("api_key")), "api_key");
        let double = SegmentationMatrix::Double {
            dimension1: dimension("region"),
            dimension2: dimension("model"),
        };
        assert_eq!(grouping_of(Some(&double), None), "model,region");
        assert_eq!(
            grouping_of(Some(&double), Some("api_key")),
            "api_key,model,region"
        );
        assert_eq!(grouping_of(Some(&double), Some("model")), "model,region");
        let single = SegmentationMatrix::Single(dimension("region"));
        assert_eq!(grouping_of(Some(&single), None), "region");
    }

    #[test]
    fn only_additive_aggregations_fold() {
        assert!(foldable(&BillingMetricAggregateEnum::Sum));
        assert!(foldable(&BillingMetricAggregateEnum::Count));
        assert!(!foldable(&BillingMetricAggregateEnum::Max));
        assert!(!foldable(&BillingMetricAggregateEnum::CountDistinct));
        assert!(!foldable(&BillingMetricAggregateEnum::Latest));
    }

    #[test]
    fn group_keys_are_canonical() {
        assert_eq!(group_key(&HashMap::new()), "");
        let a = group_key(&dims(&[("region", "eu"), ("model", "x")]));
        let b = group_key(&dims(&[("model", "x"), ("region", "eu")]));
        assert_eq!(a, b);
        assert_eq!(dimensions_of(&a), dims(&[("region", "eu"), ("model", "x")]));
    }
}
