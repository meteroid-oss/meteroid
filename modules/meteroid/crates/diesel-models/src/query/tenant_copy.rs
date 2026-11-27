use std::collections::HashMap;

use common_domain::ids::{BaseId, TenantId, join_base62, split_base62};
use diesel::sql_types::{Array, Text, Uuid as SqlUuid};
use diesel::{QueryableByName, sql_query};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

use crate::errors::{DatabaseError, DatabaseErrorContainer, IntoDbResult};
use crate::{DbResult, PgConn};

/// One table copied from a tenant into another. `source` filters the rows to copy (alias `s`,
/// `$1` the source tenant). Every uuid column must be declared, so a new reference fails
/// loudly instead of pointing the copy at the source tenant's rows.
pub struct CopiedTable {
    pub table: &'static str,
    pub source: &'static str,
    /// References to rows copied in the same run.
    pub mapped: &'static [&'static str],
    /// Arrays of such references, in order.
    pub mapped_arrays: &'static [&'static str],
    /// References kept when they point at a built-in row (`tenant_id IS NULL`) of that table.
    pub builtin: &'static [(&'static str, &'static str)],
    /// References to rows that are not copied.
    pub cleared: &'static [&'static str],
    /// References to rows every tenant of the organization shares, such as users.
    pub kept: &'static [&'static str],
    /// References that would close a cycle, filled in once every table is copied.
    pub deferred: &'static [&'static str],
    /// Keys grouping rows (not rows of a table): each value gets a new one, alike in the copy.
    pub regenerated: &'static [&'static str],
    pub set: &'static [(&'static str, &'static str)],
}

impl CopiedTable {
    /// A table whose uuid columns, other than `id` and `tenant_id`, are still to be declared.
    pub const fn new(table: &'static str, source: &'static str) -> Self {
        CopiedTable {
            table,
            source,
            mapped: &[],
            mapped_arrays: &[],
            builtin: &[],
            cleared: &[],
            kept: &[],
            deferred: &[],
            regenerated: &[],
            set: &[],
        }
    }
}

/// The rows of the source tenant, for tables with a `tenant_id`.
pub const OF_TENANT: &str = "s.tenant_id = $1";

#[derive(QueryableByName)]
struct Column {
    #[diesel(sql_type = Text)]
    column_name: String,
    #[diesel(sql_type = Text)]
    data_type: String,
}

#[derive(QueryableByName)]
struct Mapping {
    #[diesel(sql_type = SqlUuid)]
    old: Uuid,
    #[diesel(sql_type = SqlUuid)]
    new: Uuid,
}

#[derive(QueryableByName)]
struct JsonCell {
    #[diesel(sql_type = Text)]
    tid: String,
    #[diesel(sql_type = Text)]
    value: String,
}

#[derive(QueryableByName)]
struct Id {
    #[diesel(sql_type = SqlUuid)]
    id: Uuid,
}

const MAP: &str = "tenant_copy_map";

fn mapped(expr: &str) -> String {
    format!("(SELECT m.new FROM {MAP} m WHERE m.old = {expr})")
}

fn invalid(message: String) -> DatabaseErrorContainer {
    DatabaseError::Others(message).into()
}

