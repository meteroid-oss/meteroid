//! Central route → permission registry.
//!
//! Every `meteroid.api.*` RPC is classified here exactly once. Enforcement is
//! structural, not per-handler: [`enforce`] runs inside `authorize_user` for
//! every user-authenticated request, so a handler cannot forget to check.
//!
//! Two properties make this fail-proof going forward:
//!   1. Default-deny — an unclassified route resolves to `None` and is denied.
//!   2. The `every_api_method_is_classified` test walks the compiled gRPC
//!      descriptor set and fails CI the moment a new RPC is added without a
//!      classification here.
//!
//! Resource services are classified by verb (read vs. manage), so a new method
//! added to an existing service inherits the *manage* permission by default and
//! therefore fails closed for restricted roles rather than open.

use common_domain::rbac::Permission::{self, *};
use common_domain::rbac::RolePermissions;
use common_grpc::GrpcServiceMethod;
use tonic::Status;

/// What a route requires of its caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteAuth {
    /// No authentication (anonymous).
    Public,
    /// Authenticated user, no specific capability (e.g. pre-org flows).
    Authenticated,
    /// A specific capability.
    Requires(Permission),
}

/// Enforce the registry for a resolved actor. Called from `authorize_user`.
///
/// Unclassified `meteroid.api.*` routes are denied — the test below guarantees
/// this never happens for a real method, so a `None` at runtime means a genuine
/// gap and failing closed is correct.
pub fn enforce(sm: &GrpcServiceMethod, permissions: &RolePermissions) -> Result<(), Status> {
    enforce_with(sm, permissions, &[])
}

/// What a module requires of the callers of a service whose proto it owns (full service name).
#[derive(Debug, Clone, Copy)]
pub struct RoutePolicy {
    pub service: &'static str,
    pub auth: RouteAuth,
}

/// [`enforce`], with the policies of the edition's modules for the services core does not know.
pub fn enforce_with(
    sm: &GrpcServiceMethod,
    permissions: &RolePermissions,
    module_policies: &[RoutePolicy],
) -> Result<(), Status> {
    let auth = method_auth(sm).or_else(|| {
        module_policies
            .iter()
            .find(|policy| policy.service == sm.service)
            .map(|policy| policy.auth)
    });
    match auth {
        Some(RouteAuth::Requires(permission)) => {
            if permissions.contains(permission) {
                Ok(())
            } else {
                Err(Status::permission_denied(format!(
                    "Missing permission '{}' for this operation.",
                    permission.code()
                )))
            }
        }
        Some(RouteAuth::Public) | Some(RouteAuth::Authenticated) => Ok(()),
        None => Err(Status::permission_denied(
            "This operation is not authorized.",
        )),
    }
}

fn short_service(sm: &GrpcServiceMethod) -> &str {
    sm.service.rsplit('.').next().unwrap_or(sm.service.as_str())
}

/// Read verbs. Anything else is treated as a mutation (fail-safe: unknown → manage).
fn is_read(method: &str) -> bool {
    const READ_PREFIXES: [&str; 10] = [
        "Get",
        "List",
        "BatchList",
        "Search",
        "Query",
        "Count",
        "Preview",
        "Export",
        "Check",
        "Lookup",
    ];
    READ_PREFIXES.iter().any(|p| method.starts_with(p))
}

fn rw(method: &str, read: Permission, manage: Permission) -> RouteAuth {
    RouteAuth::Requires(if is_read(method) { read } else { manage })
}

