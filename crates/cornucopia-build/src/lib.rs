use std::env::VarError;
use std::process::Command;
use std::{env, fs};

use container::ContainerError;
use thiserror::Error;

mod container;

pub fn generate() -> Result<(), CornucopiaBuildError> {
    let sql_paths = get_refinery_migrations();

    if env::var("CI").is_err() && env::var("SKIP_CORNUCOPIA").is_err() {
        generate_cornucopia(sql_paths)?;
    }
    Ok(())
}

// cornucopia spawns a pg docker container, if no live instance is provided
fn generate_cornucopia(schema_files: Vec<String>) -> Result<(), CornucopiaBuildError> {
    let queries_path = "queries";
    let destination = "src/cornucopia.rs";

    let settings = cornucopia::CodegenSettings {
        is_async: true,
        derive_ser: false,
    };

    let package_name = std::env::var("CARGO_PKG_NAME")?;

    let container_settings = container::ContainerSettings::new(0, package_name);

    let max_retries = 3;
    let mut retries = 0;

    while retries < max_retries {
        match container::generate_managed(
            queries_path,
            schema_files.clone(),
            Some(destination),
            settings,
            &container_settings,
        ) {
            Ok(_) => {
                break;
            }
            Err(e) => {
                container::cleanup(&container_settings).ok();
                if let ContainerError::CornucopiaError(e) = e {
                    let error = e.to_string();
                    let report = e.report();
                    eprintln!("{}", report);

                    return Err(CornucopiaBuildError::CornucopiaGenerationError(error));
                } else {
                    if let ContainerError::ConnectionError(ref err) = e {
                        let msg = err.to_string();
                        if msg.contains("Connection reset by peer")
                            || msg.contains("the database system is starting up")
                        {
                            println!("Error happened while generating cornucopia.rs: {} , retrying... {}", msg, retries);
                            std::thread::sleep(std::time::Duration::from_millis(500));
                            retries += 1;

                            if retries > max_retries {
                                return Err(e.into());
                            }
                            continue;
                        }
                    }
                    println!("Error happened while generating cornucopia.rs: {:?}", e);
                }
                return Err(e.into());
            }
        }
    }

    format_cornucopia_file()
}

fn format_cornucopia_file() -> Result<(), CornucopiaBuildError> {
    // Call rustfmt to format the file
    let output = Command::new("rustfmt")
        .arg("--edition")
        .arg("2021")
        .arg("src/cornucopia.rs")
        .output()?;

    // Check if formatting succeeded
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CornucopiaBuildError::FormatError(stderr.to_string()));
    }

    Ok(())
}

fn get_refinery_migrations() -> Vec<String> {
    let mut migrations: Vec<String> = fs::read_dir("refinery/migrations")
        .unwrap()
        .flatten()
        .map(|x| x.path().to_string_lossy().to_string())
        .collect();

    migrations.sort();

    migrations
}

#[derive(Error, Debug)]
pub enum CornucopiaBuildError {
    #[error(transparent)]
    GenericError(#[from] std::io::Error),
    #[error(transparent)]
    EnvVarError(#[from] VarError),
    #[error("Failed to format cornucopia.rs: {0}")]
    FormatError(String),
    #[error(transparent)]
    ContainerError(#[from] container::ContainerError),
    #[error("Failed to generate sql client: {0}")]
    CornucopiaGenerationError(String),
}
