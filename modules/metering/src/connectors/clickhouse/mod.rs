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
        let event_table_ddl = sql::init::create_events_table_sql();
        // TODO replace with custom integration (with dedupe) or kafka connect, as this puts the constraints on CH
        let kafka_table_ddl = sql::init::create_kafka_event_table_sql(
            kafka_config.kafka_internal_addr.clone(),
            kafka_config.kafka_topic.clone(),
            "clickhouse".to_string(),
            "JSONEachRow".to_string(),
        );
        let kafka_mv_ddl = sql::init::create_kafka_mv_sql();

        let client = Client::default()
            .with_url(&clickhouse_config.address)
            .with_user(&clickhouse_config.username)
            .with_password(&clickhouse_config.password)
            .with_database(&clickhouse_config.database);

        let client = Arc::new(client);

        client
            .query(event_table_ddl.as_str())
            .execute()
            .await
            .map_err(|err| {
                ConnectorError::InitError(format!("Could not create event table: {}", err))
            })?;

        client
            .query(kafka_table_ddl.as_str())
            .execute()
            .await
            .map_err(|err| {
                ConnectorError::InitError(format!("Could not create kafka engine table: {}", err))
            })?;

        client
            .query(kafka_mv_ddl.as_str())
            .execute()
            .await
            .map_err(|err| {
                ConnectorError::InitError(format!("Could not create kafka MV: {}", err))
            })?;

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
            .find(|ext| params.event_name.starts_with(&ext.prefix()))
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
                .get_timestamp_utc("windowstart")
                .ok_or(ConnectorError::QueryError)?;

            let window_end = row
                .get_timestamp_utc("windowend")
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
