use crate::config::{ClickhouseConfig, KafkaConfig};
use crate::connectors::errors::ConnectorError;
use crate::connectors::Connector;
use crate::domain::{Meter, QueryMeterParams, Usage};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use clickhouse_rs::{Options, Pool};
use std::collections::HashMap;

use error_stack::{Result, ResultExt};

use std::str::FromStr;

pub mod sql;

use chrono_tz::Tz;

pub struct ClickhouseConnector {
    pool: Pool,
}

impl ClickhouseConnector {
    pub async fn init(
        clickhouse_config: &ClickhouseConfig,
        kafka_config: &KafkaConfig,
    ) -> Result<Self, ConnectorError> {
        let options = Options::from_str(&clickhouse_config.address.clone())
            .map_err(|_| {
                ConnectorError::ConfigurationError(
                    "Failed to parse clickhouse address to Url".to_string(),
                )
            })?
            .database(&clickhouse_config.database)
            .username(&clickhouse_config.username)
            .password(&clickhouse_config.password);

        let pool = Pool::new(options);

        let event_table_ddl = sql::init::create_events_table_sql();
        // TODO replace with custom integration (with dedupe) or kafka connect, as this puts the constraints on CH
        let kafka_table_ddl = sql::init::create_kafka_event_table_sql(
            kafka_config.kafka_internal_addr.clone(),
            kafka_config.kafka_topic.clone(),
            "clickhouse".to_string(),
            "JSONEachRow".to_string(),
        );
        let kafka_mv_ddl = sql::init::create_kafka_mv_sql();

        let mut client = pool.get_handle().await.map_err(|err| {
            ConnectorError::ConnectionError(format!(
                "Failed to connect to Clickhouse : {}",
                err.to_string()
            ))
        })?;

        client
            .execute(event_table_ddl)
            .await
            .change_context(ConnectorError::InitError(
                "Could not create event table".to_string(),
            ))?;
        client
            .execute(kafka_table_ddl)
            .await
            .change_context(ConnectorError::InitError(
                "Could not create kafka engine table".to_string(),
            ))?;
        client
            .execute(kafka_mv_ddl)
            .await
            .change_context(ConnectorError::InitError(
                "Could not create kafka MV".to_string(),
            ))?;

        Ok(ClickhouseConnector { pool })
    }
}

#[async_trait]
impl Connector for ClickhouseConnector {
    #[tracing::instrument(skip_all)]
    async fn register_meter(&self, meter: Meter) -> Result<(), ConnectorError> {
        let mut client = self
            .pool
            .get_handle()
            .await
            .change_context(ConnectorError::ResourceUnavailable)?;

        let ddl = sql::create_meter::create_meter_view(
            meter, true, // TODO consider making this configurable
        );

        client
            .execute(ddl)
            .await
            .change_context(ConnectorError::RegisterError)?;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn query_meter(&self, params: QueryMeterParams) -> Result<Vec<Usage>, ConnectorError> {
        let mut client = self
            .pool
            .get_handle()
            .await
            .change_context(ConnectorError::ResourceUnavailable)?;

        let sql = sql::query_meter::query_meter_view_sql(params.clone())
            .map_err(|e| ConnectorError::InvalidQuery(e))?;

        let block = client
            .query(sql.clone())
            .fetch_all()
            .await
            .map_err(|e| {
                log::error!("Query error: '{:?}' for sql '{}'", e, sql);
                e
            })
            .change_context(ConnectorError::QueryError)?;

        // TODO get from param instead if !window_size ?
        let (window_start_col, window_end_col) = match params.window_size {
            Some(_) => ("windowstart", "windowend"),
            None => ("min(windowstart)", "max(windowend)"),
        };

        let parsed = block
            .rows()
            .into_iter()
            .map(|row| {
                let window_start: DateTime<Tz> = row
                    .get(window_start_col)
                    .change_context(ConnectorError::QueryError)?;
                let window_end: DateTime<Tz> = row
                    .get(window_end_col)
                    .change_context(ConnectorError::QueryError)?;
                let value: f64 = row
                    .get("value")
                    .change_context(ConnectorError::QueryError)?;
                let customer_id: String = row
                    .get("customer_id")
                    .change_context(ConnectorError::QueryError)?;

                let window_start = window_start.with_timezone(&Utc);
                let window_end = window_end.with_timezone(&Utc);

                let mut group_by: HashMap<String, Option<String>> = HashMap::new();

                // TODO test
                for c in params.group_by.iter() {
                    let column_name = c.to_string();
                    let column_value: Option<String> = row
                        .get(column_name.as_str())
                        .change_context(ConnectorError::QueryError)?;
                    group_by.insert(column_name, column_value);
                }

                Ok(Usage {
                    window_start,
                    window_end,
                    value,
                    customer_id,
                    group_by,
                })
            })
            .collect::<Result<Vec<Usage>, ConnectorError>>();

        parsed
    }
}
