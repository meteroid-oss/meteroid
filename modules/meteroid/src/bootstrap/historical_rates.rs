use std::collections::BTreeMap;
use std::io::Cursor;
use parquet::file::reader::{FileReader, SerializedFileReader};
use anyhow::{bail, Context, Result};
use diesel::ExpressionMethods;
use parquet::record::RowAccessor;
use meteroid_store::domain::historical_rates::HistoricalRatesFromUsdNew;
use crate::errors;
use crate::services::currency_rates::ExchangeRates;

pub async fn fetch_parquet_file(url: &str) -> Result<Vec<u8>> {
    let client = reqwest::Client::new();
    let response = client.get(url)
        .send()
        .await
        .context("Failed to send request")?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Failed to fetch file: HTTP status {}",
            response.status()
        ));
    }

    let bytes = response.bytes()
        .await
        .context("Failed to read response body")?;

    Ok(bytes.to_vec())
}
pub fn read_parquet_bytes_to_exchange_rates(parquet_bytes: &[u8]) -> Result<Vec<HistoricalRatesFromUsdNew>> {
    // Create a reader from the bytes
    let cursor = Cursor::new(parquet_bytes);
    let reader = SerializedFileReader::new(cursor)
        .context("Failed to create Parquet reader")?;

    let metadata = reader.metadata();
    let schema = metadata.file_metadata().schema();

    // Get column indexes for the fields we need
    let base_idx = schema.get_field_index("base_currency")
        .context("Missing 'base_currency' column")?;
    let timestamp_idx = schema.get_field_index("timestamp")
        .context("Missing 'timestamp' column")?;

    // Get all currency column indexes
    let mut currency_indexes = Vec::new();
    let mut currency_names = Vec::new();

    for (i, field) in schema.get_fields().iter().enumerate() {
        let name = field.name();
        if name != "date" && name != "timestamp" && name != "base_currency" {
            currency_indexes.push(i);
            currency_names.push(name.clone());
        }
    }

    let mut exchange_rates_vec = Vec::new();

    // Read row groups
    for row_group_idx in 0..reader.num_row_groups() {
        let row_group = reader.get_row_group(row_group_idx)?;

        // Process rows
        for row in row_group.get_row_iter(None)? {
            let row = row.context("Failed to parse row")?;
            let base = row.get_string(base_idx)?.to_string();
            let timestamp = row?.get_long(timestamp_idx)? as u64;

            if base != "USD" {
                bail!("Invalid base currency: {}. Expected USD", base);
            }

            let date = chrono::DateTime::from_timestamp(timestamp as i64, 0)
                .context("Failed to parse timestamp to date")?
                .date_naive();

            let mut rates = BTreeMap::new();

            // Extract rates for each currency
            for (idx, currency_name) in currency_indexes.iter().zip(&currency_names) {
                if !row.is_null(*idx)? {
                    let rate = row.get_double(*idx)? as f32;
                    rates.insert(currency_name.clone(), rate);
                }
            }

            exchange_rates_vec.push(HistoricalRatesFromUsdNew {
                rates,
                date,
            });
        }
    }

    Ok(exchange_rates_vec)
}