/// Copies `tables`, in order, inside the caller's transaction. Each row gets a fresh id, and
/// ids inside JSONB columns are rewritten to the copies. An id found in two tables maps to
/// the same new id in both, which keeps every reference to either row right. Returns the rows
/// copied per table.
pub async fn copy_tenant_rows(
    conn: &mut PgConn,
    source: TenantId,
    target: TenantId,
    tables: &[&CopiedTable],
) -> DbResult<Vec<(&'static str, usize)>> {
    sql_query(format!(
        "CREATE TEMP TABLE {MAP} (old uuid PRIMARY KEY, new uuid NOT NULL) ON COMMIT DROP"
    ))
    .execute(conn)
    .await
    .attach("Error while preparing the tenant copy")
    .into_db_result()?;
    sql_query(format!(
        "CREATE OR REPLACE FUNCTION pg_temp.tenant_copy_remap(j jsonb) RETURNS jsonb AS $$
         DECLARE t text := j::text; r record;
         BEGIN
           IF j IS NULL THEN RETURN NULL; END IF;
           FOR r IN SELECT DISTINCT m.old, m.new FROM {MAP} m
             JOIN regexp_matches(t, '[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}', 'g') u
             ON m.old = u[1]::uuid
           LOOP
             t := replace(t, r.old::text, r.new::text);
           END LOOP;
           RETURN t::jsonb;
         END $$ LANGUAGE plpgsql"
    ))
    .execute(conn)
    .await
    .attach("Error while preparing the tenant copy")
    .into_db_result()?;

    let mut columns = Vec::with_capacity(tables.len());
    for table in tables {
        let cols: Vec<Column> = sql_query(
            "SELECT column_name::text, udt_name::text AS data_type FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = $1 \
               AND is_generated = 'NEVER' ORDER BY ordinal_position",
        )
        .bind::<Text, _>(table.table)
        .load(conn)
        .await
        .attach("Error while reading the copied columns")
        .into_db_result()?;
        let keyed = cols.iter().any(|c| c.column_name == "id");
        for column in keyed
            .then_some("id")
            .into_iter()
            .chain(table.regenerated.iter().copied())
        {
            let ids: Vec<Id> = sql_query(format!(
                "SELECT DISTINCT s.\"{column}\" AS id FROM {} s WHERE {} AND s.\"{column}\" IS NOT NULL ORDER BY 1",
                table.table, table.source
            ))
            .bind::<SqlUuid, _>(source.as_uuid())
            .load(conn)
            .await
            .attach("Error while listing the copied rows")
            .into_db_result()?;
            let old: Vec<Uuid> = ids.into_iter().map(|i| i.id).collect();
            let new: Vec<Uuid> = old.iter().map(|_| Uuid::now_v7()).collect();
            sql_query(format!(
                "INSERT INTO {MAP} SELECT * FROM unnest($1, $2) ON CONFLICT (old) DO NOTHING"
            ))
            .bind::<Array<SqlUuid>, _>(old)
            .bind::<Array<SqlUuid>, _>(new)
            .execute(conn)
            .await
            .attach("Error while mapping the copied rows")
            .into_db_result()?;
        }
        columns.push(cols);
    }

    let mut copied = Vec::with_capacity(tables.len());
    for (table, cols) in tables.iter().zip(&columns) {
        let mut names = Vec::with_capacity(cols.len());
        let mut values = Vec::with_capacity(cols.len());
        for Column {
            column_name: name,
            data_type,
        } in cols
        {
            let n = name.as_str();
            let source_col = format!("s.\"{n}\"");
            let value = if let Some((_, expr)) = table.set.iter().find(|(c, _)| *c == n) {
                (*expr).to_string()
            } else if n == "tenant_id" {
                "$2".to_string()
            } else if n == "id" || table.mapped.contains(&n) || table.regenerated.contains(&n) {
                mapped(&source_col)
            } else if let Some((_, of)) = table.builtin.iter().find(|(c, _)| *c == n) {
                format!(
                    "COALESCE({}, (SELECT b.id FROM {of} b WHERE b.id = {source_col} AND b.tenant_id IS NULL))",
                    mapped(&source_col)
                )
            } else if table.mapped_arrays.contains(&n) {
                format!(
                    "CASE WHEN {source_col} IS NULL THEN NULL ELSE ARRAY(SELECT m.new FROM \
                     unnest({source_col}) WITH ORDINALITY u(old, i) JOIN {MAP} m ON m.old = u.old \
                     ORDER BY u.i) END"
                )
            } else if table.kept.contains(&n) {
                source_col
            } else if table.cleared.contains(&n) || table.deferred.contains(&n) {
                "NULL".to_string()
            } else if data_type == "uuid" || data_type == "_uuid" {
                return Err(invalid(format!(
                    "{}.{n} references rows the tenant copy does not know about",
                    table.table
                )));
            } else if data_type == "jsonb" {
                format!("pg_temp.tenant_copy_remap({source_col})")
            } else {
                source_col
            };
            names.push(format!("\"{n}\""));
            values.push(value);
        }
        let rows = sql_query(format!(
            "INSERT INTO {} ({}) SELECT {} FROM {} s WHERE {}{}",
            table.table,
            names.join(", "),
            values.join(", "),
            table.table,
            table.source,
            // The rows mapped above, not ones written to the source since.
            if cols.iter().any(|c| c.column_name == "id") {
                format!(" AND s.id IN (SELECT old FROM {MAP})")
            } else {
                String::new()
            }
        ))
        .bind::<SqlUuid, _>(source.as_uuid())
        .bind::<SqlUuid, _>(target.as_uuid())
        .execute(conn)
        .await
        .attach_with(|| format!("Error while copying {}", table.table))
        .into_db_result()?;
        copied.push((table.table, rows));
    }

    for table in tables {
        for column in table.deferred {
            sql_query(format!(
                "UPDATE {t} c SET \"{column}\" = {value} FROM {t} s \
                 WHERE {source} AND c.id = {id} AND s.\"{column}\" IS NOT NULL",
                t = table.table,
                source = table.source,
                value = mapped(&format!("s.\"{column}\"")),
                id = mapped("s.id"),
            ))
            .bind::<SqlUuid, _>(source.as_uuid())
            .execute(conn)
            .await
            .attach_with(|| format!("Error while linking {}.{column}", table.table))
            .into_db_result()?;
        }
    }

    rewrite_base62_ids(conn, target, tables, &columns).await?;
    Ok(copied)
}

