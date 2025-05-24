// APACHE 2.0 license - Copyright 2021, The Tremor Team
// Adapted from https://github.com/tremor-rs/tremor-runtime/blob/main/src/connectors/tests/clickhouse/utils.rs

pub(crate) const CONTAINER_NAME: &str = "clickhouse/clickhouse-server";
pub(crate) const CONTAINER_VERSION: &str = "23.12.1-alpine";

use anyhow::{Result, bail};
use clickhouse::Client;
use log::error;
use std::time::{Duration, Instant};

pub(super) async fn wait_for_ok(port: u16) -> anyhow::Result<()> {
    let wait_for = Duration::from_secs(60);
    let start = Instant::now();

    tokio::time::sleep(Duration::from_secs(2)).await;
    while let Err(_e) = test_status_endpoint(port).await {
        if start.elapsed() > wait_for {
            let max_time = wait_for.as_secs();
            error!("We waited for more than {max_time}");
            bail!("Waiting for the ClickHouse container timed out.");
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Ok(())
}

pub(crate) fn get_client(port: u16) -> Client {
    Client::default()
        .with_url(format!("http://localhost:{port}"))
        .with_user("default")
        .with_password("default")
        .with_database("meteroid")
}

pub(super) async fn test_status_endpoint(port: u16) -> Result<()> {
    let client = get_client(port);

    let _ = client
        .query("SELECT 1")
        .fetch_one::<i32>()
        .await
        .map_err(|err| anyhow::anyhow!("Failed to query ClickHouse: {:?}", err))?;

    Ok(())
}
