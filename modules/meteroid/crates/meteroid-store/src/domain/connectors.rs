use crate::domain::enums::{
    ConnectorProviderEnum, ConnectorStatusEnum, ConnectorTypeEnum, ConnectorWebhookRouteEnum,
    PaymentMethodTypeEnum, PaymentProvider,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::{StoreResult, json_value_ser, json_value_serde};
use chrono::{DateTime, NaiveDateTime, Utc};
use common_domain::ids::{BaseId, ConnectorId, TenantId};
use diesel_models::connectors::{ConnectorRow, ConnectorRowNew};
use error_stack::{Report, ResultExt};
use meteroid_oauth::model::OAuthTokens;
use o2o::o2o;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::str::FromStr;

#[derive(Clone)]
pub struct Connector {
    pub id: ConnectorId,
    pub created_at: NaiveDateTime,
    pub tenant_id: TenantId,
    pub alias: String,
    /// Which provider this is, and everything that depends on it. The provider, the connector
    /// type and the stored data all come from here, so they can't disagree.
    pub config: ConnectorConfig,
    pub status: ConnectorStatusEnum,
    /// What the last credential check learned. One field, because an account and a rejection
    /// describe the same thing and used to be two `Option`s that could contradict each other.
    pub credentials: CredentialState,
    /// Where the provider sends webhooks; `None` when it sends none.
    pub webhook_route: Option<ConnectorWebhookRouteEnum>,
}

/// The result of the last credential check.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CredentialState {
    /// Never checked, or never checked successfully.
    #[default]
    Unverified,
    Verified(ProviderAccount),
    /// The provider refused the stored credentials. The last account we saw is kept: the UI
    /// shows what the connection used to point at alongside the error.
    Rejected {
        account: Option<ProviderAccount>,
        error: CredentialsError,
    },
}

impl CredentialState {
    /// Rebuilds the state from the columns a connector row stores.
    pub fn from_parts(account: Option<ProviderAccount>, error: Option<CredentialsError>) -> Self {
        match (account, error) {
            (account, Some(error)) => CredentialState::Rejected { account, error },
            (Some(account), None) => CredentialState::Verified(account),
            (None, None) => CredentialState::Unverified,
        }
    }

    /// What the provider account last looked like, verified or not.
    pub fn account(&self) -> Option<&ProviderAccount> {
        match self {
            CredentialState::Unverified => None,
            CredentialState::Verified(account) => Some(account),
            CredentialState::Rejected { account, .. } => account.as_ref(),
        }
    }

    pub fn error(&self) -> Option<&CredentialsError> {
        match self {
            CredentialState::Rejected { error, .. } => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialsError {
    pub message: String,
    pub at: NaiveDateTime,
}

/// What we last learned from the provider account.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderAccount {
    /// Stripe account, GoCardless creditor or Mollie profile id; `None` when the provider
    /// exposes none (Stancer).
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub account_id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    pub livemode: bool,
    /// Masked secret, e.g. `sk_live_…4f2a`.
    #[serde(default)]
    pub key_hint: Option<String>,
    /// One entry per rail the adapter supports.
    #[serde(default)]
    pub methods: Vec<PaymentMethodAvailability>,
    pub checked_at: NaiveDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaymentMethodAvailability {
    pub method: PaymentMethodTypeEnum,
    pub state: MethodState,
    /// Provider wording, when it says why the method isn't enabled.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MethodState {
    Enabled,
    /// Requested at the provider, not usable yet.
    Pending,
    Unavailable,
}

impl std::fmt::Debug for Connector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connector")
            .field("id", &self.id)
            .field("created_at", &self.created_at)
            .field("tenant_id", &self.tenant_id)
            .field("alias", &self.alias)
            .field("connector_type", &self.connector_type())
            .field("provider", &self.provider())
            .field("config", &self.config)
            .field("status", &self.status)
            .field("credentials", &self.credentials)
            .field("webhook_route", &self.webhook_route)
            .finish()
    }
}

impl Connector {
    pub fn provider(&self) -> ConnectorProviderEnum {
        self.config.provider()
    }

    pub fn connector_type(&self) -> ConnectorTypeEnum {
        self.config.connector_type()
    }

    /// `None` for a connector that takes no payments (Hubspot, Pennylane).
    pub fn payment_provider(&self) -> Option<PaymentProvider> {
        self.config.payment().map(PaymentConnectorConfig::provider)
    }

    /// The payment credentials, once they are set; `None` while the connector is a draft.
    pub fn payment_credentials(&self) -> Option<&PaymentProviderConfig> {
        self.config.payment().and_then(|p| p.credentials())
    }

    /// What the provider account last looked like, verified or not.
    pub fn account(&self) -> Option<&ProviderAccount> {
        self.credentials.account()
    }

    pub fn credentials_error(&self) -> Option<&CredentialsError> {
        self.credentials.error()
    }

    pub fn payment_credentials_mut(&mut self) -> Option<&mut PaymentProviderConfig> {
        self.config
            .payment_mut()
            .and_then(PaymentConnectorConfig::credentials_mut)
    }

    /// Stores the public data a credential check just returned.
    pub fn refresh_public_data(&mut self, public: PaymentConnectorPublicData) -> StoreResult<()> {
        let Some(payment) = self.config.payment_mut() else {
            return Err(StoreError::InvalidArgument(format!(
                "{:?} has no payment credentials to refresh",
                self.config.provider()
            ))
            .into());
        };
        let Some(credentials) = payment.credentials().cloned() else {
            return Err(StoreError::InvalidArgument(
                "this connector has no credentials yet".to_string(),
            )
            .into());
        };
        payment.set_credentials(credentials.with_public(public)?);
        Ok(())
    }

    /// The company this connector is linked to at an integration provider (Hubspot, Pennylane).
    pub fn external_company_id(&self) -> Option<&str> {
        match &self.config {
            ConnectorConfig::Hubspot { data, .. } => {
                data.as_ref().map(|d| d.external_company_id.as_str())
            }
            ConnectorConfig::Pennylane { data, .. } => {
                data.as_ref().map(|d| d.external_company_id.as_str())
            }
            ConnectorConfig::Payment(_) => None,
        }
    }

