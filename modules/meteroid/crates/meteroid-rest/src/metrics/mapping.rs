use meteroid_store::domain;
use meteroid_store::domain::billable_metrics::MetricFilters;

use super::model::*;

/// REST filter list → domain. Empty maps to `None` so nothing is persisted.
pub fn rest_filters_to_domain(filters: Vec<MetricFilter>) -> Option<MetricFilters> {
    if filters.is_empty() {
        None
    } else {
        Some(MetricFilters(filters.into_iter().map(Into::into).collect()))
    }
}

fn domain_filters_to_rest(filters: Option<MetricFilters>) -> Vec<MetricFilter> {
    filters
        .map(|MetricFilters(list)| list.into_iter().map(Into::into).collect())
        .unwrap_or_default()
}

pub fn metric_to_rest(metric: domain::BillableMetric) -> Metric {
    Metric {
        id: metric.id,
        name: metric.name,
        code: metric.code,
        description: metric.description,
        aggregation_type: metric.aggregation_type.into(),
        aggregation_key: metric.aggregation_key,
        unit_conversion: match (
            metric.unit_conversion_factor,
            metric.unit_conversion_rounding,
        ) {
            (Some(factor), Some(rounding)) => Some(UnitConversion {
                factor,
                rounding: rounding.into(),
            }),
            _ => None,
        },
        segmentation_matrix: metric.segmentation_matrix.map(Into::into),
        filters: domain_filters_to_rest(metric.filters),
        usage_group_key: metric.usage_group_key,
        unit_singular: metric.unit_singular,
        unit_plural: metric.unit_plural,
        product_family_id: metric.product_family_id,
        product_id: metric.product_id,
        created_at: metric.created_at.into(),
        archived_at: metric.archived_at.map(Into::into),
    }
}

pub fn metric_meta_to_rest(meta: domain::BillableMetricMeta) -> MetricSummary {
    MetricSummary {
        id: meta.id,
        name: meta.name,
        code: meta.code,
        description: meta.description,
        aggregation_type: meta.aggregation_type.into(),
        aggregation_key: meta.aggregation_key,
        unit_singular: meta.unit_singular,
        unit_plural: meta.unit_plural,
        created_at: meta.created_at.into(),
        archived_at: meta.archived_at.map(Into::into),
    }
}
