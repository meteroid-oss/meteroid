use croner::Cron;
use envconfig::Envconfig;
use secrecy::SecretString;
use std::net::SocketAddr;
use std::str::FromStr;

use common_config::auth::InternalAuthConfig;
use common_config::common::CommonConfig;
use common_config::idempotency::IdempotencyConfig;
use common_config::instance_telemetry::InstanceTelemetryConfig;
use common_domain::ids::OrganizationId;
use meteroid_mailer::config::MailerConfig;
use meteroid_oauth::config::OauthConfig;
use meteroid_store::store::PgConfig;

#[derive(Envconfig, Debug, Clone)]
pub struct Config {
    /// The gRPC and REST APIs only.
    #[envconfig(nested)]
    pub api: ApiConfig,

    /// The workers only.
    #[envconfig(nested)]
    pub workers: WorkersConfig,

    #[envconfig(nested)]
    pub pg: PgConfig,

    #[envconfig(from = "METEROID_PUBLIC_URL", default = "https://app.meteroid.com")]
    pub public_url: String,

    #[envconfig(from = "METEROID_MFA_ISSUER", default = "Meteroid")]
    pub mfa_issuer: String,

    #[envconfig(from = "METERING_API_EXTERNAL_URL")]
    pub metering_endpoint: String,

    #[envconfig(from = "OBJECT_STORE_URI")]
    pub object_store_uri: String,

    #[envconfig(from = "OBJECT_STORE_PREFIX")]
    pub object_store_prefix: Option<String>,

    #[envconfig(
        from = "METEROID_REST_API_EXTERNAL_URL",
        default = "http://127.0.0.1:8080"
    )]
    pub rest_api_external_url: String,

    /// Public base URL for inbound provider webhooks; unset → `rest_api_external_url`. Only needed
    /// when the REST base isn't public (e.g. local dev).
    #[envconfig(from = "METEROID_WEBHOOK_EXTERNAL_URL")]
    pub webhook_external_url: Option<String>,

    #[envconfig(from = "OPENEXCHANGERATES_API_KEY")]
    pub openexchangerates_api_key: Option<String>,

    #[envconfig(nested)]
    pub common: CommonConfig,

    #[envconfig(nested)]
    pub internal_auth: InternalAuthConfig,

    #[envconfig(nested)]
    pub idempotency: IdempotencyConfig,

    #[envconfig(nested)]
    pub instance_telemetry: InstanceTelemetryConfig,

    #[envconfig(from = "JWT_SECRET")]
    pub jwt_secret: SecretString,

    #[envconfig(from = "ENABLE_MULTI_ORGANIZATION", default = "false")]
    pub multi_organization_enabled: bool,

    #[envconfig(
        from = "SECRETS_CRYPT_KEY",
        default = "00000000000000000000000000000000"
    )]
    pub secrets_crypt_key: CryptKey,

    #[envconfig(nested)]
    pub webhook_out: WebhookOutConfig,

    #[envconfig(nested)]
    pub mailer: MailerConfig,

    #[envconfig(nested)]
    pub oauth: OauthConfig,

    #[envconfig(from = "DOMAINS_WHITELIST")]
    pub domains_whitelist: Option<DomainWhitelist>,

    #[envconfig(from = "ADMIN_ORGANIZATION_ID")]
    pub admin_organization_id: Option<OrganizationId>,

    #[envconfig(from = "ORGANIZATION_INVITE_TTL_DAYS", default = "7")]
    pub invite_ttl_days: u32,

    #[envconfig(nested)]
    pub redis: RedisConfig,

    #[envconfig(nested)]
    pub instance_bootstrap: InstanceBootstrapConfig,
}

#[derive(Envconfig, Debug, Clone)]
pub struct ApiConfig {
    #[envconfig(from = "METEROID_API_LISTEN_ADDRESS")]
    pub grpc_listen_addr: SocketAddr,

    #[envconfig(from = "METEROID_REST_API_LISTEN_ADDRESS", default = "127.0.0.1:8080")]
    pub rest_api_addr: SocketAddr,

