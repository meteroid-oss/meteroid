use crate::errors::{DatabaseError, IntoDbResult};
use crate::{DbResult, PgConn};
use common_domain::ids::ConnectorId;
use diesel::sql_types::{Text, Uuid};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

/// not ideal but allows to reuse the same code for different tables
pub async fn upsert(
    conn: &mut PgConn,
    table_name: &str,
    integration_name: &str,
    record_id: uuid::Uuid,
    connector_id: ConnectorId,
    external_id: &str,
) -> DbResult<usize> {
    let sync_at = serde_json::to_string(&chrono::Utc::now())
        .map_err(|_| DatabaseError::Others("sync_at serialization failure".into()))?
        .trim_matches('"')
        .to_string();

    let raw_query = format!(
        r#"
    UPDATE {table_name}
    SET conn_meta = jsonb_set(
        COALESCE(conn_meta, jsonb_build_object('{integration_key}', '[]'::jsonb)),
        '{{{integration_key}}}',  -- JSON path literal
        (
            SELECT jsonb_agg(elem)
            FROM (
                SELECT
                    CASE
                        WHEN elem->>'connector_id' = $1::text THEN jsonb_build_object(
                            'connector_id', $1,
                            'external_id', $2,
                            'sync_at', $3
                        )
                        ELSE elem
                    END AS elem
                FROM jsonb_array_elements(conn_meta->'{integration_key}') elem

                UNION ALL

                SELECT jsonb_build_object(
                    'connector_id', $1,
                    'external_id', $2,
                    'sync_at', $3
                )
                WHERE NOT EXISTS (
                    SELECT 1
                    FROM jsonb_array_elements(conn_meta->'{integration_key}') e
                    WHERE e->>'connector_id' = $1::text
                )
            ) sub
        )
    )
    WHERE id = $4;
    "#,
        table_name = table_name,
        integration_key = integration_name,
    );
    diesel::sql_query(raw_query)
        .bind::<Uuid, _>(connector_id)
        .bind::<Text, _>(external_id)
        .bind::<Text, _>(sync_at)
        .bind::<Uuid, _>(record_id)
        .execute(conn)
        .await
        .attach_printable("Error while upserting connection metadata")
        .into_db_result()
}
