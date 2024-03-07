pub mod subscriptions {
    use crate::api::shared;
    use meteroid_grpc::meteroid::api::subscriptions::v1 as proto;
    use meteroid_repository::subscriptions as db;

    use crate::services::subscription::ext::DbSubscriptionExt;
    use tonic::Status;

    pub fn db_to_proto(s: db::Subscription) -> Result<proto::Subscription, Status> {
        let parameters_decoded: proto::SubscriptionParameters =
            serde_json::from_value(s.input_parameters.clone())
                .map_err(|e| Status::internal(format!("Failed to decode parameters: {}", e)))?;

        let status = *(&s.status_proto()?) as i32;

        Ok(proto::Subscription {
            id: s.id.to_string(),
            tenant_id: s.tenant_id.to_string(),
            customer_id: s.customer_id.to_string(),
            plan_id: s.plan_id.to_string(),
            plan_name: s.plan_name.to_string(),
            plan_version_id: s.plan_version_id.to_string(),
            parameters: Some(parameters_decoded),
            net_terms: s.net_terms,
            currency: s.currency,
            version: s.version as u32,
            billing_end_date: s.billing_end_date.map(shared::mapping::date::to_proto),
            billing_start_date: Some(shared::mapping::date::to_proto(s.billing_start_date)),
            customer_name: s.customer_name,
            status,
            canceled_at: s
                .canceled_at
                .map(shared::mapping::datetime::datetime_to_timestamp),
        })
    }

    pub fn list_db_to_proto(s: db::SubscriptionList) -> Result<proto::Subscription, Status> {
        let parameters_decoded: proto::SubscriptionParameters =
            serde_json::from_value(s.input_parameters.clone())
                .map_err(|e| Status::internal(format!("Failed to decode parameters: {}", e)))?;

        let status = *(&s.status_proto()?) as i32;

        Ok(proto::Subscription {
            id: s.subscription_id.to_string(),
            tenant_id: s.tenant_id.to_string(),
            customer_id: s.customer_id.to_string(),
            plan_id: s.plan_id.to_string(),
            plan_name: s.plan_name.to_string(),
            plan_version_id: s.plan_version_id.to_string(),
            parameters: Some(parameters_decoded),
            net_terms: s.net_terms,
            currency: s.currency,
            version: s.version as u32,
            billing_end_date: s.billing_end_date.map(shared::mapping::date::to_proto),
            billing_start_date: Some(shared::mapping::date::to_proto(s.billing_start_date)),
            customer_name: s.customer_name,
            status,
            canceled_at: s
                .canceled_at
                .map(shared::mapping::datetime::datetime_to_timestamp),
        })
    }
}