    // Trailing X-Forwarded-For entries to skip when resolving the client IP for
    // security-audit logs. 0 = single trusted reverse proxy (Traefik, nginx
    // ingress): take the rightmost (proxy-observed) entry. Increase by one per
    // additional trusted proxy chained in front of it.
    #[envconfig(from = "METEROID_TRUSTED_PROXY_HOPS", default = "0")]
    pub trusted_proxy_hops: usize,

    #[envconfig(nested)]
    pub rate_limit: RateLimitConfig,
}

#[derive(Envconfig, Debug, Clone)]
pub struct WorkersConfig {
    // Kill switch for the payment reconciliation worker (polls providers for
    // stuck-Pending transactions). On by default; a single elected replica runs
    // it, set false to stop it entirely.
    #[envconfig(from = "RECONCILIATION_ENABLED", default = "true")]
    pub reconciliation_enabled: bool,

    // Tenant hard-deletion worker schedule; defaults to weekly, Fridays 03:00 UTC.
    #[envconfig(from = "TENANT_DELETION_CLEANUP_CRON", default = "0 3 * * FRI")]
    pub tenant_deletion_cleanup_cron: ParsedCron,

    // VIES re-validation enqueuer schedule; defaults to daily, 04:00 UTC.
    #[envconfig(from = "VAT_REVALIDATION_CRON", default = "0 4 * * *")]
    pub vat_revalidation_cron: ParsedCron,

    // Stale-draft invoice refresh schedule; defaults to hourly at :25, off the
    // hour boundary the billing runs and the milestones worker already sit on.
    #[envconfig(from = "DRAFT_REFRESH_CRON", default = "25 * * * *")]
    pub draft_refresh_cron: ParsedCron,

    // Kill switch for the stale-draft invoice refresh worker.
    #[envconfig(from = "DRAFT_REFRESH_ENABLED", default = "true")]
    pub draft_refresh_enabled: bool,

    // Payment reminder sweeper schedule (due steps + enrolment), run by the dunning module;
    // execution is further gated to each entity's local send window, so the cron only bounds
    // lateness.
    #[envconfig(from = "PAYMENT_REMINDER_SWEEP_CRON", default = "*/15 * * * *")]
    pub payment_reminder_sweep_cron: ParsedCron,

    // Kill switch for the payment reminder sweeper.
    #[envconfig(from = "PAYMENT_REMINDERS_ENABLED", default = "true")]
    pub payment_reminders_enabled: bool,
}

impl Config {
    /// Called once by each binary, which passes the config down. Editions load their own
    /// settings separately.
    pub fn load() -> Config {
        Config::init_from_env().expect("Invalid configuration")
    }

    /// Call once logging is up: the config is loaded before it.
    pub fn log_ignored_settings(ignored: &[&'static str]) {
        if !ignored.is_empty() {
            tracing::warn!("Settings ignored by this edition: {}", ignored.join(", "));
        }
    }

    pub fn mailer_enabled(&self) -> bool {
        self.mailer.smtp_enabled()
    }

    pub fn domains_whitelist(&self) -> Vec<String> {
        match &self.domains_whitelist {
            Some(DomainWhitelist(domains)) => domains.clone(),
            None => vec![],
        }
    }
}

#[derive(Debug, Clone)]
pub struct DomainWhitelist(pub Vec<String>);
impl FromStr for DomainWhitelist {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let domains: Vec<String> = s
            .split(',')
            .map(|d| d.trim().to_lowercase().to_string())
            .collect();
        if domains.is_empty() {
            return Err("Domain whitelist cannot be empty".to_string());
        }
        Ok(DomainWhitelist(domains))
    }
}

#[derive(Debug, Clone)]
pub struct CryptKey(pub SecretString);
impl FromStr for CryptKey {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 32 {
            return Err("Crypt key must be exactly 32 characters long".to_string());
        }
        Ok(CryptKey(SecretString::from(s.to_string())))
    }
}

/// Cron validated at construction, unlike croner's lazy `Cron: FromStr`, so
/// envconfig rejects a bad schedule at config load.
#[derive(Debug, Clone)]
pub struct ParsedCron(pub Cron);
impl FromStr for ParsedCron {
    type Err = croner::errors::CronError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(ParsedCron(Cron::new(s).parse()?))
    }
}
impl std::ops::Deref for ParsedCron {
    type Target = Cron;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Envconfig, Debug, Clone)]
