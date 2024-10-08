use envconfig::Envconfig;

use crate::telemetry::TelemetryConfig;

#[derive(Envconfig, Debug, Clone)]
pub struct CommonConfig {
    #[envconfig(nested)]
    pub telemetry: TelemetryConfig,
}
