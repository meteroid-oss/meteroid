use crate::adapters::payment::events::NormalizedEventSubscription;
use crate::adapters::payment::model::connector_webhook_url;
use crate::adapters::payment::{
    ConnectorError, PaymentConnector, ValidatedCredentials, credentials_rejected,
    initialize_payment_connector, provider_capabilities, requires_webhook_secret,
};
use crate::domain::connectors::{
    Connector, ConnectorAccessToken, ConnectorConfig, ConnectorMeta, ConnectorNew,
    HubspotPublicData, HubspotSensitiveData, PaymentConnectorConfig, PaymentCredentialsUpdate,
    PaymentProviderConfig, PennylanePublicData, PennylaneSensitiveData, ProviderAccount,
    ProviderData, ProviderSensitiveData, is_blank,
};
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::{
    ConnectorProviderEnum, ConnectorStatusEnum, ConnectorTypeEnum, ConnectorWebhookRouteEnum,
    PaymentProvider, TenantEnvironmentEnum,
};
use crate::domain::oauth::{OauthConnected, OauthConnection, OauthVerifierData};
use crate::domain::pgmq::{HubspotSyncRequestEvent, PgmqMessageNew, PgmqQueue};
use crate::errors::StoreError;
use crate::repositories::TenantInterface;
use crate::repositories::oauth::OauthInterface;
use crate::{Store, StoreResult};
use common_domain::ids::{BaseId, ConnectorId, InvoicingEntityId, TenantId};
use diesel_models::connectors::{ConnectorRow, ConnectorRowNew, ConnectorRowPatch};
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::query::pgmq;
use error_stack::{Report, bail};
use meteroid_oauth::model::OauthProvider;
use scoped_futures::ScopedFutureExt;
use secrecy::{ExposeSecret, SecretString};

/// Every event type the adapters parse; a self-registered endpoint subscribes to all of them.
const ALL_EVENT_SUBSCRIPTIONS: [NormalizedEventSubscription; 4] = [
    NormalizedEventSubscription::Payments,
    NormalizedEventSubscription::Mandates,
    NormalizedEventSubscription::Refunds,
    NormalizedEventSubscription::Disputes,
];

/// The test gateway is a mock: it must never be reachable from a production tenant. It lives
/// here, on the path every connect goes through, so gRPC, REST and the instance bootstrap
/// share one check.
pub async fn refuse_test_gateway_on_production(
    store: &Store,
    tenant_id: TenantId,
) -> StoreResult<()> {
    let tenant = store.find_tenant_by_id(tenant_id).await?;
    if tenant.tenant.environment == TenantEnvironmentEnum::Production {
        return Err(Report::new(StoreError::FailedPrecondition(
            "The test gateway cannot be connected to a production tenant".to_string(),
        )));
    }
    Ok(())
}

/// An invoicing entity that routes payments to a connector.
#[derive(Debug, Clone)]
pub struct RoutedInvoicingEntity {
    pub id: InvoicingEntityId,
    pub legal_name: String,
}

/// What depends on a connector, shown before disabling or deleting it.
#[derive(Debug, Clone, Default)]
pub struct ConnectorUsage {
    pub routed_card: Vec<RoutedInvoicingEntity>,
    pub routed_direct_debit: Vec<RoutedInvoicingEntity>,
    pub customers_with_saved_methods: u32,
    pub active_subscriptions: u32,
    /// Pending or ready payment transactions; they block deletion.
    pub in_flight_transactions: u32,
}

