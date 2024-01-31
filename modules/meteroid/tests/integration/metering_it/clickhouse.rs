// APACHE 2.0 license - Copyright 2021, The Tremor Team
// Adapted from https://github.com/tremor-rs/tremor-runtime/blob/main/src/connectors/tests/clickhouse/utils.rs

pub(crate) const CONTAINER_NAME: &str = "clickhouse/clickhouse-server";
pub(crate) const CONTAINER_VERSION: &str = "23.12.1-alpine";

use anyhow::{bail, Result};
use clickhouse_rs::{ClientHandle, Options, Pool};
use futures::FutureExt;
use log::error;
use std::str::FromStr;
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

fn get_options(port: u16) -> Result<Options> {
    Ok(Options::from_str(&format!("tcp://127.0.0.1:{}", port))?
        .database("meteroid")
        .username("default")
        .password("default")
        .retry_timeout(Duration::from_secs(1))
        .ping_timeout(Duration::from_millis(100))
        .connection_timeout(Duration::from_millis(100))
        .send_retries(1))
}

pub(crate) async fn get_handle(port: u16) -> anyhow::Result<ClientHandle> {
    Pool::new(get_options(port)?)
        .get_handle()
        .await
        .map_err(|err| anyhow::anyhow!("Failed to connect to ClickHouse: {}", err))
}

pub(super) async fn test_status_endpoint(port: u16) -> Result<()> {
    let options = get_options(port)?;

    Pool::new(options)
        .get_handle()
        .catch_unwind()
        .await
        .map(drop)
        .map_err(|err| anyhow::anyhow!("Failed to connect to ClickHouse: {:?}", err))
}
