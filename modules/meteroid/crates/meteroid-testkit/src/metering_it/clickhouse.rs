// APACHE 2.0 license - Copyright 2021, The Tremor Team
// Adapted from https://github.com/tremor-rs/tremor-runtime/blob/main/src/connectors/tests/clickhouse/utils.rs

use backon::Retryable;
use std::ops::Deref;
pub const CONTAINER_NAME: &str = "clickhouse/clickhouse-server";
pub const CONTAINER_VERSION: &str = "25.6.2-alpine";

use clickhouse::Client;
use std::time::Duration;

/// Waits for the server and its embedded Keeper, then creates the `meteroid` database. Created
/// here rather than through the image's `CLICKHOUSE_DB`, which starts and stops the server once
/// more before the real start (about 5 s).
pub(super) async fn wait_for_ok(port: HttpPort) -> anyhow::Result<()> {
    let poll = backon::ConstantBuilder::default()
        .with_delay(Duration::from_millis(100))
        .with_max_times(600);
    (|| async { test_status_endpoint(port).await })
        .retry(poll)
        .notify(|err: &anyhow::Error, dur: Duration| {
            log::debug!(
                "Retrying connecting to ClickHouse container after {:?}, error: {}",
                dur,
                err
            );
        })
        .await?;

    // Wait for embedded Keeper to be ready (needed for ReplicatedMergeTree ON CLUSTER).
    // ClickHouse HTTP may respond before Keeper has elected a leader.
    (|| async { test_keeper_endpoint(port).await })
        .retry(poll.with_max_times(300))
        .notify(|err: &anyhow::Error, dur: Duration| {
            log::debug!(
                "Retrying ClickHouse Keeper readiness after {:?}, error: {}",
                dur,
                err
            );
        })
        .await?;

    server_client(port)
        .query("CREATE DATABASE IF NOT EXISTS meteroid")
        .execute()
        .await?;
    Ok(())
}

async fn test_keeper_endpoint(port: HttpPort) -> anyhow::Result<()> {
    let client = server_client(port);
    // Fails until Keeper has elected a leader and ZooKeeper path access works.
    client
        .query("SELECT name FROM system.zookeeper WHERE path = '/'")
        .fetch_all::<String>()
        .await?;
    Ok(())
}

pub fn get_client(port: HttpPort) -> Client {
    server_client(port).with_database("meteroid")
}

/// A client of the server's `default` database, which exists before `meteroid` is created.
fn server_client(port: HttpPort) -> Client {
    Client::default()
        .with_url(format!("http://localhost:{port}"))
        .with_user("default")
        .with_password("default")
}

pub(super) async fn test_status_endpoint(port: HttpPort) -> anyhow::Result<()> {
    let client = server_client(port);

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
