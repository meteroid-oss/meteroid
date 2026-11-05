//! The anonymous instance report. Not the OpenTelemetry settings in [`crate::telemetry`].

use envconfig::Envconfig;

pub const DEFAULT_TELEMETRY_URL: &str = "https://api.metero.id/oss/v1/telemetry";

/// Why the report is on or off, for the startup log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryDecision {
    DoNotTrack,
    LegacyAnalyticsDisabled,
    Environment(bool),
    DevelopmentBuild,
    Cloud,
    Default,
}

impl TelemetryDecision {
    pub fn enabled(self) -> bool {
        matches!(self, TelemetryDecision::Environment(true))
    }

    pub fn reason(self) -> &'static str {
        match self {
            TelemetryDecision::DoNotTrack => "DO_NOT_TRACK is set",
            TelemetryDecision::LegacyAnalyticsDisabled => "ANALYTICS_ENABLED=false",
            TelemetryDecision::Environment(_) => "METEROID_TELEMETRY",
            TelemetryDecision::DevelopmentBuild => "development build or CI",
            TelemetryDecision::Cloud => "Meteroid Cloud",
            TelemetryDecision::Default => "off unless METEROID_TELEMETRY=on",
        }
    }
}

#[derive(Envconfig, Debug, Clone, Default)]
pub struct InstanceTelemetryConfig {
    /// `on` or `off`. The official images set `on`; a build from source is off.
    #[envconfig(from = "METEROID_TELEMETRY")]
    pub enabled: Option<String>,

    /// Logs the report instead of sending it.
    #[envconfig(from = "METEROID_TELEMETRY_DEBUG")]
    pub debug: Option<String>,

    #[envconfig(from = "METEROID_TELEMETRY_URL")]
    pub url: Option<String>,

    /// How the install was deployed (`docker`, `docker-compose`, `helm`), set by the image or chart.
    #[envconfig(from = "METEROID_DEPLOYMENT")]
    pub deployment: Option<String>,

    /// Any value other than empty, `0` or `false` turns telemetry off (consoledonottrack.com).
    #[envconfig(from = "DO_NOT_TRACK")]
    pub do_not_track: Option<String>,

    /// Opt-out of the former Segment tracking; `false` still means off.
    #[envconfig(from = "ANALYTICS_ENABLED")]
    pub legacy_analytics_enabled: Option<bool>,

    #[envconfig(from = "CI")]
    pub ci: Option<String>,
}

fn truthy(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "off" | "no"
        )
    })
}

impl InstanceTelemetryConfig {
    pub fn url(&self) -> &str {
        self.url.as_deref().unwrap_or(DEFAULT_TELEMETRY_URL)
    }

    pub fn debug(&self) -> bool {
        truthy(self.debug.as_deref())
    }

    pub fn deployment(&self) -> &str {
        self.deployment
            .as_deref()
            .filter(|d| !d.trim().is_empty())
            .unwrap_or("unknown")
    }

    /// `ANALYTICS_ENABLED=false` ranks above `METEROID_TELEMETRY`, which the images set: an
    /// operator who opted out of the former tracking stays out.
    pub fn decide(&self, debug_build: bool) -> TelemetryDecision {
        if truthy(self.do_not_track.as_deref()) {
            return TelemetryDecision::DoNotTrack;
        }
        if self.legacy_analytics_enabled == Some(false) {
            return TelemetryDecision::LegacyAnalyticsDisabled;
        }
        if let Some(value) = self.enabled.as_deref().filter(|v| !v.trim().is_empty()) {
            return TelemetryDecision::Environment(truthy(Some(value)));
        }
        if debug_build || truthy(self.ci.as_deref()) {
            return TelemetryDecision::DevelopmentBuild;
        }
        TelemetryDecision::Default
    }
}

#[cfg(test)]
mod tests {
    use super::TelemetryDecision as D;
    use super::*;

    fn with(enabled: &str) -> InstanceTelemetryConfig {
        InstanceTelemetryConfig {
            enabled: Some(enabled.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn off_by_default() {
        assert_eq!(InstanceTelemetryConfig::default().decide(false), D::Default);
        assert!(!D::Default.enabled());
    }

    #[test]
    fn environment_decides() {
        assert_eq!(with("on").decide(false), D::Environment(true));
        assert_eq!(with("true").decide(false), D::Environment(true));
        assert_eq!(with("off").decide(false), D::Environment(false));
        assert_eq!(with("").decide(false), D::Default);
        assert!(!with("off").decide(false).enabled());
    }

    #[test]
    fn opt_outs_win() {
        let dnt = InstanceTelemetryConfig {
            do_not_track: Some("1".to_string()),
            ..with("on")
        };
        assert_eq!(dnt.decide(false), D::DoNotTrack);
        let legacy = InstanceTelemetryConfig {
            legacy_analytics_enabled: Some(false),
            ..with("on")
        };
        assert_eq!(legacy.decide(false), D::LegacyAnalyticsDisabled);
        let dnt_zero = InstanceTelemetryConfig {
            do_not_track: Some("0".to_string()),
            ..with("on")
        };
        assert_eq!(dnt_zero.decide(false), D::Environment(true));
    }

    #[test]
    fn development_builds_stay_quiet_unless_asked() {
        assert_eq!(
            InstanceTelemetryConfig::default().decide(true),
            D::DevelopmentBuild
        );
        let ci = InstanceTelemetryConfig {
            ci: Some("true".to_string()),
            ..Default::default()
        };
        assert_eq!(ci.decide(false), D::DevelopmentBuild);
        assert_eq!(with("on").decide(true), D::Environment(true));
    }
}
