use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use common_domain::ids::{AliasOr, BaseId, CustomerId, PlanId, PlanVersionId, TenantId};
use csv::ReaderBuilder;
use meteroid_store::Services;
use meteroid_store::domain::CreateSubscription;
use meteroid_store::domain::batch_jobs::{BatchJob, BatchJobChunk};
use meteroid_store::repositories::{CustomersInterface, PlansInterface};
use uuid::Uuid;

use crate::services::csv_ingest::normalize_csv_encoding;
use crate::services::idempotency::IdempotencyService;
use crate::services::subscription_ingest::NewSubscriptionCsv;
use crate::workers::batch_jobs::engine::{
    BatchJobProcessor, ChunkDefinition, ChunkResult, CreatedEntity, ItemFailure,
};

use super::event_csv::parse_input_params;

const CHUNK_SIZE: i32 = 50;
const MAX_ROW_COUNT: usize = 5_000;
const IDEMPOTENCY_WINDOW: Duration = Duration::from_secs(24 * 3600);

const REQUIRED_HEADERS: &[&str] = &[
    "customer_id_or_alias",
    "plan_id",
    "start_date",
    "activation_condition",
    "auto_advance_invoices",
    "charge_automatically",
    "skip_past_invoices",
];

pub struct SubscriptionCsvProcessor {
    services: Services,
    idempotency: Arc<dyn IdempotencyService>,
}

impl SubscriptionCsvProcessor {
    pub fn new(services: Services, idempotency: Arc<dyn IdempotencyService>) -> Self {
        Self {
            services,
            idempotency,
        }
    }
}

fn build_csv_reader(data: &[u8], delimiter: u8) -> csv::Reader<&[u8]> {
    ReaderBuilder::new()
        .has_headers(true)
        .delimiter(delimiter)
        .from_reader(data)
}

fn validate_headers(headers: &csv::StringRecord) -> Result<(), String> {
    let header_set: std::collections::HashSet<&str> = headers.iter().collect();
    let missing: Vec<&str> = REQUIRED_HEADERS
        .iter()
        .filter(|h| !header_set.contains(**h))
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "Missing required CSV columns: {}",
            missing.join(", ")
        ));
    }
    Ok(())
}

struct ParsedRow {
    row_index: i32,
    csv: NewSubscriptionCsv,
}

struct CustomerValidatedRow {
    row_index: i32,
    csv: NewSubscriptionCsv,
    customer_id: CustomerId,
}

struct ValidatedRow {
    row_index: i32,
    csv: NewSubscriptionCsv,
    customer_id: CustomerId,
    plan_version_id: PlanVersionId,
}

/// Resolves `customer_id_or_alias` for each row, returning validated rows with a resolved
/// `CustomerId` and failures for any rows whose customer could not be found.
async fn validate_customers(
    store: &meteroid_store::Store,
    tenant_id: TenantId,
    rows: Vec<ParsedRow>,
) -> Result<(Vec<CustomerValidatedRow>, Vec<ItemFailure>), String> {
    let mut id_rows: Vec<CustomerValidatedRow> = Vec::new();
    let mut alias_rows: Vec<(i32, NewSubscriptionCsv, String)> = Vec::new();

    for ParsedRow { row_index, csv } in rows {
        match csv.customer_id_or_alias.clone() {
            AliasOr::Id(id) => id_rows.push(CustomerValidatedRow {
                row_index,
                csv,
                customer_id: id,
            }),
            AliasOr::Alias(alias) => alias_rows.push((row_index, csv, alias)),
        }
    }

    let id_set: Vec<CustomerId> = id_rows.iter().map(|r| r.customer_id).collect();
    let found_ids: std::collections::HashSet<CustomerId> = store
        .list_customers_by_ids(tenant_id, id_set)
        .await
        .map_err(|e| format!("Failed to validate customer IDs: {e}"))?
        .into_iter()
        .map(|c| c.id)
        .collect();

    let aliases: Vec<String> = alias_rows.iter().map(|(_, _, a)| a.clone()).collect();
    let alias_map: HashMap<String, CustomerId> = if aliases.is_empty() {
        HashMap::new()
    } else {
        store
            .find_customer_ids_by_aliases(tenant_id, aliases)
            .await
            .map_err(|e| format!("Failed to resolve customer aliases: {e}"))?
            .into_iter()
            .filter_map(|c| c.alias.map(|a| (a, c.id)))
            .collect()
    };

    let mut valid = Vec::new();
    let mut failures = Vec::new();

    for row in id_rows {
        if found_ids.contains(&row.customer_id) {
            valid.push(row);
        } else {
            failures.push(ItemFailure {
                item_index: row.row_index,
                item_identifier: None,
                reason: format!("Customer '{}' not found", row.customer_id),
            });
        }
    }

    for (row_index, csv, alias) in alias_rows {
        if let Some(customer_id) = alias_map.get(&alias).copied() {
            valid.push(CustomerValidatedRow {
                row_index,
                csv,
                customer_id,
            });
        } else {
            failures.push(ItemFailure {
                item_index: row_index,
                item_identifier: None,
                reason: format!("Customer with alias '{}' not found", alias),
            });
        }
    }

    Ok((valid, failures))
}

