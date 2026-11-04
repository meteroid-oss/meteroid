//! The anonymous instance report (ADR 0010): one aggregated payload a day, never an identifier
//! of a user, customer or invoice, never an exact amount.

use chrono::{DateTime, NaiveDate, Utc};
use common_config::instance_telemetry::{InstanceTelemetryConfig, TelemetryDecision};
use meteroid_store::repositories::instance_telemetry::{
    InstanceTelemetry, InstanceTelemetryInterface, InstanceUsage,
};
use meteroid_store::{Store, StoreResult};
use serde::Serialize;
use std::time::Duration;

pub const SCHEMA_VERSION: u32 = 1;
pub const DOCS_URL: &str = "https://docs.meteroid.com/self-hosting/telemetry";
const FIRST_REPORT_AFTER: chrono::Duration = chrono::Duration::hours(1);
const REPORT_EVERY: chrono::Duration = chrono::Duration::hours(23);

/// The release this binary was built from (`METEROID_VERSION` at build time), `dev` otherwise.
pub fn release_version() -> &'static str {
    option_env!("METEROID_VERSION")
        .filter(|v| !v.is_empty())
        .unwrap_or("dev")
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct InstanceReport {
    pub schema: u32,
    pub instance_id: uuid::Uuid,
    pub sent_at: DateTime<Utc>,
    pub install: Install,
    pub build: Build,
    pub deployment: String,
    pub infra: Infra,
    pub usage: Usage,
    pub integrations: Vec<String>,
    pub modules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Install {
    pub installed_on: NaiveDate,
    pub first_version: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Build {
    pub version: String,
    pub edition: &'static str,
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Infra {
    pub postgres_major: u32,
    pub object_storage: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Usage {
    pub organizations: u64,
    pub tenants_live: u64,
    pub tenants_other: u64,
    pub users: u64,
    pub customers: u64,
    pub subscriptions_active: u64,
    pub plans_published: u64,
    pub billable_metrics: u64,
    pub invoices_finalized_30d: u64,
    pub currencies: Vec<String>,
    pub billed_30d_usd: &'static str,
}

/// A decade band: the lower bounds add up to a floor without disclosing anyone's figure.
pub fn band(value: u64) -> &'static str {
    match value {
        0 => "0",
        1..1_000 => "<1k",
        1_000..10_000 => "1k-10k",
        10_000..100_000 => "10k-100k",
        100_000..1_000_000 => "100k-1M",
        1_000_000..10_000_000 => "1M-10M",
        _ => "10M+",
    }
}

/// The scheme of the object store URI, never the rest of it (bucket, host, credentials).
fn object_storage_kind(uri: &str) -> String {
    uri.split_once("://")
        .map(|(scheme, _)| scheme.to_ascii_lowercase())
        .filter(|s| s.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "unknown".to_string())
}

pub struct ReportInputs<'a> {
    pub identity: &'a InstanceTelemetry,
    pub sent_at: DateTime<Utc>,
    pub version: &'a str,
    pub edition: &'static str,
    pub os: &'a str,
    pub arch: &'a str,
    pub deployment: &'a str,
    pub object_store_uri: &'a str,
    pub modules: Vec<String>,
    pub usage: InstanceUsage,
}

pub fn build_report(inputs: ReportInputs<'_>) -> InstanceReport {
    let u = inputs.usage;
    InstanceReport {
        schema: SCHEMA_VERSION,
        instance_id: inputs.identity.instance_id,
        sent_at: inputs.sent_at,
        install: Install {
            installed_on: inputs.identity.installed_at.date(),
            first_version: inputs.identity.first_version.clone(),
        },
        build: Build {
            version: inputs.version.to_string(),
            edition: inputs.edition,
            os: inputs.os.to_string(),
            arch: inputs.arch.to_string(),
        },
        deployment: inputs.deployment.to_string(),
        infra: Infra {
            postgres_major: u.postgres_major,
            object_storage: object_storage_kind(inputs.object_store_uri),
        },
        usage: Usage {
            organizations: u.organizations,
            tenants_live: u.tenants_live,
            tenants_other: u.tenants_other,
            users: u.users,
            customers: u.customers,
            subscriptions_active: u.subscriptions_active,
            plans_published: u.plans_published,
            billable_metrics: u.billable_metrics,
            invoices_finalized_30d: u.invoices_finalized_30d,
            currencies: u.currencies,
            billed_30d_usd: band(u64::try_from(u.billed_30d_usd_cents / 100).unwrap_or(0)),
        },
        integrations: u.integrations,
        modules: inputs.modules,
    }
}

pub struct InstanceTelemetryService {
    store: Store,
    config: InstanceTelemetryConfig,
    object_store_uri: String,
    modules: Vec<String>,
    client: reqwest::Client,
}

impl InstanceTelemetryService {
    pub fn new(
        store: Store,
        config: InstanceTelemetryConfig,
        object_store_uri: String,
        modules: Vec<String>,
    ) -> Self {
        let client = http_client_tracing::client_builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            store,
            config,
            object_store_uri,
            modules,
            client,
        }
    }

    /// Meteroid Cloud measures itself from its own database.
    pub fn decision(&self) -> TelemetryDecision {
        if self.store.instance.is_cloud() {
            return TelemetryDecision::Cloud;
        }
        self.config.decide(cfg!(debug_assertions))
    }

    pub async fn build(&self, identity: &InstanceTelemetry) -> StoreResult<InstanceReport> {
        let now = Utc::now();
        let usage = self.store.instance_usage(now.date_naive()).await?;
        Ok(build_report(ReportInputs {
            identity,
            sent_at: now,
            version: release_version(),
            edition: self.store.instance.edition(),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            deployment: self.config.deployment(),
            object_store_uri: &self.object_store_uri,
            modules: self.modules.clone(),
            usage,
        }))
    }

    pub fn log_startup(&self) {
        let decision = self.decision();
        if self.config.debug() {
            log::info!(
                "METEROID_TELEMETRY_DEBUG: the telemetry report is logged hourly, never sent."
            );
        } else if decision.enabled() {
            log::info!(
                "Anonymous telemetry is on: one aggregated report a day to {}, no personal or customer data ({DOCS_URL}). \
                 Turn it off with METEROID_TELEMETRY=off.",
                self.config.url(),
            );
        } else {
            log::info!("Anonymous telemetry is off ({}).", decision.reason());
        }
    }

    /// Called hourly by one replica.
    pub async fn tick(&self) -> StoreResult<()> {
        let debug = self.config.debug() && !self.store.instance.is_cloud();
        if !debug && !self.decision().enabled() {
            return Ok(());
        }
        let identity = self.store.instance_telemetry(release_version()).await?;
        if debug {
            let report = self.build(&identity).await?;
            let json = serde_json::to_string_pretty(&report).unwrap_or_default();
            log::info!("telemetry report (METEROID_TELEMETRY_DEBUG, not sent):\n{json}");
            return Ok(());
        }
        let now = Utc::now().naive_utc();
        let due = match identity.last_sent_at {
            Some(last) => now - last >= REPORT_EVERY,
            None => now - identity.installed_at >= FIRST_REPORT_AFTER,
        };
        if !due {
            return Ok(());
        }
        let report = self.build(&identity).await?;
        match self
            .client
            .post(self.config.url())
            .json(&report)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            Ok(_) => self.store.record_instance_telemetry_sent().await,
            Err(e) => {
                log::debug!("telemetry report not delivered: {e}");
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;

    fn identity() -> InstanceTelemetry {
        InstanceTelemetry {
            instance_id: uuid::uuid!("5f0c3a9e-8b1d-4c2e-9f3a-0d1e2f3a4b5c"),
            installed_at: NaiveDateTime::parse_from_str("2026-10-14 08:00:00", "%F %T").unwrap(),
            first_version: "v1.4.0".to_string(),
            last_sent_at: None,
        }
    }

    fn usage() -> InstanceUsage {
        InstanceUsage {
            organizations: 1,
            tenants_live: 1,
            tenants_other: 2,
            users: 4,
            customers: 812,
            subscriptions_active: 640,
            plans_published: 6,
            billable_metrics: 9,
            invoices_finalized_30d: 701,
            currencies: vec!["EUR".to_string(), "USD".to_string()],
            integrations: vec!["hubspot".to_string(), "stripe".to_string()],
            postgres_major: 16,
            billed_30d_usd_cents: 34_567_890,
        }
    }

    #[test]
    fn standard_report() {
        let identity = identity();
        let report = build_report(ReportInputs {
            identity: &identity,
            sent_at: "2026-11-02T03:12:00Z".parse().unwrap(),
            version: "v1.5.2",
            edition: "oss",
            os: "linux",
            arch: "x86_64",
            deployment: "docker-compose",
            object_store_uri: "s3://user:secret@bucket.example.com/prefix",
            modules: vec![],
            usage: usage(),
        });
        insta::assert_json_snapshot!(report);
    }

    #[test]
    fn bands() {
        assert_eq!(band(0), "0");
        assert_eq!(band(999), "<1k");
        assert_eq!(band(1_000), "1k-10k");
        assert_eq!(band(123_456), "100k-1M");
        assert_eq!(band(10_000_000), "10M+");
    }

    #[test]
    fn object_storage_keeps_the_scheme_only() {
        assert_eq!(object_storage_kind("s3://key:secret@host/bucket"), "s3");
        assert_eq!(object_storage_kind("file:///var/lib/meteroid"), "file");
        assert_eq!(object_storage_kind("/var/lib"), "unknown");
    }
}
