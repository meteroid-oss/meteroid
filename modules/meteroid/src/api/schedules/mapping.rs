pub mod schedules {
    use crate::api::shared::mapping::period::billing_period_to_server;
    use meteroid_grpc::meteroid::api::schedules::v1 as grpc;
    use meteroid_repository as db;
    use tonic::Status;

    pub fn db_to_server(schedule: db::schedules::Schedule) -> Result<grpc::Schedule, Status> {
        let ramps_decoded: grpc::PlanRamps = serde_json::from_value(schedule.ramps)
            .map_err(|e| Status::internal(format!("Failed to decode ramps: {}", e)))?;
        Ok(grpc::Schedule {
            id: schedule.id.to_string(),
            term: billing_period_to_server(&schedule.billing_period) as i32,
            name: "".to_string(), // TODO drop from db ?
            ramps: Some(ramps_decoded),
        })
    }
}
