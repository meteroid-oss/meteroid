use http::HeaderMap;
use secrecy::SecretString;
use tonic::Status;
use tracing::log;

use chrono::{DateTime, NaiveDate, Utc};
use common_domain::auth::PortalScope;
use common_domain::ids::TenantId;
use common_grpc::GrpcServiceMethod;
use common_grpc::middleware::common::auth::PORTAL_KEY_HEADER;
use common_grpc::middleware::server::AuthorizedState;
use common_grpc::middleware::server::auth::{
    AuthenticatedState, AuthorizedAsPortalUser, ResourceAccess as GrpcResourceAccess,
};
use meteroid_store::Store;
use meteroid_store::domain::portal_tokens::{
    PORTAL_LINK_EXPIRED, PortalTokenStatus, is_portal_token,
};
use meteroid_store::jwt_claims::{PortalJwtClaims, PortalTokenError, ResourceAccess};
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;

/// Customer portal methods open to a token without the `manage` scope. Fails closed: any other
/// method, including one added later, requires `manage`.
const CUSTOMER_READ_METHODS: &[(&str, &[&str])] = &[
    (
        "meteroid.portal.customer.v1.PortalCustomerService",
        &[
            "GetCustomerPortalOverview",
            "ListInvoices",
            "ListLicenses",
            "ListUsageLimits",
        ],
    ),
    (
        "meteroid.portal.subscription.v1.PortalSubscriptionService",
        &[
            "GetSubscriptionDetails",
            "ListAvailablePlans",
            "PreviewPlanChange",
            "GetUpcomingInvoice",
            "GetSubscriptionComponentUsage",
            "ListAvailableAddOns",
            "ListAdjustableComponents",
            "PreviewAddOnQuantity",
            "PreviewSeats",
        ],
    ),
    (
        "meteroid.portal.invoice.v1.PortalInvoiceService",
        &["GetInvoicePayment"],
    ),
    (
        "meteroid.portal.wallets.v1.PortalWalletsService",
        &[
            "GetBalances",
            "GetCreditOverview",
            "ListLedger",
            "QuoteTopUp",
            "GetTopUp",
        ],
    ),
    (
        "meteroid.portal.walletcontrols.v1.PortalWalletControlsService",
        &["PreviewAutoTopUp"],
    ),
    (
        "meteroid.portal.budgets.v1.PortalBudgetsService",
        &["ListBudgets"],
    ),
    (
        "meteroid.portal.checkout.v1.PortalCheckoutService",
        &["GetSlotUpgradeCheckout"],
    ),
];

/// Services whose writes buy or grant credit or set spending controls. A delegated session may
/// only read them, as its credentials cannot reach the matching REST routes.
const DELEGATED_READ_ONLY_SERVICES: &[&str] = &[
    "meteroid.portal.wallets.v1.PortalWalletsService",
    "meteroid.portal.walletcontrols.v1.PortalWalletControlsService",
    "meteroid.portal.budgets.v1.PortalBudgetsService",
];

fn delegated_allows(gm: &GrpcServiceMethod) -> bool {
    !DELEGATED_READ_ONLY_SERVICES.contains(&gm.service.as_str()) || is_customer_read_method(gm)
}

fn is_customer_read_method(gm: &GrpcServiceMethod) -> bool {
    CUSTOMER_READ_METHODS
        .iter()
        .any(|(service, methods)| gm.service == *service && methods.contains(&gm.method.as_str()))
}

/// `None` is a legacy token minted before scopes, which keeps full access.
fn customer_scopes_allow(scopes: Option<&[PortalScope]>, gm: &GrpcServiceMethod) -> bool {
    match scopes {
        None => true,
        Some(scopes) if scopes.contains(&PortalScope::Manage) => true,
        Some(scopes) => scopes.contains(&PortalScope::Read) && is_customer_read_method(gm),
    }
}

pub async fn validate_portal_token(
    header_map: &HeaderMap,
    jwt_secret: SecretString,
    store: &Store,
) -> Result<AuthenticatedState, Status> {
    let token = header_map
        .get(PORTAL_KEY_HEADER)
        .ok_or(Status::unauthenticated("Missing portal token"))?
        .to_str()
        .map_err(|_| Status::unauthenticated("Invalid portal token"))?;

    if is_portal_token(token) {
        return validate_stored_portal_token(token, store).await;
    }
    validate_portal_jwt(
        token,
        &jwt_secret,
        store.settings.portal_legacy_tokens_until,
        Utc::now(),
    )
}