    pub fn hubspot_data(&self) -> Option<&HubspotPublicData> {
        match &self.config {
            ConnectorConfig::Hubspot { data, .. } => data.as_ref(),
            _ => None,
        }
    }

    /// Secret for verifying inbound webhooks; `None` if the provider has none or it isn't set.
    pub fn webhook_secret(&self) -> Option<SecretString> {
        self.payment_credentials().and_then(|c| c.webhook_secret())
    }

    /// Client-side key for an embedded SDK (Stripe publishable key).
    pub fn publishable_key(&self) -> Option<&str> {
        match self.payment_credentials() {
            Some(PaymentProviderConfig::Stripe(d, _)) => Some(d.api_publishable_key.as_str()),
            _ => None,
        }
    }
}

/// Everything about a connector that depends on which provider it is.
#[derive(Clone, Debug)]
pub enum ConnectorConfig {
    Payment(PaymentConnectorConfig),
    Hubspot {
        data: Option<HubspotPublicData>,
        sensitive: Option<HubspotSensitiveData>,
    },
    Pennylane {
        data: Option<PennylanePublicData>,
        sensitive: Option<PennylaneSensitiveData>,
    },
}

impl ConnectorConfig {
    pub fn provider(&self) -> ConnectorProviderEnum {
        match self {
            ConnectorConfig::Payment(p) => p.provider().into(),
            ConnectorConfig::Hubspot { .. } => ConnectorProviderEnum::Hubspot,
            ConnectorConfig::Pennylane { .. } => ConnectorProviderEnum::Pennylane,
        }
    }

    pub fn connector_type(&self) -> ConnectorTypeEnum {
        match self {
            ConnectorConfig::Payment(_) => ConnectorTypeEnum::PaymentProvider,
            ConnectorConfig::Hubspot { .. } => ConnectorTypeEnum::Crm,
            ConnectorConfig::Pennylane { .. } => ConnectorTypeEnum::Accounting,
        }
    }

    pub fn payment(&self) -> Option<&PaymentConnectorConfig> {
        match self {
            ConnectorConfig::Payment(p) => Some(p),
            _ => None,
        }
    }

    pub fn payment_mut(&mut self) -> Option<&mut PaymentConnectorConfig> {
        match self {
            ConnectorConfig::Payment(p) => Some(p),
            _ => None,
        }
    }

    /// Rebuilds the config from the columns a connector row stores. The one place where data that
    /// doesn't match its provider is caught.
    pub fn from_row_parts(
        provider: ConnectorProviderEnum,
        data: Option<ProviderData>,
        sensitive: Option<ProviderSensitiveData>,
    ) -> StoreResult<Self> {
        let mismatch = || {
            Report::from(StoreError::InvalidArgument(format!(
                "stored data isn't {provider:?} data"
            )))
        };
        Ok(match provider {
            ConnectorProviderEnum::Hubspot => ConnectorConfig::Hubspot {
                data: match data {
                    None => None,
                    Some(ProviderData::Hubspot(d)) => Some(d),
                    Some(_) => return Err(mismatch()),
                },
                sensitive: match sensitive {
                    None => None,
                    Some(ProviderSensitiveData::Hubspot(s)) => Some(s),
                    Some(_) => return Err(mismatch()),
                },
            },
            ConnectorProviderEnum::Pennylane => ConnectorConfig::Pennylane {
                data: match data {
                    None => None,
                    Some(ProviderData::Pennylane(d)) => Some(d),
                    Some(_) => return Err(mismatch()),
                },
                sensitive: match sensitive {
                    None => None,
                    Some(ProviderSensitiveData::Pennylane(s)) => Some(s),
                    Some(_) => return Err(mismatch()),
                },
            },
            // Listed one by one on purpose: a provider added to `ConnectorProviderEnum` has to be
            // classified here, and the compiler says so.
            ConnectorProviderEnum::Stripe => payment(PaymentProvider::Stripe, data, sensitive)?,
            ConnectorProviderEnum::Gocardless => {
                payment(PaymentProvider::Gocardless, data, sensitive)?
            }
            ConnectorProviderEnum::Stancer => payment(PaymentProvider::Stancer, data, sensitive)?,
            ConnectorProviderEnum::Mollie => payment(PaymentProvider::Mollie, data, sensitive)?,
            ConnectorProviderEnum::Mock => payment(PaymentProvider::Mock, data, sensitive)?,
        })
    }

    /// The two columns a connector row stores, ready to serialize and encrypt.
    pub fn to_row_parts(&self) -> (Option<ProviderData>, Option<ProviderSensitiveData>) {
        match self {
            ConnectorConfig::Payment(p) => match p.credentials() {
                Some(credentials) => {
                    let (data, sensitive) = credentials.clone().into_parts();
                    (Some(data), Some(sensitive))
                }
                None => (None, None),
            },
            ConnectorConfig::Hubspot { data, sensitive } => (
                data.clone().map(ProviderData::Hubspot),
                sensitive.clone().map(ProviderSensitiveData::Hubspot),
            ),
            ConnectorConfig::Pennylane { data, sensitive } => (
                data.clone().map(ProviderData::Pennylane),
                sensitive.clone().map(ProviderSensitiveData::Pennylane),
            ),
        }
    }
}

/// A credentials update: every field is optional, and an absent one keeps what is stored. A
/// rotation only has to carry what changes, and "leave this alone" is no longer spelled with an
/// empty string.
#[derive(Clone, Debug)]
pub enum PaymentCredentialsUpdate {
    Stripe {
        api_publishable_key: Option<String>,
        api_secret_key: Option<SecretString>,
        webhook_secret: Option<SecretString>,
    },
    Gocardless {
        creditor_id: Option<String>,
        environment: Option<GocardlessEnvironment>,
        access_token: Option<SecretString>,
        webhook_secret: Option<SecretString>,
    },
    Stancer {
        api_secret_key: Option<SecretString>,
    },
    Mollie {
        api_key: Option<SecretString>,
    },
    Mock(MockPublicData),
}

impl PaymentCredentialsUpdate {
    pub fn provider(&self) -> PaymentProvider {
        match self {
            PaymentCredentialsUpdate::Stripe { .. } => PaymentProvider::Stripe,
            PaymentCredentialsUpdate::Gocardless { .. } => PaymentProvider::Gocardless,
            PaymentCredentialsUpdate::Stancer { .. } => PaymentProvider::Stancer,
            PaymentCredentialsUpdate::Mollie { .. } => PaymentProvider::Mollie,
            PaymentCredentialsUpdate::Mock(_) => PaymentProvider::Mock,
        }
    }

