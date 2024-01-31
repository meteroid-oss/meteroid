use cornucopia::CodegenSettings;
use postgres::{error::ErrorPosition, Client, Config, NoTls};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use thiserror::Error;

pub struct ContainerSettings {
    port: u16,
    module: String,
}

impl ContainerSettings {
    pub fn new(port: u16, module: String) -> Self {
        ContainerSettings { port, module }
    }

    fn container_name(&self) -> String {
        format!("cornucopia_postgres_{}", self.module)
    }
}

pub fn generate_managed(
    queries_path: &str,
    schema_files: Vec<String>,
    destination: Option<&str>,
    codegen_settings: CodegenSettings,
    container_settings: &ContainerSettings,
) -> miette::Result<String, ContainerError> {
    cleanup(container_settings).ok();

    let port = setup(container_settings)?;
    let mut client = conn(port)?;

    load_schema(&mut client, schema_files)?;

    let generated =
        cornucopia::generate_live(&mut client, queries_path, destination, codegen_settings)?;

    cleanup(container_settings)?;
    Ok(generated)
}

/// Create a non-TLS connection to the managed container
fn conn(port: u16) -> Result<Client, ContainerError> {
    Ok(Config::new()
        .user("postgres")
        .password("postgres")
        .host("127.0.0.1")
        .port(port)
        .dbname("postgres")
        .connect(NoTls)?)
}

/// Starts Cornucopia's database container and wait until it reports healthy.
fn setup(container_settings: &ContainerSettings) -> Result<u16, ContainerError> {
    let port = spawn_container(container_settings)?;
    healthcheck(container_settings, 120, 50)?;
    Ok(port)
}

/// Stop and remove a container and its volume.
pub fn cleanup(container_settings: &ContainerSettings) -> Result<(), ContainerError> {
    stop_container(container_settings)?;
    remove_container(container_settings)?;
    Ok(())
}

fn get_random_available_port() -> Result<u16, ContainerError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| ContainerError::PortError(e.to_string()))?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| ContainerError::PortError(e.to_string()))?;
    Ok(local_addr.port())
}

/// Starts Cornucopia's database container.
fn spawn_container(container_settings: &ContainerSettings) -> Result<u16, ContainerError> {
    let mut port = container_settings.port;
    if port == 0 {
        port = get_random_available_port()?;
    }
    cmd(
        &[
            "run",
            "-d",
            "--name",
            &container_settings.container_name(),
            "-p",
            &format!("{}:5432", port),
            "-e",
            "POSTGRES_PASSWORD=postgres",
            "docker.io/library/postgres:latest",
        ],
        "spawn container",
    )?;

    Ok(port)
}

/// Checks if Cornucopia's container reports healthy
fn is_postgres_healthy(container_settings: &ContainerSettings) -> Result<bool, ContainerError> {
    Ok(cmd(
        &["exec", &container_settings.container_name(), "pg_isready"],
        "check container health",
    )
    .is_ok())
}

fn cmd(args: &[&str], action: &'static str) -> Result<(), ContainerError> {
    let command = "docker";
    let output = Command::new(command)
        .args(args)
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .output()
        .map_err(|e| ContainerError::CommandError(command.to_string(), action, e.to_string()))?;

    if output.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(ContainerError::CommandError(
            command.to_string(),
            action,
            err.to_string(),
        ))
    }
}

/// This function controls how the healthcheck retries are handled.
fn healthcheck(
    container_settings: &ContainerSettings,
    max_retries: u64,
    ms_per_retry: u64,
) -> Result<(), ContainerError> {
    let slow_threshold = 10 + max_retries / 10;
    let mut nb_retries = 0;
    while !is_postgres_healthy(container_settings)? {
        if nb_retries >= max_retries {
            return Err(ContainerError::MaxConnectionRetries);
        };
        std::thread::sleep(std::time::Duration::from_millis(ms_per_retry));
        nb_retries += 1;

        if nb_retries % slow_threshold == 0 {
            println!("Container startup slower than expected ({nb_retries} retries out of {max_retries})");
        }
    }
    // Just for extra safety...
    std::thread::sleep(std::time::Duration::from_millis(250));
    Ok(())
}

/// Stops Cornucopia's container.
fn stop_container(container_settings: &ContainerSettings) -> Result<(), ContainerError> {
    cmd(
        &["stop", &container_settings.container_name()],
        "stop container",
    )
}

/// Removes Cornucopia's container and its volume.
fn remove_container(container_settings: &ContainerSettings) -> Result<(), ContainerError> {
    cmd(
        &["rm", "-v", &container_settings.container_name()],
        "remove container",
    )
}

fn load_schema(client: &mut Client, paths: Vec<String>) -> Result<(), ContainerError> {
    for path in paths {
        let sql = std::fs::read_to_string(path.clone()).map_err(|err| ContainerError::Io {
            path: path.clone(),
            err,
        })?;

        client.batch_execute(&sql).map_err(|err| {
            let msg = format!("{err:#}");
            if let Some((_, msg, help)) = db_err(&err) {
                ContainerError::Postgres { msg, help }
            } else {
                ContainerError::Postgres { msg, help: None }
            }
        })?;
    }
    Ok(())
}

fn db_err(err: &postgres::Error) -> Option<(u32, String, Option<String>)> {
    if let Some(db_err) = err.as_db_error() {
        if let Some(ErrorPosition::Original(position)) = db_err.position() {
            Some((
                *position,
                db_err.message().to_string(),
                db_err.hint().map(String::from),
            ))
        } else {
            None
        }
    } else {
        None
    }
}

#[derive(Error, Debug)]
pub enum ContainerError {
    #[error("`{0}` couldn't {1}: {2}")]
    CommandError(String, &'static str, String),
    #[error("Cornucopia reached the max number of connection retries")]
    MaxConnectionRetries,
    #[error("Could not read schema `{path}`: ({err})")]
    Io { path: String, err: std::io::Error },
    #[error("Could not execute schema: {msg}")]
    Postgres { msg: String, help: Option<String> },
    #[error("Failed to get random port: {0}")]
    PortError(String),
    #[error("Couldn't establish a connection with the database.")]
    ConnectionError(#[from] postgres::Error),
    #[error(transparent)]
    CornucopiaError(#[from] cornucopia::Error),
}
