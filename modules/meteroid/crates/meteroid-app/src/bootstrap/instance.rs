use common_domain::country::CountryCode;
use common_domain::ids::{ApiTokenId, ConnectorId, UserId};
use common_grpc::middleware::server::auth::api_token_validator::ApiTokenValidator;
use meteroid_store::Store;
use meteroid_store::constants::Currencies;
use meteroid_store::domain::connectors::{MockPublicData, PaymentCredentialsUpdate};
use meteroid_store::domain::entity_activity::Actor;
use meteroid_store::domain::enums::{
    ConnectorProviderEnum, ConnectorTypeEnum, TenantEnvironmentEnum,
};
use meteroid_store::domain::{
    ApiTokenNew, InvoicingEntityProvidersPatch, Organization, OrganizationNew, Tenant, TenantNew,
    TenantUpdate,
};
use meteroid_store::repositories::api_tokens::ApiTokensInterface;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use meteroid_store::repositories::{OrganizationsInterface, TenantInterface, UserInterface};
use secrecy::{ExposeSecret, SecretString};

type BootstrapResult<T> = Result<T, Box<dyn std::error::Error>>;

/// Names the objects this bootstrap owns, so a re-run converges on them instead of
/// creating a second set.
const API_TOKEN_NAME: &str = "bootstrap";
const TEST_GATEWAY_ALIAS: &str = "test-gateway";

/// Brings a fresh instance up with a user, organization, tenant, API key and optionally the
/// test gateway, then emits the credentials as a dotenv fragment.
///
/// Converges instead of running once: each object is looked up by name and the credentials
/// are re-emitted on every boot, since output produced only once is useless to CI. Failures
/// abort startup rather than leave an instance without the credentials it was to publish.
pub async fn bootstrap_instance(
    store: &Store,
    config: &crate::config::InstanceBootstrapConfig,
) -> BootstrapResult<()> {
    if !config.enabled {
        return Ok(());
    }

    let email = required(config.email.as_deref(), "INSTANCE_BOOTSTRAP_EMAIL")?;
    let password = config
        .password
        .clone()
        .ok_or("INSTANCE_BOOTSTRAP_PASSWORD is required when INSTANCE_BOOTSTRAP_ENABLED=true")?;
    let country = CountryCode::parse_as_opt(&config.country).ok_or_else(|| {
        format!(
            "INSTANCE_BOOTSTRAP_COUNTRY is not an ISO-3166 alpha-2 code: {}",
            config.country
        )
    })?;
    let currency = config.currency.trim().to_uppercase();
    if Currencies::resolve_currency(&currency).is_none() {
        return Err(
            format!("INSTANCE_BOOTSTRAP_CURRENCY is not a known currency: {currency}").into(),
        );
    }

    tracing::info!("Bootstrapping instance for {email}");

    let (user, created) = store
        .find_or_create_instance_user(email.to_string(), password)
        .await?;
    if !created {
        tracing::warn!(
            "Instance bootstrap adopted the existing account {email}; \
             INSTANCE_BOOTSTRAP_PASSWORD was not applied to it"
        );
    }
    let actor = Actor::User { id: user.id };

    let Some(organization) =
        find_or_adopt_organization(store, &config.organization, country, user.id).await?
    else {
        return Ok(());
    };

    let tenant = find_or_create_tenant(store, &organization, &config.tenant, &currency).await?;

    let api_key = install_api_key(store, &actor, &tenant, config.api_key.as_ref()).await?;

    if config.test_gateway {
        connect_test_gateway(store, &actor, &tenant).await?;
    }

    emit_credentials(
        config.output_path.as_deref(),
        &credentials_fragment(&api_key, &organization, &tenant),
    )
}

fn required<'a>(value: Option<&'a str>, var: &str) -> BootstrapResult<&'a str> {
    match value.map(str::trim) {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(format!("{var} is required when INSTANCE_BOOTSTRAP_ENABLED=true").into()),
    }
}

