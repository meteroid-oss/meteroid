//! Dated versions inside v2 (ADR-0013 §2). A request names one with `Meteroid-Version:
//! YYYY-MM-DD`; it resolves to the latest version published on or before that date, the first
//! when none is sent, and the resolved version is echoed on every response.
//!
//! Handlers always speak the latest shape. A breaking change adds its date to [`VERSIONS`] and a
//! transformer that rewrites requests up from, and responses down to, the version a caller is
//! pinned on, keyed on the [`ApiVersion`] the middleware put in the request extensions.
use crate::v2::problem::{Problem, ProblemCode};
use chrono::NaiveDate;
use http::StatusCode;

pub const VERSION_HEADER: &str = "Meteroid-Version";

/// Every published version, oldest first. The first is the default.
pub const VERSIONS: &[&str] = &["2026-12-01"];

/// The version a request is served at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ApiVersion(pub NaiveDate);

impl ApiVersion {
    pub fn as_header(&self) -> String {
        self.0.format("%Y-%m-%d").to_string()
    }
}

fn published() -> impl Iterator<Item = NaiveDate> {
    VERSIONS.iter().map(|version| {
        NaiveDate::parse_from_str(version, "%Y-%m-%d").expect("a published version is a date")
    })
}

pub fn default_version() -> ApiVersion {
    ApiVersion(published().next().expect("v2 has a version"))
}

/// The version `asked` resolves to: the latest published on or before it.
pub fn resolve(asked: Option<&str>) -> Result<ApiVersion, Problem> {
    let Some(asked) = asked.map(str::trim).filter(|asked| !asked.is_empty()) else {
        return Ok(default_version());
    };
    let refused = |detail: String| {
        Problem::new(
            StatusCode::BAD_REQUEST,
            ProblemCode::UnsupportedVersion,
            detail,
        )
    };
    let date = NaiveDate::parse_from_str(asked, "%Y-%m-%d").map_err(|_| {
        refused(format!(
            "{VERSION_HEADER} is a date, YYYY-MM-DD; '{asked}' is not one"
        ))
    })?;
    published()
        .filter(|version| *version <= date)
        .last()
        .map(ApiVersion)
        .ok_or_else(|| {
            refused(format!(
                "{asked} is before the first version of the API, {}",
                VERSIONS[0]
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_resolves_to_the_version_in_force_then() {
        assert_eq!(resolve(None).unwrap().as_header(), "2026-12-01");
        assert_eq!(
            resolve(Some("2026-12-01")).unwrap().as_header(),
            "2026-12-01"
        );
        assert_eq!(
            resolve(Some("2027-03-15")).unwrap().as_header(),
            "2026-12-01"
        );
        assert!(resolve(Some("2026-11-30")).is_err());
        assert!(resolve(Some("latest")).is_err());
    }
}