pub struct WebhookOutConfig {
    #[envconfig(from = "WEBHOOK_DELIVERY_CONCURRENCY", default = "32")]
    pub delivery_concurrency: usize,

    /// CONNECT proxy for webhooks and alerts, e.g. to send from fixed IPs.
    #[envconfig(from = "WEBHOOK_EGRESS_PROXY_URL")]
    pub egress_proxy_url: Option<String>,

    /// Also route SSO through the proxy. Off by default so logins never depend on it.
    #[envconfig(from = "EGRESS_USE_PROXY_SSO", default = "false")]
    pub sso_use_proxy: bool,

    /// Lets webhooks, alerts and SSO reach private and internal addresses: self-hosters
    /// delivering inside their own network, local dev and tests.
    #[envconfig(from = "WEBHOOK_ALLOW_PRIVATE_DESTINATIONS", default = "false")]
    pub allow_private_destinations: bool,

    /// Overrides `allow_private_destinations` for SSO (OIDC discovery, token calls).
    #[envconfig(from = "EGRESS_ALLOW_PRIVATE_DESTINATIONS_SSO")]
    pub sso_allow_private_destinations: Option<bool>,

    #[envconfig(from = "WEBHOOK_RETENTION_DAYS", default = "30")]
    pub retention_days: RetentionDays,
}

/// Below two days a delivery still inside its retry ladder (about 28 hours) could have
/// its message purged from under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionDays(pub i64);

pub const MIN_RETENTION_DAYS: i64 = 2;

impl FromStr for RetentionDays {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let days: i64 = s
            .trim()
            .parse()
            .map_err(|e| format!("WEBHOOK_RETENTION_DAYS must be a number of days: {e}"))?;
        if days < MIN_RETENTION_DAYS {
            return Err(format!(
                "WEBHOOK_RETENTION_DAYS must be at least {MIN_RETENTION_DAYS}, got {days}"
            ));
        }
        Ok(RetentionDays(days))
    }
}

impl Default for WebhookOutConfig {
    fn default() -> Self {
        Self {
            delivery_concurrency: 32,
            egress_proxy_url: None,
            sso_use_proxy: false,
            allow_private_destinations: false,
            sso_allow_private_destinations: None,
            retention_days: RetentionDays(30),
        }
    }
}

/// Opt-in instance bootstrap for docker-compose and CI: a user, organization, tenant,
/// API key and (optionally) the test gateway, created at startup. `INSTANCE_`-prefixed so
/// it doesn't read as a sibling of `KAFKA_BOOTSTRAP_SERVERS`.
#[derive(Envconfig, Debug, Clone)]
pub struct InstanceBootstrapConfig {
    /// The single switch: everything below is ignored while this is false.
    #[envconfig(from = "INSTANCE_BOOTSTRAP_ENABLED", default = "false")]
    pub enabled: bool,

    #[envconfig(from = "INSTANCE_BOOTSTRAP_EMAIL")]
    pub email: Option<String>,

    #[envconfig(from = "INSTANCE_BOOTSTRAP_PASSWORD")]
    pub password: Option<SecretString>,

    #[envconfig(from = "INSTANCE_BOOTSTRAP_ORGANIZATION", default = "Meteroid")]
    pub organization: String,

    #[envconfig(from = "INSTANCE_BOOTSTRAP_TENANT", default = "Development")]
    pub tenant: String,

    #[envconfig(from = "INSTANCE_BOOTSTRAP_COUNTRY", default = "US")]
    pub country: String,

    #[envconfig(from = "INSTANCE_BOOTSTRAP_CURRENCY", default = "USD")]
    pub currency: String,

    /// Installed verbatim when set, so callers can pin a key they already hold.
    #[envconfig(from = "INSTANCE_BOOTSTRAP_API_KEY")]
    pub api_key: Option<SecretString>,

    /// Dotenv fragment destination; unset or blank prints to stdout instead.
    #[envconfig(from = "INSTANCE_BOOTSTRAP_OUTPUT_PATH")]
    pub output_path: Option<String>,

    #[envconfig(from = "INSTANCE_BOOTSTRAP_TEST_GATEWAY", default = "true")]
    pub test_gateway: bool,
}

