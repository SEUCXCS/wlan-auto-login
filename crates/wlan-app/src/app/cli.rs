mod login;
mod output;
#[cfg(target_os = "macos")]
pub(super) mod permission;
use crate::system::define::credentials::CredentialMode;
use clap::{Args, Parser, Subcommand};
use wlanlogin_lib::Line;

impl clap::ValueEnum for CredentialMode {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Native, Self::ProtectedFile]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(match self {
            Self::Native => "native",
            Self::ProtectedFile => "protected-file",
        }))
    }
}

#[derive(Parser)]
#[command(name = crate::infra::identity::APP_NAME, version, about = "Student_CX 校园网登录工具")]
pub struct Cli {
    /// 使用当前用户的安装、配置和凭据
    #[arg(long, global = true, conflicts_with = "system")]
    pub user: bool,
    /// 使用系统安装、配置和凭据
    #[arg(long, global = true, conflicts_with = "user")]
    pub system: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    #[cfg(target_os = "macos")]
    #[command(hide = true)]
    MacosHost {
        #[arg(long)]
        system_helper: bool,
    },
    /// 登录校园网；不在 Student_CX 时仅保存凭据，凭据不足时打开交互界面
    Login(LoginArgs),
    /// 注销当前校园网会话
    Logout {
        /// 注销并删除所选范围的已保存凭据；离线或门户失败时仍删除本地凭据
        #[arg(long)]
        forget: bool,
    },
    /// 在当前进程运行自动登录循环
    Daemon {
        // Compatibility with already installed service definitions.
        #[arg(long, hide = true)]
        background: bool,
        #[arg(long, hide = true)]
        service: bool,
    },
    /// 管理用户或系统范围的后台服务
    #[command(subcommand)]
    Service(ServiceCommand),
    /// 查询或获取当前平台的权限
    #[cfg(target_os = "macos")]
    #[command(subcommand)]
    Permission(PermissionCommand),
}

#[derive(Subcommand)]
pub enum ServiceCommand {
    /// 安装并启动服务；首次登录强制保存凭据
    Install(LoginArgs),
    /// 停止并卸载服务，保留已保存的账号密码
    Uninstall {
        /// 同时清理所选范围的应用日志、配置、运行文件和凭据
        #[arg(long)]
        purge: bool,
    },
    /// 启动已安装的服务
    Start,
    /// 停止服务，保留安装和自动启动设置
    Stop,
    /// 重启已安装的服务
    Restart,
    /// 查看服务安装、注册、运行状态和已安装版本
    Status,
    /// 查看服务日志
    Logs(LogsArgs),
    #[command(hide = true)]
    Run,
    #[cfg(target_os = "macos")]
    #[command(hide = true, subcommand)]
    Native(NativeServiceCommand),
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Subcommand)]
pub enum NativeServiceCommand {
    Status,
    Register,
    Unregister,
}

