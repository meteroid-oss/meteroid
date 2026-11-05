use crate::domain::Organization;
use crate::domain::enums::{EmailDeliveryMode, TenantEnvironmentEnum, TenantStatusEnum};
use crate::domain::invoicing_entities::InvoicingEntityNew;
use chrono::NaiveDateTime;
use common_domain::ids::{OrganizationId, TenantId};
use diesel_models::tenants::{TenantRow, TenantRowPatch, TenantWithOrganizationRow};
use o2o::o2o;

#[derive(Clone, Debug, o2o)]
#[from_owned(TenantRow)]
pub struct Tenant {
    pub id: TenantId,
    pub name: String,
    pub slug: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub organization_id: OrganizationId,
    pub reporting_currency: String,
    #[map(~.into())]
    pub environment: TenantEnvironmentEnum,
    pub available_currencies: Vec<Option<String>>,
    pub last_active_at: Option<NaiveDateTime>,
    #[map(~.into())]
    pub status: TenantStatusEnum,
    /// Minimum seconds between alert evaluations for this tenant. `None` = the
    /// grain-derived default (fine grains every tick, day grain hourly). Drives
    /// how often signals/monitors are re-evaluated and how incidents are batched.
    pub alert_evaluation_interval_secs: Option<i32>,
    #[map(~.into())]
    pub email_delivery_mode: EmailDeliveryMode,
    /// Addresses, or `@domain` for a whole domain, that `Test` mode also delivers to.
    pub email_test_recipients: Vec<String>,
    /// A re-sent event id replaces the earlier copy instead of being dropped.
    pub keep_last_events: bool,
    /// Where the customer portal may be framed even without the `portal_embedding` gate.
    pub portal_embed_origins: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TenantNew {
    pub name: String,
    pub environment: TenantEnvironmentEnum,
    /// `None` = [`EmailDeliveryMode::default_for`] the environment.
    pub email_delivery_mode: Option<EmailDeliveryMode>,
    pub invoicing_entity: Option<InvoicingEntityNew>,
}

#[derive(Clone, Debug, Default)]
pub struct TenantUpdate {
    pub trade_name: Option<String>,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub environment: Option<TenantEnvironmentEnum>,
    pub reporting_currency: Option<String>,
    pub email_delivery_mode: Option<EmailDeliveryMode>,
    pub email_test_recipients: Option<Vec<String>>,
    /// A client predating the delivery mode: `true` turns a `Live` workspace `Off`.
    pub legacy_disable_emails: Option<bool>,
    /// Outer `None` = leave unchanged; `Some(None)` = reset to the grain-derived
    /// default (NULL); `Some(Some(v))` = set the cadence to `v` seconds.
    pub alert_evaluation_interval_secs: Option<Option<i32>>,
    pub keep_last_events: Option<bool>,
    /// Replaces the whole list when set.
    pub portal_embed_origins: Option<Vec<String>>,
}

impl From<TenantUpdate> for TenantRowPatch {
    fn from(u: TenantUpdate) -> Self {
        TenantRowPatch {
            name: u.name,
            slug: u.slug,
            reporting_currency: u.reporting_currency,
            environment: u.environment.map(Into::into),
            // Kept in step for pods still reading it during the rollout.
            disable_emails: u.email_delivery_mode.map(|m| m != EmailDeliveryMode::Live),
            alert_evaluation_interval_secs: u.alert_evaluation_interval_secs,
            email_delivery_mode: u.email_delivery_mode.map(Into::into),
            email_test_recipients: u.email_test_recipients,
            keep_last_events: u.keep_last_events,
            portal_embed_origins: u.portal_embed_origins,
        }
    }
}

/// Trimmed, lowercased and de-duplicated test recipients: each an address or `@domain`.
pub fn normalize_test_recipients(
    recipients: Vec<String>,
) -> Result<Vec<String>, crate::errors::StoreError> {
    const MAX: usize = 50;
    let mut out: Vec<String> = Vec::new();
    for raw in recipients {
        let r = raw.trim().to_ascii_lowercase();
        if r.is_empty() || out.contains(&r) {
            continue;
        }
        let (local, domain) = r.split_once('@').unwrap_or(("", ""));
        let valid = !domain.is_empty()
            && domain.contains('.')
            && !domain.contains('@')
            && !local.contains(char::is_whitespace)
            && !domain.contains(char::is_whitespace);
        if !valid {
            return Err(crate::errors::StoreError::InvalidArgument(format!(
                "Invalid test recipient `{raw}`: use an email address or @domain"
            )));
        }
        out.push(r);
    }
    if out.len() > MAX {
        return Err(crate::errors::StoreError::InvalidArgument(format!(
            "At most {MAX} test recipients"
        )));
    }
    Ok(out)
}

/// Whether `email` is a member address or matches an allowlist entry (an address or `@domain`).
pub fn is_test_recipient(email: &str, members: &[String], allowlist: &[String]) -> bool {
    let email = email.trim().to_ascii_lowercase();
    members
        .iter()
        .any(|m| m.trim().eq_ignore_ascii_case(&email))
        || allowlist.iter().any(|entry| {
            if entry.starts_with('@') {
                email.ends_with(entry.as_str())
            } else {
                *entry == email
            }
        })
}

/// Each entry as the browser reports a frame's origin (`https://app.acme.com`), de-duplicated.
pub fn normalize_embed_origins(
    origins: Vec<String>,
) -> Result<Vec<String>, crate::errors::StoreError> {
    const MAX: usize = 20;
    let mut out: Vec<String> = Vec::new();
    for raw in origins {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let origin = url::Url::parse(trimmed)
            .ok()
            .filter(|u| {
                matches!(u.scheme(), "http" | "https")
                    && u.host_str().is_some()
                    && u.username().is_empty()
                    && u.password().is_none()
                    && u.path() == "/"
                    && u.query().is_none()
                    && u.fragment().is_none()
            })
            .map(|u| u.origin().ascii_serialization())
            .ok_or_else(|| {
                crate::errors::StoreError::InvalidArgument(format!(
                    "Invalid origin `{raw}`: use scheme://host[:port], e.g. https://app.example.com"
                ))
            })?;
        if !out.contains(&origin) {
            out.push(origin);
        }
    }
    if out.len() > MAX {
        return Err(crate::errors::StoreError::InvalidArgument(format!(
            "At most {MAX} embedding origins"
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_origins_normalize_to_what_the_browser_reports() {
        let list = normalize_embed_origins(vec![
            " HTTPS://App.Acme.com/ ".into(),
            "https://app.acme.com".into(),
            "http://localhost:3000".into(),
            "https://acme.com:443".into(),
            "".into(),
        ])
        .unwrap();
        assert_eq!(
            list,
            vec![
                "https://app.acme.com",
                "http://localhost:3000",
                "https://acme.com"
            ]
        );
        for invalid in [
            "acme.com",
            "https://acme.com/billing",
            "https://acme.com?x=1",
            "javascript:alert(1)",
            "https://user@acme.com",
        ] {
            assert!(
                normalize_embed_origins(vec![invalid.into()]).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn test_recipients_normalize_and_match() {
        let list = normalize_test_recipients(vec![
            " QA@Acme.io ".into(),
            "@acme.dev".into(),
            "qa@acme.io".into(),
            "".into(),
        ])
        .unwrap();
        assert_eq!(list, vec!["qa@acme.io", "@acme.dev"]);
        assert!(normalize_test_recipients(vec!["not-an-email".into()]).is_err());
        assert!(normalize_test_recipients(vec!["@nodot".into()]).is_err());

        let members = vec!["Dev@Team.io".to_string()];
        assert!(is_test_recipient("dev@team.io", &members, &list));
        assert!(is_test_recipient("QA@acme.io", &members, &list));
        assert!(is_test_recipient("anyone@acme.dev", &members, &list));
        assert!(!is_test_recipient("anyone@notacme.dev", &members, &list));
        assert!(!is_test_recipient("customer@bigco.com", &members, &list));
    }
}

#[derive(Clone, Debug, o2o)]
#[from_owned(TenantWithOrganizationRow)]
pub struct TenantWithOrganization {
    #[map(~.into())]
    pub tenant: Tenant,
    #[map(~.into())]
    pub organization: Organization,
}
