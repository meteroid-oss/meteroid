//! One-shot import of the webhook endpoints a pre-2.0 instance kept in its Svix server,
//! run by `meteroid-svix-import`. Meteroid created one Svix application per tenant, with
//! the tenant id as its uid.

pub mod client;
pub mod plan;

use std::collections::HashSet;
use std::fmt;

use common_domain::ids::{TenantId, WebhookEndpointId};
use egress_http::{EgressClient, Url};
use meteroid_store::Store;
use meteroid_store::domain::webhooks_out::{
    ImportedSvixWebhookEndpoint, SvixWebhookEndpointImport,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::WebhookOutInterface;
use meteroid_store::repositories::tenants::TenantInterface;
use meteroid_store::services::webhooks::out::secret::decode_key;

use crate::services::webhooks_out::max_in_flight_for;
use client::{SvixApplication, SvixClient, SvixEndpoint, SvixEndpointHeaders, SvixError};
use plan::{DisabledBecause, EndpointPlan, NewEndpoint, PlannedEndpoint, plan_application};

pub struct ImportReport {
    pub apply: bool,
    pub applications: Vec<ApplicationReport>,
}

pub struct ApplicationReport {
    pub app_id: String,
    pub app_name: Option<String>,
    pub outcome: ApplicationOutcome,
}

pub enum ApplicationOutcome {
    NotATenant(String),
    /// Reading Svix or the database failed; nothing was imported for this application.
    Failed(String),
    Planned {
        tenant_id: TenantId,
        tenant_name: String,
        endpoints: Vec<EndpointReport>,
    },
}

pub struct EndpointReport {
    pub planned: PlannedEndpoint,
    /// Set by `--apply` for the endpoints the plan creates.
    pub applied: Option<Applied>,
}

pub enum Applied {
    Created(WebhookEndpointId),
    /// Created since the plan was made, by a concurrent run.
    AlreadyPresent,
    Failed(String),
}

impl ImportReport {
    pub fn failures(&self) -> usize {
        self.applications
            .iter()
            .map(|app| match &app.outcome {
                ApplicationOutcome::NotATenant(_) => 0,
                ApplicationOutcome::Failed(_) => 1,
                ApplicationOutcome::Planned { endpoints, .. } => endpoints
                    .iter()
                    .filter(|e| matches!(e.applied, Some(Applied::Failed(_))))
                    .count(),
            })
            .sum()
    }
}

/// Reads every Svix application and plans its endpoints; with `apply`, creates them.
/// Fails only when the applications cannot be listed: any other failure is in the report.
pub async fn import(
    store: &Store,
    svix: &SvixClient,
    webhook_egress: &EgressClient,
    is_known_event_type: &dyn Fn(&str) -> bool,
    apply: bool,
) -> Result<ImportReport, SvixError> {
    let check_url = |url: &str| egress_check(webhook_egress, url);

    let mut applications = Vec::new();
    for app in svix.list_applications().await? {
        let outcome =
            import_application(store, svix, &app, is_known_event_type, &check_url, apply).await;
        applications.push(ApplicationReport {
            app_id: app.id,
            app_name: app.name,
            outcome,
        });
    }

    Ok(ImportReport {
        apply,
        applications,
    })
}

fn egress_check(egress: &EgressClient, url: &str) -> Result<(), String> {
    let parsed = Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
    egress.validate_url(&parsed).map_err(|e| e.to_string())
}

async fn import_application(
    store: &Store,
    svix: &SvixClient,
    app: &SvixApplication,
    is_known_event_type: &dyn Fn(&str) -> bool,
    check_url: &dyn Fn(&str) -> Result<(), String>,
    apply: bool,
) -> ApplicationOutcome {
    let Some(uid) = app.uid.as_deref() else {
        return ApplicationOutcome::NotATenant(
            "it has no uid, so Meteroid did not create it".to_string(),
        );
    };
    let Ok(tenant_id) = uid.parse::<TenantId>() else {
        return ApplicationOutcome::NotATenant(format!("its uid `{uid}` is not a tenant id"));
    };
    let tenant = match store.find_tenant_by_id(tenant_id).await {
        Ok(tenant) => tenant.tenant,
        Err(e) if matches!(e.current_context(), StoreError::ValueNotFound(_)) => {
            return ApplicationOutcome::NotATenant(format!(
                "tenant {tenant_id} does not exist in this database"
            ));
        }
        Err(e) => {
            return ApplicationOutcome::Failed(format!("cannot read tenant {tenant_id}: {e:?}"));
        }
    };

    let endpoints = match read_endpoints(svix, &app.id).await {
        Ok(endpoints) => endpoints,
        Err(e) => return ApplicationOutcome::Failed(e.to_string()),
    };
    let existing_urls: HashSet<String> = match store.list_webhook_endpoints(tenant_id).await {
        Ok(existing) => existing.into_iter().map(|e| e.url).collect(),
        Err(e) => {
            return ApplicationOutcome::Failed(format!(
                "cannot list the webhook endpoints of tenant {tenant_id}: {e:?}"
            ));
        }
    };

    let max_in_flight = max_in_flight_for(tenant.environment);
    let mut reports = Vec::with_capacity(endpoints.len());
    for planned in plan_application(&endpoints, &existing_urls, is_known_event_type, check_url) {
        let applied = match &planned.plan {
            EndpointPlan::Create(new) if apply => Some(
                create_endpoint(
                    store,
                    svix,
                    &app.id,
                    tenant_id,
                    &planned,
                    new,
                    max_in_flight,
                )
                .await,
            ),
            _ => None,
        };
        reports.push(EndpointReport { planned, applied });
    }

    ApplicationOutcome::Planned {
        tenant_id,
        tenant_name: tenant.name,
        endpoints: reports,
    }
}

async fn read_endpoints(
    svix: &SvixClient,
    app_id: &str,
) -> Result<Vec<(SvixEndpoint, SvixEndpointHeaders)>, SvixError> {
    let endpoints = svix.list_endpoints(app_id).await?;

    let mut out = Vec::with_capacity(endpoints.len());
    for endpoint in endpoints {
        let headers = svix.endpoint_headers(app_id, &endpoint.id).await?;
        out.push((endpoint, headers));
    }
    Ok(out)
}

async fn create_endpoint(
    store: &Store,
    svix: &SvixClient,
    app_id: &str,
    tenant_id: TenantId,
    planned: &PlannedEndpoint,
    new: &NewEndpoint,
    max_in_flight: i32,
) -> Applied {
    let secret = match svix
        .endpoint_secret(app_id, &planned.svix_endpoint_id)
        .await
    {
        Ok(secret) => secret,
        Err(e) => return Applied::Failed(e.to_string()),
    };
    if decode_key(&secret).is_err() {
        return Applied::Failed("its Svix signing secret is not a valid base64 key".to_string());
    }

    let endpoint = ImportedSvixWebhookEndpoint {
        url: planned.url.clone(),
        description: new.description.clone(),
        event_types: new.event_types.clone(),
        disabled: new.disabled(),
        secret,
        max_in_flight,
        headers: new.headers.clone(),
        rate_limit_per_sec: new.rate_limit_per_sec,
    };

    match store
        .import_svix_webhook_endpoint(tenant_id, endpoint)
        .await
    {
        Ok(SvixWebhookEndpointImport::Created(endpoint)) => Applied::Created(endpoint.id),
        Ok(SvixWebhookEndpointImport::AlreadyPresent(_)) => Applied::AlreadyPresent,
        Err(e) => Applied::Failed(format!("{e:?}")),
    }
}

#[derive(Default)]
struct Totals {
    created: usize,
    needs_a_hand: usize,
    present: usize,
    skipped: usize,
    applications_skipped: usize,
    failed: usize,
}

impl fmt::Display for ImportReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.apply {
            writeln!(f, "Importing the Svix endpoints.")?;
        } else {
            writeln!(
                f,
                "Dry run: nothing is written. Run again with --apply to import."
            )?;
        }

        let mut totals = Totals::default();
        for app in &self.applications {
            writeln!(f)?;
            let name = app
                .app_name
                .as_deref()
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            match &app.outcome {
                ApplicationOutcome::NotATenant(reason) => {
                    totals.applications_skipped += 1;
                    writeln!(
                        f,
                        "Svix application {}{name}: skipped, {reason}",
                        app.app_id
                    )?;
                }
                ApplicationOutcome::Failed(reason) => {
                    totals.failed += 1;
                    writeln!(f, "Svix application {}{name}: FAILED", app.app_id)?;
                    write_indented(f, reason)?;
                }
                ApplicationOutcome::Planned {
                    tenant_id,
                    tenant_name,
                    endpoints,
                } => {
                    writeln!(
                        f,
                        "Svix application {}{name} -> tenant {tenant_name} ({tenant_id})",
                        app.app_id
                    )?;
                    if endpoints.is_empty() {
                        writeln!(f, "  no endpoints")?;
                    }
                    for endpoint in endpoints {
                        write_endpoint(f, endpoint, &mut totals)?;
                    }
                }
            }
        }

        writeln!(f)?;
        let created = if self.apply { "created" } else { "to create" };
        writeln!(
            f,
            "{} {created} ({} of them disabled until set up by hand), {} already present, {} skipped, {} application(s) skipped, {} failed.",
            totals.created,
            totals.needs_a_hand,
            totals.present,
            totals.skipped,
            totals.applications_skipped,
            totals.failed,
        )
    }
}

