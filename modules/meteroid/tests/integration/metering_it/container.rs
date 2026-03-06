use backon::Retryable;
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

    // Wait until the server is actually accepting TCP connections rather than sleeping a
    // fixed duration. Migrations (including ON CLUSTER ReplicatedMergeTree) can take several
    // seconds, and the gRPC port is only bound after they complete.
    let addr = config.listen_addr;
    (|| async {
        tokio::net::TcpStream::connect(addr)
            .await
            .map_err(anyhow::Error::from)
    })
    .retry(
        backon::ConstantBuilder::default()
            .with_delay(Duration::from_millis(500))
            .with_max_times(120),
    )
    .notify(|_err: &anyhow::Error, _dur: Duration| {
        log::info!("Waiting for metering server to start on {addr}...");
    })
    .await
    .unwrap_or_else(|_| panic!("Metering server did not start within 60 seconds on {addr}"));

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

/// Minimal ClickHouse cluster config for integration tests.
/// Enables embedded Keeper, a single-node `meteroid` cluster
const CLICKHOUSE_CLUSTER_CONFIG: &str = r#"<clickhouse>
  <keeper_server>
    <tcp_port>9181</tcp_port>
    <server_id>1</server_id>
    <log_storage_path>/var/lib/clickhouse/coordination/log</log_storage_path>
    <snapshot_storage_path>/var/lib/clickhouse/coordination/snapshots</snapshot_storage_path>
    <coordination_settings>
      <operation_timeout_ms>10000</operation_timeout_ms>
      <session_timeout_ms>30000</session_timeout_ms>
      <raft_logs_level>warning</raft_logs_level>
    </coordination_settings>
    <raft_configuration>
      <server>
        <id>1</id>
        <hostname>localhost</hostname>
        <port>9234</port>
      </server>
    </raft_configuration>
  </keeper_server>
  <zookeeper>
    <node>
      <host>localhost</host>
      <port>9181</port>
    </node>
  </zookeeper>
  <remote_servers>
    <meteroid>
      <shard>
        <replica>
          <host>localhost</host>
          <port>9000</port>
        </replica>
      </shard>
    </meteroid>
  </remote_servers>
  <macros>
    <cluster>meteroid</cluster>
    <shard>1</shard>
    <replica>replica-1</replica>
  </macros>
</clickhouse>"#;

pub async fn start_clickhouse() -> (ContainerAsync<GenericImage>, HttpPort, TcpPort) {
    let local_http = free_local_port().expect("Could not get free http port");
    let internal_http_port = 8123;

    let local_tcp = free_local_port().expect("Could not get free tcp port");
    let internal_tcp_port = 9000;

    let container = GenericImage::new(clickhouse::CONTAINER_NAME, clickhouse::CONTAINER_VERSION)
        .with_exposed_port(local_http.tcp())
        .with_exposed_port(local_tcp.tcp())
        .with_mapped_port(local_http, internal_http_port.tcp())
        .with_mapped_port(local_tcp, internal_tcp_port.tcp())
        .with_env_var("CLICKHOUSE_DB", "meteroid")
        .with_env_var("CLICKHOUSE_USER", "default")
        .with_env_var("CLICKHOUSE_PASSWORD", "default")
        .with_copy_to(
            "/etc/clickhouse-server/config.d/cluster.xml",
            CLICKHOUSE_CLUSTER_CONFIG.as_bytes().to_vec(),
        )
        .with_container_name("it_clickhouse")
        .with_network("meteroid_net")
        .start()
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