/// `None` means this instance is already someone else's: a single-organization instance that
/// was initialised elsewhere. Creating a second organization would be refused on every boot,
/// so the bootstrap stands down with a warning instead of crash-looping the server.
async fn find_or_adopt_organization(
    store: &Store,
    trade_name: &str,
    country: CountryCode,
    user_id: UserId,
) -> BootstrapResult<Option<Organization>> {
    let existing = store
        .list_organizations_for_user(user_id)
        .await?
        .into_iter()
        .find(|organization| organization.trade_name == trade_name);

    if let Some(organization) = existing {
        return Ok(Some(organization));
    }

    let instance = store.get_instance().await?;
    if instance.instance_initiated && !instance.multi_organization_enabled {
        tracing::warn!(
            "Instance bootstrap skipped: this instance already has an organization and \
             ENABLE_MULTI_ORGANIZATION is false, so '{trade_name}' cannot be created. \
             Point INSTANCE_BOOTSTRAP_EMAIL at a member of the existing organization, or \
             disable INSTANCE_BOOTSTRAP_ENABLED."
        );
        return Ok(None);
    }

    let created = store
        .insert_organization(
            OrganizationNew {
                trade_name: trade_name.to_string(),
                country,
                invoicing_entity: None,
            },
            user_id,
        )
        .await?;

    Ok(Some(created.organization))
}

/// `Development`, never `Sandbox`: the sandbox path runs the built-in seeding preset and
/// would fill the tenant with demo data.
async fn find_or_create_tenant(
    store: &Store,
    organization: &Organization,
    name: &str,
    currency: &str,
) -> BootstrapResult<Tenant> {
    let existing = store
        .list_tenants_by_organization_id(organization.id)
        .await?
        .into_iter()
        .find(|tenant| tenant.name == name);

    let tenant = match existing {
        Some(tenant) => tenant,
        None => {
            store
                .insert_tenant(
                    TenantNew {
                        name: name.to_string(),
                        environment: TenantEnvironmentEnum::Development,
                        email_delivery_mode: None,
                        invoicing_entity: None,
                    },
                    organization.id,
                )
                .await?
        }
    };

    let mut currencies: Vec<String> = tenant
        .available_currencies
        .iter()
        .flatten()
        .cloned()
        .collect();
    if !currencies.iter().any(|c| c == currency) {
        currencies.push(currency.to_string());
        store
            .update_tenant_available_currencies(tenant.id, currencies)
            .await?;
    }

    if tenant.reporting_currency == currency {
        return Ok(tenant);
    }

    let updated = store
        .update_tenant(
            Actor::System,
            TenantUpdate {
                trade_name: None,
                name: None,
                slug: None,
                environment: None,
                reporting_currency: Some(currency.to_string()),
                email_delivery_mode: None,
                email_test_recipients: None,
                legacy_disable_emails: None,
                alert_evaluation_interval_secs: None,
                keep_last_events: None,
                portal_embed_origins: None,
            },
            organization.id,
            tenant.id,
        )
        .await?;

    Ok(updated)
}