fn write_endpoint(
    f: &mut fmt::Formatter<'_>,
    endpoint: &EndpointReport,
    totals: &mut Totals,
) -> fmt::Result {
    let url = redacted(&endpoint.planned.url);
    let svix_id = &endpoint.planned.svix_endpoint_id;

    let new = match (&endpoint.planned.plan, &endpoint.applied) {
        (EndpointPlan::AlreadyPresent, _) | (_, Some(Applied::AlreadyPresent)) => {
            totals.present += 1;
            return writeln!(
                f,
                "  present  {url} [{svix_id}]: the tenant already has an endpoint with this url"
            );
        }
        (EndpointPlan::Skip(reason), _) => {
            totals.skipped += 1;
            return writeln!(f, "  skip     {url} [{svix_id}]: {reason}");
        }
        (EndpointPlan::Create(_), Some(Applied::Failed(reason))) => {
            totals.failed += 1;
            writeln!(f, "  FAILED   {url} [{svix_id}]")?;
            return write_indented(f, reason);
        }
        (EndpointPlan::Create(new), applied) => {
            totals.created += 1;
            let verb = match applied {
                Some(Applied::Created(id)) => format!("created  {url} [{svix_id} -> {id}]"),
                _ => format!("create   {url} [{svix_id}]"),
            };
            let state = if new.disabled() {
                "disabled"
            } else {
                "enabled"
            };
            writeln!(f, "  {verb}, {state}")?;
            new
        }
    };

    let events = if new.event_types.is_empty() {
        "all event types".to_string()
    } else {
        new.event_types.join(", ")
    };
    writeln!(f, "           events: {events}")?;
    if !new.headers.is_empty() {
        let names: Vec<String> = new
            .headers
            .iter()
            .map(|h| {
                if h.sensitive {
                    format!("{} (sensitive)", h.name)
                } else {
                    h.name.clone()
                }
            })
            .collect();
        writeln!(f, "           headers: {}", names.join(", "))?;
    }

    let mut needs_a_hand = false;
    for reason in &new.disabled_because {
        let line = match reason {
            DisabledBecause::DisabledInSvix => "disabled in Svix".to_string(),
            DisabledBecause::SensitiveHeaders(names) => {
                needs_a_hand = true;
                format!(
                    "TO DO: Svix never reveals the value of {}. Enter it on the endpoint in Meteroid, then enable the endpoint",
                    names.join(", ")
                )
            }
            DisabledBecause::UrlRefused(detail) => {
                needs_a_hand = true;
                format!(
                    "TO DO: this instance refuses to send to this url ({detail}). Change the url, or allow private and plain http destinations with WEBHOOK_ALLOW_PRIVATE_DESTINATIONS=true, then enable the endpoint"
                )
            }
            DisabledBecause::Channels(channels) => {
                needs_a_hand = true;
                format!(
                    "TO DO: filtered on Svix channels ({}), which Meteroid never set, so Svix sent it no events. Enable the endpoint if it should receive them",
                    channels.join(", ")
                )
            }
        };
        writeln!(f, "           {line}")?;
    }
    if needs_a_hand {
        totals.needs_a_hand += 1;
    }
    for note in &new.notes {
        writeln!(f, "           note: {note}")?;
    }
    Ok(())
}

