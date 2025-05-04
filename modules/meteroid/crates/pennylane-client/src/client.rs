use crate::error::PennylaneError;
use meteroid_middleware::client::rate_limit::RateLimitMiddleware;
use nonzero_ext::nonzero;
use reqwest::{Client, Method, Url};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::RetryTransientMiddleware;
use reqwest_retry::policies::ExponentialBackoff;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::num::NonZeroU32;
use std::time::Duration;

/// Note: we might want to generate the client from the openapi spec in the future,
/// currently swagger-codegen is not flexible enough to generate a client from multiple specs.
///
/// We might also consider https://github.com/oxidecomputer/progenitor after https://github.com/oxidecomputer/progenitor/issues/344 is fixed.
///
/// Rate limits: 5 requests per second per token
/// https://pennylane.readme.io/v2.0/docs/rate-limiting-1
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PennylaneClient {
    pub(crate) client: ClientWithMiddleware,
    pub(crate) api_base: Url,
}

impl Default for PennylaneClient {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl PennylaneClient {
    pub fn new() -> Self {
        Self::from_parts(
            "https://app.pennylane.com",
            Duration::from_secs(5),
            Duration::from_secs(10),
            3,
            nonzero!(5u32),
        )
    }

    pub fn from_parts<'a>(
        url: impl Into<&'a str>,
        connect_timeout: Duration,
        timeout: Duration,
        max_retries: u32,
        rps: NonZeroU32,
    ) -> Self {
        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(max_retries);

        let client = Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(timeout)
            .build()
            .expect("invalid client config");

        let client = ClientBuilder::new(client)
            .with(RateLimitMiddleware::new(rps))
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
    ) -> Result<Resp, PennylaneError> {
        let url = self.api_base.join(path).expect("invalid path");

        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(access_token.expose_secret());

        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request.send().await.map_err(PennylaneError::from)?;
        let status_code = &response.status();

        if !status_code.is_success() {
            return Err(PennylaneError::ClientError {
                error: response.text().await.unwrap_or_default(),
                status_code: Some(status_code.as_u16()),
            });
        }

        response.json().await.map_err(PennylaneError::from)
    }
}