/// Returns the usable key. A supplied key is installed verbatim, id and all, so it keeps
/// working across re-runs; without one the stored hash can't be read back, so the token is
/// re-minted each boot and the fresh key goes to the output.
///
/// The replacement is always inserted before the old token is dropped, so a run that fails
/// partway leaves the tenant with the key its operators already hold.
async fn install_api_key(
    store: &Store,
    actor: &Actor,
    tenant: &Tenant,
    supplied: Option<&SecretString>,
) -> BootstrapResult<SecretString> {
    let parsed = match supplied {
        Some(key) => Some((
            key.clone(),
            parse_supplied_key(key.expose_secret(), tenant)?,
        )),
        None => None,
    };

    let existing = store
        .find_api_tokens_by_tenant_id(tenant.id)
        .await?
        .into_iter()
        .find(|token| token.name == API_TOKEN_NAME);

    if let (Some((key, (id, _))), Some(existing)) = (&parsed, &existing)
        && existing.id == *id
        && supplied_key_already_installed(store, id, key).await?
    {
        return Ok(key.clone());
    }

    let entity = ApiTokenNew {
        name: API_TOKEN_NAME.to_string(),
        tenant_id: tenant.id,
    };

    // Same id and a different secret is the one case that cannot insert first.
    let replaces_same_row =
        matches!((&parsed, &existing), (Some((_, (id, _))), Some(existing)) if existing.id == *id);
    if replaces_same_row && let Some(existing) = &existing {
        store
            .delete_api_token(actor.clone(), &existing.id, tenant.id)
            .await?;
    }

    let key = match parsed {
        Some((key, (id, secret))) => {
            store
                .insert_api_token_with_key(actor.clone(), entity, id, secret)
                .await?;
            key
        }
        None => {
            let (api_key, _) = store.insert_api_token(actor.clone(), entity).await?;
            SecretString::from(api_key)
        }
    };

    if !replaces_same_row && let Some(existing) = existing {
        store
            .delete_api_token(actor.clone(), &existing.id, tenant.id)
            .await?;
    }

    Ok(key)
}

/// A re-run with an unchanged key is a no-op: the stored hash verifies against it, so the
/// row is already the one the operator asked for.
async fn supplied_key_already_installed(
    store: &Store,
    id: &ApiTokenId,
    key: &SecretString,
) -> BootstrapResult<bool> {
    let stored = store.get_api_token_by_id_for_validation(id).await?;
    let validator = ApiTokenValidator::parse_api_key(key.expose_secret()).map_err(|err| {
        format!("INSTANCE_BOOTSTRAP_API_KEY is not a valid Meteroid API key: {err}")
    })?;

    Ok(validator.validate_hash(&stored.hash).is_ok())
}

/// A key is `pv_<env>_<random>/<base62 token id>`, `<env>` being the tenant's; auth decodes
/// the id from it, so the id the key carries is the row we must insert.
fn parse_supplied_key(
    api_key: &str,
    tenant: &Tenant,
) -> BootstrapResult<(ApiTokenId, SecretString)> {
    // Auth ignores the prefix, but a mismatched one would disagree with the stored hint.
    let prefix = meteroid_store::repositories::api_tokens::key_prefix(&tenant.environment);
    if !api_key.starts_with(&prefix) {
        return Err(format!(
            "INSTANCE_BOOTSTRAP_API_KEY must start with {prefix}, the prefix of the '{}' tenant",
            tenant.name
        )
        .into());
    }

    let validator = ApiTokenValidator::parse_api_key(api_key).map_err(|err| {
        format!("INSTANCE_BOOTSTRAP_API_KEY is not a valid Meteroid API key: {err}")
    })?;
    let id = validator.extract_identifier().map_err(|err| {
        format!("INSTANCE_BOOTSTRAP_API_KEY carries no decodable token id: {err}")
    })?;

    let secret = validator.hash_part().to_string();
    meteroid_store::repositories::api_tokens::validate_api_key_secret(&secret)
        .map_err(|err| format!("INSTANCE_BOOTSTRAP_API_KEY is not usable: {err}"))?;

    Ok((ApiTokenId::from(id), SecretString::from(secret)))
}

