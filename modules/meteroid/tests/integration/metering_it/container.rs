use std::time::Duration;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;

use metering::config::Config;

use super::clickhouse;
use super::kafka::{CONTAINER_NAME, CONTAINER_VERSION};
use crate::helpers::network::free_local_port;

pub struct MeteringSetup {
    pub token: CancellationToken,
    pub join_handle: JoinHandle<()>,
    pub channel: Channel,
    pub _config: Config,
}

pub async fn start_metering(config: Config) -> MeteringSetup {
    let token = CancellationToken::new();
    let cloned_token = token.clone();

    let config_clone = config.clone();
    log::info!("Starting metering gRPC server {}", config.listen_addr);
    let private_server = metering::server::start_api_server(config_clone);

    let join_handle_meteroid = tokio::spawn(async move {
        tokio::select! {
            _ = private_server => {},
            _ = cloned_token.cancelled() => {
                log::info!("Interrupted metering server via token");
            }
        }
    });

    tokio::time::sleep(Duration::from_secs(1)).await;

    let metering_endpoint = format!("http://{}", config.listen_addr);

    log::info!("Creating metering gRPC channel {}", metering_endpoint);

    let channel = Channel::from_shared(metering_endpoint)
        .expect("Invalid metering_endpoint")
        .connect_lazy();

    MeteringSetup {
        token,
        join_handle: join_handle_meteroid,
        channel,
        _config: config.clone(),
    }
}

// TODO check if that replaces terminate_meteroid
// impl Drop for MeteroidSetup {
//     fn drop(&mut self) {
//         self.token.cancel();
//         // wait synchronously on join_handle
//         futures::executor::block_on(&self.join_handle).unwrap();
//         log::info!("Stopped meteroid server");
//     }
// }

pub async fn terminate_metering(token: CancellationToken, join_handle: JoinHandle<()>) {
    token.cancel();
    join_handle.await.unwrap();

    log::info!("Stopped meteroid server");
}

pub async fn start_clickhouse() -> (ContainerAsync<GenericImage>, u16) {
    let local = free_local_port().expect("Could not get free port");
    let internal_port = 8123;

    let container = GenericImage::new(clickhouse::CONTAINER_NAME, clickhouse::CONTAINER_VERSION)
        .with_exposed_port(local.tcp())
        .with_mapped_port(local, internal_port.tcp())
        .with_env_var("CLICKHOUSE_DB", "meteroid")
        .with_env_var("CLICKHOUSE_USER", "default")
        .with_env_var("CLICKHOUSE_PASSWORD", "default")
        .with_container_name("it_clickhouse")
        .with_network("meteroid_net")
        .start()
        .await
        .unwrap();

    let port = container.get_host_port_ipv4(internal_port).await.unwrap();

    clickhouse::wait_for_ok(port)
        .await
        .expect("Clickhouse not ready");

    log::info!("Started testcontainers Clickhouse :{}", port);

    (container, port)
}

pub async fn start_kafka() -> anyhow::Result<(ContainerAsync<GenericImage>, u16)> {
    let kafka_port = free_local_port().expect("Could not get free port");
    let args = [
        "redpanda",
        "start",
        "--overprovisioned",
        "--smp",
        "1",
        "--memory",
        "512M",
        "--reserve-memory=0M",
        "--node-id=1",
        "--check=false",
        "--kafka-addr=INTERNAL://0.0.0.0:29092,EXTERNAL://0.0.0.0:9092",
        format!(
            "--advertise-kafka-addr=INTERNAL://it_redpanda:29092,EXTERNAL://localhost:{kafka_port}"
        )
        .as_str(),
        "--set",
        "redpanda.disable_metrics=true",
        "--set",
        "redpanda.enable_admin_api=false",
        "--set",
        "redpanda.developer_mode=true",
    ]
    .map(String::from);

    let container = GenericImage::new(CONTAINER_NAME, CONTAINER_VERSION)
        .with_wait_for(WaitFor::message_on_stderr("Successfully started Redpanda!"))
        .with_mapped_port(kafka_port, 9092_u16.tcp())
        .with_container_name("it_redpanda")
        .with_network("meteroid_net")
        .with_cmd(args)
        .start()
        .await?;

    let port = container.get_host_port_ipv4(9092).await?;

    Ok((container, port))
}
