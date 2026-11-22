use crate::errors::StoreError;
use crate::{Store, StoreResult, crypt};
use diesel_models::query::encrypted_columns::ENCRYPTED_COLUMNS;
use error_stack::{Report, ResultExt};
use secrecy::ExposeSecret;

const BATCH_SIZE: i64 = 200;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReencryptionReport {
    pub reencrypted: usize,
    /// Values that do not decrypt with the current key; left untouched.
    pub undecryptable: usize,
    /// Values rewritten by the app between the read and the update; already re-encrypted.
    pub changed_meanwhile: usize,
}

impl Store {
    /// Re-encrypts the values still sealed under the legacy key-derived nonce. Idempotent: a value
    /// is only replaced if it decrypts and has not changed since it was read.
    pub async fn reencrypt_legacy_secrets(&self) -> StoreResult<ReencryptionReport> {
        let key = &self.settings.crypt_key;
        let mut report = ReencryptionReport::default();
        let mut conn = self.get_conn().await?;

        for column in ENCRYPTED_COLUMNS {
            let mut after: Option<String> = None;
            loop {
                let rows = column
                    .list_legacy(&mut conn, after.as_deref(), BATCH_SIZE)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let Some(last) = rows.last() else { break };
                after = Some(last.id.clone());

                for row in rows {
                    let Ok(plain) = crypt::decrypt(key, &row.value) else {
                        log::warn!(
                            "{}.{} of {} does not decrypt with the current key, left as is",
                            column.table,
                            column.column,
                            row.id
                        );
                        report.undecryptable += 1;
                        continue;
                    };
                    let sealed = crypt::encrypt(key, plain.expose_secret())
                        .change_context(StoreError::CryptError("re-encryption".into()))?;
                    let replaced = column
                        .replace(&mut conn, &row.id, &row.value, &sealed)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if replaced {
                        report.reencrypted += 1;
                    } else {
                        report.changed_meanwhile += 1;
                    }
                }
            }
        }

        Ok(report)
    }
}