/// Connects the mock provider and routes the card and direct-debit rails to it, which is
/// what lets `charge_automatically` subscriptions run without a real gateway.
async fn connect_test_gateway(
    store: &Store,
    actor: &Actor,
    tenant: &Tenant,
) -> BootstrapResult<()> {
    let connectors = store
        .list_connectors(Some(ConnectorTypeEnum::PaymentProvider), tenant.id)
        .await?;
    let live: Vec<ConnectorId> = connectors.iter().map(|connector| connector.id).collect();
    let existing = connectors
        .into_iter()
        .find(|connector| matches!(connector.provider(), ConnectorProviderEnum::Mock));

    let connector_id = match existing {
        Some(connector) => connector.id,
        None => {
            store
                .connect_payment_provider(
                    actor.clone(),
                    tenant.id,
                    TEST_GATEWAY_ALIAS.to_string(),
                    PaymentCredentialsUpdate::Mock(MockPublicData::default()),
                )
                .await?
                .id
        }
    };

    let invoicing_entity = store.get_invoicing_entity(tenant.id, None).await?;

    // Only claim a free rail. One on a live provider is an operator's choice, whether it
    // predates the bootstrap or was moved off the test gateway since.
    let card = rail_to_claim(invoicing_entity.card_provider_id, connector_id, &live);
    let direct_debit = rail_to_claim(
        invoicing_entity.direct_debit_provider_id,
        connector_id,
        &live,
    );

    if card.is_none() && direct_debit.is_none() {
        return Ok(());
    }

    store
        .patch_invoicing_entity_providers(
            actor.clone(),
            InvoicingEntityProvidersPatch {
                id: invoicing_entity.id,
                card_provider_id: card,
                direct_debit_provider_id: direct_debit,
                bank_account_id: None,
            },
            tenant.id,
        )
        .await?;

    Ok(())
}

/// A rail is free when unset or pointing at a connector that was deleted since.
fn rail_to_claim(
    current: Option<ConnectorId>,
    connector_id: ConnectorId,
    live: &[ConnectorId],
) -> Option<Option<ConnectorId>> {
    match current {
        Some(existing) if existing == connector_id || live.contains(&existing) => None,
        _ => Some(Some(connector_id)),
    }
}

fn credentials_fragment(
    api_key: &SecretString,
    organization: &Organization,
    tenant: &Tenant,
) -> String {
    format!(
        "METEROID_API_KEY={}\n\
         METEROID_ORGANIZATION_ID={}\n\
         METEROID_ORGANIZATION_SLUG={}\n\
         METEROID_TENANT_ID={}\n\
         METEROID_TENANT_SLUG={}\n",
        api_key.expose_secret(),
        organization.id,
        organization.slug,
        tenant.id,
        tenant.slug,
    )
}

/// One path for both destinations. The fragment holds the key, so it never reaches the logs.
fn emit_credentials(output_path: Option<&str>, fragment: &str) -> BootstrapResult<()> {
    match output_path.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => {
            write_private_file(path, fragment).map_err(|err| {
                format!("could not write the bootstrap credentials to {path}: {err}")
            })?;
            tracing::info!("Instance bootstrap credentials written to {path}");
        }
        None => println!("{fragment}"),
    }

    Ok(())
}

/// The fragment carries a full-tenant credential, so it is created 0600 and never widened by
/// the process umask. Missing parent directories are created, as a compose volume may be bare.
fn write_private_file(path: &str, contents: &str) -> std::io::Result<()> {
    use std::io::Write;

    let path = std::path::Path::new(path);
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }

    file.write_all(contents.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::rail_to_claim;
    use common_domain::ids::{BaseId, ConnectorId};

    #[test]
    fn claims_only_free_rails() {
        let gateway = ConnectorId::new();
        let other = ConnectorId::new();

        assert_eq!(rail_to_claim(None, gateway, &[]), Some(Some(gateway)));
        assert_eq!(rail_to_claim(Some(gateway), gateway, &[gateway]), None);
        assert_eq!(
            rail_to_claim(Some(other), gateway, &[gateway, other]),
            None,
            "a rail on a live provider is left alone, even on first connect"
        );
        assert_eq!(
            rail_to_claim(Some(other), gateway, &[gateway]),
            Some(Some(gateway)),
            "a rail pointing at a deleted connector is reclaimed"
        );
    }
}