impl Default for InstanceBootstrapConfig {
    /// Derived from the `envconfig` defaults above rather than restated, so the two cannot drift.
    fn default() -> Self {
        Self::init_from_hashmap(&std::collections::HashMap::new())
            .expect("InstanceBootstrapConfig declares a default for every required field")
    }
}

#[derive(Envconfig, Debug, Clone, Default)]
pub struct RedisConfig {
    #[envconfig(from = "REDIS_URL")]
    pub url: Option<String>,

    #[envconfig(from = "REDIS_PASSWORD")]
    pub password: Option<SecretString>,
}

#[derive(Envconfig, Debug, Clone, Default)]
pub struct RateLimitConfig {
    #[envconfig(nested)]
    pub grpc: GrpcRateLimitConfig,
    #[envconfig(nested)]
    pub rest: RestRateLimitConfig,
}

#[derive(Envconfig, Debug, Clone)]
pub struct GrpcRateLimitConfig {
    #[envconfig(from = "RATE_LIMIT_GRPC_RPM", default = "60")]
    pub rpm: u64,
}

impl Default for GrpcRateLimitConfig {
    fn default() -> Self {
        Self { rpm: 60 }
    }
}

#[derive(Envconfig, Debug, Clone)]
pub struct RestRateLimitConfig {
    #[envconfig(from = "RATE_LIMIT_REST_PROD_RPS", default = "100")]
    pub prod_rps: u64,
    #[envconfig(from = "RATE_LIMIT_REST_NO_PROD_RPS", default = "25")]
    pub non_prod_rps: u64,
    #[envconfig(from = "RATE_LIMIT_REST_PROD_METERING_RPS", default = "500")]
    pub prod_metering_rps: u64,
    #[envconfig(from = "RATE_LIMIT_REST_NON_PROD_METERING_RPS", default = "125")]
    pub non_prod_metering_rps: u64,
    #[envconfig(from = "RATE_LIMIT_REST_PROD_METERING_EPS", default = "5000")]
    pub prod_metering_eps: u64,
    #[envconfig(from = "RATE_LIMIT_REST_NON_PROD_METERING_EPS", default = "1250")]
    pub non_prod_metering_eps: u64,
    /// Per customer, on the browser-facing `/api/client` routes.
    #[envconfig(from = "RATE_LIMIT_REST_CLIENT_PROD_RPS", default = "20")]
    pub client_prod_rps: u64,
    #[envconfig(from = "RATE_LIMIT_REST_CLIENT_NON_PROD_RPS", default = "5")]
    pub client_non_prod_rps: u64,
    /// Per tenant, per minute, on the usage cost reads (they price live usage). An organization's
    /// `usage_costs_rate_limit` entitlement overrides it.
    #[envconfig(from = "RATE_LIMIT_REST_PROD_USAGE_COSTS_RPM", default = "600")]
    pub prod_usage_costs_rpm: u64,
    #[envconfig(from = "RATE_LIMIT_REST_NON_PROD_USAGE_COSTS_RPM", default = "120")]
    pub non_prod_usage_costs_rpm: u64,
}

impl Default for RestRateLimitConfig {
    fn default() -> Self {
        Self {
            prod_rps: 100,
            non_prod_rps: 25,
            prod_metering_rps: 500,
            non_prod_metering_rps: 125,
            prod_metering_eps: 5000,
            non_prod_metering_eps: 1250,
            client_prod_rps: 20,
            client_non_prod_rps: 5,
            prod_usage_costs_rpm: 600,
            non_prod_usage_costs_rpm: 120,
        }
    }
}

#[cfg(test)]
mod webhook_out_config_tests {
    use super::*;

    #[test]
    fn retention_below_two_days_is_refused_at_load() {
        assert!(RetentionDays::from_str("1").is_err());
        assert!(RetentionDays::from_str("0").is_err());
        assert!(RetentionDays::from_str("-3").is_err());
        assert!(RetentionDays::from_str("soon").is_err());
        assert_eq!(RetentionDays::from_str("2"), Ok(RetentionDays(2)));
        assert_eq!(RetentionDays::from_str(" 30 "), Ok(RetentionDays(30)));
    }
}