/// The single source of truth. `None` = unclassified (denied + test failure).
pub fn method_auth(sm: &GrpcServiceMethod) -> Option<RouteAuth> {
    let method = sm.method.as_str();
    Some(match short_service(sm) {
        // ---- customer records ----
        "CustomersService" | "BudgetsService" => rw(method, CustomersRead, CustomersManage),

        // ---- billing operations, split per sub-area ----
        "SubscriptionsService" => rw(method, SubscriptionsRead, SubscriptionsManage),
        // Credit notes are invoice corrections and share the invoices area.
        "InvoicesService" | "CreditNotesService" => rw(method, InvoicesRead, InvoicesManage),
        // Customer credit is drawn by invoices and corrected by credit notes.
        "WalletsService" | "WalletControlsService" => rw(method, InvoicesRead, InvoicesManage),
        "QuotesService" => rw(method, QuotesRead, QuotesManage),

        // ---- product catalog ----
        "PlansService"
        | "PriceComponentsService"
        | "PricesService"
        | "ProductsService"
        | "ProductFamiliesService"
        | "AddOnsService"
        | "CouponsService"
        | "SchedulesService"
        | "BillableMetricsService"
        | "EntitlementsService"
        | "PurchaseLinksService" => rw(method, CatalogRead, CatalogManage),

        // ---- alerting & Signals ----
        "AlertsService" | "SignalsService" => rw(method, AlertsRead, AlertsManage),
        // Apart from alerts: running monitors does not rewrite the plan. Actuals also need
        // analytics:read, checked in the handler.
        "TargetsService" => rw(method, TargetsRead, TargetsManage),

        // ---- read-only analytics & reporting ----
        "StatsService"
        | "AnalyticsService"
        | "SegmentsService"
        | "ActivityService"
        | "ActivityFeedService"
        | "SavedViewsService"
        | "ActivationService" => RouteAuth::Requires(AnalyticsRead),

        // Raw usage/metering events explorer. Read-only: event ingestion and CSV
        // import run through the metering pipeline and BatchJobsService (settings),
        // not here. Separate from settings so a role can inspect usage without
        // seeing connectors, webhooks or API tokens.
        "EventsService" => RouteAuth::Requires(EventsRead),

        // What core screens read from raw events: discovery is part of reading events, a metric's
        // usage part of the catalog, a customer's usage part of the customer record.
        "EventInsightsService" => match method {
            "DiscoverEventCodes"
            | "DiscoverProperties"
            | "DiscoverPropertyValues"
            | "ProfileEventProperties" => RouteAuth::Requires(EventsRead),
            "QueryMetricUsage" => RouteAuth::Requires(CatalogRead),
            "GetCustomerUsage" => RouteAuth::Requires(CustomersRead),
            _ => return None,
        },

        // ---- settings & integrations ----
        "BankAccountsService"
        | "TaxesService"
        | "TaxReportsService"
        | "ConnectorsService"
        | "ConnectAdminService"
        | "NotificationChannelsService"
        | "ApiTokensService"
        | "BatchJobsService"
        | "CustomPropertiesService"
        | "CustomPropertyDefinitionsService"
        | "CreditUnitsService" => rw(method, SettingsRead, SettingsManage),

        // A correction's annexes are part of the invoice: rebuilding them edits the draft.
        "AnnexTemplatesService" => match method {
            "RegenerateInvoiceAnnexes" => RouteAuth::Requires(InvoicesManage),
            _ => rw(method, SettingsRead, SettingsManage),
        },

        // Outbound webhooks. The `Get` prefix heuristic would hand the plaintext
        // signing secret (forgeable events) and the stored event payload (customer and
        // invoice data) to a settings:read holder, so both are pinned to manage.
        "WebhooksService" => match method {
            "GetWebhookEndpointSecret" | "GetWebhookDelivery" => {
                RouteAuth::Requires(SettingsManage)
            }
            _ => rw(method, SettingsRead, SettingsManage),
        },

        // Tenant context + reference data every authenticated page needs to
        // render (current tenant, tenant list for the switcher, sandbox seeding
        // banner, available currencies for money formatting). Management stays
        // behind settings. Classifying these as read-gated would 403 restricted
        // roles on the app shell itself, stalling every page.
        "TenantsService" => match method {
            "ListTenants"
            | "GetTenantById"
            | "ActiveTenant"
            | "GetTenantSeedingStatus"
            | "ListTenantCurrencies" => RouteAuth::Authenticated,
            _ => rw(method, SettingsRead, SettingsManage),
        },

        // Invoicing entities: reading the entity (legal name, country, branding,
        // currency) is reference data used across customer/invoice/quote views;
        // creating and editing them is a settings operation.
        "InvoicingEntitiesService" => match method {
            "GetInvoicingEntity" | "ListInvoicingEntities" => RouteAuth::Authenticated,
            _ => rw(method, SettingsRead, SettingsManage),
        },
        // An invoicing-entity setting, served by the consolidation module.
        "InvoiceConsolidationService" => rw(method, SettingsRead, SettingsManage),
        "MultiEntityService" => RouteAuth::Requires(SettingsManage),

        // ---- customer emails & payment reminders ----
        // Per-invoice and per-customer operations follow their resource; sequences,
        // sender settings and kind toggles are settings.
        "PaymentRemindersService" => match method {
            "GetInvoiceReminders" | "PreviewInvoiceReminderResolution" => {
                RouteAuth::Requires(InvoicesRead)
            }
            "PauseInvoiceReminders"
            | "ResumeInvoiceReminders"
            | "StopInvoiceReminders"
            | "RestartInvoiceReminders"
            | "SendInvoiceReminderNow" => RouteAuth::Requires(InvoicesManage),
            "GetCustomerReminderPreference" => RouteAuth::Requires(CustomersRead),
            "SetCustomerReminderPreference" => RouteAuth::Requires(CustomersManage),
            _ => rw(method, SettingsRead, SettingsManage),
        },
        // PreviewEmail renders synthetic data (read); PreviewInvoiceEmail reads a real invoice.
        "CustomerEmailsService" => match method {
            "ListSentEmails" | "ListSkippedEmails" | "GetSentEmail" | "PreviewInvoiceEmail" => {
                RouteAuth::Requires(InvoicesRead)
            }
            "ResendEmail" | "ResendUnknownEmail" => RouteAuth::Requires(InvoicesManage),
            _ => rw(method, SettingsRead, SettingsManage),
        },

        // Gates drive which screens render, for any member.
        "FeaturesService" => RouteAuth::Authenticated,
        // Saved list views hold only per-user UI state (filters/columns/sort), never entity data;
        // the lists they drive stay gated by their own services. Private/shared visibility is
        // enforced in the store.
        "ListViewsService" => RouteAuth::Authenticated,

        // ---- governance ----
        "SecurityAuditService" => RouteAuth::Requires(OrganizationManage),
        "SsoService" => RouteAuth::Requires(OrganizationManage),
        "DomainsService" => RouteAuth::Requires(OrganizationManage),

        "OrganizationsService" => match method {
            // Org selection / creation happen before an org context exists.
            "ListOrganizations" | "CreateOrganization" | "GetCurrentOrganizations" => {
                RouteAuth::Authenticated
            }
            _ => RouteAuth::Requires(OrganizationManage),
        },

        "RbacService" => match method {
            // Role names & the permission vocabulary aren't sensitive: any member
            // may read them to render role badges and populate assignment pickers.
            "ListRoles" | "ListPermissions" => RouteAuth::Authenticated,
            // Assigning a role to a member is a members concern, not a roles one.
            _ => RouteAuth::Requires(MembersManage),
        },
        "CustomRolesService" => RouteAuth::Requires(RolesManage),

        "UsersService" => match method {
            // anonymous
            "InitRegistration"
            | "CompleteRegistration"
            | "InitResetPassword"
            | "ResetPassword"
            | "Login"
            | "VerifyMfa" => RouteAuth::Public,
            // authenticated, user-global or pre-org
            "Me"
            | "AcceptInvite"
            | "OnboardMe"
            | "UpdateUserPreferences"
            | "InitMfaEnrollment"
            | "ConfirmMfaEnrollment"
            | "DisableMfa"
            | "RegenerateMfaRecoveryCodes"
            | "GetUserById"
            | "ListUsers"
            | "LeaveOrganization" => RouteAuth::Authenticated,
            // member governance
            "InviteMember" | "ResendInvite" | "RevokeInvite" | "ListPendingInvites"
            | "RemoveMember" => RouteAuth::Requires(MembersManage),
            // Unlisted: fail closed AND fail CI. A catch-all granting a real
            // permission is what silently gated UpdateUserPreferences behind
            // members:manage — governance a plain Member never holds.
            _ => return None,
        },

        // Licenses a tenant issues to its customers. The activation code is a bearer secret.
        "LicensesService" => match method {
            "RotateSigningKey" => RouteAuth::Requires(SettingsManage),
            "GetActivationCode" => RouteAuth::Requires(SubscriptionsManage),
            _ => rw(method, SubscriptionsRead, SubscriptionsManage),
        },

        // The instance license governs every organization on the instance.
        "LicensingService" => RouteAuth::Requires(OrganizationManage),

        "InstanceService" => match method {
            "GetInstance" | "GetInviteDetails" | "GetCountries" => RouteAuth::Public,
            _ => RouteAuth::Authenticated,
        },

        "ConnectClientService" => match method {
            "GetOnboardingInfo" => RouteAuth::Public,
            _ => RouteAuth::Authenticated,
        },

        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;
    use prost_types::FileDescriptorSet;

    fn gm(service: &str, method: &str) -> GrpcServiceMethod {
        GrpcServiceMethod {
            service: service.to_string(),
            method: method.to_string(),
        }
    }

    /// The fail-proof guarantee: every RPC under `meteroid.api.*` must be
    /// classified. Adding an RPC without a classification here fails CI.
    #[test]
    fn every_api_method_is_classified() {
        let set = FileDescriptorSet::decode(meteroid_grpc::_reflection::FILE_DESCRIPTOR_SET)
            .expect("decode descriptor set");

        let mut unclassified = Vec::new();
        for file in &set.file {
            let package = file.package();
            if !package.starts_with("meteroid.api.") {
                continue;
            }
            for service in &file.service {
                let full_service = format!("{package}.{}", service.name());
                for method in &service.method {
                    if method_auth(&gm(&full_service, method.name())).is_none() {
                        unclassified.push(format!("{full_service}/{}", method.name()));
                    }
                }
            }
        }

        assert!(
            unclassified.is_empty(),
            "these meteroid.api.* RPCs have no RBAC classification in method_auth() — \
             add each to server/auth/rbac.rs:\n{}",
            unclassified.join("\n")
        );
    }

    /// A user editing their own row is not governance. These are keyed by the
    /// caller's own id, so requiring a permission locks a plain Member out of
    /// their own account.
    #[test]
    fn self_service_user_routes_need_no_permission() {
        let member = RolePermissions::from_codes(Vec::<&str>::new());
        for method in [
            "Me",
            "OnboardMe",
            "UpdateUserPreferences",
            "InitMfaEnrollment",
            "DisableMfa",
            "LeaveOrganization",
        ] {
            let sm = gm("meteroid.api.users.v1.UsersService", method);
            assert_eq!(
                method_auth(&sm),
                Some(RouteAuth::Authenticated),
                "{method} should not require a permission"
            );
            assert!(enforce(&sm, &member).is_ok(), "{method} denied to a member");
        }

        // Governance on the same service still is.
        let sm = gm("meteroid.api.users.v1.UsersService", "RemoveMember");
        assert_eq!(method_auth(&sm), Some(RouteAuth::Requires(MembersManage)));
        assert!(enforce(&sm, &member).is_err());
    }

    #[test]
    fn webhook_secrets_and_payloads_need_settings_manage() {
        let reader = RolePermissions::from_codes(["settings:read"]);
        for method in ["GetWebhookEndpointSecret", "GetWebhookDelivery"] {
            let sm = gm("meteroid.api.webhooks.out.v1.WebhooksService", method);
            assert_eq!(
                method_auth(&sm),
                Some(RouteAuth::Requires(SettingsManage)),
                "{method} must not be a read"
            );
            assert!(
                enforce(&sm, &reader).is_err(),
                "{method} was granted to settings:read"
            );
        }

        let list = gm(
            "meteroid.api.webhooks.out.v1.WebhooksService",
            "ListWebhookEndpoints",
        );
        assert_eq!(method_auth(&list), Some(RouteAuth::Requires(SettingsRead)));
        assert!(enforce(&list, &reader).is_ok());
    }

    #[test]
    fn resource_read_write_split() {
        assert_eq!(
            method_auth(&gm(
                "meteroid.api.customers.v1.CustomersService",
                "ListCustomers"
            )),
            Some(RouteAuth::Requires(CustomersRead))
        );
        assert_eq!(
            method_auth(&gm(
                "meteroid.api.customers.v1.CustomersService",
                "CreateCustomer"
            )),
            Some(RouteAuth::Requires(CustomersManage))
        );
        // Account ownership: assigning is a mutation -> customers:manage;
        // reading the owner rides customers:read via the Customer read path. Verb-based,
        // so no new permission code.
        assert_eq!(
            method_auth(&gm(
                "meteroid.api.customers.v1.CustomersService",
                "AssignCustomerOwner"
            )),
            Some(RouteAuth::Requires(CustomersManage))
        );
        assert_eq!(
            method_auth(&gm("meteroid.api.budgets.v1.BudgetsService", "ListBudgets")),
            Some(RouteAuth::Requires(CustomersRead))
        );
        let consolidation = "meteroid.api.consolidation.v1.InvoiceConsolidationService";
        assert_eq!(
            method_auth(&gm(consolidation, "GetInvoiceConsolidation")),
            Some(RouteAuth::Requires(SettingsRead))
        );
        // Same permission as editing the invoicing entity it used to be a field of.
        assert_eq!(
            method_auth(&gm(consolidation, "SetInvoiceConsolidation")),
            Some(RouteAuth::Requires(SettingsManage))
        );
    }

    #[test]
    fn billing_areas_require_their_own_permission() {
        let create_sub = gm(
            "meteroid.api.subscriptions.v1.SubscriptionsService",
            "CreateSubscription",
        );
        assert_eq!(
            method_auth(&create_sub),
            Some(RouteAuth::Requires(SubscriptionsManage))
        );
        // A subscriptions-only role cannot touch invoices, and vice versa.
        let subs_only = RolePermissions::from_codes(["billing:subscriptions:manage"]);
        assert!(enforce(&create_sub, &subs_only).is_ok());
        for (service, method, read) in [
            (
                "meteroid.api.invoices.v1.InvoicesService",
                "ListInvoices",
                InvoicesRead,
            ),
            (
                "meteroid.api.creditnotes.v1.CreditNotesService",
                "ListCreditNotes",
                InvoicesRead,
            ),
            (
                "meteroid.api.quotes.v1.QuotesService",
                "ListQuotes",
                QuotesRead,
            ),
            (
                "meteroid.api.wallets.v1.WalletsService",
                "ListCustomerWallets",
                InvoicesRead,
            ),
            (
                "meteroid.api.walletcontrols.v1.WalletControlsService",
                "ListCustomerCreditOutlooks",
                InvoicesRead,
            ),
        ] {
            let sm = gm(service, method);
            assert_eq!(method_auth(&sm), Some(RouteAuth::Requires(read)));
            assert!(enforce(&sm, &subs_only).is_err());
        }
        // Pre-split roles retain access to every billing area.
        let legacy = RolePermissions::from_codes(["billing:manage"]);
        assert!(enforce(&create_sub, &legacy).is_ok());
        assert!(
            enforce(
                &gm("meteroid.api.invoices.v1.InvoicesService", "RefreshInvoice"),
                &legacy
            )
            .is_ok()
        );
    }

    #[test]
    fn event_insights_follow_what_they_read() {
        let service = "meteroid.api.eventinsights.v1.EventInsightsService";
        for (method, permission) in [
            ("DiscoverEventCodes", EventsRead),
            ("DiscoverProperties", EventsRead),
            ("DiscoverPropertyValues", EventsRead),
            ("ProfileEventProperties", EventsRead),
            ("QueryMetricUsage", CatalogRead),
            ("GetCustomerUsage", CustomersRead),
        ] {
            assert_eq!(
                method_auth(&gm(service, method)),
                Some(RouteAuth::Requires(permission)),
                "{method}"
            );
        }
        assert_eq!(method_auth(&gm(service, "Unknown")), None);
    }

    #[test]
    fn events_require_events_read_not_settings() {
        let sm = gm("meteroid.api.events.v1.EventsService", "SearchEvents");
        assert_eq!(method_auth(&sm), Some(RouteAuth::Requires(EventsRead)));
        // A settings role no longer implies visibility into raw usage events.
        assert!(enforce(&sm, &RolePermissions::from_codes(["settings:manage"])).is_err());
        assert!(enforce(&sm, &RolePermissions::from_codes(["events:read"])).is_ok());
    }

    #[test]
    fn signals_classification() {
        let alerts = |m| gm("meteroid.api.alerts.v1.AlertsService", m);
        let signals = |m| gm("meteroid.api.signals.v1.SignalsService", m);
        let support = RolePermissions::read_only();

        for sm in [signals("ListSignals"), signals("GetSignal")] {
            assert_eq!(method_auth(&sm), Some(RouteAuth::Requires(AlertsRead)));
            assert!(
                enforce(&sm, &support).is_ok(),
                "{} denied read-only",
                sm.method
            );
        }

        for sm in [signals("DismissSignals"), alerts("UpdateBuiltinSignal")] {
            assert_eq!(method_auth(&sm), Some(RouteAuth::Requires(AlertsManage)));
            assert!(
                enforce(&sm, &support).is_err(),
                "{} allowed read-only",
                sm.method
            );
            assert!(enforce(&sm, &RolePermissions::from_codes(["alerts:manage"])).is_ok());
        }

        // A monitor's chart is a read; muting it is managing it.
        assert_eq!(
            method_auth(&alerts("GetMonitorSeries")),
            Some(RouteAuth::Requires(AlertsRead))
        );
        assert_eq!(
            method_auth(&alerts("MuteAlertDefinition")),
            Some(RouteAuth::Requires(AlertsManage))
        );
        assert_eq!(
            method_auth(&alerts("PreviewMonitorSeries")),
            Some(RouteAuth::Requires(AlertsRead))
        );

        let targets = |m| gm("meteroid.api.targets.v1.TargetsService", m);
        for sm in [
            targets("ListTargets"),
            targets("GetTarget"),
            targets("ListTargetPeriods"),
            targets("ListPlanChanges"),
        ] {
            assert_eq!(method_auth(&sm), Some(RouteAuth::Requires(TargetsRead)));
            assert!(
                enforce(&sm, &support).is_ok(),
                "{} denied read-only",
                sm.method
            );
        }
        let monitors_only = RolePermissions::from_codes(["alerts:manage", "analytics:read"]);
        for sm in [
            targets("CreateTarget"),
            targets("UpdateTarget"),
            targets("DeleteTarget"),
            targets("ImportTargets"),
            targets("LoadDemoPlan"),
        ] {
            assert_eq!(method_auth(&sm), Some(RouteAuth::Requires(TargetsManage)));
            assert!(
                enforce(&sm, &support).is_err(),
                "{} allowed read-only",
                sm.method
            );
            assert!(
                enforce(&sm, &monitors_only).is_err(),
                "{} allowed to a role that manages alerts only",
                sm.method
            );
        }

        let assign_owner = gm(
            "meteroid.api.customers.v1.CustomersService",
            "AssignCustomerOwner",
        );
        assert_eq!(
            method_auth(&assign_owner),
            Some(RouteAuth::Requires(CustomersManage))
        );
        assert!(
            enforce(
                &assign_owner,
                &RolePermissions::from_codes(["alerts:manage"])
            )
            .is_err()
        );
    }

    #[test]
    fn customer_emails_split_between_settings_and_invoices() {
        const SVC: &str = "meteroid.api.customeremails.v1.CustomerEmailsService";
        let settings_read = RolePermissions::from_codes(["settings:read"]);
        let invoices_read = RolePermissions::from_codes(["billing:invoices:read"]);

        for (method, permission) in [
            ("GetEmailSettings", SettingsRead),
            ("ListEmailVariables", SettingsRead),
            ("PreviewEmail", SettingsRead),
            ("UpdateEmailSettings", SettingsManage),
            ("SetEmailKindEnabled", SettingsManage),
            ("SetEmailKindAudience", SettingsManage),
            ("ClearEmailKindAudience", SettingsManage),
            ("PreviewEmailKindAudience", SettingsRead),
            ("SendTestEmail", SettingsManage),
            ("ListSentEmails", InvoicesRead),
            ("ListSkippedEmails", InvoicesRead),
            ("GetSentEmail", InvoicesRead),
            ("PreviewInvoiceEmail", InvoicesRead),
            ("ResendEmail", InvoicesManage),
            ("ResendUnknownEmail", InvoicesManage),
        ] {
            assert_eq!(
                method_auth(&gm(SVC, method)),
                Some(RouteAuth::Requires(permission)),
                "{method}"
            );
        }

        // Synthetic previews are a settings read; previews of a real invoice are not.
        assert!(enforce(&gm(SVC, "PreviewEmail"), &settings_read).is_ok());
        assert!(enforce(&gm(SVC, "PreviewInvoiceEmail"), &settings_read).is_err());
        assert!(enforce(&gm(SVC, "PreviewInvoiceEmail"), &invoices_read).is_ok());
        assert!(enforce(&gm(SVC, "GetSentEmail"), &invoices_read).is_ok());
        assert!(enforce(&gm(SVC, "GetEmailSettings"), &invoices_read).is_err());
        // "Send to" rules are entity settings: a settings reader previews, only a manager writes.
        assert!(enforce(&gm(SVC, "PreviewEmailKindAudience"), &settings_read).is_ok());
        assert!(enforce(&gm(SVC, "SetEmailKindAudience"), &settings_read).is_err());
        assert!(enforce(&gm(SVC, "ClearEmailKindAudience"), &settings_read).is_err());
        // Recovering a delivery-unknown email is an invoice action, like a resend.
        let invoices_manage = RolePermissions::from_codes(["billing:invoices:manage"]);
        assert!(enforce(&gm(SVC, "ResendUnknownEmail"), &invoices_manage).is_ok());
        assert!(enforce(&gm(SVC, "ResendUnknownEmail"), &invoices_read).is_err());
        assert!(enforce(&gm(SVC, "ResendUnknownEmail"), &settings_read).is_err());
    }

    #[test]
    fn payment_reminders_follow_their_resource() {
        const SVC: &str = "meteroid.api.paymentreminders.v1.PaymentRemindersService";
        for (method, permission) in [
            ("ListPaymentReminderSequences", SettingsRead),
            ("GetPaymentReminderSequence", SettingsRead),
            ("PreviewPaymentReminderAudience", SettingsRead),
            ("CreatePaymentReminderSequence", SettingsManage),
            ("UpdatePaymentReminderSequence", SettingsManage),
            ("ReorderPaymentReminderSequences", SettingsManage),
            ("ArchivePaymentReminderSequence", SettingsManage),
            ("GetInvoiceReminders", InvoicesRead),
            ("PreviewInvoiceReminderResolution", InvoicesRead),
            ("PauseInvoiceReminders", InvoicesManage),
            ("ResumeInvoiceReminders", InvoicesManage),
            ("StopInvoiceReminders", InvoicesManage),
            ("RestartInvoiceReminders", InvoicesManage),
            ("SendInvoiceReminderNow", InvoicesManage),
            ("GetCustomerReminderPreference", CustomersRead),
            ("SetCustomerReminderPreference", CustomersManage),
        ] {
            assert_eq!(
                method_auth(&gm(SVC, method)),
                Some(RouteAuth::Requires(permission)),
                "{method}"
            );
        }

        let customers_only = RolePermissions::from_codes(["customers:read"]);
        assert!(enforce(&gm(SVC, "GetCustomerReminderPreference"), &customers_only).is_ok());
        assert!(enforce(&gm(SVC, "ListPaymentReminderSequences"), &customers_only).is_err());
    }

    #[test]
    fn unknown_service_is_denied() {
        assert_eq!(
            method_auth(&gm("meteroid.api.unknown.v1.GhostService", "Do")),
            None
        );
    }

    #[test]
    fn member_denied_governance_admin_allowed() {
        let sm = gm(
            "meteroid.api.rbac.v1.CustomRolesService",
            "CreateCustomRole",
        );
        assert!(enforce(&sm, &RolePermissions::All).is_ok());
        assert!(enforce(&sm, &RolePermissions::member_default()).is_err());
    }

    #[test]
    fn module_owned_services_follow_their_module_policy() {
        let sm = gm(
            "meteroid.admin.platform.v1.PlatformAdminService",
            "ListOrganizations",
        );
        let policies = [RoutePolicy {
            service: "meteroid.admin.platform.v1.PlatformAdminService",
            auth: RouteAuth::Authenticated,
        }];
        assert!(enforce(&sm, &RolePermissions::member_default()).is_err());
        assert!(enforce_with(&sm, &RolePermissions::member_default(), &policies).is_ok());
    }
}
