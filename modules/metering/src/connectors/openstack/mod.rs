use crate::connectors::Connector;
use crate::config::{OpenstackConfig};
use crate::connectors::errors::ConnectorError;
use crate::domain::{Meter, QueryMeterParams, Usage};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

use error_stack::{Result, ResultExt, bail};

use std::str::FromStr;
use std::sync::Arc;
use hyper::http;
use osauth::common::IdOrName;
use osauth::Session;
use reqwest::{Client};
use http::method::Method;
use tonic::IntoRequest;

mod simple_tenant_usage;
mod gnocchi_usage
;

use tap::tap::TapFallible;


pub struct OpenstackConnector {
    session: Arc<Session>,
    auth_url: String,
    gnocchi_url: String,
}

impl OpenstackConnector {
    pub async fn init(
        openstack_config: &OpenstackConfig
    ) -> Result<Self, ConnectorError> {
        // TODO osauth::identity:: applicationcredentials
        let auth = osauth::identity::Password::new(
            openstack_config.auth_url.clone(),
            openstack_config.username.clone(),
            openstack_config.password.clone(),
            openstack_config.user_domain_name.clone(),
        )
            .change_context(ConnectorError::QueryError)?
            .with_project_scope(
                IdOrName::Name(openstack_config.project_name.clone()),
                IdOrName::Name(openstack_config.project_domain_name.clone()),
            );

        let session = osauth::Session::new(auth).await
            .change_context(ConnectorError::QueryError)?;

        log::info!("Openstack session created");

        Ok(OpenstackConnector {
            session: Arc::new(session),
            auth_url: openstack_config.auth_url.clone(),
            gnocchi_url: openstack_config.gnocchi_url.clone(),
        })
    }
}


#[async_trait]
impl Connector for OpenstackConnector {
    #[tracing::instrument(skip_all)]
    async fn register_meter(&self, meter: Meter) -> Result<(), ConnectorError> {
        log::warn!("OpenstackConnector::register_meter is not implemented");
        Ok(())
    }

    async fn query_meter(&self, params: QueryMeterParams) -> Result<Vec<Usage>, ConnectorError> {


        // for now, we consider that we only need to fetch 1 project by customer. This is wrong, but let's wait to know
        // if it's nested under a parent project or under a domain

        // Currently we consider that the ext customer id is the project, and that we don't need to fetch nested ones. So, we can call directly the gnocchi api

        // TODO params.namespace ? => meteroid tenant I guess, that would be a different scope ? or a different auth_url etc ?


        // 2 possibility :
        // - from /compute/v2.1/os-simple-tenant-usage
        if params.event_name == "meteroid_connector.instance_hours" {
            let params_clone = params.clone();
            let details = self.get_tenant_usage(params_clone).await
                .tap_err(|e| log::error!("Error while fetching tenant usage: {:?}", e))
                ?;

            let usage_per_instance_type = details.iter().flat_map(|tenant_usage| {
                let instance_hours_per_flavor = tenant_usage.compute_instance_hours_per_flavor();

                instance_hours_per_flavor.iter().map(|(flavor, hours)| {
                    Usage {
                        window_start: tenant_usage.start.and_utc(),
                        window_end: tenant_usage.stop.and_utc(),
                        customer_id: tenant_usage.tenant_id.clone(),
                        value: *hours,
                        group_by: HashMap::from([("flavor".to_string(), Some(flavor.clone()))]),
                    }
                }).collect::<Vec<Usage>>()
            }).collect::<Vec<Usage>>();

            log::info!("Usage per instance type: {:?}", usage_per_instance_type);

            Ok(usage_per_instance_type)
            // - from gnocchi
        } else if params.event_name == "network.outgoing.bytes" {
            // in production, we should have a day-by-day aggregation with the mean:rate, then we can simply sum that.
            // Here we'll aggregate live

            let params_clone = params.clone();

            let from = params_clone.from;
            let end = params_clone.to.unwrap_or(Utc::now());
            let details = self.query_metric(params_clone).await
                .tap_err(|e| log::error!("Error while fetching gnocchi metric: {:?}", e))
                ?;


            let usage_per_project = details.iter().map(|(project_id, value)| {
                Usage {
                    window_start: from,
                    window_end: end,
                    customer_id: project_id.to_string(),
                    value: *value,
                    group_by: HashMap::new(),
                }
            }).collect::<Vec<Usage>>();


            // We need to do the following calls :
            // - from /v1/search/resource/network.outgoing.bytes.rate

            Ok(usage_per_project)
        } else {
            bail!(ConnectorError::InvalidQuery(format!("Unknown meter slug: {}", params.meter_slug)))
        }
    }
}


pub fn date_opt_to_str(date: &Option<DateTime<Utc>>) -> String {
    date.map(|d| date_to_str(&d)).unwrap_or("".to_string())
}

pub fn date_to_str(date: &DateTime<Utc>) -> String {
    date.naive_utc()
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string()
}