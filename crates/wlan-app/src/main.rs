mod cli;
mod config;
mod credentials;
mod login;
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
        cli::Command::Login(args) => login::run(&client, &config::Paths::new()?, args, false).await,
        cli::Command::Logout => {
            client.logout().await?;
            println!("已注销");
            Ok(())
        }
    }
}
