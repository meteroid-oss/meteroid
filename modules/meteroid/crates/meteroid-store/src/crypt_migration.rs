use crate::StoreResult;
use crate::crypt::{ENVELOPE_V1_PREFIX, decrypt, encrypt, is_legacy_envelope};
use crate::errors::StoreError;
use crate::store::Store;
use diesel_models::connectors::ConnectorRow;
use diesel_models::oauth_verifiers::OauthVerifierRow;
use error_stack::{Report, ResultExt};
use scoped_futures::ScopedFutureExt;
use secrecy::ExposeSecret;

/// Row counts rewritten by one migration run, for the operator-facing startup log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigratedSecrets {
    pub connectors: usize,
    pub oauth_verifiers: usize,
}

impl MigratedSecrets {
    pub fn is_empty(&self) -> bool {
        self.connectors == 0 && self.oauth_verifiers == 0
    }
}

/// Rewrites every stored secret still using the key-derived nonce into a versioned envelope
/// carrying its own random nonce.
///
/// Runs as one transaction with the affected rows locked, and fails closed: a value that will
/// not authenticate aborts the run rather than leaving a half-migrated table. Idempotent, so a
/// restart after a successful run rewrites nothing.
///
/// Re-encryption stops future nonce reuse but cannot un-expose secrets already readable in an
/// older database snapshot. Operators must still rotate the provider credentials afterwards.
pub async fn migrate_legacy_secrets(store: &Store) -> StoreResult<MigratedSecrets> {
    let crypt_key = store.settings.crypt_key.clone();

    store
        .transaction(|conn| {
            let crypt_key = crypt_key.clone();
            async move {
                let mut migrated = MigratedSecrets::default();

                for (id, legacy) in ConnectorRow::lock_legacy_sensitive(conn, ENVELOPE_V1_PREFIX)
                    .await?
                    .into_iter()
                    .filter_map(|(id, sensitive)| sensitive.map(|s| (id, s)))
                {
                    let rewritten = reencrypt(&crypt_key, &legacy)
                        .map_err(|err| err.attach(format!("connector {id}")))?;

                    ConnectorRow::set_sensitive(conn, id, &rewritten).await?;
                    migrated.connectors += 1;
                }

                for (id, legacy) in
                    OauthVerifierRow::lock_legacy_pkce_verifiers(conn, ENVELOPE_V1_PREFIX).await?
                {
                    let rewritten = reencrypt(&crypt_key, &legacy)
                        .map_err(|err| err.attach(format!("oauth verifier {id}")))?;

                    OauthVerifierRow::set_pkce_verifier(conn, id, &rewritten).await?;
                    migrated.oauth_verifiers += 1;
                }

                Ok(migrated)
            }
            .scope_boxed()
        })
        .await
}

/// Never logs or attaches the value itself: these are live provider credentials.
fn reencrypt(crypt_key: &secrecy::SecretString, legacy: &str) -> StoreResult<String> {
    if !is_legacy_envelope(legacy) {
        // The SQL filter already excluded these; reaching here means the filter and the envelope
        // format disagree, which would silently double-encrypt.
        return Err(Report::new(StoreError::CryptError(
            "value selected for migration is already versioned".into(),
        )));
    }

    let plaintext = decrypt(crypt_key, legacy).change_context(StoreError::CryptError(
        "legacy secret decryption error".into(),
    ))?;

    encrypt(crypt_key, plaintext.expose_secret())
        .change_context(StoreError::CryptError("secret re-encryption error".into()))
}
