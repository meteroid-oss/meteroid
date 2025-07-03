use crate::config::{ClickhouseConfig, KafkaConfig};
use crate::connectors::Connector;
use crate::connectors::errors::ConnectorError;
use crate::domain::{Meter, QueryMeterParams, Usage};
use async_trait::async_trait;
use std::collections::HashMap;

use error_stack::{Result, ResultExt};

use std::sync::Arc;

pub mod extensions;
pub mod sql;

use crate::connectors::clickhouse::extensions::ConnectorClickhouseExtension;
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
    ) -> Result<Self, ConnectorError> {
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

        Ok(ClickhouseConnector { extensions, client })
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
    async fn register_meter(&self, meter: Meter) -> Result<(), ConnectorError> {
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
    async fn query_meter(&self, params: QueryMeterParams) -> Result<Vec<Usage>, ConnectorError> {
        let query = match self
            .match_extension(&params)
            .and_then(|ext| ext.build_query(&params))
        {
            Some(ext) => ext,
            None => sql::query_meter::query_meter_view_sql(params.clone())
                .map_err(ConnectorError::InvalidQuery)?,
        };

        let mut lines = self
            .client
            .query(query.as_str())
            .fetch_bytes("JSONEachRow")
            .change_context(ConnectorError::QueryError)?
            .lines();

        let mut parsed = Vec::new();

        while let Some(line) = lines
            .next_line()
            .await
            .change_context(ConnectorError::QueryError)?
        {
            let row: serde_json::Value =
                serde_json::from_str(&line).change_context(ConnectorError::QueryError)?;

            let window_start = row
                .get_timestamp_utc("window_start")
                .ok_or(ConnectorError::QueryError)?;

            let window_end = row
                .get_timestamp_utc("window_end")
                .ok_or(ConnectorError::QueryError)?;
            let value = row.get_f64("value").ok_or(ConnectorError::QueryError)?;

            let customer_id = row
                .get_string("customer_id")
                .ok_or(ConnectorError::QueryError)?;

            let mut group_by: HashMap<String, Option<String>> = HashMap::new();

            // TODO test
            for by in params.group_by.iter() {
                let column_name = by.to_string();
                let column_value: Option<String> = row.get_string(column_name.as_str());
                group_by.insert(column_name, column_value);
            }

            parsed.push(Usage {
                window_start,
                window_end,
                value,
                customer_id,
                group_by,
            })
        }

        Ok(parsed)
    }
}
