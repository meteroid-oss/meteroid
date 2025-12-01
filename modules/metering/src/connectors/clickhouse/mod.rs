use crate::config::{ClickhouseConfig, KafkaConfig};
use crate::connectors::Connector;
use crate::connectors::errors::ConnectorError;
use crate::domain::{Meter, QueryMeterParams, QueryRawEventsParams, QueryRawEventsResult, Usage};
use async_trait::async_trait;
use std::collections::HashMap;

use error_stack::{Report, ResultExt};

use std::sync::Arc;

pub mod extensions;
pub mod sql;

use crate::connectors::clickhouse::extensions::ConnectorClickhouseExtension;
use crate::connectors::clickhouse::sql::PropertyColumn;
use crate::connectors::json::JsonFieldExtractor;
use crate::migrations;
use clickhouse::Client;
use tokio::io::AsyncBufReadExt;

#[derive(Clone)]
pub struct ClickhouseConnector {
    client: Arc<Client>,
    extensions: Vec<Arc<dyn ConnectorClickhouseExtension + Send + Sync>>,
}

impl ClickhouseConnector {
    pub async fn init(
        clickhouse_config: &ClickhouseConfig,
        kafka_config: &KafkaConfig,
        extensions: Vec<Arc<dyn ConnectorClickhouseExtension + Send + Sync>>,
    ) -> Result<Self, Report<ConnectorError>> {
        migrations::run(clickhouse_config, kafka_config)
            .await
            .change_context(ConnectorError::InitError(
                "Failed to run migrations".to_string(),
            ))?;

        let client = Client::default()
            .with_url(&clickhouse_config.http_address)
            .with_user(&clickhouse_config.username)
            .with_password(&clickhouse_config.password)
            .with_database(&clickhouse_config.database);

        let client = Arc::new(client);

        for ext in &extensions {
            ext.init(client.clone()).await?;
        }

        Ok(ClickhouseConnector { client, extensions })
    }

    fn match_extension(
        &self,
        params: &QueryMeterParams,
    ) -> Option<Arc<dyn ConnectorClickhouseExtension + Send + Sync>> {
        self.extensions
            .iter()
            .find(|ext| params.code.starts_with(&ext.prefix()))
            .cloned()
    }
}

#[async_trait]
impl Connector for ClickhouseConnector {
    #[tracing::instrument(skip_all)]
    async fn register_meter(&self, meter: Meter) -> Result<(), Report<ConnectorError>> {
        let ddl = sql::create_meter::create_meter_view(
            meter, true, // TODO consider making this configurable
        );
        self.client
            .query(ddl.as_str())
            .execute()
            .await
            .change_context(ConnectorError::RegisterError)?;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn query_meter(
        &self,
        params: QueryMeterParams,
    ) -> Result<Vec<Usage>, Report<ConnectorError>> {
        let query = match self
            .match_extension(&params)
            .and_then(|ext| ext.build_query(&params))
        {
            Some(ext) => ext,
            None => sql::query_raw::query_meter_view_sql(params.clone())
                .map_err(ConnectorError::InvalidQuery)?,
        };

        tracing::debug!("Generated query: {}", query.as_str());

        let mut lines = self
            .client
            .query(query.as_str())
            .fetch_bytes("JSONEachRow")
            .change_context(ConnectorError::QueryError)
            .attach("Failed to execute query with JSONEachRow")?
            .lines();

        let mut parsed = Vec::new();

        while let Some(line) = lines
            .next_line()
            .await
            .change_context(ConnectorError::QueryError)?
        {
            let row: serde_json::Value = serde_json::from_str(&line)
                .change_context(ConnectorError::QueryError)
                .attach("Failed to parse JSON row")?;

            let window_start = row
                .get_timestamp_utc("window_start")
                .ok_or(ConnectorError::QueryError)
                .attach("Missing window_start field")?;

            let window_end = row
                .get_timestamp_utc("window_end")
                .ok_or(ConnectorError::QueryError)
                .attach("Missing window_end field")?;
            let value = row
                .get_f64("value")
                .ok_or(ConnectorError::QueryError)
                .attach("Missing value field")?;

            let customer_id = row
                .get_string("customer_id")
                .ok_or(ConnectorError::QueryError)
                .attach("Missing customer_id field")?;

            let mut group_by: HashMap<String, Option<String>> = HashMap::new();

            for column_name in &params.group_by {
                let col = PropertyColumn::from_str_ref(column_name);
                let column_value: Option<String> = row.get_string(&col.as_alias());
                group_by.insert(column_name.clone(), column_value);
            }

            if let Some(ref segmentation) = params.segmentation_filter {
                match segmentation {
                    crate::domain::SegmentationFilter::Independent(filters) => {
                        for (column_name, _) in filters {
                            let col = PropertyColumn::from_str_ref(column_name);
                            let column_value: Option<String> = row.get_string(&col.as_alias());
                            group_by.insert(column_name.clone(), column_value);
                        }
                    }
                    crate::domain::SegmentationFilter::Linked {
                        dimension1_key,
                        dimension2_key,
                        ..
                    } => {
                        let col1 = PropertyColumn::from_str_ref(dimension1_key);
                        let col2 = PropertyColumn::from_str_ref(dimension2_key);
                        let dim1_value: Option<String> = row.get_string(col1.as_alias().as_str());
                        let dim2_value: Option<String> = row.get_string(col2.as_alias().as_str());
                        group_by.insert(dimension1_key.clone(), dim1_value);
                        group_by.insert(dimension2_key.clone(), dim2_value);
                    }
                }
            }

            parsed.push(Usage {
                window_start,
                window_end,
                value,
                customer_id,
                group_by,
            });
        }

        Ok(parsed)
    }

    #[tracing::instrument(skip_all)]
    async fn query_raw_events(
        &self,
        params: QueryRawEventsParams,
    ) -> Result<QueryRawEventsResult, Report<ConnectorError>> {
        let query = sql::query_raw::query_raw_events_sql(params.clone())
            .map_err(ConnectorError::InvalidQuery)?;

        let mut lines = self
            .client
            .query(query.as_str())
            .fetch_bytes("JSONEachRow")
            .change_context(ConnectorError::QueryError)?
            .lines();

        let mut events = Vec::new();

        while let Some(line) = lines
            .next_line()
            .await
            .change_context(ConnectorError::QueryError)?
        {
            let row: serde_json::Value =
                serde_json::from_str(&line).change_context(ConnectorError::QueryError)?;

            let id = row.get_string("id").ok_or(ConnectorError::QueryError)?;
            let code = row.get_string("code").ok_or(ConnectorError::QueryError)?;
            let customer_id = row
                .get_string("customer_id")
                .ok_or(ConnectorError::QueryError)?;
            let tenant_id = row
                .get_string("tenant_id")
                .ok_or(ConnectorError::QueryError)?;
            let timestamp = row
                .get_timestamp_utc("timestamp")
                .ok_or(ConnectorError::QueryError)?;
            let ingested_at = row
                .get_timestamp_utc("ingested_at")
                .ok_or(ConnectorError::QueryError)?;

            let properties: HashMap<String, String> = row
                .get("properties")
                .and_then(|v| v.as_object())
                .unwrap_or(&serde_json::Map::new())
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
                .collect();

            events.push(crate::ingest::domain::RawEvent {
                id,
                code,
                customer_id,
                tenant_id,
                timestamp: timestamp.naive_utc(),
                ingested_at: ingested_at.naive_utc(),
                properties,
            });
        }

        Ok(QueryRawEventsResult { events })
    }
}
