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

impl TelemetryConfig {
    pub fn set_otel_env_vars(&self) {
        if self.tracing_enabled || self.metrics_enabled {
            // setting env variables needed by [init_tracing_opentelemetry]
            //   see also
            //     https://lib.rs/crates/init-tracing-opentelemetry
            //     https://opentelemetry.io/docs/concepts/sdk-configuration/otlp-exporter-configuration/
            //
            // todo fix unsafe code, move these env vars to .env?
            unsafe {
                std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", self.otel_endpoint.clone());
                std::env::set_var("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL", "grpc");
                std::env::set_var("OTEL_SERVICE_NAME", self.get_service_name());
            }
        }
    }

    fn get_service_name(&self) -> String {
        // todo: better way to get service name?
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or("unknown_service".to_string())
    }
}
