use envconfig::Envconfig;

#[derive(Envconfig, Debug, Clone, Default)]
pub struct TelemetryConfig {
    #[envconfig(from = "TELEMETRY_TRACING_ENABLED", default = "false")]
    pub tracing_enabled: bool,

    #[envconfig(from = "TELEMETRY_METRICS_ENABLED", default = "false")]
    pub metrics_enabled: bool,

    #[envconfig(from = "TELEMETRY_OTEL_ENDPOINT", default = "http://127.0.0.1:4317")]
    pub otel_endpoint: String,
}