async fn validate_stored_portal_token(
    token: &str,
    store: &Store,
) -> Result<AuthenticatedState, Status> {
    let portal_token = store
        .authenticate_portal_token(token)
        .await
        .map_err(|err| {
            log::error!("Error resolving portal token: {err:?}");
            Status::internal("Failed to resolve portal token")
        })?
        .ok_or_else(|| Status::unauthenticated("Invalid portal token"))?;

    match portal_token.status(Utc::now()) {
        PortalTokenStatus::Active => {}
        PortalTokenStatus::Expired => return Err(Status::unauthenticated(PORTAL_LINK_EXPIRED)),
        PortalTokenStatus::Revoked => return Err(Status::unauthenticated("Invalid portal token")),
    }

    Ok(AuthenticatedState::Shared {
        tenant_id: portal_token.tenant_id,
        resource_access: to_grpc_resource(portal_token.resource),
        scopes: portal_token.scopes,
        delegated: portal_token.delegated,
    })
}

/// Tokens minted before stored portal tokens. The non-expiring ones (invoice, quote, checkout
/// and dashboard links) stop working after `legacy_until`.
fn validate_portal_jwt(
    token: &str,
    jwt_secret: &SecretString,
    legacy_until: Option<NaiveDate>,
    now: DateTime<Utc>,
) -> Result<AuthenticatedState, Status> {
    let decoded = PortalJwtClaims::decode(token, jwt_secret).map_err(|err| match err {
        PortalTokenError::Expired => Status::unauthenticated("JWT expired"),
        PortalTokenError::Invalid => Status::unauthenticated("Invalid JWT"),
    })?;

    // A customer token minted before scopes and this claim existed cannot prove it was not
    // opened by delegated credentials.
    let delegated = decoded.delegated
        || matches!(decoded.resource, ResourceAccess::Customer(_)) && decoded.scopes.is_none();
    if decoded.exp.is_none() {
        if legacy_until.is_some_and(|until| now.date_naive() > until) {
            return Err(Status::unauthenticated("JWT expired"));
        }
        log::info!(
            "Legacy non-expiring portal JWT used (tenant {})",
            decoded.tenant_id
        );
    }

    Ok(AuthenticatedState::Shared {
        tenant_id: decoded.tenant_id,
        resource_access: to_grpc_resource(decoded.resource),
        scopes: decoded.scopes,
        delegated,
    })
}

fn to_grpc_resource(resource: ResourceAccess) -> GrpcResourceAccess {
    match resource {
        ResourceAccess::CheckoutSession(id) => GrpcResourceAccess::CheckoutSession(id),
        ResourceAccess::Customer(id) => GrpcResourceAccess::CustomerPortal(id),
        ResourceAccess::Invoice(id) => GrpcResourceAccess::InvoicePortal(id),
        ResourceAccess::Quote {
            quote_id,
            recipient_email,
        } => GrpcResourceAccess::QuotePortal {
            quote_id,
            recipient_email,
        },
        ResourceAccess::Dashboard(id) => GrpcResourceAccess::DashboardPortal(id),
    }
}

