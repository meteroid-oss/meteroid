use envconfig::Envconfig;

use crate::telemetry::TelemetryConfig;

#[derive(Envconfig, Debug, Clone)]
pub struct CommonConfig {
    #[envconfig(nested = true)]
    pub telemetry: TelemetryConfig,
}
