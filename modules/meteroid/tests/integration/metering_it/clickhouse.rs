// APACHE 2.0 license - Copyright 2021, The Tremor Team
// Adapted from https://github.com/tremor-rs/tremor-runtime/blob/main/src/connectors/tests/clickhouse/utils.rs

use backon::Retryable;
pub(crate) const CONTAINER_NAME: &str = "clickhouse/clickhouse-server";
pub(crate) const CONTAINER_VERSION: &str = "23.12.1-alpine";

use clickhouse::Client;
use std::time::Duration;

pub(super) async fn wait_for_ok(port: u16) -> anyhow::Result<()> {
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

pub(crate) fn get_client(port: u16) -> Client {
    Client::default()
        .with_url(format!("http://localhost:{port}"))
        .with_user("default")
        .with_password("default")
        .with_database("meteroid")
}

pub(super) async fn test_status_endpoint(port: u16) -> anyhow::Result<()> {
    let client = get_client(port);

    let values: Vec<u8> = client.query("SELECT 1").fetch_all().await?;
    assert_eq!(values, vec![1]);

    Ok(())
}