#[cfg(target_os = "macos")]
#[derive(Subcommand)]
pub enum PermissionCommand {
    /// 列出当前平台支持的权限
    List {
        #[arg(long)]
        json: bool,
    },
    /// 查询权限状态，不弹出授权窗口；省略 ID 时查询全部
    Check {
        id: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 请求指定权限并报告结果；需要交互式终端
    Get {
        id: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Args, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogsArgs {
    /// 显示最近的日志行数；0 表示不显示历史日志
    #[arg(short = 'n', long, default_value_t = 100)]
    pub lines: u32,
    /// 持续查看新日志，按 Ctrl+C 退出
    #[arg(short, long)]
    pub follow: bool,
}

#[derive(Args, Default)]
pub struct LoginArgs {
    /// 系统凭据存储；Linux 旧系统可显式选择 protected-file
    #[arg(long, value_enum)]
    pub credential_store: Option<crate::system::credentials::CredentialMode>,
    #[arg(short, long, env = "WLANLOGIN_USERNAME", hide_env_values = true)]
    pub username: Option<String>,
    #[arg(
        short,
        long,
        env = "WLANLOGIN_PASSWORD",
        hide_env_values = true,
        conflicts_with = "password_stdin"
    )]
    pub password: Option<String>,
    /// 从标准输入读取一行密码
    #[arg(long)]
    pub password_stdin: bool,
    #[arg(long, env = "WLANLOGIN_LINE", value_name = "free1|free2|campus")]
    pub line: Option<Line>,
    /// 登录成功后保存账号和密码至所选范围的凭据存储
    #[arg(long)]
    pub remember_me: bool,
    /// 即使已有保存的凭据，也打开 TUI 以修改账号、密码或线路
    #[arg(long, alias = "tui", conflicts_with = "non_interactive")]
    pub interactive: bool,
    /// 缺少凭据时直接报错，不打开 TUI
    #[arg(long)]
    pub non_interactive: bool,
}

pub(super) async fn dispatch(
    args: Cli,
    context: &super::context::Context,
) -> crate::infra::error::Result<()> {
    use super::controller;
    let paths = &context.paths;
    if !matches!(
        args.command,
        Command::Service(ServiceCommand::Status | ServiceCommand::Logs(_))
    ) {
        paths.scope.require_access()?;
    }
    match args.command {
        Command::Login(args) => login::run(context, args, false).await,
        Command::Logout { forget } => {
            #[cfg(target_os = "macos")]
            {
                use std::io::IsTerminal;
                if let Err(error) =
                    crate::system::macos::host::prepare(std::io::stderr().is_terminal()).await
                {
                    tracing::warn!(%error, "Wi-Fi authorization unavailable during logout");
                }
            }
            controller::logout::run(&context.client, paths, forget).await
        }
        Command::Daemon {
            background,
            service,
        } => {
            crate::infra::error::ensure!(
                !service,
                crate::infra::error::Message::ServiceIsOnlySupportedByWindowsService
            );
            let managed =
                background && std::env::var_os("WLANLOGIN_MANAGED").is_some_and(|v| v == "1");
            controller::daemon::run(context, background, managed).await
        }
        Command::Service(command) => match command {
            ServiceCommand::Install(args) => login::run(context, args, true).await,
            ServiceCommand::Uninstall { purge } => {
                controller::service::uninstall::run(paths, purge).await
            }
            ServiceCommand::Start => controller::service::start::run(paths).await,
            ServiceCommand::Stop => controller::service::stop::run(paths).await,
            ServiceCommand::Restart => controller::service::restart::run(paths).await,
            ServiceCommand::Status => {
                let reports =
                    controller::service::status::run(paths, !args.user && !args.system).await?;
                println!("{}", output::status(&reports));
                controller::service::status::check(&reports)
            }
            ServiceCommand::Logs(options) => {
                controller::service::logs::run(
                    paths,
                    crate::system::define::services::LogOptions {
                        lines: options.lines,
                        follow: options.follow,
                    },
                    &mut std::io::stdout(),
                    tokio::signal::ctrl_c(),
                )
                .await
            }
            ServiceCommand::Run => unreachable!("service process entry is handled by bootstrap"),
            #[cfg(target_os = "macos")]
            ServiceCommand::Native(_) => {
                unreachable!("native service entry is handled by bootstrap")
            }
        },
        #[cfg(target_os = "macos")]
        Command::Permission(_) | Command::MacosHost { .. } => {
            unreachable!("platform entry is handled by bootstrap")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn service_commands_keep_scope_and_cleanup_options() {
        for scope in ["--user", "--system"] {
            for action in [
                "install",
                "uninstall",
                "start",
                "stop",
                "restart",
                "status",
                "logs",
                "run",
            ] {
                assert!(Cli::try_parse_from(["app", scope, "service", action]).is_ok());
                assert!(Cli::try_parse_from(["app", "service", action, scope]).is_ok());
            }
            assert!(matches!(
                Cli::try_parse_from(["app", scope, "service", "uninstall", "--purge"]),
                Ok(Cli {
                    command: Command::Service(ServiceCommand::Uninstall { purge: true }),
                    ..
                })
            ));
            assert!(matches!(
                Cli::try_parse_from(["app", "logout", "--forget", scope]),
                Ok(Cli {
                    command: Command::Logout { forget: true },
                    ..
                })
            ));
        }
        assert!(Cli::try_parse_from(["app", "--user", "--system", "service", "install"]).is_err());
        assert!(Cli::try_parse_from(["app", "service", "uninstall", "--forget"]).is_err());
        assert!(Cli::try_parse_from(["app", "logout", "--purge"]).is_err());
    }

    #[test]
    fn daemon_runs_only_the_loop_and_service_controls_are_separate() {
        for args in [
            vec!["app", "daemon"],
            vec!["app", "daemon", "--background"],
            vec!["app", "daemon", "--service"],
        ] {
            assert!(matches!(
                Cli::try_parse_from(args),
                Ok(Cli {
                    command: Command::Daemon { .. },
                    ..
                })
            ));
        }
        for action in ["start", "stop", "restart", "status", "logs"] {
            assert!(Cli::try_parse_from(["app", "daemon", action]).is_err());
        }
        for old in [
            "install",
            "uninstall",
            "macos-service",
            "macos-credential-check",
        ] {
            assert!(Cli::try_parse_from(["app", old]).is_err());
        }
    }

    #[test]
    fn logs_parse_without_starting_a_service() {
        assert!(matches!(
            Cli::try_parse_from(["app", "--system", "service", "logs", "-n", "0", "-f"]),
            Ok(Cli {
                command: Command::Service(ServiceCommand::Logs(LogsArgs {
                    lines: 0,
                    follow: true
                })),
                ..
            })
        ));
        assert!(Cli::try_parse_from(["app", "service", "logs", "-n", "-1"]).is_err());
        assert!(Cli::try_parse_from(["app", "service", "--background", "logs"]).is_err());
    }

    #[test]
    fn public_help_exposes_service_and_hides_process_entries() {
        let mut command = Cli::command();
        let help = command.render_help().to_string();
        assert!(help.contains("service"));
        assert!(!help.contains("macos-host"));
        let service = command.find_subcommand_mut("service");
        assert!(service.is_some());
        if let Some(service) = service {
            let help = service.render_help().to_string();
            assert!(help.contains("install") && help.contains("status"));
            assert!(!help.contains("native") && !help.contains("  run"));
        }
        #[cfg(target_os = "macos")]
        assert!(help.contains("permission"));
        #[cfg(not(target_os = "macos"))]
        {
            assert!(!help.contains("permission"));
            assert!(Cli::try_parse_from(["app", "permission", "list"]).is_err());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn permission_and_native_service_arguments_are_explicit() {
        for args in [
            vec!["app", "permission", "list"],
            vec!["app", "permission", "check"],
            vec![
                "app",
                "--system",
                "permission",
                "check",
                "macos.keychain",
                "--json",
            ],
            vec!["app", "permission", "get", "macos.location"],
            vec!["app", "service", "native", "status"],
            vec!["app", "--user", "service", "native", "register"],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
        assert!(Cli::try_parse_from(["app", "permission", "get"]).is_err());
    }

    #[test]
    fn explicit_tui_and_non_interactive_are_mutually_exclusive() {
        for command in [vec!["login"], vec!["service", "install"]] {
            let mut args = vec!["app"];
            args.extend(command);
            args.extend(["--interactive", "--non-interactive"]);
            assert!(Cli::try_parse_from(args).is_err());
        }
    }
}
