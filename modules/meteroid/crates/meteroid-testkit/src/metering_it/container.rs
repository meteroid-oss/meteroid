use std::time::Duration;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;
use tonic::transport::server::TcpIncoming;
use tonic_health::pb::HealthCheckRequest;
use tonic_health::pb::health_check_response::ServingStatus;
use tonic_health::pb::health_client::HealthClient;

use metering::config::Config;
use metering_server::server::ServerMode;

use super::clickhouse;
use super::kafka::{CONTAINER_NAME, CONTAINER_VERSION};
use crate::helpers::network::free_local_port;
use crate::metering_it::clickhouse::{HttpPort, TcpPort};
use crate::meteroid_it::container::is_transient_start_error;

pub struct MeteringSetup {
    pub token: CancellationToken,
    pub join_handle: JoinHandle<()>,
    pub channel: Channel,
    pub _config: Config,
}

/// Held from the moment its port is picked, so no other listener can take it before the server
/// serves: the port is cross-wired into configs first, then passed to [`start_metering`].
pub async fn bind_metering_grpc() -> tokio::net::TcpListener {
    tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Could not bind the metering gRPC listener")
}

pub async fn start_metering(config: Config, listener: tokio::net::TcpListener) -> MeteringSetup {
    assert_eq!(
        listener.local_addr().expect("the listener's address"),
        config.listen_addr,
        "the metering server serves on the listener of its config"
    );
    let token = CancellationToken::new();
    let cloned_token = token.clone();

    log::info!("Starting metering gRPC server {}", config.listen_addr);
    let private_server = metering_server::server::start_server_on(
        config.clone(),
        ServerMode::Combined,
        TcpIncoming::from(listener),
    );

    let join_handle_meteroid = tokio::spawn(async move {
        tokio::select! {
            result = private_server => {
                if let Err(e) = result {
                    log::error!("Metering server stopped: {e}");
                }
            },
            _ = cloned_token.cancelled() => {
                log::info!("Interrupted metering server via token");
            }
        }
    });

    let metering_endpoint = format!("http://{}", config.listen_addr);

    log::info!("Creating metering gRPC channel {}", metering_endpoint);

    let channel = Channel::from_shared(metering_endpoint)
        .expect("Invalid metering_endpoint")
        .connect_lazy();

    // The health check waits in the listener's backlog until the services are served, after
    // the migrations; a server that fails to start drops the listener and fails it.
    let health = tokio::time::timeout(
        Duration::from_secs(60),
        HealthClient::new(channel.clone()).check(HealthCheckRequest::default()),
    )
    .await
    .unwrap_or_else(|_| panic!("Metering server did not serve within 60 seconds"))
    .unwrap_or_else(|error| panic!("Metering server did not start: {error}"))
    .into_inner();
    assert_eq!(health.status(), ServingStatus::Serving);

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
    <enable_ipv6>false</enable_ipv6>
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

/// Grants the `default` user `named_collection_control` so the analytics V11
/// migration's `CREATE NAMED COLLECTION` is permitted (a per-user flag that must
/// live in users.d, mirroring deploy/dev and the managed Altinity cluster).
const CLICKHOUSE_NAMED_COLLECTION_USERS: &str = r#"<clickhouse>
  <users>
    <default>
      <named_collection_control>1</named_collection_control>
    </default>
  </users>
</clickhouse>"#;

/// Suffixed so tests running in parallel, and concurrent checkouts (`IT_CONTAINER_PREFIX`), never
/// collide on a container name.
fn container_name(base: &str, suffix: impl std::fmt::Display) -> String {
    format!(
        "{}{base}-{suffix}",
        std::env::var("IT_CONTAINER_PREFIX").unwrap_or_default()
    )
}

pub fn redpanda_name(kafka_port: u16) -> String {
    container_name("it_redpanda", kafka_port)
}

const NETWORK: &str = "meteroid_net";

/// Created here rather than by testcontainers, whose exists-then-create races across parallel test
/// processes; an existing network is also left alone by testcontainers, never removed while in use.
async fn ensure_network() -> anyhow::Result<()> {
    let created = bollard::Docker::connect_with_defaults()?
        .create_network(bollard::models::NetworkCreateRequest {
            name: NETWORK.to_string(),
            ..Default::default()
        })
        .await;
    match created {
        Ok(_)
        | Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 409, ..
        }) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Host ports tried before giving up on a container whose ports keep being taken.
const PORT_ATTEMPTS: usize = 5;

/// Starts a container on host ports picked before Docker binds them: a container keeps them
/// across a restart, and a broker advertises its own. Another process taking one in between
/// fails the start, which is retried on the next ports, removing what Docker left created; so is
/// a transient failure such as an aborted image pull.
async fn start_on_ports<const N: usize, Fut>(
    ports: impl IntoIterator<Item = [u16; N]>,
    start: impl Fn([u16; N]) -> (String, Fut),
) -> anyhow::Result<(ContainerAsync<GenericImage>, [u16; N])>
where
    Fut: std::future::Future<
            Output = Result<ContainerAsync<GenericImage>, testcontainers::TestcontainersError>,
        >,
{
    ensure_network().await?;
    let mut last = None;
    for picked in ports.into_iter().take(PORT_ATTEMPTS) {
        let (name, started) = start(picked);
        match started.await {
            Err(error) if port_taken(&error) => {
                log::warn!("host ports {picked:?} of {name} were taken before Docker bound them");
                remove_unstarted(&name).await;
                last = Some(error);
            }
            Err(error) if is_transient_start_error(&error) => {
                log::warn!("{name} failed to start on a transient error, retrying: {error}");
                remove_unstarted(&name).await;
                last = Some(error);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            started => return Ok((started?, picked)),
        }
    }
    Err(anyhow::anyhow!(
        "no host ports could be bound in {PORT_ATTEMPTS} attempts: {last:?}"
    ))
}

fn free_ports<const N: usize>() -> impl Iterator<Item = [u16; N]> {
    std::iter::repeat_with(|| {
        std::array::from_fn(|_| free_local_port().expect("Could not get free port"))
    })
}

/// Kafka and ClickHouse, started side by side.
pub async fn start_kafka_and_clickhouse() -> (
    (ContainerAsync<GenericImage>, u16),
    (ContainerAsync<GenericImage>, HttpPort, TcpPort),
) {
    let (kafka, clickhouse) = tokio::join!(start_kafka(), start_clickhouse());
    (kafka.expect("Could not start kafka"), clickhouse)
}

pub async fn start_clickhouse() -> (ContainerAsync<GenericImage>, HttpPort, TcpPort) {
    let internal_http_port = 8123;
    let internal_tcp_port = 9000;

    let (container, [local_http, local_tcp]) = start_on_ports(free_ports(), |[http, tcp]| {
        let name = container_name("it_clickhouse", http);
        let started = GenericImage::new(clickhouse::CONTAINER_NAME, clickhouse::CONTAINER_VERSION)
            .with_exposed_port(http.tcp())
            .with_exposed_port(tcp.tcp())
            .with_mapped_port(http, internal_http_port.tcp())
            .with_mapped_port(tcp, internal_tcp_port.tcp())
            .with_env_var("CLICKHOUSE_USER", "default")
            .with_env_var("CLICKHOUSE_PASSWORD", "default")
            .with_copy_to(
                "/etc/clickhouse-server/config.d/cluster.xml",
                CLICKHOUSE_CLUSTER_CONFIG.as_bytes().to_vec(),
            )
            .with_copy_to(
                "/etc/clickhouse-server/users.d/named_collection_control.xml",
                CLICKHOUSE_NAMED_COLLECTION_USERS.as_bytes().to_vec(),
            )
            .with_container_name(name.clone())
            .with_network(NETWORK)
            .start();
        (name, started)
    })
    .await
    .expect("Could not start ClickHouse");

    let http_port = HttpPort(local_http);
    clickhouse::wait_for_ok(http_port)
        .await
        .expect("Clickhouse not ready");

    log::info!("Started testcontainers Clickhouse :{}", http_port.0);

    (container, http_port, TcpPort(local_tcp))
}

pub async fn start_kafka() -> anyhow::Result<(ContainerAsync<GenericImage>, u16)> {
    start_kafka_on(free_ports::<1>().map(|[port]| port)).await
}

/// Starts the broker on the first of `ports` Docker can bind.
pub async fn start_kafka_on(
    ports: impl IntoIterator<Item = u16>,
) -> anyhow::Result<(ContainerAsync<GenericImage>, u16)> {
    let (container, [port]) = start_on_ports(ports.into_iter().map(|port| [port]), |[port]| {
        (redpanda_name(port), start_kafka_at(port))
    })
    .await?;
    Ok((container, port))
}

fn port_taken(error: &testcontainers::TestcontainersError) -> bool {
    let message = format!("{error:?}");
    message.contains("port is already allocated") || message.contains("address already in use")
}

/// A container whose start failed is left created by Docker: removed so it never lingers.
async fn remove_unstarted(name: &str) {
    let removed = async {
        bollard::Docker::connect_with_defaults()?
            .remove_container(
                name,
                Some(bollard::query_parameters::RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
    };
    if let Err(error) = removed.await {
        log::warn!("Could not remove the unstarted container {name}: {error}");
    }
}

async fn start_kafka_at(
    kafka_port: u16,
) -> Result<ContainerAsync<GenericImage>, testcontainers::TestcontainersError> {
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
        // Seastar reserves 10000 networking AIO slots per broker by default: CI's
        // fs.aio-max-nr of 65536 then fits only ~5 brokers across parallel tests.
        "--max-networking-io-control-blocks=1000",
        "--kafka-addr=INTERNAL://0.0.0.0:29092,EXTERNAL://0.0.0.0:9092",
        format!(
            "--advertise-kafka-addr=INTERNAL://{}:29092,EXTERNAL://localhost:{kafka_port}",
            redpanda_name(kafka_port)
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
        .with_container_name(redpanda_name(kafka_port))
        .with_network(NETWORK)
        .with_cmd(args)
        .start()
        .await
}
