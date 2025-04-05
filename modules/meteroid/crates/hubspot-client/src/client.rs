use crate::error::HubspotError;
use crate::model::{BatchUpsertRequest, BatchUpsertResponse};
use reqwest::{Client, Method, Url};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::RetryTransientMiddleware;
use reqwest_retry::policies::ExponentialBackoff;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;

/// Note: we might want to generate the client from the https://api.hubspot.com/public/api/spec/v1/specs in the future,
/// currently swagger-codegen is not flexible enough to generate a client from multiple specs.
///
/// We might also consider https://github.com/oxidecomputer/progenitor after https://github.com/oxidecomputer/progenitor/issues/344 is fixed.
///
/// Rate limits: 110 requests per 10 seconds per app installed (tenant)
/// https://developers.hubspot.com/docs/guides/apps/api-usage/usage-details#public-apps
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct HubspotClient {
    client: ClientWithMiddleware,
    api_base: Url,
}

impl Default for HubspotClient {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl HubspotClient {
    pub fn new() -> Self {
        Self::from_parts(
            "https://api.hubspot.com",
            Duration::from_secs(5),
            Duration::from_secs(10),
            3,
        )
    }

    pub fn from_parts<'a>(
        url: impl Into<&'a str>,
        connect_timeout: Duration,
        timeout: Duration,
        max_retries: u32,
    ) -> Self {
        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(max_retries);

        let client = Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(timeout)
            .build()
            .expect("invalid client config");

        let client = ClientBuilder::new(client)
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        Self {
            client,
            api_base: Url::parse(url.into()).expect("invalid url"),
        }
    }

    pub(crate) async fn execute<Req: Serialize, Resp: DeserializeOwned + Send + 'static>(
        &self,
        path: &str,
        method: Method,
        access_token: &SecretString,
        body: Option<Req>,
    ) -> Result<Resp, HubspotError> {
        let url = self.api_base.join(path).expect("invalid path");

        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(access_token.expose_secret());

        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request.send().await.map_err(HubspotError::from)?;
        let status_code = &response.status();

        if !status_code.is_success() {
            return Err(HubspotError::ClientError {
                error: response.text().await.unwrap_or_default(),
                status_code: Some(status_code.as_u16()),
            });
        }

        response.json().await.map_err(HubspotError::from)
    }

    pub(crate) async fn batch_upsert(
        &self,
        path: &str,
        request: BatchUpsertRequest,
        access_token: &SecretString,
    ) -> Result<BatchUpsertResponse, HubspotError> {
        self.execute(path, Method::POST, access_token, Some(request))
            .await
    }
}