pub fn authorize_portal(
    tenant_id: TenantId,
    resource_access: common_grpc::middleware::server::auth::ResourceAccess,
    scopes: Option<Vec<PortalScope>>,
    delegated: bool,
    gm: GrpcServiceMethod,
) -> Result<AuthorizedState, Status> {
    match resource_access {
        common_grpc::middleware::server::auth::ResourceAccess::QuotePortal { .. } => {
            if !gm.service.starts_with("meteroid.portal.quotes.") {
                return Err(Status::permission_denied("Unauthorized"));
            }
        }
        common_grpc::middleware::server::auth::ResourceAccess::InvoicePortal { .. } => {
            if !gm.service.starts_with("meteroid.portal.invoice.")
                && !gm.service.starts_with("meteroid.portal.shared.")
            {
                return Err(Status::permission_denied("Unauthorized"));
            }
        }
        common_grpc::middleware::server::auth::ResourceAccess::CheckoutSession { .. } => {
            if !gm.service.starts_with("meteroid.portal.checkout.")
                && !gm.service.starts_with("meteroid.portal.shared.")
            {
                return Err(Status::permission_denied("Unauthorized"));
            }
        }
        common_grpc::middleware::server::auth::ResourceAccess::CustomerPortal { .. } => {
            if !gm.service.starts_with("meteroid.portal.invoice.")
                && !gm.service.starts_with("meteroid.portal.checkout.")
                && !gm.service.starts_with("meteroid.portal.shared.")
                && !gm.service.starts_with("meteroid.portal.customer.")
                && !gm.service.starts_with("meteroid.portal.subscription.")
                && !gm.service.starts_with("meteroid.portal.wallets.")
                && !gm.service.starts_with("meteroid.portal.walletcontrols.")
                && !gm.service.starts_with("meteroid.portal.budgets.")
            {
                return Err(Status::permission_denied("Unauthorized"));
            }
            if !customer_scopes_allow(scopes.as_deref(), &gm) {
                return Err(Status::permission_denied(
                    "The token's scopes do not allow this operation.",
                ));
            }
            if delegated && !delegated_allows(&gm) {
                return Err(Status::permission_denied(
                    "This operation is not available in a portal session opened by a platform.",
                ));
            }
        }
        common_grpc::middleware::server::auth::ResourceAccess::DashboardPortal { .. } => {
            if !gm.service.starts_with("meteroid.portal.dashboard.") {
                return Err(Status::permission_denied("Unauthorized"));
            }
        }
    }

    Ok(AuthorizedState::Shared(AuthorizedAsPortalUser {
        tenant_id,
        resource_access,
        scopes,
        delegated,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use common_domain::ids::{BaseId, CustomerId, InvoiceId};
    use common_grpc::middleware::server::auth::ResourceAccess as PortalResource;
    use meteroid_store::jwt_claims::generate_portal_token;
    use prost::Message;
    use prost_types::FileDescriptorSet;
    use tonic::Code;

    const SUBSCRIPTION: &str = "meteroid.portal.subscription.v1.PortalSubscriptionService";
    const SHARED: &str = "meteroid.portal.shared.v1.PortalSharedService";
    const WALLETS: &str = "meteroid.portal.wallets.v1.PortalWalletsService";
    const WALLET_CONTROLS: &str = "meteroid.portal.walletcontrols.v1.PortalWalletControlsService";

    fn gm(service: &str, method: &str) -> GrpcServiceMethod {
        GrpcServiceMethod {
            service: service.to_string(),
            method: method.to_string(),
        }
    }

    fn secret() -> SecretString {
        SecretString::from("test-secret".to_string())
    }

    fn authorize_customer(
        scopes: Option<Vec<PortalScope>>,
        service: &str,
        method: &str,
    ) -> Result<AuthorizedState, Status> {
        authorize_portal(
            TenantId::new(),
            PortalResource::CustomerPortal(CustomerId::new()),
            scopes,
            false,
            gm(service, method),
        )
    }

    fn validate(token: &str) -> Result<AuthenticatedState, Status> {
        validate_portal_jwt(token, &secret(), None, Utc::now())
    }

    #[test]
    fn read_token_is_limited_to_the_allowlist() {
        let read = Some(vec![PortalScope::Read]);

        assert!(authorize_customer(read.clone(), SUBSCRIPTION, "GetSubscriptionDetails").is_ok());
        assert!(authorize_customer(read.clone(), WALLETS, "GetBalances").is_ok());
        assert!(authorize_customer(read.clone(), WALLETS, "GetCreditOverview").is_ok());
        assert!(authorize_customer(read.clone(), WALLET_CONTROLS, "PreviewAutoTopUp").is_ok());
        assert!(
            authorize_customer(
                read.clone(),
                "meteroid.portal.customer.v1.PortalCustomerService",
                "ListUsageLimits"
            )
            .is_ok()
        );
        for (service, method) in [
            (WALLETS, "TopUpWallet"),
            (WALLET_CONTROLS, "SetAutoTopUp"),
            (SUBSCRIPTION, "ConfirmPlanChange"),
            (SUBSCRIPTION, "CancelSubscription"),
            (SHARED, "SetupIntent"),
            (SHARED, "UpdateCustomer"),
            (
                "meteroid.portal.invoice.v1.PortalInvoiceService",
                "ConfirmInvoicePayment",
            ),
        ] {
            let denied = authorize_customer(read.clone(), service, method).err();
            assert_eq!(
                denied.map(|s| s.code()),
                Some(Code::PermissionDenied),
                "{service}/{method}"
            );
        }
    }

    #[test]
    fn manage_token_has_full_customer_access() {
        for scopes in [PortalScope::ALL.to_vec(), vec![PortalScope::Manage]] {
            for (service, method) in [
                (SUBSCRIPTION, "GetSubscriptionDetails"),
                (SUBSCRIPTION, "ConfirmPlanChange"),
                (SHARED, "SetupIntent"),
            ] {
                assert!(
                    authorize_customer(Some(scopes.clone()), service, method).is_ok(),
                    "{scopes:?} {service}/{method}"
                );
            }
        }
    }

    #[test]
    fn legacy_customer_token_keeps_full_access() {
        assert!(authorize_customer(None, SUBSCRIPTION, "ConfirmPlanChange").is_ok());
        assert!(authorize_customer(None, SHARED, "SetupIntent").is_ok());
    }

    #[test]
    fn scopes_do_not_widen_the_customer_services() {
        let denied = authorize_customer(
            Some(PortalScope::ALL.to_vec()),
            "meteroid.portal.quotes.v1.PortalQuoteService",
            "SignQuote",
        )
        .err();
        assert_eq!(denied.map(|s| s.code()), Some(Code::PermissionDenied));

        let empty = authorize_customer(Some(vec![]), SUBSCRIPTION, "GetSubscriptionDetails").err();
        assert_eq!(empty.map(|s| s.code()), Some(Code::PermissionDenied));
    }

    #[test]
    fn other_resources_keep_their_rules() {
        let invoice = authorize_portal(
            TenantId::new(),
            PortalResource::InvoicePortal(InvoiceId::new()),
            None,
            false,
            gm(SHARED, "SetupIntent"),
        );
        assert!(invoice.is_ok());
    }

    #[test]
    fn token_scopes_reach_the_authenticated_state() {
        let token = generate_portal_token(
            &secret(),
            TenantId::new(),
            meteroid_store::jwt_claims::ResourceAccess::Customer(CustomerId::new()),
            Some(vec![PortalScope::Read]),
            Some(chrono::Duration::hours(1)),
            false,
        )
        .unwrap();

        match validate(&token).ok() {
            Some(AuthenticatedState::Shared { scopes, .. }) => {
                assert_eq!(scopes, Some(vec![PortalScope::Read]))
            }
            _ => panic!("expected a portal state"),
        }
    }

    #[test]
    fn a_delegated_session_only_reads_credit_and_spending_controls() {
        let token = PortalJwtClaims::new(
            TenantId::new(),
            meteroid_store::jwt_claims::ResourceAccess::Customer(CustomerId::new()),
            None,
            Some(chrono::Duration::hours(1)),
        )
        .unwrap()
        .with_delegated(true)
        .encode(&secret())
        .unwrap();
        let Some(AuthenticatedState::Shared {
            tenant_id,
            resource_access,
            scopes,
            delegated,
        }) = validate(&token).ok()
        else {
            panic!("expected a portal state");
        };
        assert!(delegated);
        let authorize = |service: &str, method: &str| {
            authorize_portal(
                tenant_id,
                resource_access.clone(),
                scopes.clone(),
                delegated,
                gm(service, method),
            )
            .map(|_| ())
            .map_err(|status| status.code())
        };
        const BUDGETS: &str = "meteroid.portal.budgets.v1.PortalBudgetsService";
        for (service, method) in [
            (WALLETS, "TopUpWallet"),
            (WALLET_CONTROLS, "SetAutoTopUp"),
            (WALLETS, "RedeemCode"),
            (BUDGETS, "CreateBudget"),
            (BUDGETS, "UpdateBudget"),
            (BUDGETS, "DeleteBudget"),
        ] {
            assert_eq!(
                authorize(service, method),
                Err(Code::PermissionDenied),
                "{service}/{method}"
            );
        }
        for (service, method) in [
            (WALLETS, "GetBalances"),
            (WALLETS, "QuoteTopUp"),
            (BUDGETS, "ListBudgets"),
            (SUBSCRIPTION, "ConfirmPlanChange"),
            (SHARED, "SetupIntent"),
        ] {
            assert_eq!(authorize(service, method), Ok(()), "{service}/{method}");
        }

        let direct = generate_portal_token(
            &secret(),
            TenantId::new(),
            meteroid_store::jwt_claims::ResourceAccess::Customer(CustomerId::new()),
            None,
            Some(chrono::Duration::hours(1)),
            false,
        )
        .unwrap();
        assert!(matches!(
            validate(&direct).ok(),
            Some(AuthenticatedState::Shared {
                delegated: false,
                ..
            })
        ));
        assert!(authorize_customer(None, WALLETS, "TopUpWallet").is_ok());
    }

    #[test]
    fn a_legacy_customer_token_is_treated_as_delegated() {
        let mut legacy = PortalJwtClaims::new(
            TenantId::new(),
            meteroid_store::jwt_claims::ResourceAccess::Customer(CustomerId::new()),
            None,
            Some(chrono::Duration::hours(1)),
        )
        .unwrap();
        legacy.scopes = None;
        let legacy = legacy.encode(&secret()).unwrap();
        assert!(matches!(
            validate(&legacy).ok(),
            Some(AuthenticatedState::Shared {
                delegated: true,
                ..
            })
        ));
    }

    #[test]
    fn bad_tokens_are_unauthenticated() {
        let expired = generate_portal_token(
            &secret(),
            TenantId::new(),
            meteroid_store::jwt_claims::ResourceAccess::Customer(CustomerId::new()),
            None,
            Some(chrono::Duration::seconds(-10)),
            false,
        )
        .unwrap();

        for token in [expired.as_str(), "not-a-jwt"] {
            assert_eq!(
                validate(token).err().map(|s| s.code()),
                Some(Code::Unauthenticated)
            );
        }
    }

    #[test]
    fn non_expiring_legacy_tokens_stop_after_the_cutoff() {
        let invoice_token = generate_portal_token(
            &secret(),
            TenantId::new(),
            meteroid_store::jwt_claims::ResourceAccess::Invoice(InvoiceId::new()),
            None,
            None,
            false,
        )
        .unwrap();
        let sdk_token = generate_portal_token(
            &secret(),
            TenantId::new(),
            meteroid_store::jwt_claims::ResourceAccess::Customer(CustomerId::new()),
            None,
            Some(chrono::Duration::hours(1)),
            false,
        )
        .unwrap();
        let cutoff = NaiveDate::from_ymd_opt(2026, 6, 30);
        let before = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let after = Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap();

        assert!(validate_portal_jwt(&invoice_token, &secret(), cutoff, before).is_ok());
        assert_eq!(
            validate_portal_jwt(&invoice_token, &secret(), cutoff, after)
                .err()
                .map(|s| s.code()),
            Some(Code::Unauthenticated)
        );
        assert!(validate_portal_jwt(&sdk_token, &secret(), cutoff, after).is_ok());
    }

    #[test]
    fn read_allowlist_matches_the_protos() {
        let set = FileDescriptorSet::decode(meteroid_grpc::_reflection::FILE_DESCRIPTOR_SET)
            .expect("decode descriptor set");
        let known: Vec<(String, String)> = set
            .file
            .iter()
            .flat_map(|file| {
                file.service.iter().flat_map(move |service| {
                    let full = format!("{}.{}", file.package(), service.name());
                    service
                        .method
                        .iter()
                        .map(move |method| (full.clone(), method.name().to_string()))
                })
            })
            .collect();

        for (service, methods) in CUSTOMER_READ_METHODS {
            for method in *methods {
                assert!(
                    known.contains(&(service.to_string(), method.to_string())),
                    "{service}/{method} is not a known RPC"
                );
            }
        }
    }
}
