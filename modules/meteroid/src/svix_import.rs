//! `meteroid-svix-import`: carries the webhook endpoints of a pre-2.0 instance over from its
//! Svix server. Run once, after the 2.0 API has migrated the database; safe to re-run.

use std::process::ExitCode;
use std::str::FromStr;

use envconfig::Envconfig;
use secrecy::SecretString;

use crate::api_rest::webhooks::out_model::WebhookOutEventTypeEnum;
use crate::clients::usage::MeteringUsageClient;
use crate::config::Config;
use crate::migrations;
use crate::module::Modules;
use crate::process::build_store;
use crate::services::egress::EgressPurpose;
use crate::services::svix_import::{self, client::SvixClient};

const USAGE: &str = "\
Usage: meteroid-svix-import [--apply]

Imports the webhook endpoints of the Svix server Meteroid used before 2.0.
Without --apply, prints what would be imported and writes nothing.

Reads the API's configuration, plus SVIX_SERVER_URL and SVIX_JWT_TOKEN.";

#[derive(Envconfig)]
struct SvixSettings {
    #[envconfig(from = "SVIX_SERVER_URL")]
    server_url: Option<String>,
    #[envconfig(from = "SVIX_JWT_TOKEN")]
    token: Option<SecretString>,
}

enum Command {
    DryRun,
    Apply,
    Help,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Command, String> {
    let mut command = Command::DryRun;
    for arg in args {
        match arg.as_str() {
            "--apply" => command = Command::Apply,
            "-h" | "--help" => return Ok(Command::Help),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(command)
}

pub async fn run(edition: impl FnOnce() -> Modules) -> ExitCode {
    let apply = match parse_args(std::env::args().skip(1)) {
        Ok(Command::DryRun) => false,
        Ok(Command::Apply) => true,
        Ok(Command::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match import(edition, apply).await {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn import(edition: impl FnOnce() -> Modules, apply: bool) -> Result<ExitCode, String> {
    crate::module::load_dotenv().map_err(|e| format!("cannot read the .env file: {e}"))?;

    let svix = SvixSettings::init_from_env().map_err(|e| e.to_string())?;
    let (Some(server_url), Some(token)) = (
        svix.server_url.filter(|url| !url.trim().is_empty()),
        svix.token,
    ) else {
        return Err(
            "set SVIX_SERVER_URL and SVIX_JWT_TOKEN to the Svix server and token the instance used before 2.0"
                .to_string(),
        );
    };
    let svix = SvixClient::new(&server_url, token).map_err(|e| e.to_string())?;

    // The API's own store and egress settings: secrets are encrypted with its key, and an
    // endpoint it would refuse to send to is imported disabled.
    let config = Config::init_from_env().map_err(|e| format!("invalid configuration: {e}"))?;
    let modules = edition();
    let egress = crate::process::egress(&config.webhook_out);
    let webhook_egress = egress
        .client(EgressPurpose::Webhooks)
        .map_err(|e| format!("cannot build the webhook egress client: {e}"))?;
    let usage_client = MeteringUsageClient::connect(&config);
    let store = build_store(&modules, &config, &usage_client, &egress);

    let pending = migrations::pending(&store.pool)
        .await
        .map_err(|e| format!("cannot read the database's migrations: {e}"))?;
    if let Some(first) = pending.first() {
        return Err(format!(
            "the database is not migrated to this version ({} migration(s) pending, from {first}): start meteroid-api once, then run this again",
            pending.len()
        ));
    }

    let is_known_event_type =
        |event_type: &str| WebhookOutEventTypeEnum::from_str(event_type).is_ok();
    let report = svix_import::import(&store, &svix, &webhook_egress, &is_known_event_type, apply)
        .await
        .map_err(|e| e.to_string())?;

    print!("{report}");

    Ok(if report.failures() == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
