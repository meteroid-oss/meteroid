use crate::repo::errors::RepoError;
use crate::repo::provider_config::model::{
    ApiSecurityDb, InvoicingProvider, RepoProviderConfig, WebhookSecurityDb,
};
use common_repository::Pool;
use cornucopia_async::Params;
use meteroid_repository as db;

use crate::api::services::utils::uuid_gen;
use error_stack::{Result, ResultExt};
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

use super::get_pool;

static PROVIDER_CONF_REPO_CORN: std::sync::OnceLock<ProviderConfigRepoCornucopia> =
    std::sync::OnceLock::new();

#[async_trait::async_trait]
pub trait ProviderConfigRepo: Send + Sync + core::fmt::Debug + 'static {
    async fn get_config_by_provider_and_tenant(
        &self,
        provider: InvoicingProvider,
        tenant_id: uuid::Uuid,
    ) -> Result<RepoProviderConfig, RepoError>;

    async fn create_provider_config(
        &self,
        provider: InvoicingProvider,
        tenant_id: uuid::Uuid,
        api_secret: secrecy::SecretString,
        webhook_secret: secrecy::SecretString,
    ) -> Result<RepoProviderConfig, RepoError>;
}

#[derive(Clone, Debug)]
pub struct ProviderConfigRepoCornucopia {
    pub pool: Pool,
    pub crypt_key: secrecy::SecretString,
}

impl ProviderConfigRepoCornucopia {
    pub fn get() -> &'static Self {
        PROVIDER_CONF_REPO_CORN.get_or_init(|| {
            let config = crate::config::Config::get();
            ProviderConfigRepoCornucopia {
                pool: get_pool().clone(),
                crypt_key: config.secrets_crypt_key.clone(),
            }
        })
    }
}

#[async_trait::async_trait]
impl ProviderConfigRepo for ProviderConfigRepoCornucopia {
    #[tracing::instrument(skip(self))]
    async fn get_config_by_provider_and_tenant(
        &self,
        provider: InvoicingProvider,
        tenant_id: uuid::Uuid,
    ) -> Result<RepoProviderConfig, RepoError> {
        let conn = self
            .pool
            .get()
            .await
            .change_context(RepoError::DatabaseError)?;

        let provider_config = db::provider_configs::get_config_by_provider_and_endpoint()
            .params(
                &conn,
                &db::provider_configs::GetConfigByProviderAndEndpointParams {
                    invoicing_provider: provider.into(),
                    tenant_id,
                },
            )
            .one()
            .await
            .change_context(RepoError::DatabaseError)?;

        mapping::from_db(provider_config, &self.crypt_key)
    }

    async fn create_provider_config(
        &self,
        provider: InvoicingProvider,
        tenant_id: Uuid,
        api_secret: SecretString,
        webhook_secret: SecretString,
    ) -> Result<RepoProviderConfig, RepoError> {
        let conn = self
            .pool
            .get()
            .await
            .change_context(RepoError::DatabaseError)?;

        let wh_security = WebhookSecurityDb {
            secret: webhook_secret.expose_secret().into(),
        }
        .encrypt_and_serialize(&self.crypt_key)?;

        let api_security = ApiSecurityDb {
            api_key: api_secret.expose_secret().into(),
        }
        .encrypt_and_serialize(&self.crypt_key)?;

        let params = db::provider_configs::CreateProviderConfigParams {
            id: uuid_gen::v7(),
            tenant_id,
            invoicing_provider: provider.into(),
            enabled: true,
            webhook_security: Some(wh_security),
            api_security: Some(api_security),
        };

        let provider_config = db::provider_configs::create_provider_config()
            .params(&conn, &params)
            .one()
            .await
            .change_context(RepoError::DatabaseError)?;

        mapping::from_db(provider_config, &self.crypt_key)
    }
}

pub mod model {
    use crate::repo::crypt;
    use crate::repo::errors::RepoError;
    use error_stack::{Result, ResultExt};
    use meteroid_repository::InvoicingProviderEnum;
    use secrecy::{ExposeSecret, SecretString};
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    pub struct RepoProviderConfig {
        pub id: uuid::Uuid,
        pub tenant_id: uuid::Uuid,
        pub invoicing_provider: InvoicingProvider,
        pub enabled: bool,
        pub webhook_secret: Option<secrecy::SecretString>,
        pub api_key: Option<secrecy::SecretString>,
    }

