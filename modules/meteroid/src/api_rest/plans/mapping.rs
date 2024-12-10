use crate::api_rest::plans::model::Plan;
use meteroid_store::domain;

pub fn domain_to_rest(d: domain::PlanOverview) -> Plan {
    Plan {
        local_id: d.local_id,
        name: d.name,
        description: d.description,
        created_at: d.created_at,
        plan_type: d.plan_type.to_string(),
        status: d.status.to_string(),
        product_family_name: d.product_family_name,
        product_family_id: d.product_family_local_id,
        has_draft_version: d.has_draft_version,
        subscription_count: d.subscription_count,
    }
}
