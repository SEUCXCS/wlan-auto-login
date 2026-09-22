mod cli;
mod config;
mod control;
mod credentials;
mod daemon;
mod login;
mod platform;
mod transport;
mod tui;

use anyhow::Result;
use clap::Parser;
use cxxy_wlanlogin_lib::{Client, PortalConfig};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run(cli::Cli::parse()).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("错误：{error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run(args: cli::Cli) -> Result<()> {
    let client = Client::new(PortalConfig::default())?;
    match args.command {
        cli::Command::Daemon => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "info".into()),
                )
                .with_writer(std::io::stderr)
                .init();
            daemon::run(&client, &config::Paths::new()?).await
        }
        cli::Command::Login(args) => login::run(&client, &config::Paths::new()?, args, false).await,
        cli::Command::Logout => {
            let paths = config::Paths::new()?;
            let _operation = control::operation_lock(&paths).await?;
            let previous = control::State::load(&paths)?;
            let mut paused = previous.clone();
            paused.paused = true;
            paused.save(&paths)?;
            if let Err(error) = client.logout().await {
                previous.save(&paths)?;
                return Err(error.into());
            }
            control::wake(&paths).await;
            println!("已注销");
            Ok(())
        }
    }
}
