use distributed_lock::LeaderElection;
use meteroid_app::config::ParsedCron;
use meteroid_app::services::instance_telemetry::InstanceTelemetryService;
use std::str::FromStr;
use std::sync::Arc;

pub async fn run(telemetry: Arc<InstanceTelemetryService>, elector: Arc<dyn LeaderElection>) {
    telemetry.log_startup();
    let schedule = ParsedCron::from_str("41 * * * *").expect("valid cron");
    super::run_scheduled("telemetry report", schedule, elector, move || {
        let telemetry = telemetry.clone();
        async move { telemetry.tick().await }
    })
    .await;
}
