use meteroid_store::domain;

use super::model::*;

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
        usage_group_key: metric.usage_group_key,
        product_family_id: metric.product_family_id,
        product_id: metric.product_id,
        created_at: metric.created_at,
        archived_at: metric.archived_at,
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
        created_at: meta.created_at,
        archived_at: meta.archived_at,
    }
}
