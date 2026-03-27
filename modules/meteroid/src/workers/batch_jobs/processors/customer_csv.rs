use std::collections::HashMap;
use std::sync::Arc;

use crate::services::csv_ingest::{CsvString, normalize_csv_encoding};
use crate::services::customer_ingest::{CustomTaxRatesCsv, NewCustomerCsv};
use crate::workers::batch_jobs::engine::{
    BatchJobProcessor, ChunkDefinition, ChunkResult, CreatedEntity, ItemFailure,
};
use async_trait::async_trait;
use bytes::Bytes;
use common_domain::ids::BaseId;
use csv::ReaderBuilder;
use meteroid_store::Store;
use meteroid_store::domain::batch_jobs::{BatchJob, BatchJobChunk};
use meteroid_store::domain::{CustomerCustomTax, CustomerNew};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CustomersInterface;
use rust_decimal::Decimal;

use super::event_csv::parse_input_params;

const CHUNK_SIZE: i32 = 50;

pub struct CustomerCsvProcessor {
    store: Arc<Store>,
}

impl CustomerCsvProcessor {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

fn map_to_domain(actor: uuid::Uuid, csv: NewCustomerCsv) -> Result<CustomerNew, String> {
    let billing_address = if csv.billing_address.country.is_some() {
        Some(csv.billing_address.into())
    } else {
        None
    };

    let shipping_address = if csv.shipping_address.country.is_some() {
        Some(csv.shipping_address.into())
    } else {
        None
    };

    let custom_taxes = parse_tax_rates(&csv.tax_rates)?;

    Ok(CustomerNew {
        name: csv.name.0,
        created_by: actor,
        alias: csv.alias.map(|a| a.0),
        billing_email: csv.billing_email.map(|e| e.0),
        invoicing_emails: csv.invoicing_emails.map(|x| x.0).unwrap_or_default(),
        phone: csv.phone.map(|p| p.0),
        balance_value_cents: 0,
        currency: csv.currency.0,
        billing_address,
        shipping_address,
        force_created_date: None,
        vat_number: csv.vat_number.map(|v| v.0),
        invoicing_entity_id: csv.invoicing_entity_id,
        custom_taxes,
        is_tax_exempt: csv.is_tax_exempt.unwrap_or(false),
        connected_account_id: None,
    })
}

fn parse_tax_rates(tax_rates: &CustomTaxRatesCsv) -> Result<Vec<CustomerCustomTax>, String> {
    let mut taxes = Vec::new();

    if let Some(rate) = tax_rates.rate1 {
        taxes.push(parse_tax_rate(
            rate,
            &tax_rates.tax_code1,
            &tax_rates.name1,
            "tax_rate1",
        )?);
    }

    if let Some(rate) = tax_rates.rate2 {
        taxes.push(parse_tax_rate(
            rate,
            &tax_rates.tax_code2,
            &tax_rates.name2,
            "tax_rate2",
        )?);
    }

    Ok(taxes)
}

fn parse_tax_rate(
    rate: Decimal,
    tax_code: &Option<CsvString>,
    name: &Option<CsvString>,
    field_name: &str,
) -> Result<CustomerCustomTax, String> {
    let tax_code = tax_code
        .as_ref()
        .ok_or(format!(
            "{field_name}.tax_code is required if rate is provided"
        ))?
        .0
        .clone();

    let name = name
        .as_ref()
        .ok_or(format!("{field_name}.name is required if rate is provided"))?
        .0
        .clone();

    Ok(CustomerCustomTax {
        tax_code,
        name,
        rate,
    })
}

#[async_trait]
impl BatchJobProcessor for CustomerCsvProcessor {
    async fn prepare_chunks(
        &self,
        job: &BatchJob,
        input_data: Option<Bytes>,
    ) -> Result<Vec<ChunkDefinition>, String> {
        let data = input_data.ok_or("No input data provided for CSV job")?;
        let params = parse_input_params(job)?;
        let normalized = normalize_csv_encoding(&data);
        let delimiter = params.delimiter as u8;

        let mut reader = ReaderBuilder::new()
            .has_headers(true)
            .delimiter(delimiter)
            .from_reader(normalized.as_ref());

        let headers = reader
            .headers()
            .map_err(|e| format!("Failed to read CSV headers: {e}"))?;

        let has_name = headers.iter().any(|h| h == "name");
        let has_currency = headers.iter().any(|h| h == "currency");
        if !has_name || !has_currency {
            return Err("CSV must contain 'name' and 'currency' columns".to_string());
        }

        let row_count = reader.records().count() as i32;
        if row_count == 0 {
            return Err("CSV file contains no data rows".to_string());
        }

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

        let mut reader = ReaderBuilder::new()
            .has_headers(true)
            .delimiter(delimiter)
            .from_reader(normalized.as_ref());

        let offset = chunk.item_offset as usize;
        let count = chunk.item_count as usize;

        let mut customers = Vec::with_capacity(count);
        let mut customer_indices = Vec::with_capacity(count);
        let mut failures = Vec::new();

        for (local_idx, result) in reader
            .deserialize::<NewCustomerCsv>()
            .enumerate()
            .skip(offset)
            .take(count)
        {
            let global_idx = local_idx as i32;

            match result {
                Ok(csv_row) => {
                    let alias = csv_row.alias.as_ref().map(|a| a.0.clone());
                    match map_to_domain(job.created_by, csv_row) {
                        Ok(customer) => {
                            customers.push(customer);
                            customer_indices.push(global_idx);
                        }
                        Err(reason) => {
                            failures.push(ItemFailure {
                                item_index: global_idx,
                                item_identifier: alias,
                                reason: format!("Failed to convert to domain: {reason}"),
                            });
                        }
                    }
                }
                Err(e) => {
                    failures.push(ItemFailure {
                        item_index: global_idx,
                        item_identifier: None,
                        reason: format!("Failed to parse row: {e}"),
                    });
                }
            }
        }

        // Deduplicate by alias within the chunk: if multiple rows share the same alias,
        // keep only the first and report the rest as failures.
        let mut seen_aliases: HashMap<String, i32> = HashMap::new();
        let mut deduped_customers = Vec::with_capacity(customers.len());
        let mut deduped_indices = Vec::with_capacity(customers.len());

        for (customer, global_idx) in customers.into_iter().zip(customer_indices.iter().copied()) {
            if let Some(ref alias) = customer.alias {
                if let Some(first_row) = seen_aliases.get(alias) {
                    failures.push(ItemFailure {
                        item_index: global_idx,
                        item_identifier: Some(alias.clone()),
                        reason: format!(
                            "Duplicate alias '{}' within the same batch (first seen at row {})",
                            alias,
                            first_row + 1
                        ),
                    });
                    continue;
                }
                seen_aliases.insert(alias.clone(), global_idx);
            }
            deduped_customers.push(customer);
            deduped_indices.push(global_idx);
        }

        if params.fail_on_error && !failures.is_empty() {
            return Ok(ChunkResult {
                processed: 0,
                failures,
                created_entities: vec![],
            });
        }

        if deduped_customers.is_empty() {
            return Ok(ChunkResult {
                processed: 0,
                failures,
                created_entities: vec![],
            });
        }

        match self
            .store
            .upsert_customer_batch_lenient(deduped_customers.clone(), job.tenant_id)
            .await
        {
            Ok(result) => {
                // Map store-level per-row failures back to CSV row indices
                for (batch_idx, reason) in result.failures {
                    let global_idx = deduped_indices[batch_idx];
                    let alias = deduped_customers[batch_idx].alias.clone();
                    failures.push(ItemFailure {
                        item_index: global_idx,
                        item_identifier: alias,
                        reason,
                    });
                }

                let entities = result
                    .created
                    .iter()
                    .map(|c| CreatedEntity {
                        entity_type: "customer",
                        entity_id: c.id.as_uuid(),
                    })
                    .collect();

                Ok(ChunkResult {
                    processed: result.created.len() as i32,
                    failures,
                    created_entities: entities,
                })
            }
            Err(e) => {
                // Transient error (DB down, etc.) → trigger auto-retry
                let msg = match e.current_context() {
                    StoreError::DatabaseError(db_report) => {
                        format!("Database error: {}", db_report.current_context())
                    }
                    other => other.to_string(),
                };
                Err(msg)
            }
        }
    }

    fn max_retries(&self) -> i32 {
        3
    }

    fn chunk_size(&self) -> i32 {
        CHUNK_SIZE
    }
}