#[async_trait::async_trait]
pub trait ConnectorsInterface {
    /// Excludes deleted connectors.
    async fn list_connectors(
        &self,
        connector_type_filter: Option<ConnectorTypeEnum>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Connector>>;

    /// Any status, so history can still name a deleted connector.
    async fn get_connector_with_data(
        &self,
        id: ConnectorId,
        tenant_id: TenantId,
    ) -> StoreResult<Connector>;

    /// Legacy webhook route lookup; skips deleted connectors.
    async fn get_connector_with_data_by_alias(
        &self,
        alias: String,
        tenant_id: TenantId,
    ) -> StoreResult<Connector>;

    /// For the connector-id webhook route, which carries no tenant.
    async fn get_connector_unscoped(&self, id: ConnectorId) -> StoreResult<Connector>;

    /// One-shot connect (API/automation): draft, credentials and activation in one call.
    async fn connect_payment_provider(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        alias: String,
        update: PaymentCredentialsUpdate,
    ) -> StoreResult<ConnectorMeta>;

    /// First wizard step: reserves the alias and the webhook URL.
    async fn create_payment_provider_draft(
        &self,
        tenant_id: TenantId,
        alias: String,
        provider: PaymentProvider,
    ) -> StoreResult<Connector>;

    /// Sets or rotates credentials: checks them with the provider, refreshes the account and
    /// registers our webhook endpoint where the provider allows it.
    async fn update_payment_provider_credentials(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: ConnectorId,
        update: PaymentCredentialsUpdate,
    ) -> StoreResult<Connector>;

    async fn activate_connector(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: ConnectorId,
    ) -> StoreResult<Connector>;

    async fn disable_connector(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: ConnectorId,
    ) -> StoreResult<Connector>;

    /// Re-checks the credentials with the provider first.
    async fn enable_connector(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: ConnectorId,
    ) -> StoreResult<Connector>;

    /// Drafts are removed; others are soft-deleted, refused while payments are in flight.
    async fn delete_connector(
        &self,
        actor: Actor,
        id: ConnectorId,
        tenant_id: TenantId,
    ) -> StoreResult<()>;

    /// Only while the connection is a draft: once live, its alias is how people and integrations
    /// refer to it (and legacy webhook URLs embed it).
    async fn rename_draft_connector(
        &self,
        tenant_id: TenantId,
        id: ConnectorId,
        alias: String,
    ) -> StoreResult<Connector>;

    /// Re-checks the connection with the provider: the credentials (flagging or clearing a
    /// rejection) and the account details.
    async fn check_connector(&self, tenant_id: TenantId, id: ConnectorId)
    -> StoreResult<Connector>;

    /// Pushes the current event subscription set to every self-registered webhook endpoint whose
    /// stored fingerprint is stale, so endpoints created before a release learn the event types
    /// it added. Returns how many were updated; per-connector failures are logged, not propagated.
    async fn sync_all_webhook_subscriptions(&self) -> StoreResult<usize>;

    async fn connector_usage(
        &self,
        tenant_id: TenantId,
        id: ConnectorId,
    ) -> StoreResult<ConnectorUsage>;

    async fn connect_oauth(
        &self,
        provider: OauthProvider,
        oauth_code: SecretString,
        oauth_state: SecretString,
    ) -> StoreResult<OauthConnected>;

    async fn get_hubspot_connector(&self, tenant_id: TenantId) -> StoreResult<Option<Connector>>;

    async fn update_hubspot_connector(
        &self,
        connector_id: ConnectorId,
        tenant_id: TenantId,
        data: HubspotPublicData,
    ) -> StoreResult<Connector>;

    async fn get_pennylane_connector(&self, tenant_id: TenantId) -> StoreResult<Option<Connector>>;

    async fn get_pennylane_connector_access_token(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Option<ConnectorAccessToken>>;
}

#[async_trait::async_trait]
impl ConnectorsInterface for Store {
    async fn list_connectors(
        &self,
        connector_type_filter: Option<ConnectorTypeEnum>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Connector>> {
        let mut conn = self.get_conn().await?;

        let rows = ConnectorRow::list_connectors(
            &mut conn,
            tenant_id,
            connector_type_filter.map(Into::into),
            None,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(|x| Connector::from_row(&self.settings.crypt_key, x))
            .collect()
    }

    async fn get_connector_with_data(
        &self,
        id: ConnectorId,
        tenant_id: TenantId,
    ) -> StoreResult<Connector> {
        let mut conn = self.get_conn().await?;

        let row = ConnectorRow::get_connector_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Connector::from_row(&self.settings.crypt_key, row)
    }

    async fn get_connector_with_data_by_alias(
        &self,
        alias: String,
        tenant_id: TenantId,
    ) -> StoreResult<Connector> {
        let mut conn = self.get_conn().await?;

        let row = ConnectorRow::get_connector_by_alias(&mut conn, alias, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Connector::from_row(&self.settings.crypt_key, row)
    }

    async fn get_connector_unscoped(&self, id: ConnectorId) -> StoreResult<Connector> {
        let mut conn = self.get_conn().await?;

        let row = ConnectorRow::get_connector_by_id_unscoped(&mut conn, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Connector::from_row(&self.settings.crypt_key, row)
    }

    async fn connect_payment_provider(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        alias: String,
        update: PaymentCredentialsUpdate,
    ) -> StoreResult<ConnectorMeta> {
        let draft = self
            .create_payment_provider_draft(tenant_id, alias, update.provider())
            .await?;

        let activated = match self
            .update_payment_provider_credentials(actor.clone(), tenant_id, draft.id, update)
            .await
        {
            Ok(_) => {
                self.activate_connector(actor.clone(), tenant_id, draft.id)
                    .await
            }
            Err(err) => Err(err),
        };

        match activated {
            Ok(connector) => Ok(ConnectorMeta {
                id: connector.id,
                created_at: connector.created_at,
                tenant_id: connector.tenant_id,
                connector_type: connector.connector_type(),
                provider: connector.provider(),
                status: connector.status,
                alias: connector.alias,
            }),
            Err(err) => {
                // Don't leave a half-configured draft behind a failed one-shot connect.
                if let Err(cleanup) = self.delete_connector(actor, draft.id, tenant_id).await {
                    log::warn!(
                        "Failed to discard draft {} after a failed connect: {cleanup:?}",
                        draft.id
                    );
                }
                Err(err)
            }
        }
    }

    async fn create_payment_provider_draft(
        &self,
        tenant_id: TenantId,
        alias: String,
        provider: PaymentProvider,
    ) -> StoreResult<Connector> {
        let alias = validate_alias(&alias)?;
        let capabilities = provider_capabilities(provider);
        if capabilities.is_test_gateway {
            refuse_test_gateway_on_production(self, tenant_id).await?;
        }
        let webhook_route = capabilities
            .receives_webhooks()
            .then_some(ConnectorWebhookRouteEnum::ConnectorId);
        let row = ConnectorNew {
            tenant_id,
            alias,
            config: ConnectorConfig::Payment(PaymentConnectorConfig::draft(provider)),
            status: ConnectorStatusEnum::Draft,
            account: None,
            webhook_route,
        }
        .to_row(&self.settings.crypt_key)?;

        let mut conn = self.get_conn().await?;
        let inserted = row
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Connector::from_row(&self.settings.crypt_key, inserted)
    }

    async fn update_payment_provider_credentials(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: ConnectorId,
        update: PaymentCredentialsUpdate,
    ) -> StoreResult<Connector> {
        let existing = self.get_connector_with_data(id, tenant_id).await?;
        if existing.status == ConnectorStatusEnum::Deleted {
            bail!(StoreError::ValueNotFound("connector".to_string()));
        }

        let mut candidate = existing.clone();
        let Some(payment) = candidate.config.payment_mut() else {
            bail!(StoreError::InvalidArgument(format!(
                "{:?} takes no payments",
                existing.provider()
            )));
        };
        payment.apply(update)?;
        ensure_required_credentials(payment.credentials().expect("just applied"))?;

        let connector_impl = initialize_payment_connector(&candidate).map_err(|e| {
            Report::new(StoreError::InvalidArgument(e.current_context().to_string()))
        })?;

        let ValidatedCredentials { public, account } = connector_impl
            .validate_credentials()
            .await
            .map_err(credential_error)?;
        candidate.refresh_public_data(public)?;

        // Not refused: the credentials modal shows the account it now points at.
        let account_changed = moved_to_another_account(&existing, &account);
        // The endpoint we registered lives on the old account; a new one is registered below.
        if account_changed
            && existing_webhook_secret_kept(&existing, &candidate)
            && let Some(PaymentProviderConfig::Stripe(_, d)) = candidate.payment_credentials_mut()
            && d.webhook_endpoint_id.is_some()
        {
            d.webhook_secret = SecretString::from(String::new());
        }

        // A draft whose key can't create endpoints still saves: the wizard's webhook step then
        // asks for a signing secret instead.
        let registered_endpoint = match self
            .register_webhook_if_needed(connector_impl.as_ref(), &mut candidate)
            .await
        {
            Ok(endpoint) => endpoint,
            Err(e) if existing.status == ConnectorStatusEnum::Draft => {
                log::info!("Webhook auto-registration skipped for draft {id}: {e:?}");
                None
            }
            Err(e) => return Err(e),
        };

        let persisted = self
            .persist_credentials(&actor, tenant_id, &existing, &candidate, &account)
            .await;

        match (&persisted, &registered_endpoint) {
            // Persisting failed after creating a live endpoint in the merchant's account: remove
            // it so it isn't orphaned. Best effort; never hides the original error.
            (Err(_), Some(endpoint_id)) => {
                if let Err(e) = connector_impl.unregister_webhook(endpoint_id).await {
                    log::warn!(
                        "Failed to clean up webhook endpoint {endpoint_id} of connector {id}: {e:?}"
                    );
                }
            }
            // Moved to another account: the old account's endpoint no longer serves us.
            (Ok(_), Some(_)) if account_changed => {
                self.unregister_owned_webhook(&existing).await;
            }
            _ => {}
        }

        persisted
    }

    async fn activate_connector(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: ConnectorId,
    ) -> StoreResult<Connector> {
        let key = self.settings.crypt_key.clone();
        self.transaction(|conn| {
            let actor = &actor;
            let key = &key;
            async move {
                let row = ConnectorRow::lock_by_id(conn, id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let connector = Connector::from_row(key, row)?;
                match connector.status {
                    ConnectorStatusEnum::Draft => {}
                    ConnectorStatusEnum::Active => return Ok(connector),
                    _ => bail!(StoreError::FailedPrecondition(
                        "Only a connection that is being set up can be activated".to_string()
                    )),
                }
                if connector.payment_credentials().is_none() || connector.account().is_none() {
                    bail!(StoreError::FailedPrecondition(
                        "Verify the credentials before activating the connection".to_string()
                    ));
                }
                if connector
                    .provider()
                    .as_payment()
                    .is_some_and(requires_webhook_secret)
                    && connector.webhook_secret().is_none()
                {
                    bail!(StoreError::FailedPrecondition(
                        "Add the webhook signing secret before activating the connection"
                            .to_string()
                    ));
                }

                let row = ConnectorRow::set_status(
                    conn,
                    id,
                    tenant_id,
                    ConnectorStatusEnum::Active.into(),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                let activity = connector_activity(ActivityType::ConnectorConnected, &connector)
                    .with_metadata(serde_json::json!({
                        "provider": provider_key(&connector),
                        "alias": connector.alias,
                        "account_id": connector.account().and_then(|a| a.account_id.clone()),
                    }));
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;

                Connector::from_row(key, row)
            }
            .scope_boxed()
        })
        .await
    }

    async fn disable_connector(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: ConnectorId,
    ) -> StoreResult<Connector> {
        self.transition_status(
            actor,
            tenant_id,
            id,
            ConnectorStatusEnum::Active,
            ConnectorStatusEnum::Disabled,
            ActivityType::ConnectorDisabled,
            None,
        )
        .await
    }

    async fn enable_connector(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: ConnectorId,
    ) -> StoreResult<Connector> {
        let connector = self.get_connector_with_data(id, tenant_id).await?;
        if connector.status != ConnectorStatusEnum::Disabled {
            bail!(StoreError::FailedPrecondition(
                "Only a disabled connection can be enabled".to_string()
            ));
        }

        // Keys may have been revoked while disabled: don't enable something that can't charge.
        let account = if connector.connector_type() == ConnectorTypeEnum::PaymentProvider {
            let connector_impl = initialize_payment_connector(&connector).map_err(|e| {
                Report::new(StoreError::InvalidArgument(e.current_context().to_string()))
            })?;
            let validated = connector_impl.validate_credentials().await;
            self.note_rejected_credentials(&connector, &validated).await;
            let validated = validated.map_err(credential_error)?;
            Some(validated.account)
        } else {
            None
        };

        self.transition_status(
            actor,
            tenant_id,
            id,
            ConnectorStatusEnum::Disabled,
            ConnectorStatusEnum::Active,
            ActivityType::ConnectorEnabled,
            account,
        )
        .await
    }

    async fn delete_connector(
        &self,
        actor: Actor,
        id: ConnectorId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let key = self.settings.crypt_key.clone();
        let deleted = self
            .transaction_bumping_tenants(|conn| {
                let actor = &actor;
                let key = &key;
                async move {
                    // The routed entities first, then the connector without waiting: a checkout
                    // holds the connector (KEY SHARE) from its charge until it numbers its invoice.
                    let mut entities: Vec<InvoicingEntityId> =
                        InvoicingEntityRow::list_routing_to_connector(conn, tenant_id, id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                            .into_iter()
                            .map(|entity| entity.id)
                            .collect();
                    entities.sort_by_key(|entity| entity.as_uuid());
                    for entity in &entities {
                        InvoicingEntityRow::select_for_update_by_id_and_tenant(
                            conn, *entity, tenant_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    }
                    let Some(row) = ConnectorRow::try_lock_by_id(conn, id, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                    else {
                        // Absent, or held by a payment in progress through it.
                        ConnectorRow::get_connector_by_id(conn, id, tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        bail!(StoreError::Conflict(
                            "The connection is in use by a payment in progress; try again in a moment."
                                .to_string()
                        ));
                    };
                    let connector = Connector::from_row(key, row)?;

                    match connector.status {
                        ConnectorStatusEnum::Deleted => return Ok((None, vec![])),
                        ConnectorStatusEnum::Draft => {
                            ConnectorRow::delete_draft(conn, id, tenant_id)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                            return Ok((Some(connector), vec![]));
                        }
                        ConnectorStatusEnum::Active | ConnectorStatusEnum::Disabled => {}
                    }

                    let usage = ConnectorRow::usage(conn, id, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if usage.in_flight_transactions > 0 {
                        bail!(StoreError::FailedPrecondition(format!(
                            "{} payment{} still in progress through '{}'. Disable the connection \
                             now and delete it once they have settled.",
                            usage.in_flight_transactions,
                            if usage.in_flight_transactions == 1 {
                                " is"
                            } else {
                                "s are"
                            },
                            connector.alias
                        )));
                    }

                    let routed = InvoicingEntityRow::list_routing_to_connector(conn, tenant_id, id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if routed.iter().any(|entity| !entities.contains(&entity.id)) {
                        bail!(StoreError::Conflict(format!(
                            "An invoicing entity started routing payments to '{}' meanwhile; \
                             delete the connection again.",
                            connector.alias
                        )));
                    }
                    InvoicingEntityRow::clear_connector_routing(conn, tenant_id, id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    for entity in &routed {
                        let activity = Activity::new(
                            ActivityType::FieldChanged,
                            EntityType::InvoicingEntity,
                            entity.id.as_uuid(),
                        )
                        .with_metadata(serde_json::json!({
                            "field": "payment_providers",
                            "from": connector.alias,
                            "to": serde_json::Value::Null,
                        }));
                        self.internal
                            .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                            .await?;
                    }

                    ConnectorRow::soft_delete(conn, id, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let activity = connector_activity(ActivityType::ConnectorDeleted, &connector)
                        .with_metadata(serde_json::json!({
                            "provider": provider_key(&connector),
                            "alias": connector.alias,
                        }));
                    self.internal
                        .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                        .await?;

                    // A tax connector computes live prices; payment routing is read by no reader.
                    let bumped = (connector.connector_type() == ConnectorTypeEnum::Tax)
                        .then_some(tenant_id);
                    Ok((Some(connector), bumped.into_iter().collect()))
                }
                .scope_boxed()
            })
            .await?;

        // The secrets are gone from the row; the copy read in the transaction still has them.
        if let Some(connector) = deleted {
            self.unregister_owned_webhook(&connector).await;
        }
        Ok(())
    }

    async fn rename_draft_connector(
        &self,
        tenant_id: TenantId,
        id: ConnectorId,
        alias: String,
    ) -> StoreResult<Connector> {
        let alias = validate_alias(&alias)?;
        let key = self.settings.crypt_key.clone();
        self.transaction(|conn| {
            let key = &key;
            let alias = &alias;
            async move {
                let row = ConnectorRow::lock_by_id(conn, id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let connector = Connector::from_row(key, row)?;
                if connector.status != ConnectorStatusEnum::Draft {
                    bail!(StoreError::FailedPrecondition(
                        "A connection's name can only be changed while it's being set up"
                            .to_string()
                    ));
                }
                if &connector.alias == alias {
                    return Ok(connector);
                }

                let row = ConnectorRowPatch {
                    id,
                    data: None,
                    sensitive: None,
                    account: None,
                    alias: Some(alias.clone()),
                    updated_at: Some(common_domain::clock::now().naive_utc()),
                }
                .patch(conn, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                Connector::from_row(key, row)
            }
            .scope_boxed()
        })
        .await
    }

    async fn check_connector(
        &self,
        tenant_id: TenantId,
        id: ConnectorId,
    ) -> StoreResult<Connector> {
        let connector = self.get_connector_with_data(id, tenant_id).await?;
        self.refresh_account(connector).await
    }

    async fn sync_all_webhook_subscriptions(&self) -> StoreResult<usize> {
        let mut conn = self.get_conn().await?;
        let rows = ConnectorRow::list_by_provider_unscoped(
            &mut conn,
            diesel_models::enums::ConnectorProviderEnum::Stripe,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        drop(conn);

        let mut synced = 0;
        for row in rows {
            let connector = match Connector::from_row(&self.settings.crypt_key, row) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("Skipping a connector whose credentials failed to load: {e:?}");
                    continue;
                }
            };
            let Ok(connector_impl) = initialize_payment_connector(&connector) else {
                continue;
            };
            match self
                .sync_owned_webhook_subscriptions(&connector, connector_impl.as_ref(), false)
                .await
            {
                Ok(true) => synced += 1,
                Ok(false) => {}
                Err(e) => log::warn!(
                    "Failed to sync webhook subscriptions of connector {}: {e:?}",
                    connector.id
                ),
            }
        }
        Ok(synced)
    }

    async fn connector_usage(
        &self,
        tenant_id: TenantId,
        id: ConnectorId,
    ) -> StoreResult<ConnectorUsage> {
        let mut conn = self.get_conn().await?;
        // Existence and tenant check.
        ConnectorRow::get_connector_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let counts = ConnectorRow::usage(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let routed = InvoicingEntityRow::list_routing_to_connector(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let pick = |card: bool| {
            routed
                .iter()
                .filter(|e| {
                    if card {
                        e.card_provider_id == Some(id)
                    } else {
                        e.direct_debit_provider_id == Some(id)
                    }
                })
                .map(|e| RoutedInvoicingEntity {
                    id: e.id,
                    legal_name: e.legal_name.clone(),
                })
                .collect()
        };

        Ok(ConnectorUsage {
            routed_card: pick(true),
            routed_direct_debit: pick(false),
            customers_with_saved_methods: counts.customers_with_saved_methods as u32,
            active_subscriptions: counts.active_subscriptions as u32,
            in_flight_transactions: counts.in_flight_transactions as u32,
        })
    }

    async fn connect_oauth(
        &self,
        provider: OauthProvider,
        oauth_code: SecretString,
        oauth_state: SecretString,
    ) -> StoreResult<OauthConnected> {
        match provider {
            OauthProvider::Hubspot => connect_hubspot(self, oauth_code, oauth_state).await,
            OauthProvider::Pennylane => connect_pennylane(self, oauth_code, oauth_state).await,
            OauthProvider::Google => {
                bail!(StoreError::OauthError(
                    "google not supported as a connector".to_string()
                ));
            }
        }
    }

    async fn get_hubspot_connector(&self, tenant_id: TenantId) -> StoreResult<Option<Connector>> {
        get_connector(
            self,
            tenant_id,
            ConnectorTypeEnum::Crm,
            ConnectorProviderEnum::Hubspot,
        )
        .await
    }

    async fn update_hubspot_connector(
        &self,
        connector_id: ConnectorId,
        tenant_id: TenantId,
        data: HubspotPublicData,
    ) -> StoreResult<Connector> {
        let mut conn = self.get_conn().await?;

        let patch = ConnectorRowPatch {
            id: connector_id,
            alias: None,
            sensitive: None,
            account: None,
            data: Some(Some(ProviderData::Hubspot(data).try_into()?)),
            updated_at: Some(common_domain::clock::now().naive_utc()),
        };

        let row = patch
            .patch(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Connector::from_row(&self.settings.crypt_key, row)
    }

    async fn get_pennylane_connector(&self, tenant_id: TenantId) -> StoreResult<Option<Connector>> {
        get_connector(
            self,
            tenant_id,
            ConnectorTypeEnum::Accounting,
            ConnectorProviderEnum::Pennylane,
        )
        .await
    }

    async fn get_pennylane_connector_access_token(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Option<ConnectorAccessToken>> {
        let connector = self.get_pennylane_connector(tenant_id).await?;

        // todo handle concurrent refreshes + refresh_token expiry (90 days)
        if let Some(connector) = connector
            && let ConnectorConfig::Pennylane {
                data: Some(data),
                sensitive: Some(sensitive),
            } = connector.config
        {
            let is_expired = sensitive.expires_at.as_ref().is_some_and(|exp| {
                exp <= &(common_domain::clock::now() + chrono::Duration::minutes(30))
            });

            let auth_token = if is_expired {
                let tokens = self
                    .oauth_exchange_refresh_token(
                        OauthProvider::Pennylane,
                        SecretString::from(sensitive.refresh_token),
                    )
                    .await?;

                let sensitive = PennylaneSensitiveData::try_from(tokens)?;

                let access_token = ConnectorAccessToken {
                    connector_id: connector.id,
                    external_company_id: data.external_company_id,
                    access_token: SecretString::from(sensitive.access_token.clone()),
                    expires_at: sensitive.expires_at,
                };

                let new_sensitive = ProviderSensitiveData::Pennylane(sensitive)
                    .encrypt(&self.settings.crypt_key)?;

                let patch = ConnectorRowPatch {
                    id: connector.id,
                    alias: None,
                    data: None,
                    account: None,
                    sensitive: Some(Some(new_sensitive)),
                    updated_at: Some(common_domain::clock::now().naive_utc()),
                };

                let mut conn = self.get_conn().await?;
                patch
                    .patch(&mut conn, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                access_token
            } else {
                ConnectorAccessToken {
                    connector_id: connector.id,
                    external_company_id: data.external_company_id,
                    access_token: SecretString::from(sensitive.access_token),
                    expires_at: sensitive.expires_at,
                }
            };

            return Ok(Some(auth_token));
        }

        Ok(None)
    }
}

impl Store {
    /// Flags the connector when a provider call failed because the provider refused its stored
    /// credentials (401/403). Timeouts, network errors and declines never flag. Call it after the
    /// provider call, outside any timeout, so the write can't be cancelled. The flag is written on
    /// its own connection: callers are usually inside a transaction the error will roll back.
    pub async fn note_rejected_credentials<T>(
        &self,
        connector: &Connector,
        result: &Result<T, Report<ConnectorError>>,
    ) {
        if let Err(report) = result
            && credentials_rejected(report)
        {
            self.record_credentials_error(connector, report.current_context().to_string())
                .await;
        }
    }

    /// Best effort: a failure to record must never mask the provider error.
    async fn record_credentials_error(&self, connector: &Connector, message: String) {
        let recorded = async {
            let mut conn = self.get_conn().await?;
            ConnectorRow::set_credentials_error(
                &mut conn,
                connector.id,
                connector.tenant_id,
                Some(message),
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)
        }
        .await;
        if let Err(e) = recorded {
            log::warn!(
                "Failed to flag rejected credentials on connector {}: {e:?}",
                connector.id
            );
        }
    }

    /// Registers our endpoint on providers that allow it when no webhook secret is stored.
    /// Returns the endpoint id to clean up if persisting fails.
    async fn register_webhook_if_needed(
        &self,
        connector_impl: &dyn PaymentConnector,
        candidate: &mut Connector,
    ) -> StoreResult<Option<String>> {
        if !connector_impl
            .capabilities()
            .supports_self_webhook_registration()
            || !candidate
                .provider()
                .as_payment()
                .is_some_and(requires_webhook_secret)
            || candidate.webhook_secret().is_some()
        {
            return Ok(None);
        }

        let Some(url) = connector_webhook_url(self.settings.webhook_base_url(), candidate) else {
            return Ok(None);
        };
        let registered = connector_impl
            .register_webhook(&url, &ALL_EVENT_SUBSCRIPTIONS)
            .await
            .map_err(|e| {
                log::warn!(
                    "Auto-registering webhook for connector {} failed: {e:?}",
                    candidate.id
                );
                Report::new(StoreError::InvalidArgument(format!(
                    "Webhook auto-registration failed: {}. Paste a webhook secret manually, or \
                     grant the API key the scope to create webhook endpoints.",
                    e.current_context()
                )))
            })?;

        let stored = candidate
            .payment_credentials()
            .cloned()
            .and_then(|c| c.with_registered_webhook(&registered.endpoint_id, &registered.secret));
        let Some(stored) = stored else {
            if let Err(e) = connector_impl
                .unregister_webhook(&registered.endpoint_id)
                .await
            {
                log::warn!(
                    "Failed to remove webhook endpoint {} whose secret has nowhere to live: {e:?}",
                    registered.endpoint_id
                );
            }
            bail!(StoreError::InvalidArgument(
                "this provider cannot store a self-registered webhook secret".to_string()
            ));
        };
        candidate
            .config
            .payment_mut()
            .expect("a payment connector registered this webhook")
            .set_credentials(stored);
        Ok(Some(registered.endpoint_id))
    }

    async fn persist_credentials(
        &self,
        actor: &Actor,
        tenant_id: TenantId,
        existing: &Connector,
        candidate: &Connector,
        account: &ProviderAccount,
    ) -> StoreResult<Connector> {
        let (data, sensitive) = candidate.config.to_row_parts();
        let sensitive = sensitive
            .as_ref()
            .map(|s| s.encrypt(&self.settings.crypt_key))
            .transpose()?;
        let data = data
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| {
                StoreError::SerdeError("Failed to serialize connector data".to_string(), e)
            })?;
        let account_json = serde_json::to_value(account).map_err(|e| {
            StoreError::SerdeError("Failed to serialize connector account".to_string(), e)
        })?;
        let patch = ConnectorRowPatch {
            id: existing.id,
            alias: None,
            data: Some(data),
            sensitive: Some(sensitive),
            account: Some(Some(account_json)),
            updated_at: Some(common_domain::clock::now().naive_utc()),
        };
        let id = existing.id;
        let key = self.settings.crypt_key.clone();

        self.transaction(|conn| {
            let patch = &patch;
            let key = &key;
            async move {
                let row = ConnectorRow::lock_by_id(conn, id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if row.status == ConnectorStatusEnum::Deleted.into() {
                    bail!(StoreError::ValueNotFound("connector".to_string()));
                }
                // New credentials that the provider just accepted.
                ConnectorRow::set_credentials_error(conn, id, tenant_id, None)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let row = patch
                    .patch(conn, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // A draft's credentials are part of connecting, which activation records.
                if existing.status != ConnectorStatusEnum::Draft {
                    let activity = connector_activity(
                        ActivityType::ConnectorCredentialsUpdated,
                        existing,
                    )
                    .with_metadata(serde_json::json!({
                        "provider": provider_key(existing),
                        "alias": existing.alias,
                        "from_key_hint": existing.account().and_then(|a| a.key_hint.clone()),
                        "to_key_hint": account.key_hint,
                    }));
                    self.internal
                        .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                        .await?;
                }

                Connector::from_row(key, row)
            }
            .scope_boxed()
        })
        .await
    }

    async fn refresh_account(&self, connector: Connector) -> StoreResult<Connector> {
        if connector.status == ConnectorStatusEnum::Deleted
            || connector.connector_type() != ConnectorTypeEnum::PaymentProvider
            || connector.payment_credentials().is_none()
        {
            bail!(StoreError::FailedPrecondition(
                "This connection has no credentials to check".to_string()
            ));
        }
        let connector_impl = initialize_payment_connector(&connector).map_err(|e| {
            Report::new(StoreError::InvalidArgument(e.current_context().to_string()))
        })?;
        let validated = connector_impl.validate_credentials().await;
        self.note_rejected_credentials(&connector, &validated).await;
        let validated = validated.map_err(credential_error)?;
        let account = serde_json::to_value(&validated.account).map_err(|e| {
            StoreError::SerdeError("Failed to serialize connector account".to_string(), e)
        })?;

        if let Err(e) = self
            .sync_owned_webhook_subscriptions(&connector, connector_impl.as_ref(), true)
            .await
        {
            log::warn!(
                "Checked connector {} but could not sync its webhook subscriptions: {e:?}",
                connector.id
            );
        }

        let mut conn = self.get_conn().await?;
        ConnectorRow::set_credentials_error(&mut conn, connector.id, connector.tenant_id, None)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let row = ConnectorRowPatch {
            id: connector.id,
            alias: None,
            data: None,
            sensitive: None,
            updated_at: None,
            account: Some(Some(account)),
        }
        .patch(&mut conn, connector.tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        Connector::from_row(&self.settings.crypt_key, row)
    }

    #[allow(clippy::too_many_arguments)]
    async fn transition_status(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: ConnectorId,
        from: ConnectorStatusEnum,
        to: ConnectorStatusEnum,
        activity_type: ActivityType,
        account: Option<ProviderAccount>,
    ) -> StoreResult<Connector> {
        let key = self.settings.crypt_key.clone();
        let account = account
            .map(|a| serde_json::to_value(&a))
            .transpose()
            .map_err(|e| {
                StoreError::SerdeError("Failed to serialize connector account".to_string(), e)
            })?;
        self.transaction_bumping_tenants(|conn| {
            let actor = &actor;
            let key = &key;
            let account = account.clone();
            async move {
                let row = ConnectorRow::lock_by_id(conn, id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let connector = Connector::from_row(key, row)?;
                if connector.status == to {
                    return Ok((connector, vec![]));
                }
                if connector.status != from {
                    bail!(StoreError::FailedPrecondition(format!(
                        "The connection is {}",
                        status_label(connector.status)
                    )));
                }

                if let Some(account) = account {
                    ConnectorRow::set_credentials_error(conn, id, tenant_id, None)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    ConnectorRowPatch {
                        id,
                        alias: None,
                        data: None,
                        sensitive: None,
                        updated_at: None,
                        account: Some(Some(account)),
                    }
                    .patch(conn, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                }
                let row = ConnectorRow::set_status(conn, id, tenant_id, to.into())
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let activity = connector_activity(activity_type, &connector).with_metadata(
                    serde_json::json!({
                        "provider": provider_key(&connector),
                        "alias": connector.alias,
                    }),
                );
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;

                let bumped =
                    (connector.connector_type() == ConnectorTypeEnum::Tax).then_some(tenant_id);
                Ok((Connector::from_row(key, row)?, bumped.into_iter().collect()))
            }
            .scope_boxed()
        })
        .await
    }

    /// Re-subscribes the endpoint we registered to every event the adapter parses today and
    /// records the resulting fingerprint. `Ok(false)` when the connector has no endpoint of ours
    /// (secret pasted by hand) or, unless `force`, when its fingerprint is already current.
    async fn sync_owned_webhook_subscriptions(
        &self,
        connector: &Connector,
        connector_impl: &dyn PaymentConnector,
        force: bool,
    ) -> StoreResult<bool> {
        let Some(PaymentProviderConfig::Stripe(public, sensitive)) =
            connector.payment_credentials()
        else {
            return Ok(false);
        };
        let Some(endpoint_id) = sensitive.webhook_endpoint_id.as_deref() else {
            return Ok(false);
        };
        let fingerprint = connector_impl.webhook_subscription_fingerprint(&ALL_EVENT_SUBSCRIPTIONS);
        if !force && public.webhook_events_fingerprint.as_deref() == Some(fingerprint.as_str()) {
            return Ok(false);
        }
        connector_impl
            .sync_webhook_events(endpoint_id, &ALL_EVENT_SUBSCRIPTIONS)
            .await
            .map_err(|e| {
                Report::new(StoreError::WebhookServiceError(format!(
                    "webhook subscription sync: {}",
                    e.current_context()
                )))
            })?;

        let mut public = public.clone();
        public.webhook_events_fingerprint = Some(fingerprint);
        let data: serde_json::Value = ProviderData::Stripe(public).try_into()?;
        let mut conn = self.get_conn().await?;
        ConnectorRowPatch {
            id: connector.id,
            alias: None,
            data: Some(Some(data)),
            sensitive: None,
            account: None,
            updated_at: Some(common_domain::clock::now().naive_utc()),
        }
        .patch(&mut conn, connector.tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        Ok(true)
    }

    /// Removes the webhook endpoint we registered in the merchant's account. Best effort.
    async fn unregister_owned_webhook(&self, connector: &Connector) {
        let Some(PaymentProviderConfig::Stripe(_, d)) = connector.payment_credentials() else {
            return;
        };
        let Some(endpoint_id) = d.webhook_endpoint_id.as_deref() else {
            return;
        };
        let Ok(connector_impl) = initialize_payment_connector(connector) else {
            return;
        };
        if let Err(e) = connector_impl.unregister_webhook(endpoint_id).await {
            log::warn!(
                "Failed to remove webhook endpoint {endpoint_id} of connector {}: {e:?}",
                connector.id
            );
        }
    }
}

fn connector_activity(activity_type: ActivityType, connector: &Connector) -> Activity {
    Activity::new(activity_type, EntityType::Connector, connector.id.as_uuid())
}

fn provider_key(connector: &Connector) -> String {
    let provider: diesel_models::enums::ConnectorProviderEnum = connector.provider().clone().into();
    provider.as_meta_key().to_string()
}

fn status_label(status: ConnectorStatusEnum) -> &'static str {
    match status {
        ConnectorStatusEnum::Draft => "still being set up",
        ConnectorStatusEnum::Active => "active",
        ConnectorStatusEnum::Disabled => "disabled",
        ConnectorStatusEnum::Deleted => "deleted",
    }
}

/// Lowercase letters, digits and hyphens: legacy webhook URLs embed the alias.
fn validate_alias(alias: &str) -> StoreResult<String> {
    let alias = alias.trim();
    let valid = !alias.is_empty()
        && alias.len() <= 64
        && alias
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        bail!(StoreError::InvalidArgument(
            "The name can only contain lowercase letters, numbers and hyphens (64 at most)"
                .to_string()
        ));
    }
    Ok(alias.to_string())
}

/// After merging, every secret the provider needs must be present.
fn ensure_required_credentials(credentials: &PaymentProviderConfig) -> StoreResult<()> {
    let missing = match credentials {
        PaymentProviderConfig::Stripe(_, s) if is_blank(&s.api_secret_key) => {
            Some("the secret key")
        }
        PaymentProviderConfig::Stripe(d, _) if d.api_publishable_key.trim().is_empty() => {
            Some("the publishable key")
        }
        PaymentProviderConfig::Gocardless(_, s) if is_blank(&s.access_token) => {
            Some("the access token")
        }
        PaymentProviderConfig::Stancer(s) if is_blank(&s.api_secret_key) => Some("the secret key"),
        PaymentProviderConfig::Mollie(s) if is_blank(&s.api_key) => Some("the API key"),
        _ => None,
    };
    match missing {
        Some(field) => bail!(StoreError::InvalidArgument(format!("Enter {field}"))),
        None => Ok(()),
    }
}

/// Whether the new credentials point at another provider account than the stored ones.
/// Providers without an account id (Stancer) never report a change.
fn moved_to_another_account(existing: &Connector, account: &ProviderAccount) -> bool {
    let previous = existing
        .account()
        .and_then(|previous| previous.account_id.as_deref());
    match (previous, account.account_id.as_deref()) {
        (Some(previous), Some(current)) => previous != current,
        // A provider that exposes no account id never reports a move.
        _ => false,
    }
}

/// Whether the candidate still carries the webhook secret the connector had before.
fn existing_webhook_secret_kept(existing: &Connector, candidate: &Connector) -> bool {
    existing
        .webhook_secret()
        .map(|s| s.expose_secret().to_string())
        == candidate
            .webhook_secret()
            .map(|s| s.expose_secret().to_string())
}

/// Rejected credentials get a user-facing message; an unreachable provider says nothing about
/// the credentials.
fn credential_error(report: Report<ConnectorError>) -> Report<StoreError> {
    let err = match report.current_context() {
        ConnectorError::Configuration(message) => {
            // The message is short on purpose; the provider's own wording is attached here.
            log::info!("credentials rejected: {report:?}");
            StoreError::InvalidArgument(message.clone())
        }
        ConnectorError::Transport(_) => {
            log::warn!("credential check failed: {report:?}");
            StoreError::InvalidArgument(
                "Couldn't reach the payment provider to verify the credentials. Please try again."
                    .to_string(),
            )
        }
        other => StoreError::InvalidArgument(other.to_string()),
    };
    Report::new(err)
}

async fn connect_hubspot(
    store: &Store,
    oauth_code: SecretString,
    oauth_state: SecretString,
) -> StoreResult<OauthConnected> {
    let OauthConnection {
        user,
        tokens,
        verifier_data,
    } = store
        .oauth_exchange_code(OauthProvider::Hubspot, oauth_code, oauth_state)
        .await?;

    let crm_data = match verifier_data {
        OauthVerifierData::ConnectHubspot(data) => data,
        _ => {
            bail!(StoreError::OauthError("Invalid verifier data".to_string(),))
        }
    };

    let refresh_token = tokens
        .refresh_token
        .ok_or_else(|| StoreError::OauthError("Missing refresh token".to_string()))?;

    let row: ConnectorRowNew = ConnectorNew {
        tenant_id: crm_data.tenant_id,
        alias: "hubspot".to_owned(),
        config: ConnectorConfig::Hubspot {
            data: Some(HubspotPublicData {
                auto_sync: crm_data.auto_sync,
                external_company_id: user.company_id,
            }),
            sensitive: Some(HubspotSensitiveData { refresh_token }),
        },
        status: ConnectorStatusEnum::Active,
        account: None,
        webhook_route: None,
    }
    .to_row(&store.settings.crypt_key)?;

    let initiated_by = crm_data.initiated_by;
    let res = store
        .transaction(|tx| {
            async move {
                let inserted = row
                    .insert(tx)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let msg: PgmqMessageNew =
                    HubspotSyncRequestEvent::CustomProperties(inserted.tenant_id).try_into()?;

                pgmq::send_batch(tx, PgmqQueue::HubspotSync.as_str(), &[msg.into()])
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(inserted)
            }
            .scope_boxed()
        })
        .await?;

    Ok(OauthConnected {
        connector: res.into(),
        initiated_by,
    })
}

async fn connect_pennylane(
    store: &Store,
    oauth_code: SecretString,
    oauth_state: SecretString,
) -> StoreResult<OauthConnected> {
    let OauthConnection {
        user,
        tokens,
        verifier_data,
    } = store
        .oauth_exchange_code(OauthProvider::Pennylane, oauth_code, oauth_state)
        .await?;

    let crm_data = match verifier_data {
        OauthVerifierData::ConnectPennylane(data) => data,
        _ => {
            bail!(StoreError::OauthError("Invalid verifier data".to_string(),))
        }
    };

    let refresh_token = tokens
        .refresh_token
        .ok_or_else(|| StoreError::OauthError("Missing refresh token".to_string()))?;

    let mut conn = store.get_conn().await?;

    let row: ConnectorRowNew = ConnectorNew {
        tenant_id: crm_data.tenant_id,
        alias: "pennylane".to_owned(),
        config: ConnectorConfig::Pennylane {
            data: Some(PennylanePublicData {
                external_company_id: user.company_id,
            }),
            sensitive: Some(PennylaneSensitiveData {
                refresh_token,
                access_token: tokens.access_token,
                expires_at: tokens
                    .expires_in
                    .map(|duration| common_domain::clock::now() + duration),
            }),
        },
        status: ConnectorStatusEnum::Active,
        account: None,
        webhook_route: None,
    }
    .to_row(&store.settings.crypt_key)?;

    let inserted = row
        .insert(&mut conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    Ok(OauthConnected {
        connector: inserted.into(),
        initiated_by: crm_data.initiated_by,
    })
}

async fn get_connector(
    store: &Store,
    tenant_id: TenantId,
    connector_type: ConnectorTypeEnum,
    connector_provider: ConnectorProviderEnum,
) -> StoreResult<Option<Connector>> {
    let mut conn = store.get_conn().await?;
    let row = ConnectorRow::list_connectors(
        &mut conn,
        tenant_id,
        Some(connector_type.into()),
        Some(connector_provider.into()),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?
    .into_iter()
    .next();

    row.map(|row| Connector::from_row(&store.settings.crypt_key, row))
        .transpose()
}
