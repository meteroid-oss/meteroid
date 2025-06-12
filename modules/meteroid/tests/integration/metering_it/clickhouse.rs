// APACHE 2.0 license - Copyright 2021, The Tremor Team
// Adapted from https://github.com/tremor-rs/tremor-runtime/blob/main/src/connectors/tests/clickhouse/utils.rs

use backon::Retryable;
use std::ops::Deref;
pub(crate) const CONTAINER_NAME: &str = "clickhouse/clickhouse-server";
pub(crate) const CONTAINER_VERSION: &str = "23.12.1-alpine";

use clickhouse::Client;
use std::time::Duration;

pub(super) async fn wait_for_ok(port: HttpPort) -> anyhow::Result<()> {
    (|| async { test_status_endpoint(port).await })
        .retry(
            backon::ConstantBuilder::default()
                .with_delay(Duration::from_secs(1))
                .with_max_times(60),
        )
        .notify(|err: &anyhow::Error, dur: Duration| {
            log::warn!(
                "Retrying to connect to ClickHouse container after {:?}, error: {}",
                dur,
                err
            );
        })
        .await?;

    Ok(())
}

pub(crate) fn get_client(port: HttpPort) -> Client {
    Client::default()
        .with_url(format!("http://localhost:{port}"))
        .with_user("default")
        .with_password("default")
        .with_database("meteroid")
}

pub(super) async fn test_status_endpoint(port: HttpPort) -> anyhow::Result<()> {
    let client = get_client(port);

    let values: Vec<u8> = client.query("SELECT 1").fetch_all().await?;
    assert_eq!(values, vec![1]);

    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct HttpPort(pub u16);
impl Deref for HttpPort {
    type Target = u16;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::fmt::Display for HttpPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TcpPort(pub u16);
impl Deref for TcpPort {
    type Target = u16;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::fmt::Display for TcpPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
