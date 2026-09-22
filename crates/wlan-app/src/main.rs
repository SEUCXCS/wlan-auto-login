mod cli;
mod config;
mod control;
mod credentials;
mod daemon;
mod login;
mod platform;
mod service;
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
        cli::Command::Daemon { background } => {
            let paths = config::Paths::new()?;
            let filter = tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into());
            if background {
                let log = service::background_log(&paths)?;
                #[cfg(windows)]
                unsafe {
                    windows_sys::Win32::System::Console::FreeConsole();
                }
                tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_ansi(false)
                    .with_writer(std::sync::Mutex::new(log))
                    .init();
            } else {
                tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_writer(std::io::stderr)
                    .init();
            }
            daemon::run(&client, &paths).await
        }
        cli::Command::Install(args) => {
            let paths = config::Paths::new()?;
            let saved = credentials::load().await?;
            if saved.is_none()
                || args.username.is_some()
                || args.password.is_some()
                || args.password_stdin
                || args.interactive
            {
                eprintln!("安装后台服务需要保存账号和密码；本次登录的 Remember me 已强制开启。");
                login::run(&client, &paths, args, true).await?;
            }
            service::install(&paths).await
        }
        cli::Command::Uninstall => service::uninstall(&config::Paths::new()?).await,
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
