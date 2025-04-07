use anyhow::{Context, Result, bail};
use meteroid_store::domain::historical_rates::HistoricalRatesFromUsdNew;
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::RowAccessor;
use std::collections::BTreeMap;

pub async fn fetch_parquet_file(url: &str) -> Result<Vec<u8>> {
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .send()
        .await
        .context("Failed to send request")?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Failed to fetch file: HTTP status {}",
            response.status()
        ));
    }

    let bytes = response
        .bytes()
        .await
        .context("Failed to read response body")?;

    Ok(bytes.to_vec())
}
pub fn read_parquet_bytes_to_exchange_rates(
    parquet_bytes: &[u8],
) -> Result<Vec<HistoricalRatesFromUsdNew>> {
    // Convert &[u8] to bytes::Bytes which implements ChunkReader
    let bytes = bytes::Bytes::copy_from_slice(parquet_bytes);

    // Create a reader from the bytes
    let reader = SerializedFileReader::new(bytes).context("Failed to create Parquet reader")?;

    let metadata = reader.metadata();
    let schema = metadata.file_metadata().schema();
    let fields = schema.get_fields();

    // Find field indexes manually
    let mut base_idx = None;
    let mut timestamp_idx = None;
    let mut currency_indexes = Vec::new();
    let mut currency_names = Vec::new();

    for (i, field) in fields.iter().enumerate() {
        let name = field.name();
        if name == "base_currency" {
            base_idx = Some(i);
        } else if name == "timestamp" {
            timestamp_idx = Some(i);
        } else if name != "date" {
            currency_indexes.push(i);
            currency_names.push(name.to_string()); // Use to_string() instead of clone() for &str -> String
        }
    }

    let base_idx = base_idx.context("Missing 'base_currency' column")?;
    let timestamp_idx = timestamp_idx.context("Missing 'timestamp' column")?;

    let mut exchange_rates_vec = Vec::new();

    // Read row groups
    for row_group_idx in 0..reader.num_row_groups() {
        let row_group = reader.get_row_group(row_group_idx)?;

        // Process rows
        for row_result in row_group.get_row_iter(None)? {
            let row = row_result.context("Failed to parse row")?;
            let base = row.get_string(base_idx)?.to_string();
            let timestamp = row.get_long(timestamp_idx)? as u64;

            if base != "USD" {
                bail!("Invalid base currency: {}. Expected USD", base);
            }

            let date = chrono::DateTime::from_timestamp(timestamp as i64, 0)
                .context("Failed to parse timestamp to date")?
                .date_naive();

            let mut rates = BTreeMap::new();

            // Extract rates for each currency
            for (idx, currency_name) in currency_indexes.iter().zip(&currency_names) {
                if let Ok(rate) = row.get_double(*idx) {
                    rates.insert(currency_name.clone(), rate as f32);
                }
            }

            exchange_rates_vec.push(HistoricalRatesFromUsdNew { rates, date });
        }
    }

    Ok(exchange_rates_vec)
}
