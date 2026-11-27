//! Raw access to the columns sealed with the store crypt key, to re-encrypt their legacy values.

use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};

use diesel::{QueryableByName, sql_types};
use error_stack::ResultExt;

pub struct EncryptedColumn {
    pub table: &'static str,
    pub column: &'static str,
    id_type: &'static str,
}

const fn column(table: &'static str, column: &'static str) -> EncryptedColumn {
    EncryptedColumn {
        table,
        column,
        id_type: "uuid",
    }
}

pub const ENCRYPTED_COLUMNS: &[EncryptedColumn] = &[
    column("connector", "sensitive"),
    column("webhook_out_endpoint", "secret"),
    column("webhook_out_endpoint", "previous_secret"),
    column("webhook_out_endpoint", "sensitive_header_values"),
    column("webhook_out_retired_secret", "secret"),
    column("organization_sso_connection", "client_secret"),
    EncryptedColumn {
        table: "instance_license",
        column: "signing_key",
        id_type: "boolean",
    },
    EncryptedColumn {
        table: "instance_license",
        column: "refresh_token",
        id_type: "boolean",
    },
    column("issued_license", "activation_code_encrypted"),
    column("license_signing_key", "private_key_encrypted"),
    column("oauth_verifier", "pkce_verifier"),
    column("migration_source", "credentials"),
];

#[derive(QueryableByName, Debug)]
pub struct EncryptedValueRow {
    #[diesel(sql_type = sql_types::Text)]
    pub id: String,
    #[diesel(sql_type = sql_types::Text)]
    pub value: String,
}

impl EncryptedColumn {
    /// Values not yet in the random-nonce format, after the `after` id (as text).
    pub async fn list_legacy(
        &self,
        conn: &mut PgConn,
        after: Option<&str>,
        limit: i64,
    ) -> DbResult<Vec<EncryptedValueRow>> {
        use diesel_async::RunQueryDsl;

        let (table, column) = (self.table, self.column);
        diesel::sql_query(format!(
            r#"SELECT id::text AS id, "{column}" AS value FROM "{table}"
            WHERE "{column}" IS NOT NULL AND "{column}" NOT LIKE 'v2:%'
              AND ($1::text IS NULL OR id::text > $1)
            ORDER BY id::text LIMIT $2"#
        ))
        .bind::<sql_types::Nullable<sql_types::Text>, _>(after)
        .bind::<sql_types::BigInt, _>(limit)
        .get_results(conn)
        .await
        .attach("Error while listing legacy encrypted values")
        .into_db_result()
    }

    /// Replaces the value unless it changed since it was read. Returns whether it was replaced.
    pub async fn replace(
        &self,
        conn: &mut PgConn,
        id: &str,
        old: &str,
        new: &str,
    ) -> DbResult<bool> {
        use diesel_async::RunQueryDsl;

        let (table, column, id_type) = (self.table, self.column, self.id_type);
        diesel::sql_query(format!(
            r#"UPDATE "{table}" SET "{column}" = $3 WHERE id = $1::{id_type} AND "{column}" = $2"#
        ))
        .bind::<sql_types::Text, _>(id)
        .bind::<sql_types::Text, _>(old)
        .bind::<sql_types::Text, _>(new)
        .execute(conn)
        .await
        .map(|updated| updated == 1)
        .attach("Error while replacing an encrypted value")
        .into_db_result()
    }
}