    /// Applies the update to what is stored, or to nothing when the connector is still a draft.
    /// A field this update leaves out keeps its stored value; a field with nowhere to fall back
    /// to starts empty, and [`super::super::repositories`] refuses to save it.
    pub fn apply_to(
        self,
        stored: Option<&PaymentProviderConfig>,
    ) -> StoreResult<PaymentProviderConfig> {
        let provider = self.provider();
        let mismatch = move |stored: &PaymentProviderConfig| {
            Report::from(StoreError::ProviderMismatch {
                sent: format!("{provider:?}"),
                stored: format!("{:?}", stored.provider()),
            })
        };
        Ok(match self {
            PaymentCredentialsUpdate::Stripe {
                api_publishable_key,
                api_secret_key,
                webhook_secret,
            } => {
                let (public, sensitive) = match stored {
                    None => (StripePublicData::empty(), StripeSensitiveData::empty()),
                    Some(PaymentProviderConfig::Stripe(d, s)) => (d.clone(), s.clone()),
                    Some(other) => return Err(mismatch(other)),
                };
                PaymentProviderConfig::Stripe(
                    StripePublicData {
                        api_publishable_key: api_publishable_key
                            .unwrap_or(public.api_publishable_key),
                        // Never input: the credential check fills it in.
                        account_id: public.account_id,
                        webhook_events_fingerprint: public.webhook_events_fingerprint,
                    },
                    StripeSensitiveData {
                        api_secret_key: api_secret_key.unwrap_or(sensitive.api_secret_key),
                        // A new webhook secret is someone else's, so the endpoint id we
                        // registered no longer names ours.
                        webhook_endpoint_id: sensitive
                            .webhook_endpoint_id
                            .filter(|_| webhook_secret.is_none()),
                        webhook_secret: webhook_secret.unwrap_or(sensitive.webhook_secret),
                    },
                )
            }
            PaymentCredentialsUpdate::Gocardless {
                creditor_id,
                environment,
                access_token,
                webhook_secret,
            } => {
                let (public, sensitive) = match stored {
                    None => (
                        GocardlessPublicData::empty(),
                        GocardlessSensitiveData::empty(),
                    ),
                    Some(PaymentProviderConfig::Gocardless(d, s)) => (d.clone(), s.clone()),
                    Some(other) => return Err(mismatch(other)),
                };
                PaymentProviderConfig::Gocardless(
                    GocardlessPublicData {
                        creditor_id: creditor_id.or(public.creditor_id),
                        environment: environment.unwrap_or(public.environment),
                    },
                    GocardlessSensitiveData {
                        access_token: access_token.unwrap_or(sensitive.access_token),
                        webhook_secret: webhook_secret.unwrap_or(sensitive.webhook_secret),
                    },
                )
            }
            PaymentCredentialsUpdate::Stancer { api_secret_key } => {
                let sensitive = match stored {
                    None => StancerSensitiveData::empty(),
                    Some(PaymentProviderConfig::Stancer(s)) => s.clone(),
                    Some(other) => return Err(mismatch(other)),
                };
                PaymentProviderConfig::Stancer(StancerSensitiveData {
                    api_secret_key: api_secret_key.unwrap_or(sensitive.api_secret_key),
                })
            }
            PaymentCredentialsUpdate::Mollie { api_key } => {
                let sensitive = match stored {
                    // Mollie doesn't sign webhooks, so a new connection mints a URL token.
                    None => MollieSensitiveData::new(SecretString::from(String::new())),
                    Some(PaymentProviderConfig::Mollie(s)) => s.clone(),
                    Some(other) => return Err(mismatch(other)),
                };
                PaymentProviderConfig::Mollie(MollieSensitiveData {
                    api_key: api_key.unwrap_or(sensitive.api_key),
                    // In-flight payments carry the stored token in their webhook URL.
                    webhook_token: sensitive.webhook_token,
                })
            }
            PaymentCredentialsUpdate::Mock(public) => {
                let sensitive = match stored {
                    None => MockSensitiveData {},
                    Some(PaymentProviderConfig::Mock(_, s)) => s.clone(),
                    Some(other) => return Err(mismatch(other)),
                };
                PaymentProviderConfig::Mock(public, sensitive)
            }
        })
    }
}

/// The public half of one payment provider's data, as a credential check returns it.
#[derive(Clone, Debug)]
pub enum PaymentConnectorPublicData {
    Stripe(StripePublicData),
    Gocardless(GocardlessPublicData),
    Stancer(StancerPublicData),
    Mollie(MolliePublicData),
    Mock(MockPublicData),
}

impl PaymentConnectorPublicData {
    pub fn provider(&self) -> PaymentProvider {
        match self {
            PaymentConnectorPublicData::Stripe(_) => PaymentProvider::Stripe,
            PaymentConnectorPublicData::Gocardless(_) => PaymentProvider::Gocardless,
            PaymentConnectorPublicData::Stancer(_) => PaymentProvider::Stancer,
            PaymentConnectorPublicData::Mollie(_) => PaymentProvider::Mollie,
            PaymentConnectorPublicData::Mock(_) => PaymentProvider::Mock,
        }
    }
}

/// One payment provider's stored columns, as either configured credentials or a draft.
fn payment(
    provider: PaymentProvider,
    data: Option<ProviderData>,
    sensitive: Option<ProviderSensitiveData>,
) -> StoreResult<ConnectorConfig> {
    let credentials = PaymentProviderConfig::from_row_parts(provider, data, sensitive)?;
    Ok(ConnectorConfig::Payment(match credentials {
        Some(credentials) => PaymentConnectorConfig::configured(credentials),
        None => PaymentConnectorConfig::draft(provider),
    }))
}

/// A payment connector: the provider it is for, plus its credentials once they are set. The
/// provider always follows the credentials, so the two can never name different providers.
#[derive(Clone, Debug)]
pub struct PaymentConnectorConfig {
    provider: PaymentProvider,
    credentials: Option<PaymentProviderConfig>,
}

impl PaymentConnectorConfig {
    /// A connector whose credentials haven't been set yet.
    pub fn draft(provider: PaymentProvider) -> Self {
        Self {
            provider,
            credentials: None,
        }
    }

