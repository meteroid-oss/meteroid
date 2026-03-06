use backon::{ConstantBuilder, Retryable};
use std::time::Duration;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt, TestcontainersError};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;

use metering::config::Config;

use super::clickhouse;
use super::kafka::{CONTAINER_NAME, CONTAINER_VERSION};
use crate::helpers::network::free_local_port;
use crate::metering_it::clickhouse::{HttpPort, TcpPort};

pub struct MeteringSetup {
    pub token: CancellationToken,
    pub join_handle: JoinHandle<()>,
    pub channel: Channel,
    pub _config: Config,
}

pub async fn start_metering(config: Config) -> MeteringSetup {
    let token = CancellationToken::new();
    let cloned_token = token.clone();

    log::info!("Starting metering gRPC server {}", config.listen_addr);
    let private_server = metering::server::start_server(config.clone());

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

impl Drop for MeteringSetup {
    fn drop(&mut self) {
        self.token.cancel();
        self.join_handle.abort();
        log::info!("Stopped metering server  ");
    }
}

pub async fn start_clickhouse() -> (ContainerAsync<GenericImage>, HttpPort, TcpPort) {
    let internal_http_port = 8123;
    let internal_tcp_port = 9000;
    let suffix = uuid::Uuid::now_v7();

    let container = (|| async {
        let local_http = free_local_port().expect("Could not get free http port");
        let local_tcp = free_local_port().expect("Could not get free tcp port");

        GenericImage::new(clickhouse::CONTAINER_NAME, clickhouse::CONTAINER_VERSION)
            .with_exposed_port(local_http.tcp())
            .with_exposed_port(local_tcp.tcp())
            .with_mapped_port(local_http, internal_http_port.tcp())
            .with_mapped_port(local_tcp, internal_tcp_port.tcp())
            .with_env_var("CLICKHOUSE_DB", "meteroid")
            .with_env_var("CLICKHOUSE_USER", "default")
            .with_env_var("CLICKHOUSE_PASSWORD", "default")
            .with_container_name(format!("it_clickhouse_{suffix}"))
            .with_network(format!("meteroid_net_{suffix}"))
            .start()
            .await
    })
    .retry(
        ConstantBuilder::default()
            .with_delay(Duration::from_secs(1))
            .with_max_times(3),
    )
    .notify(|err: &TestcontainersError, dur: Duration| {
        log::warn!(
            "Retrying clickhouse container start after {:?}: {:?}",
            dur,
            err
        );
    })
    .await
    .unwrap();

    let http_port = HttpPort(
        container
            .get_host_port_ipv4(internal_http_port)
            .await
            .unwrap(),
    );

    clickhouse::wait_for_ok(http_port)
        .await
        .expect("Clickhouse not ready");

    log::info!("Started testcontainers Clickhouse :{}", http_port.0);

    let tcp_port = TcpPort(
        container
            .get_host_port_ipv4(internal_tcp_port)
            .await
            .unwrap(),
    );

    (container, http_port, tcp_port)
}

pub struct KafkaSetup {
    pub container: ContainerAsync<GenericImage>,
    pub port: u16,
    pub internal_addr: String,
}

pub async fn start_kafka() -> anyhow::Result<KafkaSetup> {
    let suffix = uuid::Uuid::now_v7();
    let container_name = format!("it_redpanda_{suffix}");
    let network_name = format!("meteroid_net_{suffix}");
    let internal_addr = format!("{container_name}:29092");

    let container = (|| async {
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
                "--advertise-kafka-addr=INTERNAL://{container_name}:29092,EXTERNAL://localhost:{kafka_port}"
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

        GenericImage::new(CONTAINER_NAME, CONTAINER_VERSION)
            .with_wait_for(WaitFor::message_on_stderr("Successfully started Redpanda!"))
            .with_mapped_port(kafka_port, 9092_u16.tcp())
            .with_container_name(container_name.clone())
            .with_network(network_name.clone())
            .with_cmd(args)
            .start()
            .await
    })
    .retry(ConstantBuilder::default().with_delay(Duration::from_secs(1)).with_max_times(3))
    .notify(|err: &TestcontainersError, dur: Duration| {
        log::warn!("Retrying redpanda container start after {:?}: {:?}", dur, err);
    })
    .await?;

    let port = container.get_host_port_ipv4(9092).await?;

    Ok(KafkaSetup {
        container,
        port,
        internal_addr,
    })
}