/// Userinfo and query strings often carry credentials; the Svix id tells endpoints apart.
fn redacted(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return url.to_string();
    };
    let _ = parsed.set_password(None);
    let _ = parsed.set_username("");
    if parsed.query().is_some() {
        parsed.set_query(Some("redacted"));
    }
    parsed.to_string()
}

fn write_indented(f: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    for line in text.lines() {
        writeln!(f, "           {line}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use common_domain::ids::BaseId;
    use meteroid_store::domain::webhooks_out::WebhookHeaderInput;

    use super::*;

    #[test]
    fn printed_urls_lose_their_credentials() {
        assert_eq!(
            redacted("https://user:pass@a.example/hook?token=x"),
            "https://a.example/hook?redacted"
        );
        assert_eq!(redacted("https://a.example/hook"), "https://a.example/hook");
    }

    fn created(
        url: &str,
        headers: Vec<WebhookHeaderInput>,
        applied: Option<Applied>,
    ) -> EndpointReport {
        let sensitive: Vec<String> = headers
            .iter()
            .filter(|h| h.sensitive)
            .map(|h| h.name.clone())
            .collect();
        EndpointReport {
            planned: PlannedEndpoint {
                svix_endpoint_id: "ep_1".to_string(),
                url: url.to_string(),
                plan: EndpointPlan::Create(NewEndpoint {
                    description: None,
                    event_types: vec![],
                    headers,
                    rate_limit_per_sec: None,
                    disabled_because: if sensitive.is_empty() {
                        vec![]
                    } else {
                        vec![DisabledBecause::SensitiveHeaders(sensitive)]
                    },
                    notes: vec![],
                }),
            },
            applied,
        }
    }

    #[test]
    fn the_report_counts_failures_and_never_prints_header_values() {
        let report = ImportReport {
            apply: true,
            applications: vec![
                ApplicationReport {
                    app_id: "app_1".to_string(),
                    app_name: None,
                    outcome: ApplicationOutcome::NotATenant("no uid".to_string()),
                },
                ApplicationReport {
                    app_id: "app_2".to_string(),
                    app_name: None,
                    outcome: ApplicationOutcome::Failed("Svix answered 500".to_string()),
                },
                ApplicationReport {
                    app_id: "app_3".to_string(),
                    app_name: Some("Acme".to_string()),
                    outcome: ApplicationOutcome::Planned {
                        tenant_id: TenantId::new(),
                        tenant_name: "Acme".to_string(),
                        endpoints: vec![
                            created(
                                "https://a.example",
                                vec![
                                    WebhookHeaderInput {
                                        name: "X-Team".to_string(),
                                        value: Some("billing-team".to_string()),
                                        sensitive: false,
                                    },
                                    WebhookHeaderInput {
                                        name: "Authorization".to_string(),
                                        value: None,
                                        sensitive: true,
                                    },
                                ],
                                Some(Applied::Created(WebhookEndpointId::new())),
                            ),
                            created(
                                "https://b.example",
                                vec![],
                                Some(Applied::Failed("database unavailable".to_string())),
                            ),
                        ],
                    },
                },
            ],
        };

        assert_eq!(report.failures(), 2);

        let printed = report.to_string();
        assert!(printed.contains("TO DO"));
        assert!(printed.contains("Authorization (sensitive)"));
        assert!(!printed.contains("billing-team"));
        assert!(printed.contains("1 created (1 of them disabled until set up by hand)"));
        assert!(printed.contains("2 failed"));
    }
}
