use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use error_stack::{ResultExt, Result};

use crate::connectors::errors::ConnectorError;
use crate::connectors::openstack::{date_opt_to_str, date_to_str, OpenstackConnector};
use crate::domain::QueryMeterParams;
use futures::prelude::*;
use reqwest::{Url};
use futures::stream::{self, StreamExt};
use serde_json::json;

#[derive(Serialize, Deserialize, Debug)]
pub struct ApiResponse {
    pub measures: Measures,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Measures {
    pub aggregated: Vec<AggregatedData>,
}

// You can choose to represent the aggregated data in a more structured form:
#[derive(Serialize, Deserialize, Debug)]
pub struct AggregatedData {
    pub date: DateTime<Utc>,
    pub duration: f64,
    pub value: f64,
}


impl OpenstackConnector {
    async fn query_project_metric(&self, from: &str, to: &str, project_id: &str) -> Result<f64, ConnectorError> {
        log::info!("Querying project metric for project_id: {}", project_id);

        let url = format!("{}/v1/aggregates", self.gnocchi_url);
        log::info!("url: {}  -- {}", self.gnocchi_url, url);
        let url = Url::parse(&url)
            .change_context(ConnectorError::InitError("Invalid gnocchi_url provided".to_string()))?;

        let result = self.session.client()
            .request(http::method::Method::POST, url)
            .query(&json!({
                "fill": 0,
                "start": from,
                "end": to
            }))
            .json(&json!({
                "operations": "(aggregate rate:sum (resample max 1h (metric network.outgoing.bytes mean)))",
                "resource_type": "instance_network_interface",
                "search": {
                    "=": {
                        "project_id": project_id
                    }
                }
            }))
            .fetch::<ApiResponse>()
            .await
            .change_context(ConnectorError::QueryError)?;

        let total = result.measures.aggregated.iter().fold(0.0, |acc, x| acc + x.value);

        Ok(total)
    }

    pub async fn query_metric(&self, params: QueryMeterParams) -> Result<HashMap<String, f64>, ConnectorError> {
        let from = date_to_str(&params.from);
        let to = date_opt_to_str(&params.to);

        let futures = params.customers.into_iter().map(|customer| {
            let from = from.clone();
            let to = to.clone();
            async move {
                self.query_project_metric(&from, &to, &customer.external_id)
                    .await
                    .map(|value| (customer.external_id, value))
            }
        });

        futures::stream::iter(futures)
            .buffer_unordered(10)
            .collect::<Vec<Result<(String, f64), ConnectorError>>>()
            .await
            .into_iter()
            .collect()
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_aggregated_data_decoding() {
        let json_data = r#"
        {
            "measures": {
                "aggregated": [
                    [
                        "2024-07-17T00:00:00+00:00",
                        86400.0,
                        9313.0
                    ],
                    [
                        "2024-07-18T00:00:00+00:00",
                        86400.0,
                        12873.0
                    ]
                ]
            }
        }
        "#;

        let decoded: std::result::Result<ApiResponse, serde_json::Error> = serde_json::from_str(json_data);


        assert!(decoded.is_ok());

        let api_response = decoded.unwrap();

        assert_eq!(api_response.measures.aggregated.len(), 2);

        let first_entry = &api_response.measures.aggregated[0];
        assert_eq!(first_entry.date, Utc.ymd(2024, 7, 17).and_hms(0, 0, 0));
        assert_eq!(first_entry.duration, 86400.0);
        assert_eq!(first_entry.value, 9313.0);

        let second_entry = &api_response.measures.aggregated[1];
        assert_eq!(second_entry.date, Utc.ymd(2024, 7, 18).and_hms(0, 0, 0));
        assert_eq!(second_entry.duration, 86400.0);
        assert_eq!(second_entry.value, 12873.0);
    }
}