/// Payloads written by the web app hold ids as `prefix_<base62>`, which the SQL remap can't
/// see: rewritten here, for every id that has a copy.
async fn rewrite_base62_ids(
    conn: &mut PgConn,
    target: TenantId,
    tables: &[&CopiedTable],
    columns: &[Vec<Column>],
) -> DbResult<()> {
    let map: HashMap<Uuid, Uuid> = sql_query(format!("SELECT old, new FROM {MAP}"))
        .load::<Mapping>(conn)
        .await
        .attach("Error while reading the copied ids")
        .into_db_result()?
        .into_iter()
        .map(|m| (m.old, m.new))
        .collect();
    for (table, cols) in tables.iter().zip(columns) {
        let has = |name: &str| cols.iter().any(|c| c.column_name == name);
        let filter = if has("tenant_id") {
            "tenant_id = $1".to_string()
        } else if has("id") {
            format!("id IN (SELECT new FROM {MAP})")
        } else {
            continue;
        };
        for col in cols.iter().filter(|c| c.data_type == "jsonb") {
            let cells: Vec<JsonCell> = sql_query(format!(
                "SELECT ctid::text AS tid, \"{c}\"::text AS value FROM {t} \
                 WHERE {filter} AND \"{c}\" IS NOT NULL",
                c = col.column_name,
                t = table.table,
            ))
            .bind::<SqlUuid, _>(target.as_uuid())
            .load(conn)
            .await
            .attach_with(|| format!("Error while reading {}.{}", table.table, col.column_name))
            .into_db_result()?;
            for cell in cells {
                let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&cell.value) else {
                    continue;
                };
                if !remap_base62(&mut value, &map) {
                    continue;
                }
                sql_query(format!(
                    "UPDATE {t} SET \"{c}\" = $1::jsonb WHERE ctid = $2::tid",
                    c = col.column_name,
                    t = table.table,
                ))
                .bind::<Text, _>(value.to_string())
                .bind::<Text, _>(cell.tid)
                .execute(conn)
                .await
                .attach_with(|| {
                    format!("Error while rewriting {}.{}", table.table, col.column_name)
                })
                .into_db_result()?;
            }
        }
    }
    Ok(())
}

fn remap_id(s: &str, map: &HashMap<Uuid, Uuid>) -> Option<String> {
    let (prefix, id) = split_base62(s)?;
    map.get(&id).map(|new| join_base62(prefix, *new))
}

/// Whether anything changed.
fn remap_base62(value: &mut serde_json::Value, map: &HashMap<Uuid, Uuid>) -> bool {
    use serde_json::Value;
    match value {
        Value::String(s) => match remap_id(s, map) {
            Some(new) => {
                *s = new;
                true
            }
            None => false,
        },
        Value::Array(items) => items
            .iter_mut()
            .fold(false, |changed, item| remap_base62(item, map) | changed),
        Value::Object(entries) => {
            let renamed: Vec<(String, String)> = entries
                .keys()
                .filter_map(|k| remap_id(k, map).map(|new| (k.clone(), new)))
                .collect();
            let mut changed = !renamed.is_empty();
            for (old, new) in renamed {
                if let Some(v) = entries.remove(&old) {
                    entries.insert(new, v);
                }
            }
            for v in entries.values_mut() {
                changed |= remap_base62(v, map);
            }
            changed
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::PlanVersionId;

    #[test]
    fn base62_ids_with_a_copy_are_rewritten_wherever_they_are() {
        let (old, new, other) = (
            PlanVersionId::new(),
            PlanVersionId::new(),
            PlanVersionId::new(),
        );
        let map = HashMap::from([(old.as_uuid(), new.as_uuid())]);
        let mut value = serde_json::json!({
            "plan_version_id": old.as_base62(),
            "keep": other.as_base62(),
            (old.as_base62()): [old.as_base62(), "plain text"],
        });
        assert!(remap_base62(&mut value, &map));
        assert_eq!(
            value,
            serde_json::json!({
                "plan_version_id": new.as_base62(),
                "keep": other.as_base62(),
                (new.as_base62()): [new.as_base62(), "plain text"],
            })
        );
        assert!(!remap_base62(&mut serde_json::json!({ "a": "b" }), &map));
    }
}
