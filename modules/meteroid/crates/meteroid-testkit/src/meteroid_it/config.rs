use std::collections::HashMap;
use std::net::SocketAddr;

use envconfig::Envconfig;

use common_config::telemetry::TelemetryConfig;
use meteroid::config::Config;
use meteroid_mailer::config::MailerConfig;
use meteroid_oauth::config::OauthConfig;

/// The core config integration tests run with: the defaults a deployment gets, except what
/// the variables below set for tests.
pub fn mocked_config(
    postgres_connection_string: String,
    rest_api_addr: SocketAddr,
    meteroid_port: u16,
    metering_port: u16,
) -> Config {
    let vars: HashMap<String, String> = [
        ("DATABASE_URL", postgres_connection_string),
        (
            "METEROID_API_LISTEN_ADDRESS",
            format!("127.0.0.1:{meteroid_port}"),
        ),
        (
            "METEROID_REST_API_LISTEN_ADDRESS",
            rest_api_addr.to_string(),
        ),
        (
            "METEROID_REST_API_EXTERNAL_URL",
            format!("http://127.0.0.1:{}", rest_api_addr.port()),
        ),
        (
            "METERING_API_EXTERNAL_URL",
            format!("http://127.0.0.1:{metering_port}"),
        ),
        ("METEROID_PUBLIC_URL", "http://localhost:8080".to_owned()),
        ("OBJECT_STORE_URI", String::new()),
        ("JWT_SECRET", "secret".to_owned()),
        ("INTERNAL_API_SECRET", "secret".to_owned()),
        ("SECRETS_CRYPT_KEY", "0".repeat(32)),
        ("IDEMPOTENCY_TTL", "5s".to_owned()),
        ("RECONCILIATION_ENABLED", "false".to_owned()),
        // Off in tests: the sweep would race fixtures that assert on a draft's stored lines.
        ("DRAFT_REFRESH_ENABLED", "false".to_owned()),
        // Off in tests: sweeps are driven explicitly with a bound `now`.
        ("PAYMENT_REMINDERS_ENABLED", "false".to_owned()),
        ("WEBHOOK_DELIVERY_CONCURRENCY", "4".to_owned()),
        // Test receivers listen on loopback.
        ("WEBHOOK_ALLOW_PRIVATE_DESTINATIONS", "true".to_owned()),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value))
    .collect();

    let mut config = Config::init_from_hashmap(&vars).expect("Invalid test config");
    config.common.telemetry = TelemetryConfig::init_from_env().unwrap();
    config.mailer = MailerConfig::dummy();
    config.oauth = OauthConfig::dummy();
    config.instance_telemetry = Default::default();
    config
}
