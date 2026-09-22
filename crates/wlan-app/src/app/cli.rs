use clap::{Args, Parser, Subcommand};
use cxxy_wlanlogin_lib::Line;

#[derive(Parser)]
#[command(version, about = "Student_CX 校园网登录工具")]
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
    /// 登录校园网；不在 Student_CX 时仅保存凭据，凭据不足时打开交互界面
    Login(LoginArgs),
    /// 注销当前校园网会话
    Logout,
    /// 直接运行后台登录循环，或管理已安装的系统服务
    #[command(subcommand_negates_reqs = true, args_conflicts_with_subcommands = true)]
    Daemon {
        #[command(subcommand)]
        command: Option<DaemonCommand>,
        #[arg(long, hide = true)]
        background: bool,
        #[arg(long, hide = true)]
        service: bool,
    },
    /// 安装自动启动服务；管理员默认安装系统服务，首次登录强制保存凭据
    Install(LoginArgs),
    /// 停止并卸载后台服务，保留已保存的账号密码
    Uninstall,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum DaemonCommand {
    /// 启动已安装的系统服务
    Start,
    /// 停止已安装的系统服务，保留安装和自动启动设置
    Stop,
    /// 重启已安装的系统服务
    Restart,
    /// 查看系统服务的安装、运行状态和已安装版本
    Status,
    /// 查看已安装服务的日志
    Logs(LogsArgs),
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
    pub credential_store: Option<crate::credentials::CredentialMode>,
    #[arg(short, long, env = "CXXY_WLAN_USERNAME", hide_env_values = true)]
    pub username: Option<String>,
    #[arg(
        short,
        long,
        env = "CXXY_WLAN_PASSWORD",
        hide_env_values = true,
        conflicts_with = "password_stdin"
    )]
    pub password: Option<String>,
    /// 从标准输入读取一行密码
    #[arg(long)]
    pub password_stdin: bool,
    #[arg(long, env = "CXXY_WLAN_LINE", value_name = "free1|free2|campus")]
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_flags_work_with_every_command() {
        for args in [
            vec!["app", "--system", "daemon", "status"],
            vec!["app", "daemon", "start", "--system"],
            vec!["app", "--user", "daemon"],
            vec!["app", "--system", "install"],
            vec!["app", "daemon", "logs", "--system"],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
        assert!(Cli::try_parse_from(["app", "--user", "--system", "install"]).is_err());
    }
    #[test]
    fn log_options_parse_without_starting_a_daemon() {
        for (arguments, expected) in [
            (
                vec!["app", "daemon", "logs"],
                LogsArgs {
                    lines: 100,
                    follow: false,
                },
            ),
            (
                vec!["app", "--system", "daemon", "logs", "-n", "0", "-f"],
                LogsArgs {
                    lines: 0,
                    follow: true,
                },
            ),
        ] {
            assert!(matches!(Cli::try_parse_from(arguments), Ok(Cli {
                command: Command::Daemon { command: Some(DaemonCommand::Logs(options)), .. }, ..
            }) if options == expected));
        }
        assert!(Cli::try_parse_from(["app", "daemon", "logs", "-n", "-1"]).is_err());
        assert!(Cli::try_parse_from(["app", "daemon", "--background", "logs"]).is_err());
    }

    #[test]
    fn daemon_without_subcommand_still_runs_the_process() {
        for arguments in [vec!["app", "daemon"], vec!["app", "daemon", "--background"]] {
            assert!(matches!(
                Cli::try_parse_from(arguments),
                Ok(Cli {
                    user: _,
                    system: _,
                    command: Command::Daemon { command: None, .. }
                })
            ));
        }
        assert!(matches!(
            Cli::try_parse_from(["app", "daemon", "status"]),
            Ok(Cli {
                user: _,
                system: _,
                command: Command::Daemon {
                    command: Some(DaemonCommand::Status),
                    background: false,
                    service: false
                }
            })
        ));
        assert!(Cli::try_parse_from(["app", "daemon", "--background", "status"]).is_err());
    }

    #[test]
    fn daemon_service_controls_are_subcommands() {
        for (argument, expected) in [
            ("start", DaemonCommand::Start),
            ("stop", DaemonCommand::Stop),
            ("restart", DaemonCommand::Restart),
        ] {
            let parsed = Cli::try_parse_from(["app", "daemon", argument]);
            assert!(matches!(parsed, Ok(Cli { user: _, system: _,
                command: Command::Daemon { command: Some(command), background: false, service: false }
            }) if command == expected));
            assert!(Cli::try_parse_from(["app", "daemon", "--background", argument]).is_err());
        }
    }

    #[test]
    fn explicit_tui_and_non_interactive_are_mutually_exclusive() {
        assert!(
            Cli::try_parse_from(["app", "login", "--interactive", "--non-interactive"]).is_err()
        );
        let parsed = Cli::try_parse_from(["app", "login", "--interactive", "--remember-me"]);
        assert!(matches!(
            parsed,
            Ok(Cli {
                user: _,
                system: _,
                command: Command::Login(LoginArgs {
                    interactive: true,
                    remember_me: true,
                    ..
                })
            })
        ));
    }
}