    pub fn configured(credentials: PaymentProviderConfig) -> Self {
        Self {
            provider: credentials.provider(),
            credentials: Some(credentials),
        }
    }

    pub fn provider(&self) -> PaymentProvider {
        self.provider
    }

    pub fn credentials(&self) -> Option<&PaymentProviderConfig> {
        self.credentials.as_ref()
    }

    pub fn credentials_mut(&mut self) -> Option<&mut PaymentProviderConfig> {
        self.credentials.as_mut()
    }

    /// Sets or rotates the credentials, filling blank fields from what is already stored. The
    /// only place a connector's provider is checked against incoming credentials: an RPC names a
    /// connector by id and carries credentials for some provider, and the two can disagree.
    pub fn apply(&mut self, update: PaymentCredentialsUpdate) -> StoreResult<()> {
        if update.provider() != self.provider {
            return Err(StoreError::ProviderMismatch {
                sent: format!("{:?}", update.provider()),
                stored: format!("{:?}", self.provider),
            }
            .into());
        }
        self.credentials = Some(update.apply_to(self.credentials.as_ref())?);
        Ok(())
    }

    /// Replaces the credentials wholesale, for a connector built from scratch.
    pub fn set_credentials(&mut self, credentials: PaymentProviderConfig) {
        self.provider = credentials.provider();
        self.credentials = Some(credentials);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ProviderData {
    Stripe(StripePublicData),
    Hubspot(HubspotPublicData),
    Pennylane(PennylanePublicData),
    Mock(MockPublicData),
    Gocardless(GocardlessPublicData),
    Stancer(StancerPublicData),
    Mollie(MolliePublicData),
}

json_value_ser!(ProviderData);

/// What a merchant supplies to connect a payment provider, or to rotate its credentials. Each
/// variant pairs the public and sensitive data of one provider, so they can't disagree.
#[derive(Clone, Debug)]
pub enum PaymentProviderConfig {
    Stripe(StripePublicData, StripeSensitiveData),
    Gocardless(GocardlessPublicData, GocardlessSensitiveData),
    Stancer(StancerSensitiveData),
    Mollie(MollieSensitiveData),
    Mock(MockPublicData, MockSensitiveData),
}

impl PaymentProviderConfig {
    pub fn provider(&self) -> PaymentProvider {
        match self {
            PaymentProviderConfig::Stripe(..) => PaymentProvider::Stripe,
            PaymentProviderConfig::Gocardless(..) => PaymentProvider::Gocardless,
            PaymentProviderConfig::Stancer(_) => PaymentProvider::Stancer,
            PaymentProviderConfig::Mollie(_) => PaymentProvider::Mollie,
            PaymentProviderConfig::Mock(..) => PaymentProvider::Mock,
        }
    }

    pub fn into_parts(self) -> (ProviderData, ProviderSensitiveData) {
        match self {
            PaymentProviderConfig::Stripe(d, s) => {
                (ProviderData::Stripe(d), ProviderSensitiveData::Stripe(s))
            }
            PaymentProviderConfig::Gocardless(d, s) => (
                ProviderData::Gocardless(d),
                ProviderSensitiveData::Gocardless(s),
            ),
            PaymentProviderConfig::Stancer(s) => (
                ProviderData::Stancer(StancerPublicData::default()),
                ProviderSensitiveData::Stancer(s),
            ),
            PaymentProviderConfig::Mollie(s) => (
                ProviderData::Mollie(MolliePublicData::default()),
                ProviderSensitiveData::Mollie(s),
            ),
            PaymentProviderConfig::Mock(d, s) => {
                (ProviderData::Mock(d), ProviderSensitiveData::Mock(s))
            }
        }
    }

    /// The credentials a connector row stores; `None` for a draft that has none yet. Errors when
    /// the stored data isn't `provider`'s.
    pub fn from_row_parts(
        provider: PaymentProvider,
        data: Option<ProviderData>,
        sensitive: Option<ProviderSensitiveData>,
    ) -> StoreResult<Option<Self>> {
        // No secret blob, no credentials: a draft has never had one, and a soft-deleted
        // connector's was wiped while its public data stayed for history.
        if sensitive.is_none() {
            return Ok(None);
        }
        let config = match (&data, &sensitive) {
            (Some(ProviderData::Stripe(d)), Some(ProviderSensitiveData::Stripe(s))) => {
                PaymentProviderConfig::Stripe(d.clone(), s.clone())
            }
            (Some(ProviderData::Gocardless(d)), Some(ProviderSensitiveData::Gocardless(s))) => {
                PaymentProviderConfig::Gocardless(d.clone(), s.clone())
            }
            (None | Some(ProviderData::Stancer(_)), Some(ProviderSensitiveData::Stancer(s))) => {
                PaymentProviderConfig::Stancer(s.clone())
            }
            (None | Some(ProviderData::Mollie(_)), Some(ProviderSensitiveData::Mollie(s))) => {
                PaymentProviderConfig::Mollie(s.clone())
            }
            (Some(ProviderData::Mock(d)), Some(ProviderSensitiveData::Mock(s))) => {
                PaymentProviderConfig::Mock(d.clone(), s.clone())
            }
            _ => {
                return Err(StoreError::InvalidArgument(format!(
                    "stored data isn't {provider:?} credentials"
                ))
                .into());
            }
        };
        if config.provider() != provider {
            return Err(StoreError::InvalidArgument(format!(
                "stored data is {:?} credentials, not {provider:?}",
                config.provider()
            ))
            .into());
        }
        Ok(Some(config))
    }

    /// The value inbound webhooks are verified against: a signing secret, or the URL token for
    /// unsigned webhooks (Mollie). `None` when the provider has none or it isn't set.
    pub fn webhook_secret(&self) -> Option<SecretString> {
        let secret = match self {
            PaymentProviderConfig::Stripe(_, s) => &s.webhook_secret,
            PaymentProviderConfig::Gocardless(_, s) => &s.webhook_secret,
            PaymentProviderConfig::Mollie(s) => &s.webhook_token,
            PaymentProviderConfig::Stancer(_) | PaymentProviderConfig::Mock(..) => {
                return None;
            }
        };
        (!is_blank(secret)).then(|| secret.clone())
    }

    /// The secret the key hint is derived from.
    pub fn primary_secret(&self) -> Option<&SecretString> {
        let secret = match self {
            PaymentProviderConfig::Stripe(_, s) => &s.api_secret_key,
            PaymentProviderConfig::Gocardless(_, s) => &s.access_token,
            PaymentProviderConfig::Stancer(s) => &s.api_secret_key,
            PaymentProviderConfig::Mollie(s) => &s.api_key,
            PaymentProviderConfig::Mock(..) => return None,
        };
        (!is_blank(secret)).then_some(secret)
    }

    /// Stores the secret and endpoint id of a webhook endpoint we registered. `None` if this
    /// provider has nowhere to keep them, so the caller never persists a connector it can't
    /// verify webhooks for.
    pub fn with_registered_webhook(
        mut self,
        endpoint_id: &str,
        secret: &SecretString,
    ) -> Option<Self> {
        match &mut self {
            PaymentProviderConfig::Stripe(_, s) => {
                s.webhook_secret = secret.clone();
                s.webhook_endpoint_id = Some(endpoint_id.to_string());
                Some(self)
            }
            _ => None,
        }
    }

    /// Replaces the public half with what the provider just told us.
    pub fn with_public(self, public: PaymentConnectorPublicData) -> StoreResult<Self> {
        Ok(match (self, public) {
            (PaymentProviderConfig::Stripe(_, s), PaymentConnectorPublicData::Stripe(d)) => {
                PaymentProviderConfig::Stripe(d, s)
            }
            (
                PaymentProviderConfig::Gocardless(_, s),
                PaymentConnectorPublicData::Gocardless(d),
            ) => PaymentProviderConfig::Gocardless(d, s),
            (PaymentProviderConfig::Stancer(s), PaymentConnectorPublicData::Stancer(_)) => {
                PaymentProviderConfig::Stancer(s)
            }
            (PaymentProviderConfig::Mollie(s), PaymentConnectorPublicData::Mollie(_)) => {
                PaymentProviderConfig::Mollie(s)
            }
            (PaymentProviderConfig::Mock(_, s), PaymentConnectorPublicData::Mock(d)) => {
                PaymentProviderConfig::Mock(d, s)
            }
            (config, public) => {
                return Err(StoreError::InvalidArgument(format!(
                    "the credential check returned {:?} data for a {:?} connector",
                    public.provider(),
                    config.provider()
                ))
                .into());
            }
        })
    }
}

/// An empty account id means the provider exposes none, or the credential check hasn't run.
fn empty_string_as_none<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<String>, D::Error> {
    let value = Option::<String>::deserialize(d)?;
    Ok(value.filter(|s| !s.is_empty()))
}

/// Serializes a secret into the connector's `sensitive` blob, which is encrypted before it
/// leaves the process. [`SecretString`] refuses to serialize by default, on purpose: every use
/// of this helper is a place a secret is written down.
mod secret_string {
    use secrecy::{ExposeSecret, SecretString};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &SecretString, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(value.expose_secret())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SecretString, D::Error> {
        Ok(SecretString::from(String::deserialize(d)?))
    }
}

/// Whether a secret carries no value at all.
pub fn is_blank(secret: &SecretString) -> bool {
    secret.expose_secret().trim().is_empty()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StripePublicData {
    pub api_publishable_key: String,
    /// Filled in by the credential check; `None` until it first succeeds.
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub account_id: Option<String>,
    /// What our self-registered endpoint was last synced to; the startup resync skips a
    /// connector whose fingerprint already matches the current event set.
    #[serde(default)]
    pub webhook_events_fingerprint: Option<String>,
}

impl StripePublicData {
    /// What a brand-new connection starts from, before anything is entered.
    fn empty() -> Self {
        Self {
            api_publishable_key: String::new(),
            account_id: None,
            webhook_events_fingerprint: None,
        }
    }
}

impl StripeSensitiveData {
    fn empty() -> Self {
        Self {
            api_secret_key: SecretString::from(String::new()),
            webhook_secret: SecretString::from(String::new()),
            webhook_endpoint_id: None,
        }
    }
}

impl StancerSensitiveData {
    fn empty() -> Self {
        Self {
            api_secret_key: SecretString::from(String::new()),
        }
    }
}

impl GocardlessPublicData {
    fn empty() -> Self {
        Self {
            creditor_id: None,
            environment: GocardlessEnvironment::default(),
        }
    }
}

impl GocardlessSensitiveData {
    fn empty() -> Self {
        Self {
            access_token: SecretString::from(String::new()),
            webhook_secret: SecretString::from(String::new()),
        }
    }
}

/// Stancer has no publishable key and no account id: the secret key alone
/// authenticates every call, and the hosted-page URL comes fully formed from
/// the payment-intent response. Nothing public to store.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StancerPublicData {}

/// The API key prefix selects test/live; webhook URLs are built server-side.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MolliePublicData {}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HubspotPublicData {
    pub auto_sync: bool,
    pub external_company_id: String, // hub_id
}

json_value_ser!(HubspotPublicData);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PennylanePublicData {
    pub external_company_id: String, // company.id
}

json_value_ser!(PennylanePublicData);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ProviderSensitiveData {
    Stripe(StripeSensitiveData),
    Hubspot(HubspotSensitiveData),
    Pennylane(PennylaneSensitiveData),
    Mock(MockSensitiveData),
    Gocardless(GocardlessSensitiveData),
    Stancer(StancerSensitiveData),
    Mollie(MollieSensitiveData),
}

impl ProviderSensitiveData {
    pub fn encrypt(&self, key: &SecretString) -> StoreResult<String> {
        let s = serde_json::to_string(self).map_err(|e| {
            StoreError::SerdeError(
                "Failed to serialize sensitive connector data".to_string(),
                e,
            )
        })?;

        crate::crypt::encrypt(key, s.as_str())
            .change_context(StoreError::CryptError("connector encryption error".into()))
    }

    pub fn decrypt(key: &SecretString, enc: &str) -> StoreResult<Self> {
        let decrypted = crate::crypt::decrypt(key, enc)
            .change_context(StoreError::CryptError("connector decryption error".into()))?;

        let sensitive: ProviderSensitiveData = serde_json::from_str(decrypted.expose_secret())
            .map_err(|e| {
                StoreError::SerdeError(
                    "Failed to deserialize sensitive connector data".to_string(),
                    e,
                )
            })?;
        Ok(sensitive)
    }
}

impl TryFrom<OAuthTokens> for PennylaneSensitiveData {
    type Error = StoreErrorReport;

    fn try_from(value: OAuthTokens) -> Result<Self, Self::Error> {
        Ok(PennylaneSensitiveData {
            access_token: value.access_token,
            refresh_token: value
                .refresh_token
                .ok_or_else(|| StoreError::OauthError("missing refresh_token".into()))?,
            expires_at: value.expires_in.map(|duration| Utc::now() + duration),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StripeSensitiveData {
    #[serde(with = "secret_string")]
    pub api_secret_key: SecretString,
    #[serde(with = "secret_string")]
    pub webhook_secret: SecretString,
    /// Stripe's `we_…` identifier returned by `POST /v1/webhook_endpoints`.
    /// Populated when we auto-register the endpoint on connect; `None` when
    /// the customer pasted the webhook secret manually. We need the id to
    /// update the subscription set as Meteroid adds new event types and to
    /// clean up the endpoint on disconnect.
    #[serde(default)]
    pub webhook_endpoint_id: Option<String>,
}

/// Stancer's only credential. Test vs live mode rides on the key prefix
/// (`stest_…` / `sprod_…`) — there is no environment toggle. No webhook
/// secret: Stancer has no webhook mechanism at all (spec-verified).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StancerSensitiveData {
    #[serde(with = "secret_string")]
    pub api_secret_key: SecretString,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MollieSensitiveData {
    /// `test_…` / `live_…` API key; the prefix selects the mode.
    #[serde(with = "secret_string")]
    pub api_key: SecretString,
    /// Random token appended to `webhookUrl` to authenticate Mollie's unsigned webhooks.
    #[serde(with = "secret_string")]
    pub webhook_token: SecretString,
}

impl MollieSensitiveData {
    /// Mollie doesn't sign webhooks, so we generate a 256-bit token for the URL.
    pub fn new(api_key: SecretString) -> Self {
        Self {
            api_key,
            webhook_token: SecretString::from(hex::encode(rand::random::<[u8; 32]>())),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HubspotSensitiveData {
    #[serde(with = "secret_string")]
    pub refresh_token: SecretString,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PennylaneSensitiveData {
    #[serde(with = "secret_string")]
    pub refresh_token: SecretString,
    #[serde(with = "secret_string")]
    pub access_token: SecretString,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MockPublicData {
    #[serde(default)]
    pub fail_payment_intent: bool,
    #[serde(default)]
    pub fail_setup_intent: bool,
    /// Forces a charge outcome for e2e tests without a real provider; unset means success.
    #[serde(default)]
    pub charge_behavior: Option<MockChargeBehavior>,
    /// Account id the credential check reports; lets tests simulate rotating to another account.
    #[serde(default)]
    pub account_id: Option<String>,
    /// Methods the credential check reports as pending at the provider.
    #[serde(default)]
    pub pending_methods: Vec<PaymentMethodTypeEnum>,
    #[serde(default)]
    pub reject_credentials: bool,
    /// What the provider reports to the reconciliation sweep; unset means still in flight.
    #[serde(default)]
    pub reconcile_status: Option<MockReconcileStatus>,
    /// Forces a refund outcome; unset means success.
    #[serde(default)]
    pub refund_behavior: Option<MockRefundBehavior>,
}

json_value_ser!(MockPublicData);

/// What the mock provider does with a charge. The serialized names are the ones stored in the
/// connector's data blob, so a test scenario is pinned by a value, not a spelling.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MockChargeBehavior {
    Succeeded,
    /// Settles later, through a webhook.
    Pending,
    /// 3DS-style challenge, completed on the mock challenge page.
    RequiresAction,
    Failed,
    /// A hard decline: the card expired, so charging it again cannot succeed.
    Expired,
    /// A hard decline: the card was reported lost, so charging it again cannot succeed.
    HardDeclined,
    Cancelled,
    /// The provider refuses the stored key.
    RejectedCredentials,
    /// Transport failure: the outcome is UNCERTAIN, not a decline. Only reconciliation can tell
    /// whether the money moved.
    Unknown,
}

/// What the provider reports to the reconciliation sweep.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MockReconcileStatus {
    Succeeded,
    Failed,
    Cancelled,
    /// The provider has no record of it: our outbound call never landed.
    Unknown,
    Pending,
    /// Pending on the provider alone: the customer finished what was asked of them.
    Processing,
}

/// What the mock provider does with a refund.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MockRefundBehavior {
    Succeeded,
    Pending,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MockSensitiveData {}

/// GoCardless public data. The `creditor_id` is the merchant's GC identifier;
/// surfaced to the frontend so we can prefill it on the BR creation form.
/// `environment` switches between sandbox and live API base URLs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GocardlessPublicData {
    pub creditor_id: Option<String>,
    /// Drives the API base URL used by the client.
    #[serde(default)]
    pub environment: GocardlessEnvironment,
}

/// Which GoCardless deployment a connector talks to. A row with no environment stored falls
/// back to the sandbox: a misconfiguration must never route real charges to live.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GocardlessEnvironment {
    #[default]
    Sandbox,
    Live,
}

impl GocardlessEnvironment {
    pub fn is_sandbox(&self) -> bool {
        *self == GocardlessEnvironment::Sandbox
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            GocardlessEnvironment::Sandbox => "sandbox",
            GocardlessEnvironment::Live => "live",
        }
    }
}

impl FromStr for GocardlessEnvironment {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "sandbox" => Ok(GocardlessEnvironment::Sandbox),
            "live" => Ok(GocardlessEnvironment::Live),
            _ => Err(()),
        }
    }
}

impl GocardlessPublicData {
    pub fn is_sandbox(&self) -> bool {
        self.environment.is_sandbox()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GocardlessSensitiveData {
    /// Bearer access token. Long-lived; rotates rarely.
    #[serde(with = "secret_string")]
    pub access_token: SecretString,
    /// Endpoint signing secret. Configured by the merchant in the GoCardless
    /// dashboard (GoCardless does not currently expose a public API to
    /// create webhook endpoints, so this is always pasted manually).
    #[serde(with = "secret_string")]
    pub webhook_secret: SecretString,
}

impl Connector {
    pub fn from_row(key: &SecretString, row: ConnectorRow) -> StoreResult<Connector> {
        let sensitive = if let Some(s) = row.sensitive {
            let decrypted = ProviderSensitiveData::decrypt(key, &s)?;
            Some(decrypted)
        } else {
            None
        };

        let data = row
            .data
            .map(|d| {
                serde_json::from_value(d).map_err(|e| {
                    StoreError::SerdeError("Failed to deserialize connector data".to_string(), e)
                })
            })
            .transpose()?;

        let account = row
            .account
            .map(|a| {
                serde_json::from_value(a).map_err(|e| {
                    StoreError::SerdeError("Failed to deserialize connector account".to_string(), e)
                })
            })
            .transpose()?;

        Ok(Connector {
            id: row.id,
            created_at: row.created_at,
            tenant_id: row.tenant_id,
            alias: row.alias,
            config: ConnectorConfig::from_row_parts(row.provider.into(), data, sensitive)?,
            status: row.status.into(),
            credentials: CredentialState::from_parts(
                account,
                row.credentials_error
                    .zip(row.credentials_error_at)
                    .map(|(message, at)| CredentialsError { message, at }),
            ),
            webhook_route: row.webhook_route.map(Into::into),
        })
    }
}

#[derive(Clone, Debug, o2o)]
#[from_owned(ConnectorRow)]
pub struct ConnectorMeta {
    pub id: ConnectorId,
    pub created_at: NaiveDateTime,
    pub tenant_id: TenantId,
    pub alias: String,
    // pub data: Option<ProviderData>,
    #[map(~.into())]
    pub connector_type: ConnectorTypeEnum,
    #[map(~.into())]
    pub provider: ConnectorProviderEnum,
    #[map(~.into())]
    pub status: ConnectorStatusEnum,
}

#[derive(Clone, Debug)]
pub struct ConnectorNew {
    pub tenant_id: TenantId,
    pub alias: String,
    pub config: ConnectorConfig,
    pub status: ConnectorStatusEnum,
    pub account: Option<ProviderAccount>,
    pub webhook_route: Option<ConnectorWebhookRouteEnum>,
}

impl ConnectorNew {
    pub fn to_row(&self, key: &SecretString) -> StoreResult<ConnectorRowNew> {
        let (data, sensitive) = self.config.to_row_parts();
        let sensitive = match &sensitive {
            Some(sensitive) => Some(sensitive.encrypt(key)?),
            None => None,
        };

        let data = match &data {
            Some(d) => serde_json::to_value(d)
                .map_err(|e| {
                    StoreError::SerdeError("Failed to serialize webhook data".to_string(), e)
                })
                .map(Some),
            None => Ok(None),
        }?;

        Ok(ConnectorRowNew {
            id: ConnectorId::new(),
            tenant_id: self.tenant_id,
            alias: self.alias.clone(),
            connector_type: self.config.connector_type().into(),
            provider: self.config.provider().into(),
            sensitive,
            data,
            status: self.status.into(),
            webhook_route: self.webhook_route.map(Into::into),
            account: self
                .account
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .map_err(|e| {
                    StoreError::SerdeError("Failed to serialize connector account".to_string(), e)
                })?,
        })
    }
}

#[skip_serializing_none]
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct ConnectionMeta {
    pub hubspot: Option<Vec<ConnectionMetaItem>>,
    pub pennylane: Option<Vec<ConnectionMetaItem>>,
}

impl ConnectionMeta {
    pub fn get_pennylane_id(&self, connector_id: ConnectorId) -> Option<i64> {
        self.pennylane
            .as_ref()
            .unwrap_or(&vec![])
            .iter()
            .find_map(|x| {
                if x.connector_id == connector_id {
                    i64::from_str(&x.external_id).ok()
                } else {
                    None
                }
            })
    }
}

json_value_serde!(ConnectionMeta);

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct ConnectionMetaItem {
    pub connector_id: ConnectorId,
    pub external_id: String,
    pub sync_at: DateTime<Utc>,
    pub external_company_id: String, // pennylane: company.id, hubspot: hub_id
}

json_value_serde!(ConnectionMetaItem);

#[derive(Clone, Debug)]
pub struct ConnectorAccessToken {
    pub connector_id: ConnectorId,
    pub external_company_id: String,
    pub access_token: SecretString,
    pub expires_at: Option<DateTime<Utc>>,
}

// The connector's `data` and `sensitive` columns are stored formats: these pin their JSON so a
// renamed field or a retyped value shows up as a golden diff rather than an unreadable row.
golden::golden!(ProviderData, {
    "stripe" => ProviderData::Stripe(StripePublicData {
        api_publishable_key: "pk_test_x".into(),
        account_id: Some("acct_test_x".into()),
        webhook_events_fingerprint: None,
    }),
    "stripe_unverified" => ProviderData::Stripe(StripePublicData {
        api_publishable_key: "pk_test_x".into(),
        account_id: None,
        webhook_events_fingerprint: None,
    }),
    "gocardless" => ProviderData::Gocardless(GocardlessPublicData {
        creditor_id: Some("CR000TEST".into()),
        environment: GocardlessEnvironment::Sandbox,
    }),
    "stancer" => ProviderData::Stancer(StancerPublicData::default()),
    "mollie" => ProviderData::Mollie(MolliePublicData::default()),
    "mock" => ProviderData::Mock(MockPublicData {
        charge_behavior: Some(MockChargeBehavior::RequiresAction),
        reconcile_status: Some(MockReconcileStatus::Pending),
        refund_behavior: Some(MockRefundBehavior::Succeeded),
        account_id: Some("acct_mock".into()),
        pending_methods: vec![PaymentMethodTypeEnum::DirectDebitSepa],
        ..Default::default()
    }),
    "hubspot" => ProviderData::Hubspot(HubspotPublicData {
        auto_sync: true,
        external_company_id: "hub_1".into(),
    }),
    "pennylane" => ProviderData::Pennylane(PennylanePublicData {
        external_company_id: "company_1".into(),
    })
});

golden::golden!(ProviderSensitiveData, {
    "stripe" => ProviderSensitiveData::Stripe(StripeSensitiveData {
        api_secret_key: "sk_test_x".into(),
        webhook_secret: "whsec_x".into(),
        webhook_endpoint_id: Some("we_x".into()),
    }),
    "gocardless" => ProviderSensitiveData::Gocardless(GocardlessSensitiveData {
        access_token: "gc_token".into(),
        webhook_secret: "gc_secret".into(),
    }),
    "stancer" => ProviderSensitiveData::Stancer(StancerSensitiveData {
        api_secret_key: "stest_x".into(),
    }),
    "mollie" => ProviderSensitiveData::Mollie(MollieSensitiveData {
        api_key: "test_x".into(),
        webhook_token: "token_x".into(),
    }),
    "mock" => ProviderSensitiveData::Mock(MockSensitiveData {}),
    "hubspot" => ProviderSensitiveData::Hubspot(HubspotSensitiveData {
        refresh_token: "hub_refresh".into(),
    }),
    "pennylane" => ProviderSensitiveData::Pennylane(PennylaneSensitiveData {
        refresh_token: "pl_refresh".into(),
        access_token: "pl_access".into(),
        expires_at: None,
    })
});

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn stripe_stored() -> PaymentProviderConfig {
        PaymentProviderConfig::Stripe(
            StripePublicData {
                api_publishable_key: "pk_stored".into(),
                account_id: Some("acct_stored".into()),
                webhook_events_fingerprint: None,
            },
            StripeSensitiveData {
                api_secret_key: "sk_stored".into(),
                webhook_secret: "whsec_stored".into(),
                webhook_endpoint_id: Some("we_stored".into()),
            },
        )
    }

    /// A rotation only carries what changes; everything else keeps its stored value.
    #[test]
    fn an_absent_field_keeps_what_is_stored() {
        let update = PaymentCredentialsUpdate::Stripe {
            api_publishable_key: None,
            api_secret_key: Some("sk_new".into()),
            webhook_secret: None,
        };
        let merged = update.apply_to(Some(&stripe_stored())).unwrap();
        let PaymentProviderConfig::Stripe(public, sensitive) = merged else {
            panic!("expected stripe credentials");
        };
        assert_eq!(public.api_publishable_key, "pk_stored");
        assert_eq!(public.account_id.as_deref(), Some("acct_stored"));
        assert_eq!(sensitive.api_secret_key.expose_secret(), "sk_new");
        assert_eq!(sensitive.webhook_secret.expose_secret(), "whsec_stored");
        assert_eq!(
            sensitive.webhook_endpoint_id.as_deref(),
            Some("we_stored"),
            "the endpoint we registered still matches the secret we kept"
        );
    }

    /// The endpoint id names an endpoint whose secret we knew; a pasted secret is someone else's.
    #[test]
    fn a_new_webhook_secret_drops_the_endpoint_id() {
        let update = PaymentCredentialsUpdate::Stripe {
            api_publishable_key: None,
            api_secret_key: None,
            webhook_secret: Some("whsec_pasted".into()),
        };
        let merged = update.apply_to(Some(&stripe_stored())).unwrap();
        let PaymentProviderConfig::Stripe(_, sensitive) = merged else {
            panic!("expected stripe credentials");
        };
        assert_eq!(sensitive.webhook_secret.expose_secret(), "whsec_pasted");
        assert_eq!(sensitive.webhook_endpoint_id, None);
    }

    /// In-flight Mollie payments carry the stored token in their webhook URL.
    #[test]
    fn rotating_a_mollie_key_keeps_the_webhook_token() {
        let stored = PaymentProviderConfig::Mollie(MollieSensitiveData {
            api_key: "live_old".into(),
            webhook_token: "token_stored".into(),
        });
        let update = PaymentCredentialsUpdate::Mollie {
            api_key: Some("live_new".into()),
        };
        let PaymentProviderConfig::Mollie(sensitive) = update.apply_to(Some(&stored)).unwrap()
        else {
            panic!("expected mollie credentials");
        };
        assert_eq!(sensitive.api_key.expose_secret(), "live_new");
        assert_eq!(sensitive.webhook_token.expose_secret(), "token_stored");
    }

    /// A first connection has nothing to fall back on, and Mollie mints its URL token there.
    #[test]
    fn a_first_connection_mints_a_mollie_token() {
        let update = PaymentCredentialsUpdate::Mollie {
            api_key: Some("live_first".into()),
        };
        let PaymentProviderConfig::Mollie(sensitive) = update.apply_to(None).unwrap() else {
            panic!("expected mollie credentials");
        };
        assert_eq!(sensitive.api_key.expose_secret(), "live_first");
        assert_eq!(sensitive.webhook_token.expose_secret().len(), 64);
    }

    /// Credentials meant for another provider are refused, not merged.
    #[test]
    fn credentials_for_another_provider_are_refused() {
        let update = PaymentCredentialsUpdate::Mollie {
            api_key: Some("live_x".into()),
        };
        let err = update.apply_to(Some(&stripe_stored())).unwrap_err();
        assert!(matches!(
            err.current_context(),
            StoreError::ProviderMismatch { .. }
        ));
    }

    #[test]
    fn mollie_credentials_mint_a_fresh_token() {
        let a = MollieSensitiveData::new("test_k".into());
        let b = MollieSensitiveData::new("test_k".into());
        let (a, b) = (
            a.webhook_token.expose_secret(),
            b.webhook_token.expose_secret(),
        );
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
