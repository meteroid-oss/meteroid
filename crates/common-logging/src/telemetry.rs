use common_config::telemetry::TelemetryConfig;

pub fn init(cfg: &TelemetryConfig) {
    cfg.set_otel_env_vars();

    crate::otel::init_otel_tracing_and_logging(cfg.tracing_enabled);

    if cfg.metrics_enabled {
        let _ = crate::otel::init_meter_provider(cfg);
    }
}
