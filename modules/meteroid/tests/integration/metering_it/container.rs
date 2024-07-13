use std::time::Duration;

use testcontainers::clients::Cli;
use testcontainers::core::{Port, WaitFor};
use testcontainers::{Container, GenericImage, RunnableImage};

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;

use metering::config::Config;

use super::clickhouse;
use super::kafka::{CONTAINER_NAME, CONTAINER_VERSION};
use crate::helpers;
use crate::helpers::network::free_local_port;

pub struct MeteringSetup {
    pub token: CancellationToken,
    pub join_handle: JoinHandle<()>,
    pub channel: Channel,
    pub config: Config,
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
        config: config.clone(),
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

pub async fn start_clickhouse<'a>(docker: &'a Cli) -> (Container<'a, GenericImage>, u16) {
    let image = GenericImage::new(clickhouse::CONTAINER_NAME, clickhouse::CONTAINER_VERSION);

    let local = helpers::network::free_local_port().expect("Could not get free port");

    let port_to_expose = Port {
        internal: 9000,
        local,
    };
    let image = RunnableImage::from(image)
        .with_mapped_port(port_to_expose)
        .with_env_var(("CLICKHOUSE_DB", "meteroid"))
        .with_env_var(("CLICKHOUSE_USER", "default"))
        .with_env_var(("CLICKHOUSE_PASSWORD", "default"))
        .with_container_name("it_clickhouse")
        .with_network("meteroid_net");

    let container = docker.run(image);
    let port = container.get_host_port_ipv4(9000);

    clickhouse::wait_for_ok(port)
        .await
        .expect("Clickhouse not ready");

    log::info!("Started testcontainers Clickhouse :{}", port);

    (container, port)
}

pub async fn start_kafka<'a>(
    docker: &'a Cli,
) -> anyhow::Result<(Container<'a, GenericImage>, u16)> {
    let kafka_port = free_local_port().expect("Could not get free port");
    let args = vec![
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
        &format!(
            "--advertise-kafka-addr=INTERNAL://it_redpanda:29092,EXTERNAL://localhost:{kafka_port}"
        ),
        "--set",
        "redpanda.disable_metrics=true",
        "--set",
        "redpanda.enable_admin_api=false",
        "--set",
        "redpanda.developer_mode=true",
    ]
    .into_iter()
    .map(ToString::to_string)
    .collect();
    let image = GenericImage::new(CONTAINER_NAME, CONTAINER_VERSION).with_wait_for(
        WaitFor::StdErrMessage {
            message: "Successfully started Redpanda!".to_string(),
        },
    );
    let image = RunnableImage::from((image, args))
        .with_mapped_port((kafka_port, 9092_u16))
        .with_container_name("it_redpanda")
        .with_network("meteroid_net");

    let container = docker.run(image);
    let port = container.get_host_port_ipv4(9092);

    Ok((container, port))
}
