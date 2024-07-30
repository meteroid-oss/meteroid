use std::collections::HashMap;
use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use chrono::{DateTime, NaiveDateTime, Utc};
use error_stack::{ResultExt, Result};
use osauth::PaginatedResource;
use crate::connectors::errors::ConnectorError;
use crate::connectors::openstack::{date_opt_to_str, date_to_str, OpenstackConnector};
use crate::domain::QueryMeterParams;
use futures::prelude::*;

#[derive(Serialize, Deserialize, Debug)]
pub struct TenantUsage {
    pub tenant_id: String,
    pub server_usages: Vec<ServerUsage>,
    pub total_local_gb_usage: f64,
    pub total_vcpus_usage: f64,
    pub total_memory_mb_usage: f64,
    pub total_hours: f64,
    pub start: NaiveDateTime,
    pub stop: NaiveDateTime,
}


impl TenantUsage {
    pub fn compute_instance_hours_per_flavor(&self) -> HashMap<String, f64> {
        let mut instance_hours: HashMap<String, f64> = HashMap::new();
        for usage in &self.server_usages {
            let entry = instance_hours.entry(usage.flavor.clone()).or_insert(0.0);
            *entry += usage.hours;
        }
        instance_hours
    }
}


#[derive(Serialize, Deserialize, Debug)]
pub struct ServerUsage {
    hours: f64,
    flavor: String,
    instance_id: String,
    name: String,
    tenant_id: String,
    memory_mb: u32,
    local_gb: u32,
    vcpus: u32,
    started_at: NaiveDateTime,
    ended_at: Option<NaiveDateTime>,
    state: String,
    uptime: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SimpleTenantUsageResponse {
    tenant_usage: TenantUsage,
}


use std::borrow::Cow;

#[derive(Debug, Clone)]
enum QueryParams {
    Start(DateTime<Utc>),
    End(Option<DateTime<Utc>>),
}


impl osauth::QueryItem for QueryParams {
    fn query_item(&self) -> std::result::Result<(&str, Cow<str>), osauth::Error> {
        Ok(match self {
            QueryParams::Start(a) => {
                // we need the CCYY-MM-DDThh:mm:ss format
                let date = date_to_str(a);
                log::info!("Start date: {}", date);
                ("start", date.into())
            }
            QueryParams::End(a) => {
                let date = date_opt_to_str(a);
                log::info!("End date: {}", date);
                ("end", date.into())
            }
        })
    }
}

impl OpenstackConnector {
    async fn get_tenant_usage_for_project(&self, query: &osauth::Query<QueryParams>, project_id: String) -> Result<TenantUsage, ConnectorError> {
        let response = self.session.get(osauth::services::COMPUTE, &["os-simple-tenant-usage", &project_id])
            .query(query)
            .fetch::<serde_json::Value>()
            .await
            .change_context(ConnectorError::QueryError)?;


        log::info!("Response: {:?}", response);
        let response: SimpleTenantUsageResponse = serde_json::from_value(response)
            .change_context(ConnectorError::QueryError)?;


        Ok(response.tenant_usage)
    }

    pub async fn get_tenant_usage(&self, params: QueryMeterParams) -> Result<Vec<TenantUsage>, ConnectorError> {
        let query = osauth::Query::default()
            .with(QueryParams::Start(params.from))
            .with(QueryParams::End(params.to));


        let futures = params.customers.into_iter().map(|customer| {

            // TODO from custom field maybe
            let project_id = customer.external_id.to_string();

            let query = query.clone();
            async move {
                self.get_tenant_usage_for_project(&query, project_id).await
            }
        });

        futures::stream::iter(futures)
            .buffer_unordered(10)
            .collect::<Vec<Result<TenantUsage, ConnectorError>>>()
            .await
            .into_iter()
            .collect()
    }
}