    #[derive(Clone, Debug)]
    pub enum InvoicingProvider {
        Stripe,
    }

    impl From<InvoicingProviderEnum> for InvoicingProvider {
        fn from(value: InvoicingProviderEnum) -> Self {
            match value {
                InvoicingProviderEnum::STRIPE => InvoicingProvider::Stripe,
            }
        }
    }

    impl From<InvoicingProvider> for InvoicingProviderEnum {
        fn from(val: InvoicingProvider) -> Self {
            match val {
                InvoicingProvider::Stripe => InvoicingProviderEnum::STRIPE,
            }
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct WebhookSecurityDb {
        pub secret: String,
    }

    impl WebhookSecurityDb {
        pub fn parse_and_decrypt(
            json: Value,
            crypt_key: &SecretString,
        ) -> Result<WebhookSecurityDb, RepoError> {
            let db_encrypted: WebhookSecurityDb =
                serde_json::from_value(json).change_context(RepoError::JsonFieldDecodingError)?;

            let decrypted_secret = crypt::decrypt(crypt_key, db_encrypted.secret.as_str())
                .change_context(RepoError::JsonFieldDecodingError)?;

            Ok(WebhookSecurityDb {
                secret: decrypted_secret.expose_secret().to_string(),
            })
        }

        pub fn encrypt_and_serialize(&self, crypt_key: &SecretString) -> Result<Value, RepoError> {
            let encrypted_secret = crypt::encrypt(crypt_key, self.secret.as_str())
                .change_context(RepoError::JsonFieldEncodingError)?;

            let encrypted = WebhookSecurityDb {
                secret: encrypted_secret.expose_secret().to_string(),
            };

            serde_json::to_value(encrypted).change_context(RepoError::JsonFieldEncodingError)
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct ApiSecurityDb {
        pub api_key: String,
    }

    impl ApiSecurityDb {
        pub fn parse_and_decrypt(
            json: Value,
            crypt_key: &SecretString,
        ) -> Result<ApiSecurityDb, RepoError> {
            let db_encrypted: ApiSecurityDb =
                serde_json::from_value(json).change_context(RepoError::JsonFieldDecodingError)?;

            let decrypted_secret = crypt::decrypt(crypt_key, db_encrypted.api_key.as_str())
                .change_context(RepoError::JsonFieldDecodingError)?;

            Ok(ApiSecurityDb {
                api_key: decrypted_secret.expose_secret().to_string(),
            })
        }

        pub fn encrypt_and_serialize(&self, crypt_key: &SecretString) -> Result<Value, RepoError> {
            let encrypted_secret = crypt::encrypt(crypt_key, self.api_key.as_str())
                .change_context(RepoError::JsonFieldEncodingError)?;

            let encrypted = ApiSecurityDb {
                api_key: encrypted_secret.expose_secret().to_string(),
            };

            serde_json::to_value(encrypted).change_context(RepoError::JsonFieldEncodingError)
        }
    }
}

pub mod mapping {
    use super::model::{ApiSecurityDb, RepoProviderConfig, WebhookSecurityDb};
    use crate::repo::errors::RepoError;
    use error_stack::Result;
    use meteroid_repository::provider_configs as db;

    pub fn from_db(
        db_config: db::ProviderConfig,
        crypt_key: &secrecy::SecretString,
    ) -> Result<RepoProviderConfig, RepoError> {
        let wh_security = db_config
            .webhook_security
            .map(|security| WebhookSecurityDb::parse_and_decrypt(security, crypt_key))
            .transpose()?;

        let api_security = db_config
            .api_security
            .map(|security| ApiSecurityDb::parse_and_decrypt(security, crypt_key))
            .transpose()?;

        Ok(RepoProviderConfig {
            id: db_config.id,
            tenant_id: db_config.tenant_id,
            invoicing_provider: db_config.invoicing_provider.into(),
            enabled: db_config.enabled,
            webhook_secret: wh_security.map(|security| security.secret.into()),
            api_key: api_security.map(|security| security.api_key.into()),
        })
    }
}
