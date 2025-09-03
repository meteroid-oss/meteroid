use crate::logging::init_regular_logging;
use common_config::telemetry::TelemetryConfig;

pub fn init(cfg: &TelemetryConfig) {
    cfg.set_otel_env_vars();

    if cfg.tracing_enabled {
        crate::otel::init_otel_tracing_and_logging();
    } else {
        init_regular_logging();
    };

    if cfg.metrics_enabled {
        crate::otel::init_meter_provider(cfg);
    }
}