/// Resolves `plan_version_id` for each row using a single batched DB call.
async fn validate_plan_versions(
    store: &meteroid_store::Store,
    tenant_id: TenantId,
    rows: Vec<CustomerValidatedRow>,
) -> Result<(Vec<ValidatedRow>, Vec<ItemFailure>), String> {
    let plan_ids: Vec<PlanId> = rows
        .iter()
        .map(|r| r.csv.plan_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let mut version_map: HashMap<PlanId, std::collections::BTreeMap<i32, PlanVersionId>> =
        HashMap::new();
    for pv in store
        .list_published_versions_by_plan_ids(plan_ids, tenant_id)
        .await
        .map_err(|e| format!("Failed to fetch plan versions: {e}"))?
    {
        version_map
            .entry(pv.plan_id)
            .or_default()
            .insert(pv.version, pv.id);
    }

    let mut valid = Vec::new();
    let mut failures = Vec::new();

    for CustomerValidatedRow {
        row_index,
        csv,
        customer_id,
    } in rows
    {
        let versions = version_map.get(&csv.plan_id);
        let resolved = match csv.plan_version {
            Some(v) => versions.and_then(|m| m.get(&(v as i32))).copied(),
            None => versions.and_then(|m| m.values().next_back()).copied(),
        };

        match resolved {
            Some(plan_version_id) => valid.push(ValidatedRow {
                row_index,
                plan_version_id,
                csv,
                customer_id,
            }),
            None => {
                let reason = match csv.plan_version {
                    Some(v) => format!(
                        "No published version {} found for plan '{}'",
                        v, csv.plan_id
                    ),
                    None => format!("No published version found for plan '{}'", csv.plan_id),
                };
                failures.push(ItemFailure {
                    item_index: row_index,
                    item_identifier: None,
                    reason,
                });
            }
        }
    }

    Ok((valid, failures))
}

fn map_to_domain(
    actor: Uuid,
    csv: NewSubscriptionCsv,
    customer_id: CustomerId,
    plan_version_id: PlanVersionId,
) -> CreateSubscription {
    use meteroid_store::domain::SubscriptionNew;

    CreateSubscription {
        subscription: SubscriptionNew {
            customer_id,
            plan_version_id,
            created_by: actor,
            net_terms: csv.net_terms,
            invoice_memo: None,
            invoice_threshold: None,
            start_date: csv.start_date,
            end_date: csv.end_date,
            billing_start_date: None,
            activation_condition: csv.activation_condition.into(),
            trial_duration: None,
            billing_day_anchor: csv.billing_day_anchor,
            payment_methods_config: csv.payment_method.map(Into::into),
            auto_advance_invoices: csv.auto_advance_invoices,
            charge_automatically: csv.charge_automatically,
            purchase_order: csv.purchase_order.map(|s| s.0),
            backdate_invoices: false,
            skip_checkout_session: false,
            skip_past_invoices: csv.skip_past_invoices,
        },
        price_components: None,
        add_ons: None,
        coupons: None,
    }
}

#[async_trait]
impl BatchJobProcessor for SubscriptionCsvProcessor {
    async fn prepare_chunks(
        &self,
        job: &BatchJob,
        input_data: Option<Bytes>,
    ) -> Result<Vec<ChunkDefinition>, String> {
        let data = input_data.ok_or("No input data provided for CSV job")?;
        let params = parse_input_params(job)?;
        let normalized = normalize_csv_encoding(&data);
        let delimiter = params.delimiter as u8;

        let mut reader = build_csv_reader(&normalized, delimiter);

        let headers = reader
            .headers()
            .map_err(|e| format!("Failed to read CSV headers: {e}"))?
            .clone();

        validate_headers(&headers)?;

        let row_count = reader.records().count();
        if row_count == 0 {
            return Err("CSV file contains no data rows".to_string());
        }
        if row_count > MAX_ROW_COUNT {
            return Err(format!(
                "Row count ({row_count}) exceeds maximum allowed ({MAX_ROW_COUNT})"
            ));
        }

        let row_count = row_count as i32;
        let mut chunks = Vec::new();
        let mut offset = 0;
        while offset < row_count {
            let count = (row_count - offset).min(CHUNK_SIZE);
            chunks.push(ChunkDefinition {
                item_offset: offset,
                item_count: count,
            });
            offset += count;
        }

        Ok(chunks)
    }

    async fn process_chunk(
        &self,
        job: &BatchJob,
        chunk: &BatchJobChunk,
        input_data: Option<Bytes>,
    ) -> Result<ChunkResult, String> {
        let data = input_data.ok_or("No input data provided for CSV chunk")?;
        let params = parse_input_params(job)?;
        let normalized = normalize_csv_encoding(&data);
        let delimiter = params.delimiter as u8;

        let mut reader = build_csv_reader(&normalized, delimiter);

        let _headers = reader
            .headers()
            .map_err(|e| format!("Failed to read CSV headers: {e}"))?
            .clone();

        let offset = chunk.item_offset as usize;
        let count = chunk.item_count as usize;

        // Stage 1: Parse CSV rows for this chunk
        let mut parsed_rows = Vec::with_capacity(count);
        let mut failures: Vec<ItemFailure> = Vec::new();

        for (local_idx, result) in reader
            .deserialize::<NewSubscriptionCsv>()
            .enumerate()
            .skip(offset)
            .take(count)
        {
            let row_index = local_idx as i32;
            match result {
                Ok(csv) => parsed_rows.push(ParsedRow { row_index, csv }),
                Err(e) => {
                    failures.push(ItemFailure {
                        item_index: row_index,
                        item_identifier: None,
                        reason: format!("Failed to parse row: {e}"),
                    });
                }
            }
        }

        if params.fail_on_error && !failures.is_empty() {
            return Ok(ChunkResult {
                processed: 0,
                failures,
                created_entities: vec![],
            });
        }

        if parsed_rows.is_empty() {
            return Ok(ChunkResult {
                processed: 0,
                failures,
                created_entities: vec![],
            });
        }

        let store = self.services.store();

        // Stage 2: Resolve customer IDs
        let (customer_validated, customer_failures) =
            validate_customers(store, job.tenant_id, parsed_rows).await?;
        failures.extend(customer_failures);

        if params.fail_on_error && !failures.is_empty() {
            return Ok(ChunkResult {
                processed: 0,
                failures,
                created_entities: vec![],
            });
        }

        // Stage 3: Resolve plan versions
        let (validated, plan_failures) =
            validate_plan_versions(store, job.tenant_id, customer_validated).await?;
        failures.extend(plan_failures);

        if params.fail_on_error && !failures.is_empty() {
            return Ok(ChunkResult {
                processed: 0,
                failures,
                created_entities: vec![],
            });
        }

        // Stage 4: Check idempotency and map to domain types
        let mut to_insert: Vec<CreateSubscription> = Vec::new();
        let mut insert_row_indices: Vec<i32> = Vec::new();
        let mut insert_idempotency_keys: Vec<Option<String>> = Vec::new();
        let mut skipped_rows = 0i32;

        for row in validated {
            let idempotency_key = row
                .csv
                .idempotency_key
                .as_ref()
                .map(|k| format!("{}:{}", job.tenant_id, k));

            if let Some(ref key) = idempotency_key
                && self
                    .idempotency
                    .check_and_set(key.clone(), IDEMPOTENCY_WINDOW)
                    .await
            {
                skipped_rows += 1;
                continue;
            }

            insert_row_indices.push(row.row_index);
            insert_idempotency_keys.push(idempotency_key);
            to_insert.push(map_to_domain(
                job.created_by,
                row.csv,
                row.customer_id,
                row.plan_version_id,
            ));
        }

        // Stage 5: Insert subscriptions
        let mut successful_rows = skipped_rows;
        let mut created_entities: Vec<CreatedEntity> = Vec::new();

        if !to_insert.is_empty() {
            let mut conn = self
                .services
                .store()
                .get_conn()
                .await
                .map_err(|e| format!("Failed to get DB connection: {e}"))?;

            match self
                .services
                .insert_subscription_batch_tx(&mut conn, to_insert.clone(), job.tenant_id)
                .await
            {
                Ok(results) => {
                    successful_rows += results.len() as i32;
                    created_entities.extend(results.iter().map(|s| CreatedEntity {
                        entity_type: "subscription",
                        entity_id: s.id.as_uuid(),
                    }));
                }
                Err(e) if params.fail_on_error => {
                    for key in insert_idempotency_keys.into_iter().flatten() {
                        self.idempotency.invalidate(key).await;
                    }
                    let reason = e.to_string();
                    for row_index in insert_row_indices {
                        failures.push(ItemFailure {
                            item_index: row_index,
                            item_identifier: None,
                            reason: reason.clone(),
                        });
                    }
                }
                Err(_) => {
                    // Batch failed in continue mode: retry individually for per-row errors
                    drop(conn);
                    let mut retry_conn = self
                        .services
                        .store()
                        .get_conn()
                        .await
                        .map_err(|e| format!("Failed to get DB connection for retry: {e}"))?;

                    for (i, sub) in to_insert.into_iter().enumerate() {
                        match self
                            .services
                            .insert_subscription_batch_tx(&mut retry_conn, vec![sub], job.tenant_id)
                            .await
                        {
                            Ok(results) => {
                                successful_rows += 1;
                                created_entities.extend(results.iter().map(|s| CreatedEntity {
                                    entity_type: "subscription",
                                    entity_id: s.id.as_uuid(),
                                }));
                            }
                            Err(e) => {
                                if let Some(key) = insert_idempotency_keys[i].as_ref() {
                                    self.idempotency.invalidate(key.clone()).await;
                                }
                                failures.push(ItemFailure {
                                    item_index: insert_row_indices[i],
                                    item_identifier: None,
                                    reason: e.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }

        Ok(ChunkResult {
            processed: successful_rows,
            failures,
            created_entities,
        })
    }

    fn max_retries(&self) -> i32 {
        3
    }

    fn chunk_size(&self) -> i32 {
        CHUNK_SIZE
    }
}
