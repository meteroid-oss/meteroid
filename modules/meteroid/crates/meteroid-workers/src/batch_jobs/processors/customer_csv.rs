use std::collections::HashMap;
use std::sync::Arc;

use crate::batch_jobs::engine::{
    BatchJobProcessor, ChunkDefinition, ChunkResult, CreatedEntity, ItemFailure,
};
use async_trait::async_trait;
use bytes::Bytes;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, TenantId, UserId};
use csv::ReaderBuilder;
use meteroid_app::services::csv_ingest::{CsvString, normalize_csv_encoding};
use meteroid_app::services::custom_properties_csv::CustomPropertyColumns;
use meteroid_app::services::customer_ingest::{CustomTaxRatesCsv, NewCustomerCsv};
use meteroid_store::Store;
use meteroid_store::domain::batch_jobs::{BatchJob, BatchJobChunk};
use meteroid_store::domain::custom_property_definitions::CustomPropertyDefinition;
use meteroid_store::domain::enums::{CustomPropertyEntityType, CustomerType};
use meteroid_store::domain::{CustomerNew, CustomerTaxRate, CustomerTaxStatus};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::{CustomPropertyDefinitionsInterface, CustomersInterface};
use rust_decimal::Decimal;
use std::str::FromStr;

use super::event_csv::parse_input_params;

const CHUNK_SIZE: i32 = 50;

pub struct CustomerCsvProcessor {
    store: Arc<Store>,
}

impl CustomerCsvProcessor {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    async fn load_definitions(
        &self,
        tenant_id: TenantId,
    ) -> Result<Vec<CustomPropertyDefinition>, String> {
        self.store
            .list_custom_property_definitions(
                tenant_id,
                Some(CustomPropertyEntityType::Customer),
                false,
            )
            .await
            .map_err(|e| format!("Failed to load custom property definitions: {e}"))
    }
}

/// Absent/blank keeps the legacy `COMPANY` default; otherwise case-insensitive `COMPANY`/`INDIVIDUAL`.
fn parse_customer_type(raw: Option<&str>) -> Result<CustomerType, String> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(CustomerType::Company),
        Some(s) => CustomerType::from_str(&s.to_ascii_uppercase())
            .map_err(|_| format!("Invalid customer_type '{s}' (expected COMPANY or INDIVIDUAL)")),
    }
}

fn map_to_domain(
    mut csv: NewCustomerCsv,
    custom_properties: serde_json::Value,
) -> Result<CustomerNew, String> {
    let preferred_locales = parse_preferred_locales(&mut csv)?;
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
        name: csv.name.map(|s| s.0).unwrap_or_default(),
        customer_type: parse_customer_type(csv.customer_type.as_deref())?,
        first_name: csv.first_name.map(|s| s.0),
        last_name: csv.last_name.map(|s| s.0),
        alias: csv.alias.map(|a| a.0),
        billing_email: csv.billing_email.map(|e| e.0),
        invoicing_emails: csv.invoicing_emails.map(|x| x.0).unwrap_or_default(),
        phone: csv.phone.map(|p| p.0),
        legal_number: csv.legal_number.map(|n| n.0),
        buyer_reference: csv.buyer_reference.map(|n| n.0),
        currency: csv.currency.0,
        billing_address,
        shipping_address,
        force_created_date: None,
        vat_number: csv.vat_number.map(|v| v.0),
        invoicing_entity_id: csv.invoicing_entity_id,
        custom_taxes,
        tax_status: if csv.is_tax_exempt.unwrap_or(false) {
            CustomerTaxStatus::Exempt
        } else {
            CustomerTaxStatus::Taxable
        },
        exemption_reason: None,
        connected_account_id: None,
        // `Null` when the file declares no `custom_properties.*` column, which leaves the
        // entity's properties untouched (and skips validation) in the store.
        custom_properties,
        preferred_locales,
    })
}

/// `invoicing_language` is the deprecated one-entry spelling; an explicit
/// `preferred_locales` column wins over it. A list we can serve no entry of fails its
/// row, as it would on the API — a bulk import is the easiest place to lose a language
/// silently.
fn parse_preferred_locales(csv: &mut NewCustomerCsv) -> Result<Vec<String>, String> {
    let locales = csv
        .preferred_locales
        .take()
        .map(|l| l.0)
        .or_else(|| csv.invoicing_language.take().map(|l| vec![l.0]))
        .unwrap_or_default();
    meteroid_invoicing::lang::reject_unsupported_preferences(&locales).map_err(|tag| {
        format!(
            "Unsupported preferred_locales '{tag}'; supported: {:?}",
            meteroid_invoicing::lang::SUPPORTED
        )
    })?;
    Ok(locales)
}

fn parse_tax_rates(tax_rates: &CustomTaxRatesCsv) -> Result<Vec<CustomerTaxRate>, String> {
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
) -> Result<CustomerTaxRate, String> {
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

    Ok(CustomerTaxRate {
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
            .map_err(|e| format!("Failed to read CSV headers: {e}"))?
            .clone();

        // `name` is only required per company row (individuals derive it from first/last
        // name), so an individuals-only file may carry first_name/last_name instead.
        let has_name = headers
            .iter()
            .any(|h| h == "name" || h == "first_name" || h == "last_name");
        let has_currency = headers.iter().any(|h| h == "currency");
        if !has_name || !has_currency {
            return Err(
                "CSV must contain a 'name' (or 'first_name'/'last_name') and a 'currency' column"
                    .to_string(),
            );
        }

        // Fail the whole job upfront on a mistyped `custom_properties.*` header rather than
        // dropping those values row after row. Chunking is not retried, so a transient lookup
        // failure must not fail the job — `process_chunk` re-checks on a retryable path.
        match self.load_definitions(job.tenant_id).await {
            Ok(definitions) => {
                CustomPropertyColumns::from_headers(&headers, &definitions)?;
            }
            Err(e) => log::warn!("Skipping upfront custom-property header check: {e}"),
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

        let headers = reader
            .headers()
            .map_err(|e| format!("Failed to read CSV headers: {e}"))?
            .clone();

        let definitions = self.load_definitions(job.tenant_id).await?;
        let custom_property_columns = CustomPropertyColumns::from_headers(&headers, &definitions)?;

        let offset = chunk.item_offset as usize;
        let count = chunk.item_count as usize;

        let mut customers = Vec::with_capacity(count);
        let mut customer_indices = Vec::with_capacity(count);
        let mut failures = Vec::new();

        for (local_idx, result) in reader.records().enumerate().skip(offset).take(count) {
            let global_idx = local_idx as i32;

            let record = match result {
                Ok(record) => record,
                Err(e) => {
                    failures.push(ItemFailure {
                        item_index: global_idx,
                        item_identifier: None,
                        reason: format!("Failed to parse row: {e}"),
                    });
                    continue;
                }
            };

            match record.deserialize::<NewCustomerCsv>(Some(&headers)) {
                Ok(csv_row) => {
                    let alias = csv_row.alias.as_ref().map(|a| a.0.clone());
                    let custom_properties = match custom_property_columns.parse_row(&record) {
                        Ok(properties) => properties,
                        Err(reason) => {
                            failures.push(ItemFailure {
                                item_index: global_idx,
                                item_identifier: alias.clone(),
                                reason,
                            });
                            continue;
                        }
                    };

                    match map_to_domain(csv_row, custom_properties) {
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
            .upsert_customer_batch_lenient(
                Actor::User {
                    id: UserId::from(job.created_by),
                },
                deduped_customers.clone(),
                job.tenant_id,
            )
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
